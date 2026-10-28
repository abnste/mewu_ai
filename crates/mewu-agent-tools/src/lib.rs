// SPDX-License-Identifier: MPL-2.0
//! General execution primitives, not an application feature/command allowlist.
//! Paths use the supplied cwd and normal OS account access. Cwd is NOT a sandbox.
mod files;
pub mod shell;
pub use files::execute;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, String>;
// JSON can expand each control byte sixfold. Keep the bounded model result
// below the journal's 128 KiB even for hostile file/command output.
pub const MAX_OUTPUT: usize = 16 * 1024;
pub const MAX_FILE: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellCall {
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Optional seconds. No implicit command deadline; cancellation remains active.
    #[serde(default)]
    pub timeout: Option<f64>,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadCall {
    pub path: String,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteCall {
    pub path: String,
    pub content: String,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Replacement {
    pub old_text: String,
    pub new_text: String,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditCall {
    pub path: String,
    pub edits: Vec<Replacement>,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchCall {
    #[serde(default = "dot")]
    pub path: String,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub glob: Option<String>,
    #[serde(default)]
    pub literal: bool,
    #[serde(default)]
    pub ignore_case: bool,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub hidden: bool,
}
fn dot() -> String {
    ".".into()
}
pub fn parse<T: serde::de::DeserializeOwned>(arguments: &str) -> Result<T> {
    if arguments.len() > 512 * 1024 {
        return Err("Tool arguments exceed 512 KiB".into());
    }
    serde_json::from_str(arguments).map_err(|e| format!("Invalid arguments: {e}"))
}
fn schema<T: schemars::JsonSchema>() -> Value {
    let mut value = serde_json::to_value(
        schemars::generate::SchemaSettings::draft07()
            .with(|s| s.inline_subschemas = true)
            .into_generator()
            .into_root_schema_for::<T>(),
    )
    .expect("tool schema");
    if let Some(o) = value.as_object_mut() {
        o.remove("$schema");
        o.remove("title");
    }
    value
}
pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "read" | "write" | "edit" | "ls" | "find" | "grep" | "bash" | "powershell"
    )
}
pub fn mutates(name: &str) -> bool {
    matches!(name, "write" | "edit" | "bash" | "powershell")
}
pub fn definitions() -> Vec<Value> {
    let mut values=vec![
        ("read","Read a local UTF-8 file using an absolute path or the session cwd. offset is a 1-based LINE number; limit is lines. Returns actual content, SHA256 and nextOffset if truncated. Read successive pages instead of rerunning commands. Filesystem access uses the normal OS account, not a plugin-project sandbox.",schema::<ReadCall>()),
        ("write","Create or overwrite a local UTF-8 file; creates parent directories. Use for new files/complete rewrites; use edit for targeted changes. Returns actual committed path and SHA256. Relative paths resolve against the session cwd.",schema::<WriteCall>()),
        ("edit","Precisely edit a local UTF-8 file. Each edits[].oldText must uniquely match the ORIGINAL file and edits must not overlap. All edits commit together or none do. Preserve BOM, line endings and existing file permissions; returns a real diff and SHA256. Read before editing.",schema::<EditCall>()),
        ("ls","List a local directory, sorted by name. path defaults to session cwd; offset/limit paginate. Does not recursively search drives.",schema::<SearchCall>()),
        ("find","Find local files by glob pattern under path. Respects ignore files and skips symlink traversal. pattern is a glob such as **/*.rs. offset/limit paginate; hidden includes hidden entries.",schema::<SearchCall>()),
        ("grep","Search local UTF-8 files by regex (or literal=true), with actual path and line numbers. path may be a file or directory. glob optionally restricts files; respects ignore files. offset/limit paginate matches.",schema::<SearchCall>()),
    ];
    if shell::resolve("bash").is_ok() {
        values.push(("bash","Execute a real local bash command, with pipes, programs and scripts. cwd optionally selects an existing local working directory. timeout is optional seconds; absent means no command timeout. Returns output, actual exit code and a complete output log path when truncated. Nonzero exit results are errors to diagnose and correct, not proof of success. Runs with Mewu's OS account permissions.",schema::<ShellCall>()));
    }
    #[cfg(windows)]
    if shell::resolve("powershell").is_ok() {
        values.push(("powershell","Execute a real local PowerShell command with UTF-8 output. Use Windows commands/CIM/registry through this general tool, not a system-info plugin. Supports cwd and optional timeout seconds. Returns actual output, exit code and full output log when truncated. Runs with Mewu's OS account permissions.",schema::<ShellCall>()));
    }
    values.into_iter().map(|(name,description,parameters)|json!({"type":"function","function":{"name":name,"description":description,"parameters":parameters}})).collect()
}
pub fn resolve_path(cwd: &Path, value: &str) -> Result<PathBuf> {
    if value.is_empty() || value.contains('\0') || value.len() > 8192 {
        return Err(
            "Path must be nonempty, contain no NUL and fit the 8192-byte path budget".into(),
        );
    }
    let value = if value == "~" || value.starts_with("~/") || value.starts_with("~\\") {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .ok_or("Home directory is unavailable")?;
        PathBuf::from(home).join(value.get(2..).unwrap_or(""))
    } else {
        PathBuf::from(value)
    };
    Ok(if value.is_absolute() {
        value
    } else {
        cwd.join(value)
    })
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellSpec {
    pub executable: PathBuf,
    pub args: Vec<String>,
}
