// SPDX-License-Identifier: MPL-2.0
//! Migration preflight for references that the asset resolver cannot relocate.
//! This is lexical analysis of typed configuration, never filesystem resolution,
//! environment expansion, shell parsing or executable inspection. In particular,
//! opaque arguments are not rewritten. 8.3 names, junctions and application-
//! specific configuration files cannot be proven independent by this check.
use crate::plugins::PluginSource;
use mewu_core::{McpCommand, McpServer};
use std::path::Path;

const MCP_DEPENDENCY: &str = "MCP 配置仍引用当前数据目录，请先修改";
const PLUGIN_DEPENDENCY: &str = "本地插件来源仍引用当前数据目录，请先修改";
const CATALOG_DEPENDENCY: &str = "插件目录来源仍引用当前数据目录，请先修改";
const ENV_DEPENDENCY: &str = "MCP 环境配置仍引用当前数据目录，请先修改";
const INVALID_ROOT: &str = "无法确认当前数据目录";
const MAX_TEXT: usize = 128 * 1024;

/// The caller supplies already loaded metadata. This function performs no I/O,
/// acquires no locks and does not change a snapshot or plugin authority.
/// Disabled servers/plugins are included: enabling them later must not revive a
/// dependency on an abandoned root. The host provides current and rollback
/// sources of installed plugins; removed plugin tombstones need no source file.
pub fn validate_dependencies(
    old_root: &Path,
    mcp_servers: &[McpServer],
    plugin_sources: &[PluginSource],
    catalog_source: Option<&str>,
) -> Result<(), String> {
    let root = old_root
        .to_str()
        .and_then(LexicalPath::absolute)
        .ok_or(INVALID_ROOT)?;
    for server in mcp_servers {
        if command_depends(&root, &server.command) {
            return Err(MCP_DEPENDENCY.into());
        }
    }
    for source in plugin_sources {
        if let PluginSource::Local { path } = source {
            if text_depends(&root, path, None) {
                return Err(PLUGIN_DEPENDENCY.into());
            }
        }
    }
    // Production currently saves GitHub HTTPS sources only. Include an explicit
    // local/file URI form defensively for any imported/older preference.
    if catalog_source.is_some_and(|source| text_depends(&root, source, None)) {
        return Err(CATALOG_DEPENDENCY.into());
    }
    Ok(())
}

