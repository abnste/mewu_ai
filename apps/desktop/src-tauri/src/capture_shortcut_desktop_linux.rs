// SPDX-License-Identifier: MPL-2.0
//! Normal per-user desktop integration for the host portal Registry. Never
//! changes the portal daemon's environment or overwrites another desktop entry.
use serde::{Deserialize, Serialize};
use std::{fs::{self, File, OpenOptions}, io::{Read, Write}, os::unix::fs::{MetadataExt, OpenOptionsExt}, path::{Path, PathBuf}};

pub(crate) const APP_ID: &str = "dev.mewu.remake";
const RECEIPT: &str = "wayland-shortcut-desktop.json";
const LIMIT: u64 = 16 * 1024;
const ERROR: &str = "无法注册 Mewu 桌面入口，请检查用户应用目录";
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt { version: u32, entries: Vec<String> }

fn safe_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, std::path::Component::ParentDir)) { return Err(ERROR.into()); }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(ERROR.into()),
            Ok(_) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(_) => return Err(ERROR.into()),
        }
    }
    Ok(())
}
fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    safe_path(path)?;
    let file = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ERROR.into()),
    };
    let meta = file.metadata().map_err(|_| ERROR)?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.len() > LIMIT { return Err(ERROR.into()); }
    let mut bytes = Vec::new(); file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(|_| ERROR)?;
    if bytes.len() as u64 > LIMIT { return Err(ERROR.into()); } Ok(Some(bytes))
}
fn render(executable: &Path) -> Result<String, String> {
    safe_path(executable)?;
    if !executable.is_file() { return Err(ERROR.into()); }
    let value = executable.to_str().ok_or(ERROR)?;
    if value.chars().any(char::is_control) { return Err(ERROR.into()); }
    // Desktop Entry general escaping is applied AFTER Exec argument quoting.
    // Literal percent must never become an Exec field code.
    let mut argument = String::from("\"");
    for c in value.chars() {
        match c {
            '%' => argument.push_str("%%"),
            '"' | '`' | '$' | '\\' => { argument.push('\\'); argument.push(c); },
            _ => argument.push(c),
        }
    }
    argument.push('"');
    let exec = argument.replace('\\', "\\\\");
    Ok(format!("[Desktop Entry]\nType=Application\nName=MewuAI\nComment=Mewu AI desktop\nExec={exec}\nIcon=MewuAI\nTerminal=false\nStartupWMClass=MewuAI\nX-Mewu-Desktop-Integration=1\n"))
}
struct Temporary(PathBuf);
impl Drop for Temporary { fn drop(&mut self) { let _ = fs::remove_file(&self.0); } }
fn replace(path: &Path, expected: Option<&[u8]>, bytes: &[u8], allowed: &impl Fn() -> bool) -> Result<(), String> {
    if bytes.len() as u64>LIMIT{return Err(ERROR.into())}
    let parent = path.parent().ok_or(ERROR)?;
    safe_path(parent)?;
    let temporary = Temporary(parent.join(format!(".mewu-desktop-{}.tmp", uuid::Uuid::new_v4())));
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(&temporary.0).map_err(|_| ERROR)?;
    file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|_| ERROR)?;
    if !allowed() || read(path)?.as_deref() != expected { return Err(ERROR.into()); }
    if expected.is_none() {
        // Hard-link publication is atomic and refuses a concurrent new entry.
        fs::hard_link(&temporary.0, path).map_err(|_| ERROR)?;
    } else { fs::rename(&temporary.0, path).map_err(|_| ERROR)?; }
    File::open(parent).and_then(|f| f.sync_all()).map_err(|_| ERROR)?;
    Ok(())
}
pub(crate) fn ensure(root: &Path, create: bool, allowed: impl Fn() -> bool) -> Result<(), String> {
    let data = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(std::env::var_os("HOME").ok_or(ERROR)?).join(".local/share"),
    };
    let executable=std::env::current_exe().map_err(|_| ERROR)?;
    let launcher=launcher(&executable,std::env::var_os("APPDIR").map(PathBuf::from).as_deref(),std::env::var_os("APPIMAGE").map(PathBuf::from).as_deref())?;
    integrate(root, &data.join("applications"), &launcher, create, allowed)
}
fn executable(path:&Path)->Result<File,String>{
    safe_path(path)?;
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(path).map_err(|_|ERROR)?;
    let meta=file.metadata().map_err(|_|ERROR)?;
    if !meta.is_file()||meta.mode()&0o111==0||!(meta.uid()==0||meta.uid()==unsafe{libc::geteuid()}){return Err(ERROR.into())}Ok(file)
}
fn launcher(exe:&Path,appdir:Option<&Path>,appimage:Option<&Path>)->Result<PathBuf,String>{
    executable(exe)?;
    let own_dir=exe.parent().filter(|p|p.file_name()==Some(std::ffi::OsStr::new("bin")))
        .and_then(Path::parent).filter(|p|p.file_name()==Some(std::ffi::OsStr::new("usr"))).and_then(Path::parent);
    let Some(own_dir)=own_dir else {
        if appdir.is_some()||appimage.is_some(){return Err(ERROR.into())}return Ok(exe.into());
    };
    let app_run=own_dir.join("AppRun");
    if !app_run.exists() {
        if appdir.is_some()||appimage.is_some(){return Err(ERROR.into())}return Ok(exe.into());
    }
    // linuxdeploy's own AppRun carries GTK/GStreamer/PipeWire hooks. Registry
    // metadata must launch the regular bundle entry, never its raw child ELF.
    executable(&app_run)?;
    if appdir.is_some_and(|dir|dir!=own_dir){return Err(ERROR.into())}
    if let Some(image)=appimage{
        if appdir!=Some(own_dir){return Err(ERROR.into())}
        let mut file=executable(image)?;let mut header=[0u8;11];file.read_exact(&mut header).map_err(|_|ERROR)?;
        if &header[..4]!=b"\x7fELF"||&header[8..11]!=b"AI\x02"{return Err("AppImage 启动入口无效".into())}
        return Ok(image.into());
    }
    Ok(app_run)
}
fn integrate(root: &Path, applications: &Path, executable: &Path, create: bool, allowed: impl Fn() -> bool) -> Result<(), String> {
    safe_path(root)?; safe_path(applications)?;
    let next = render(executable)?;
    let entry = applications.join(format!("{APP_ID}.desktop"));
    let previous = read(&entry)?;
    // An exact current entry (including a system installer-created own entry)
    // needs no write. Missing registration may only be created on user action.
    if previous.as_deref() == Some(next.as_bytes()) { return Ok(()); }
    if (!create && previous.is_none()) || !allowed() { return Err(ERROR.into()); }
    let lock_path = root.join("wayland-shortcut-desktop.lock"); safe_path(&lock_path)?;
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(lock_path).map_err(|_| ERROR)?;
    let lock_meta=lock.metadata().map_err(|_|ERROR)?;
    if !lock_meta.is_file()||lock_meta.uid()!=unsafe{libc::geteuid()}{return Err(ERROR.into())}
    lock.try_lock().map_err(|_| "桌面入口正在更新")?;
    let receipt_path = root.join(RECEIPT);
    let old_receipt = read(&receipt_path)?;
    let receipt: Option<Receipt> = old_receipt.as_deref().map(serde_json::from_slice).transpose().map_err(|_| ERROR)?;
    if let Some(bytes) = &previous {
        if !receipt.as_ref().is_some_and(|r| r.version == 1 && r.entries.len()<=2 && r.entries.iter().any(|entry|entry.as_bytes()==bytes)) {
            return Err("已有同名桌面入口，已保留原文件".into());
        }
    }
    fs::create_dir_all(applications).map_err(|_| ERROR)?; safe_path(applications)?;
    // Retain the exact previous and next bytes across the two-file transaction.
    // A cancel/crash between writes permits a retry, never unknown entry bytes.
    let mut entries=vec![next.clone()];
    if let Some(previous)=&previous {
        let previous=String::from_utf8(previous.clone()).map_err(|_|ERROR)?;
        if previous!=next {entries.push(previous);}
    }
    let bytes = serde_json::to_vec(&Receipt { version: 1, entries }).map_err(|_| ERROR)?;
    replace(&receipt_path, old_receipt.as_deref(), &bytes, &allowed)?;
    replace(&entry, previous.as_deref(), next.as_bytes(), &allowed)
}

