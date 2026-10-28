// SPDX-License-Identifier: MPL-2.0
//! Read-only host facts. No plugin creation, browser inference, environment dump
//! or model-supplied command is involved in observing the operating system.
use serde_json::{json, Value};

pub fn definition() -> Value {
    json!({"type":"function","function":{"name":"system_info","description":"Read the actual local computer operating-system version, build and architecture from the host. Use this directly for questions about this computer; do not search memory, infer from browser userAgent, or create a plugin. Read-only; does not expose account names, environment variables, credentials or files.","parameters":{"type":"object","properties":{},"additionalProperties":false}}})
}

#[cfg(windows)]
fn os_version() -> Result<Value,String> {
    use windows_sys::Win32::System::Registry::*;
    fn read(name:&str)->Result<String,String> {
        let key:Vec<u16>="SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\0".encode_utf16().collect();
        let name:Vec<u16>=name.encode_utf16().chain(Some(0)).collect();
        let mut data=[0u16;256];let mut bytes=std::mem::size_of_val(&data) as u32;
        let result=unsafe {RegGetValueW(HKEY_LOCAL_MACHINE,key.as_ptr(),name.as_ptr(),RRF_RT_REG_SZ,std::ptr::null_mut(),data.as_mut_ptr().cast(),&mut bytes)};
        if result!=0{return Err(format!("无法读取系统版本（系统错误 {result}）"));}
        Ok(String::from_utf16_lossy(&data[..data.iter().position(|v|*v==0).unwrap_or(data.len())]))
    }
    let build=read("CurrentBuildNumber")?;
    let product=read("ProductName")?;
    let version=read("DisplayVersion").or_else(|_|read("ReleaseId"))?;
    let key:Vec<u16>="SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\0".encode_utf16().collect();
    let name:Vec<u16>="UBR\0".encode_utf16().collect();let mut revision=0u32;let mut bytes=4u32;
    let status=unsafe{RegGetValueW(HKEY_LOCAL_MACHINE,key.as_ptr(),name.as_ptr(),RRF_RT_REG_DWORD,std::ptr::null_mut(),(&mut revision as *mut u32).cast(),&mut bytes)};
    let full_build=if status==0{format!("{build}.{revision}")}else{build.clone()};
    // ProductName can still say Windows 10 on Windows 11; the native build is
    // authoritative. Preserve the raw registry product alongside this name.
    let name=if product.starts_with("Windows 10")&&build.parse::<u32>().unwrap_or(0)>=22000{product.replacen("Windows 10","Windows 11",1)}else{product.clone()};
    Ok(json!({"name":name,"registryProduct":product,"version":version,"build":full_build,"source":"HKLM/Software/Microsoft/Windows NT/CurrentVersion"}))
}
#[cfg(target_os="macos")]
fn os_version()->Result<Value,String> {
    // Apple stores the exact product version in a plist; use its native parser
    // instead of scraping shell output or making a user-agent estimate.
    let value=plist::Value::from_file("/System/Library/CoreServices/SystemVersion.plist").map_err(|_|"无法读取 macOS 系统版本")?;
    let dictionary=value.as_dictionary().ok_or("系统版本格式无效")?;
    let get=|key:&str|dictionary.get(key).and_then(plist::Value::as_string).unwrap_or("");
    Ok(json!({"name":get("ProductName"),"version":get("ProductVersion"),"build":get("ProductBuildVersion"),"source":"/System/Library/CoreServices/SystemVersion.plist"}))
}
#[cfg(target_os="linux")]
fn os_version()->Result<Value,String> {
    // os-release is the freedesktop key/value format, not JSON or shell code.
    // Return its bounded original data without sourcing/evaluating it.
    use std::io::Read;
    let mut text=String::new();
    std::fs::File::open("/etc/os-release").or_else(|_|std::fs::File::open("/usr/lib/os-release"))
        .map_err(|_|"无法读取 Linux 发行版本")?.take(16385).read_to_string(&mut text).map_err(|_|"系统版本不是 UTF-8 文本")?;
    if text.len()>16384{return Err("系统版本内容超出范围".into());}
    Ok(json!({"release":text,"source":"os-release"}))
}
pub fn read(arguments:&str)->Result<Value,String> {
    #[derive(serde::Deserialize)] #[serde(deny_unknown_fields)] struct Empty {}
    let _:Empty=serde_json::from_str(arguments).map_err(|_|"system_info 不接受参数")?;
    Ok(json!({"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"os":os_version()?,"observed":true}))
}
#[cfg(test)]
mod tests {
    #[test] fn reads_real_host_and_rejects_commands(){
        let result=super::read("{}").unwrap();assert_eq!(result["platform"],std::env::consts::OS);
        assert!(result["os"].is_object());assert!(super::read(r#"{"command":"anything"}"#).is_err());
        assert!(!result.to_string().contains("API_KEY"));
    }
}
