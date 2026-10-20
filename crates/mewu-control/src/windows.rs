// SPDX-License-Identifier: MPL-2.0
use super::denied;
use interprocess::{
    local_socket::{GenericNamespaced, ListenerOptions, Name, Stream, ToNsName},
    os::windows::{local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor},
};
use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{LocalFree, HANDLE},
    Security::{
        Authorization::ConvertSidToStringSidW, GetTokenInformation, TokenLogonSid, TOKEN_GROUPS,
        TOKEN_QUERY,
    },
    System::Threading::{
        GetCurrentProcess, GetCurrentThread, OpenProcess, OpenProcessToken, OpenThreadToken,
        QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    },
};

#[derive(Clone, Debug)]
pub(super) struct Endpoint {
    name: String,
    logon_sid: String,
}
pub(crate) struct Guard;
impl Endpoint {
    pub(super) fn new(namespace: &str) -> io::Result<Self> {
        let logon_sid = current_logon_sid()?;
        Ok(Self {
            name: format!("mewu-control-{logon_sid}-{namespace}"),
            logon_sid,
        })
    }
    pub(super) fn name(&self) -> io::Result<Name<'_>> {
        self.name.as_str().to_ns_name::<GenericNamespaced>()
    }
    pub(super) fn prepare(&self) -> io::Result<Guard> {
        Ok(Guard)
    }
    pub(super) fn options<'a>(
        &self,
        options: ListenerOptions<'a>,
    ) -> io::Result<ListenerOptions<'a>> {
        // Explicit protected DACL: only this interactive logon can connect or
        // create another pipe instance. Same-logon processes are trusted clients.
        // Interprocess's real CreateNamedPipeW path always sets FIRST_PIPE_INSTANCE
        // on the first instance and rejects remote clients by default.
        let sddl = widestring::U16CString::from_str(format!("D:P(A;;GA;;;{})", self.logon_sid))
            .map_err(|_| denied())?;
        Ok(options.security_descriptor(SecurityDescriptor::deserialize(&sddl)?))
    }
    pub(super) fn after_bind(&self) -> io::Result<()> {
        Ok(())
    }
    pub(super) fn verify_peer(&self, stream: &Stream, server: bool) -> io::Result<()> {
        let Stream::NamedPipe(stream) = stream;
        let actual = if server {
            // Must run after reading a frame: the OS impersonates the identity
            // associated with the last bytes read, without a PID lookup race.
            let _impersonation = stream.inner().impersonate_client()?;
            let mut token: HANDLE = ptr::null_mut();
            if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let token = unsafe { OwnedHandle::from_raw_handle(token) };
            token_logon_sid(token.as_raw_handle())?
        } else {
            let pid = stream.inner().server_process_id()?;
            let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if process.is_null() {
                return Err(io::Error::last_os_error());
            }
            let process = unsafe { OwnedHandle::from_raw_handle(process) };
            let mut token: HANDLE = ptr::null_mut();
            if unsafe { OpenProcessToken(process.as_raw_handle(), TOKEN_QUERY, &mut token) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let token = unsafe { OwnedHandle::from_raw_handle(token) };
            let sid = token_logon_sid(token.as_raw_handle())?;
            if stream.inner().server_process_id()? != pid {
                return Err(denied());
            }
            sid
        };
        if actual != self.logon_sid {
            return Err(denied());
        }
        Ok(())
    }
    pub(super) fn verify_server_image(
        &self,
        stream: &Stream,
        expected: &std::path::Path,
    ) -> io::Result<()> {
        use std::os::windows::ffi::OsStringExt;
        self.verify_peer(stream, false)?;
        let expected = std::fs::canonicalize(expected)?;
        let Stream::NamedPipe(stream) = stream;
        let pid = stream.inner().server_process_id()?;
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        let mut path = vec![0u16; 32768];
        let mut size = path.len() as u32;
        if unsafe {
            QueryFullProcessImageNameW(
                process.as_raw_handle(),
                PROCESS_NAME_WIN32,
                path.as_mut_ptr(),
                &mut size,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        path.truncate(size as usize);
        let actual = std::path::PathBuf::from(std::ffi::OsString::from_wide(&path));
        if std::fs::canonicalize(actual)? != expected || stream.inner().server_process_id()? != pid
        {
            return Err(denied());
        }
        Ok(())
    }
}
fn current_logon_sid() -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    token_logon_sid(token.as_raw_handle())
}
fn token_logon_sid(token: HANDLE) -> io::Result<String> {
    let mut size = 0;
    unsafe {
        GetTokenInformation(token, TokenLogonSid, ptr::null_mut(), 0, &mut size);
    }
    if size < std::mem::size_of::<TOKEN_GROUPS>() as u32 || size > 16 * 1024 {
        return Err(denied());
    }
    // TOKEN_GROUPS contains pointers, so allocate pointer-aligned storage.
    let mut bytes = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token,
            TokenLogonSid,
            bytes.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let groups = unsafe { &*bytes.as_ptr().cast::<TOKEN_GROUPS>() };
    if groups.GroupCount != 1 {
        return Err(denied());
    }
    let mut text = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(groups.Groups[0].Sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { widestring::U16CStr::from_ptr_str(text) }
        .to_string()
        .map_err(|_| denied());
    unsafe {
        LocalFree(text.cast());
    }
    result
}