#[cfg(test)] mod tests {
    use super::*;
    struct Dir(PathBuf);
    impl Drop for Dir { fn drop(&mut self) { let _=fs::remove_dir_all(&self.0); } }
    fn dir() -> Dir { let p=std::env::temp_dir().join(format!("mewu-shortcut-desktop-{}",uuid::Uuid::new_v4())); fs::create_dir(&p).unwrap(); Dir(p) }
    #[test] fn user_action_only_and_unknown_entry_bytes_are_preserved() {
        let d=dir(); let apps=d.0.join("applications"); let exe=d.0.join("Mewu AI"); fs::write(&exe,b"owned").unwrap();
        assert!(integrate(&d.0,&apps,&exe,false,||true).is_err()); assert!(!apps.exists());
        integrate(&d.0,&apps,&exe,true,||true).unwrap();
        let entry=apps.join(format!("{APP_ID}.desktop")); fs::write(&entry,b"[Desktop Entry]\nExec=other\n").unwrap();
        assert!(integrate(&d.0,&apps,&exe,true,||true).is_err()); assert_eq!(fs::read(entry).unwrap(),b"[Desktop Entry]\nExec=other\n");
    }
    #[test] fn exact_receipt_allows_upgrade_but_links_and_control_chars_are_rejected() {
        let d=dir();let apps=d.0.join("applications");let old=d.0.join("old");let new=d.0.join("new");fs::write(&old,b"1").unwrap();fs::write(&new,b"2").unwrap();
        integrate(&d.0,&apps,&old,true,||true).unwrap();integrate(&d.0,&apps,&new,true,||true).unwrap();
        assert_eq!(read(&apps.join(format!("{APP_ID}.desktop"))).unwrap().unwrap(),render(&new).unwrap().as_bytes());
        let linked=d.0.join("linked");std::os::unix::fs::symlink(&apps,&linked).unwrap();assert!(integrate(&d.0,&linked,&new,true,||true).is_err());
        let bad=d.0.join("bad\nname");fs::write(&bad,b"3").unwrap();assert!(render(&bad).is_err());assert!(render(Path::new("relative")).is_err());
    }
    #[test] fn exec_has_literal_percent_and_double_layer_escaping() {
        let d=dir();let exe=d.0.join("a%$`\\\"b");fs::write(&exe,b"x").unwrap();let rendered=render(&exe).unwrap();
        assert!(rendered.contains("a%%\\\\$\\\\`\\\\\\\\\\\\\"b"));
        let key=gtk::glib::KeyFile::new();key.load_from_data(&rendered,gtk::glib::KeyFileFlags::NONE).unwrap();
        assert!(key.string("Desktop Entry","Exec").unwrap().contains("a%%\\$\\`\\\\\\\"b"));
    }
    #[test]fn bundle_entry_and_appimage_are_regular_stable_launchers_not_raw_child_elf(){
        use std::os::unix::fs::PermissionsExt;
        let d=dir();let own=d.0.join("Mewu.AppDir");let bin=own.join("usr/bin");fs::create_dir_all(&bin).unwrap();
        let exe=bin.join("MewuAI");let entry=own.join("AppRun");
        for path in [&exe,&entry]{fs::write(path,b"fixture").unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}
        assert_eq!(launcher(&exe,Some(&own),None).unwrap(),entry);
        let image=d.0.join("Mewu.AppImage");fs::write(&image,b"\x7fELF\0\0\0\0AI\x02").unwrap();fs::set_permissions(&image,fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(launcher(&exe,Some(&own),Some(&image)).unwrap(),image);
        assert!(launcher(&exe,None,Some(&image)).is_err());
        assert!(launcher(&exe,Some(&d.0),None).is_err());
        fs::write(&image,b"not an AppImage").unwrap();assert!(launcher(&exe,Some(&own),Some(&image)).is_err());
        fs::remove_file(&entry).unwrap();std::os::unix::fs::symlink(&exe,&entry).unwrap();assert!(launcher(&exe,Some(&own),None).is_err());
    }
}
