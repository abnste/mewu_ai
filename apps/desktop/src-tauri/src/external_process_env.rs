// SPDX-License-Identifier: MPL-2.0
//! Keep AppImage's private runtime out of independently installed programs.
use std::collections::BTreeMap;

pub(crate) fn inherited() -> BTreeMap<String, String> {
    #[allow(unused_mut)]
    let mut environment: BTreeMap<_, _> = std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect();
    #[cfg(target_os = "linux")]
    if let (Some(root), Ok(executable)) = (environment.get("APPDIR"), std::env::current_exe()) {
        // APPDIR alone is not evidence that this process belongs to that bundle.
        if let (Ok(root), Ok(executable)) = (std::fs::canonicalize(root), executable.canonicalize()) {
            if root.parent().is_some() && executable.starts_with(&root) {
                detach_bundle(&mut environment, &root);
            }
        }
    }
    environment
}

#[cfg(target_os = "linux")]
fn detach_bundle(environment: &mut BTreeMap<String, String>, root: &std::path::Path) {
    // Remove only bundle-owned path entries, preserving the user's unrelated
    // runtime configuration and the caller's explicit per-process overrides.
    for name in [
        "PATH", "LD_LIBRARY_PATH", "PYTHONHOME", "PYTHONPATH", "PERLLIB",
        "XDG_DATA_DIRS", "GI_TYPELIB_PATH", "GIO_MODULE_DIR", "GIO_EXTRA_MODULES",
        "GSETTINGS_SCHEMA_DIR", "GTK_PATH", "GTK_DATA_PREFIX", "GTK_EXE_PREFIX",
        "GTK_IM_MODULE_FILE", "GDK_PIXBUF_MODULE_FILE", "GDK_PIXBUF_MODULEDIR",
        "QT_PLUGIN_PATH", "QT_QPA_PLATFORM_PLUGIN_PATH", "GCONV_PATH",
        "GST_PLUGIN_PATH", "GST_PLUGIN_SYSTEM_PATH", "GST_PLUGIN_PATH_1_0",
        "GST_PLUGIN_SYSTEM_PATH_1_0", "GST_PLUGIN_SCANNER", "GST_PLUGIN_SCANNER_1_0",
        "GST_PTP_HELPER_1_0", "PIPEWIRE_MODULE_DIR", "SPA_PLUGIN_DIR", "PIPEWIRE_CONFIG_DIR",
    ] {
        let Some(value) = environment.get(name) else { continue };
        let parts: Vec<_> = value.split(':').filter(|part| {
            !std::path::Path::new(part).starts_with(root)
        }).collect();
        if parts.is_empty() { environment.remove(name); }
        else { let value = parts.join(":"); environment.insert(name.into(), value); }
    }
    for name in ["APPDIR", "APPIMAGE", "ARGV0", "OWD"] { environment.remove(name); }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn package_runtime_does_not_poison_system_python_or_other_clients() {
        let mut env = BTreeMap::from([
            ("APPDIR".into(), "/tmp/mewu.AppDir".into()),
            ("APPIMAGE".into(), "/opt/Mewu.AppImage".into()),
            ("PYTHONHOME".into(), "/tmp/mewu.AppDir/usr".into()),
            ("PYTHONPATH".into(), "/tmp/mewu.AppDir/usr/lib/python:/home/user/python".into()),
            ("LD_LIBRARY_PATH".into(), "/tmp/mewu.AppDir/usr/lib:/opt/vendor/lib".into()),
            ("PATH".into(), "/tmp/mewu.AppDir/usr/bin:/usr/bin:/home/user/.local/bin".into()),
            ("CODEX_HOME".into(), "/home/user/.codex".into()),
        ]);
        detach_bundle(&mut env, std::path::Path::new("/tmp/mewu.AppDir"));
        assert!(!env.contains_key("PYTHONHOME"));
        assert!(!env.contains_key("APPDIR"));
        assert!(!env.contains_key("APPIMAGE"));
        assert_eq!(env["PYTHONPATH"], "/home/user/python");
        assert_eq!(env["LD_LIBRARY_PATH"], "/opt/vendor/lib");
        assert_eq!(env["PATH"], "/usr/bin:/home/user/.local/bin");
        assert_eq!(env["CODEX_HOME"], "/home/user/.codex");
    }

    #[test]
    fn unrelated_prefixes_empty_entries_and_user_overrides_survive() {
        let mut env = BTreeMap::from([
            ("PATH".into(), "/tmp/mewu.AppDir-other/bin::/usr/bin".into()),
            ("PYTHONHOME".into(), "/home/user/python".into()),
        ]);
        detach_bundle(&mut env, std::path::Path::new("/tmp/mewu.AppDir"));
        assert_eq!(env["PATH"], "/tmp/mewu.AppDir-other/bin::/usr/bin");
        assert_eq!(env["PYTHONHOME"], "/home/user/python");
        env.extend(BTreeMap::from([("PYTHONHOME".into(), "/opt/explicit-python".into())]));
        assert_eq!(env["PYTHONHOME"], "/opt/explicit-python");
    }
}