/// McpCommand currently has no env field. The host may pass the values of the
/// exact inherited allowlist used by mcp_runtime, captured outside shared locks.
/// Never pass or log the entire process environment. PATH-like `;` lists and
/// KEY=absolute-path values use the same conservative literal-path check.
pub fn validate_environment<'a>(
    old_root: &Path,
    values: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let root = old_root
        .to_str()
        .and_then(LexicalPath::absolute)
        .ok_or(INVALID_ROOT)?;
    if values
        .into_iter()
        .any(|value| text_depends(&root, value, None))
    {
        Err(ENV_DEPENDENCY.into())
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct LexicalPath {
    // Windows drive or POSIX root. UNC/device namespaces are not silently
    // interpreted as local DOS paths; the managed root is a local directory.
    volume: String,
    windows: bool,
    components: Vec<String>,
}
impl LexicalPath {
    fn absolute(value: &str) -> Option<Self> {
        if value.len() > MAX_TEXT || value.chars().any(char::is_control) {
            return None;
        }
        let value = unquote(value);
        let value = value.strip_prefix(r"\\?\").unwrap_or(value);
        let bytes = value.as_bytes();
        let (volume, windows, tail) = if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/')
        {
            (value[..2].to_ascii_uppercase(), true, &value[3..])
        } else if value.starts_with('/') && !value.starts_with("//") {
            ("/".into(), false, &value[1..])
        } else {
            return None;
        };
        let mut result = Self {
            volume,
            windows,
            components: vec![],
        };
        result.append(tail);
        Some(result)
    }
    fn append(&mut self, tail: &str) {
        // Conservative for verbatim DOS too: report a dependency for either
        // spelling rather than claim the child program's path API semantics.
        for part in tail.split(|ch| ch == '/' || (self.windows && ch == '\\')) {
            let part = if self.windows {
                part.trim_end_matches(' ')
            } else {
                part
            };
            match part {
                "" | "." => {}
                ".." => {
                    self.components.pop();
                }
                part => self.components.push(if self.windows {
                    part.trim_end_matches([' ', '.']).to_string()
                } else {
                    part.to_string()
                }),
            }
        }
    }
    fn contains(&self, other: &Self) -> bool {
        self.windows == other.windows
            && self.volume == other.volume
            && self.components.len() <= other.components.len()
            && self
                .components
                .iter()
                .zip(&other.components)
                .all(|(left, right)| {
                    if self.windows {
                        ordinal_eq(left, right)
                    } else {
                        left == right
                    }
                })
    }
    fn relative(&self, value: &str) -> Option<Self> {
        let value = unquote(value);
        if value.is_empty() || value.contains(':') || value.starts_with(['/', '\\']) {
            return None;
        }
        let mut result = self.clone();
        result.append(value);
        Some(result)
    }
}

fn ordinal_eq(left: &str, right: &str) -> bool {
    #[cfg(windows)]
    {
        let left: Vec<u16> = left.encode_utf16().collect();
        let right: Vec<u16> = right.encode_utf16().collect();
        // Microsoft CompareStringOrdinal uses OS invariant uppercase rules,
        // not locale-dependent casing. No path lookup or window operation.
        unsafe {
            windows_sys::Win32::Globalization::CompareStringOrdinal(
                left.as_ptr(),
                left.len() as i32,
                right.as_ptr(),
                right.len() as i32,
                1,
            ) == 2
        }
    }
    #[cfg(not(windows))]
    {
        left.to_uppercase() == right.to_uppercase()
    }
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn command_depends(root: &LexicalPath, command: &McpCommand) -> bool {
    if text_depends(root, &command.executable, None)
        || command
            .cwd
            .as_deref()
            .is_some_and(|cwd| text_depends(root, cwd, None))
    {
        return true;
    }
    // This matches mcp_runtime's explicit cwd or executable parent. It does not
    // use the migrating process's cwd to guess relative child arguments.
    let cwd = command
        .cwd
        .as_deref()
        .and_then(LexicalPath::absolute)
        .or_else(|| {
            let mut path = LexicalPath::absolute(&command.executable)?;
            path.components.pop();
            Some(path)
        });
    command
        .args
        .iter()
        .any(|value| text_depends(root, value, cwd.as_ref()))
}

fn value_depends(root: &LexicalPath, value: &str, cwd: Option<&LexicalPath>) -> bool {
    let value = unquote(value);
    if value
        .get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case("file:"))
    {
        // URL parsing and percent decoding belong to the URL library, not a
        // handwritten replacement of %xx inside an opaque argument.
        if let Ok(url) = url::Url::parse(value) {
            if let Ok(path) = url.to_file_path() {
                if let Some(value) = path.to_str() {
                    // to_file_path is platform dependent. Preserve DOS identity
                    // in the pure tests when they run on a non-Windows host.
                    let bytes = value.as_bytes();
                    let value = if bytes.len() >= 4
                        && bytes[0] == b'/'
                        && bytes[1].is_ascii_alphabetic()
                        && bytes[2] == b':'
                    {
                        &value[1..]
                    } else {
                        value
                    };
                    return LexicalPath::absolute(value).is_some_and(|path| root.contains(&path));
                }
            }
        }
        return false;
    }
    LexicalPath::absolute(value)
        .or_else(|| cwd.and_then(|cwd| cwd.relative(value)))
        .is_some_and(|path| root.contains(&path))
}

fn text_depends(root: &LexicalPath, text: &str, cwd: Option<&LexicalPath>) -> bool {
    // Invalid/unbounded configuration cannot be certified safe. Existing typed
    // config limits are smaller; this also bounds scanning of inherited values.
    if text.len() > MAX_TEXT || text.contains('\0') {
        return true;
    }
    if value_depends(root, text, cwd) {
        return true;
    }
    let mut candidates = 0;
    for part in text.split(';') {
        candidates += 1;
        if candidates > 256 {
            return true;
        }
        if value_depends(root, part, cwd) {
            return true;
        }
        // One argv value may be --flag=PATH, NAME=PATH, or quoted absolute
        // paths in a child-runtime option. Find explicit token boundaries;
        // do not treat a substring of a filename/URL as a separate drive.
        for (index, ch) in part.char_indices() {
            if ch == '=' || ch == '"' || ch == '\'' || ch.is_whitespace() {
                candidates += 1;
                if candidates > 256 {
                    return true;
                }
                let tail = &part[index + ch.len_utf8()..];
                let end = tail.find(['"', '\'']).unwrap_or(tail.len());
                if value_depends(root, &tail[..end], if ch == '=' { cwd } else { None }) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(command: McpCommand) -> mewu_core::McpServer {
        mewu_core::McpServer {
            id: "synthetic".into(),
            label: "Synthetic".into(),
            revision: 1,
            command,
            tools: vec![],
            grants: vec![],
            enabled: false,
        }
    }
    fn root() -> LexicalPath {
        LexicalPath::absolute(r"D:\Mewu Data").unwrap()
    }

    #[test]
    fn windows_case_verbatim_and_component_boundaries() {
        for value in [
            r"d:\MEWU DATA",
            r"D:\Mewu Data\tools\server.exe",
            r"\\?\d:\mewu data\assets\x",
            "D:/Mewu Data/a",
            r"D:\else\..\Mewu Data\x",
            r"D:\else\.. \Mewu Data\x",
            r"D:\Mewu Data.\x",
            r"D:\Mewu Data \x",
        ] {
            assert!(text_depends(&root(), value, None), "{value}");
        }
        for value in [
            r"D:\Mewu Database\x",
            r"D:\Mewu Data-old\x",
            r"E:\Mewu Data\x",
            r"D:\Mewu Data\..\else\x",
            r"prefixD:\Mewu Data\x",
            "https://host/Mewu%20Data/x",
        ] {
            assert!(!text_depends(&root(), value, None), "{value}");
        }
    }

    #[test]
    fn explicit_flags_quotes_lists_and_file_urls_are_checked_without_expansion() {
        for value in [
            r"--config=D:\Mewu Data\config.json",
            r#"--config="D:\Mewu Data\x""#,
            r"DATA=D:\Mewu Data\x",
            r"C:\tools;D:\Mewu Data\tools;E:\tools",
            "--config=file:///D:/Mewu%20Data/config.json",
            r#"--options "D:\Mewu Data\x""#,
        ] {
            assert!(text_depends(&root(), value, None), "{value}");
        }
        for value in [
            r"--config=D:\Mewu Data-old\config.json",
            r"PATH=C:\tools;E:\other",
            r"%APPDATA%\server.json",
            r"${HOME}/server.json",
        ] {
            assert!(!text_depends(&root(), value, None), "{value}");
        }
    }

    #[test]
    fn relative_arguments_use_the_actual_child_cwd() {
        let mut command = McpCommand {
            executable: r"D:\tools\node.exe".into(),
            args: vec![r"--config=..\Mewu Data\server.json".into()],
            cwd: None,
        };
        assert!(command_depends(&root(), &command));
        command.cwd = Some(r"E:\tools".into());
        assert!(!command_depends(&root(), &command));
        command.cwd = Some(r"D:\Mewu Data".into());
        command.args.clear();
        assert!(command_depends(&root(), &command));
    }

    #[test]
    fn typed_snapshot_rejects_disabled_dependencies_and_never_mutates_metadata() {
        let mut state = vec![server(McpCommand {
            executable: r"D:\Mewu Data\mcp.exe".into(),
            args: vec![],
            cwd: None,
        })];
        let before = serde_json::to_vec(&state).unwrap();
        assert_eq!(
            validate_dependencies(Path::new(r"D:\Mewu Data"), &state, &[], None),
            Err(MCP_DEPENDENCY.into())
        );
        assert_eq!(before, serde_json::to_vec(&state).unwrap());
        state[0].command.executable = r"D:\tools\mcp.exe".into();
        assert!(validate_dependencies(Path::new(r"D:\Mewu Data"), &state, &[], None).is_ok());
    }

    #[test]
    fn local_plugin_provenance_and_catalog_are_not_asset_aliases() {
        let github = PluginSource::Github {
            repository: "author/package".into(),
            commit: "a".repeat(40),
            path: "plugin.json".into(),
        };
        let local = PluginSource::Local {
            path: r"\\?\D:\Mewu Data\plugin.json".into(),
        };
        assert_eq!(
            validate_dependencies(
                Path::new(r"D:\Mewu Data"),
                &[],
                &[github.clone(), local],
                None
            ),
            Err(PLUGIN_DEPENDENCY.into())
        );
        assert!(validate_dependencies(Path::new(r"D:\Mewu Data"), &[], &[github], None).is_ok());
        assert_eq!(
            validate_dependencies(
                Path::new(r"D:\Mewu Data"),
                &[],
                &[],
                Some("file:///D:/Mewu%20Data/catalog.json")
            ),
            Err(CATALOG_DEPENDENCY.into())
        );
        assert!(validate_dependencies(
            Path::new(r"D:\Mewu Data"),
            &[],
            &[],
            Some("https://github.com/author/catalog/blob/main/catalog.json")
        )
        .is_ok());
    }

    #[test]
    fn inherited_environment_values_are_bounded_and_errors_do_not_echo_them() {
        let secret = r"PRIVATE=secret;D:\Mewu Data\cache";
        let error = validate_environment(Path::new(r"D:\Mewu Data"), [secret]).unwrap_err();
        assert_eq!(error, ENV_DEPENDENCY);
        assert!(!error.contains("secret"));
        assert!(validate_environment(
            Path::new(r"D:\Mewu Data"),
            [r"C:\Windows", r"C:\Users\example"]
        )
        .is_ok());
        assert!(
            validate_environment(Path::new(r"D:\Mewu Data"), [&"x".repeat(MAX_TEXT + 1)[..]])
                .is_err()
        );
        assert!(validate_environment(Path::new(r"D:\Mewu Data"), [&" ;".repeat(300)[..]]).is_err());
        assert!(validate_dependencies(Path::new("relative"), &[], &[], None).is_err());
    }

    #[test]
    fn posix_component_boundaries_preserve_case() {
        let root = LexicalPath::absolute("/home/synthetic/.mewu").unwrap();
        assert!(text_depends(
            &root,
            "--config=/home/synthetic/.mewu/x",
            None
        ));
        assert!(!text_depends(&root, "/home/synthetic/.mewu-old/x", None));
        assert!(!text_depends(&root, "/home/Synthetic/.mewu/x", None));
    }
}
