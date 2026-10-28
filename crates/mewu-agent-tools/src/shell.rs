// SPDX-License-Identifier: MPL-2.0
use super::{Result, ShellCall, ShellSpec};
use std::path::PathBuf;

pub fn resolve(kind: &str) -> Result<ShellSpec> {
    if kind == "powershell" {
        if !cfg!(windows) {
            return Err("PowerShell tool is available on Windows only".into());
        }
        let executable = which::which("pwsh")
            .or_else(|_| which::which("powershell"))
            .ok()
            .or_else(|| {
                std::env::var_os("ProgramFiles")
                    .map(PathBuf::from)
                    .map(|p| p.join("PowerShell/7/pwsh.exe"))
                    .filter(|p| p.is_file())
            })
            .or_else(|| {
                std::env::var_os("SystemRoot")
                    .map(PathBuf::from)
                    .map(|p| p.join("System32/WindowsPowerShell/v1.0/powershell.exe"))
                    .filter(|p| p.is_file())
            })
            .ok_or("No PowerShell executable found; install it or add it to PATH")?;
        return Ok(ShellSpec {
            executable,
            args: vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
            ],
        });
    }
    if kind != "bash" {
        return Err("Unknown shell".into());
    }
    let mut candidates = Vec::new();
    #[cfg(windows)]
    for name in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(name) {
            candidates.push(PathBuf::from(root).join("Git/bin/bash.exe"));
        }
    }
    #[cfg(unix)]
    candidates.push(PathBuf::from("/bin/bash"));
    if let Ok(path) = which::which("bash") {
        candidates.push(path);
    }
    let executable = candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or("No bash executable found; install bash or select the available PowerShell tool")?;
    // Legacy System32 bash is a WSL launcher. Feed a script over stdin rather
    // than assuming it accepts Git Bash's -c invocation contract.
    let legacy = cfg!(windows)
        && std::env::var_os("SystemRoot").is_some_and(|root| {
            ["System32/bash.exe", "Sysnative/bash.exe"]
                .iter()
                .any(|suffix| {
                    PathBuf::from(&root)
                        .join(suffix)
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&executable.to_string_lossy())
                })
        });
    Ok(ShellSpec {
        executable,
        args: vec![if legacy { "-s" } else { "-c" }.into()],
    })
}
pub fn prepare(kind: &str, call: &ShellCall) -> Result<(ShellSpec, Option<Vec<u8>>)> {
    if call.command.trim().is_empty()
        || call.command.contains('\0')
        || call.command.len() > 128 * 1024
    {
        return Err("Command must be nonempty and at most 128 KiB".into());
    }
    if call
        .timeout
        .is_some_and(|v| !v.is_finite() || v <= 0.0 || v > 2_147_483.0)
    {
        return Err("timeout must be finite positive seconds, at most 2147483".into());
    }
    let mut spec = resolve(kind)?;
    let command = if kind == "powershell" {
        format!("[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); $OutputEncoding=[Console]::OutputEncoding;\n{}",call.command)
    } else {
        call.command.clone()
    };
    if spec.args == ["-s"] {
        Ok((spec, Some(command.into_bytes())))
    } else {
        spec.args.push(command);
        Ok((spec, None))
    }
}
