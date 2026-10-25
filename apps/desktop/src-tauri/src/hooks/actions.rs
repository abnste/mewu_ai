// SPDX-License-Identifier: MPL-2.0
use super::*;
use base64::Engine;
use mewu_hooks::Shell;
use std::path::PathBuf;

#[derive(Clone)]
pub(super) struct Fence {
    shared: Arc<Shared>,
    rule_id: String,
    revision: u64,
    generation: u64,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    before_quit: bool,
}
impl Fence {
    pub fn new(work: &Execution) -> Self {
        Self {
            shared: work.shared.clone(),
            rule_id: work.rule.id.clone(),
            revision: work.rule.revision,
            generation: work.generation,
            cancel: work.cancel.clone(),
            deadline: work.deadline,
            before_quit: work.event.name == "app.beforeQuit",
        }
    }
    pub fn check(&self) -> Result<()> {
        require(&self.shared)?;
        if self.cancel.load(Ordering::Acquire)
            || self.shared.generation.load(Ordering::Acquire) != self.generation
            || Instant::now() >= self.deadline
            || (self.shared.quitting.load(Ordering::Acquire) && !self.before_quit)
        {
            return Err("规则已停止、更改或超时".into());
        }
        if !lock(&self.shared.store)?
            .rule(&self.rule_id)?
            .is_some_and(|r| r.enabled && r.revision == self.revision)
        {
            return Err("规则已更改或删除".into());
        }
        Ok(())
    }
}
pub(super) fn execute<'a>(
    work: &'a Execution,
    action: &'a Action,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<String>>> + Send + 'a>> {
    Box::pin(async move {
        work.check()?;
        mewu_hooks::validate_action(action)?;
        let app = &work.shared.app;
        match action {
            Action::InjectPrompt { text } => {
                if !matches!(work.rule.trigger, Trigger::Prompt { .. }) {
                    return Err("此事件不能注入当前提示词".into());
                }
                Ok(vec![text.clone()])
            }
            Action::SendPrompt { text, scene_id } => {
                if matches!(work.rule.trigger, Trigger::Prompt { .. }) {
                    return Err("提示词触发请使用注入提示词".into());
                }
                super::send_prompt(work, text, scene_id.as_deref()).await?;
                Ok(vec![])
            }
            Action::Mcp {
                server_id,
                tool,
                arguments,
            } => {
                let server = {
                    let host = app.state::<crate::Host>();
                    let engine = host.lock()?;
                    engine
                        .store
                        .mcp_servers()
                        .iter()
                        .find(|s| {
                            s.enabled
                                && s.id == *server_id
                                && s.tools.iter().any(|t| t.name == *tool)
                        })
                        .cloned()
                        .ok_or("MCP 服务或工具已删除或停用，请重新选择")?
                };
                let canceled = async {
                    loop {
                        if work.check().is_err() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                };
                let mut session = tokio::select! {r=tokio::time::timeout(work.deadline.saturating_duration_since(Instant::now()),crate::mcp_runtime::connect(&server.command))=>r.map_err(|_|"连接 MCP 超时")??,_=canceled=>return Err("MCP 连接已取消".into())};
                let outcome = async {
                    session.catalog().await?;
                    let fence = Fence::new(work);
                    let app = app.clone();
                    let id = server.id.clone();
                    let revision = server.revision;
                    let command = server.command.clone();
                    match session
                        .call_recorded(tool, arguments.clone(), move || {
                            fence.check().map_err(|_| ())?;
                            let host = app.state::<crate::Host>();
                            let engine = host.lock().map_err(|_| ())?;
                            engine
                                .store
                                .mcp_servers()
                                .iter()
                                .any(|s| {
                                    s.enabled
                                        && s.id == id
                                        && s.revision == revision
                                        && s.command == command
                                })
                                .then_some(())
                                .ok_or(())
                        })
                        .await
                    {
                        crate::mcp_runtime::McpCallOutcome::Responded(result)
                            if result.returned_error != Some(true)
                                && result.response_kind
                                    == mewu_core::ToolResponseKind::Complete =>
                        {
                            Ok(())
                        }
                        crate::mcp_runtime::McpCallOutcome::Responded(_) => {
                            Err("MCP 工具返回错误或未完成".into())
                        }
                        crate::mcp_runtime::McpCallOutcome::NotSent(_) => {
                            Err("MCP 参数无效或工具不可用，未执行".into())
                        }
                        crate::mcp_runtime::McpCallOutcome::Unknown(_) => {
                            Err("MCP 执行结果不确定，未自动重试".into())
                        }
                    }
                };
                let canceled = async {
                    loop {
                        if work.check().is_err() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                };
                let result: Result<()> = tokio::select! {r=tokio::time::timeout(work.deadline.saturating_duration_since(Instant::now()),outcome)=>r.unwrap_or_else(|_|Err("MCP 执行超时，未自动重试".into())),_=canceled=>Err("MCP 已停止，未自动重试".into())};
                session.close().await;
                result?;
                Ok(vec![])
            }
            Action::Command { shell, code, cwd } => {
                let (config, temp, environment) =
                    shell_command(shell, code, working_directory(work, cwd.as_deref())?)?;
                let fence = Fence::new(work);
                let input = serde_json::to_vec(&work.event).map_err(|_| "事件内容无效")?;
                let result = crate::mcp_runtime::run_user_command(
                    &config,
                    input,
                    work.cancel.clone(),
                    work.deadline.saturating_duration_since(Instant::now()),
                    environment,
                    Box::new(move || fence.check()),
                )
                .await;
                drop(temp);
                result?;
                Ok(vec![])
            }
            Action::Program {
                executable,
                arguments,
                cwd,
            } => {
                let config = mewu_core::McpCommand {
                    executable: executable.clone(),
                    args: arguments.clone(),
                    cwd: working_directory(work, cwd.as_deref())?,
                };
                let fence = Fence::new(work);
                let input = serde_json::to_vec(&work.event).map_err(|_| "事件内容无效")?;
                crate::mcp_runtime::run_user_command(
                    &config,
                    input,
                    work.cancel.clone(),
                    work.deadline.saturating_duration_since(Instant::now()),
                    Default::default(),
                    Box::new(move || fence.check()),
                )
                .await?;
                Ok(vec![])
            }
            Action::OpenUrl { url } => {
                let address = crate::code_open::http_url(url).ok_or("网址无效")?;
                let fence = Fence::new(work);
                tauri::async_runtime::spawn_blocking(move || {
                    crate::code_open::open(address, move || fence.check())
                })
                .await
                .map_err(|_| "打开网页中断")??;
                Ok(vec![])
            }
            Action::OpenApp { path } => {
                let path = PathBuf::from(path);
                let fence = Fence::new(work);
                tauri::async_runtime::spawn_blocking(move || {
                    crate::code_open::open_application(&path, move || fence.check())
                })
                .await
                .map_err(|_| "打开应用中断")??;
                Ok(vec![])
            }
            Action::Input { steps } => {
                super::input::execute(work, steps.clone()).await?;
                Ok(vec![])
            }
            Action::Script { code } => {
                let code = code.clone();
                let event = work.event.clone();
                let cancel = work.cancel.clone();
                let value = tauri::async_runtime::spawn_blocking(move || {
                    mewu_hooks::evaluate(&code, &event, cancel)
                })
                .await
                .map_err(|_| "脚本运行中断")??;
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Plan {
                    #[serde(default)]
                    prompt: Option<String>,
                    #[serde(default)]
                    actions: Vec<Action>,
                }
                let plan: Plan = serde_json::from_value(value)
                    .map_err(|_| "脚本须返回包含 prompt 或 actions 的 JSON 对象")?;
                if plan.actions.is_empty()
                    && plan.prompt.as_ref().is_none_or(|p| p.trim().is_empty())
                {
                    return Err("脚本未返回提示词或动作".into());
                }
                if plan.actions.len() > 16
                    || plan
                        .actions
                        .iter()
                        .any(|a| matches!(a, Action::Script { .. }))
                {
                    return Err("脚本动作过多或包含递归脚本".into());
                }
                let mut output = vec![];
                if let Some(text) = plan.prompt {
                    if !matches!(work.rule.trigger, Trigger::Prompt { .. }) {
                        return Err("当前事件不支持注入提示词".into());
                    }
                    if text.len() > 64 * 1024 {
                        return Err("脚本注入内容过大".into());
                    }
                    output.push(text);
                }
                for a in &plan.actions {
                    work.check()?;
                    output.extend(execute(work, a).await?);
                }
                Ok(output)
            }
            Action::Emit { name, payload } => {
                let event = work.child_event(name, payload.clone());
                event.validate()?;
                emit(app, event);
                Ok(vec![])
            }
            Action::App { method, params } => {
                if method == "chat.send" {
                    return Err("请使用发送提示词动作，以保留触发来源并防止递归".into());
                }
                super::app_action(work, method, params.clone()).await?;
                Ok(vec![])
            }
            Action::Delay { milliseconds } => {
                let until = Instant::now() + Duration::from_millis(*milliseconds);
                while Instant::now() < until {
                    work.check()?;
                    tokio::time::sleep(
                        Duration::from_millis(20)
                            .min(until.saturating_duration_since(Instant::now())),
                    )
                    .await;
                }
                Ok(vec![])
            }
        }
    })
}

fn working_directory(work: &Execution, requested: Option<&str>) -> Result<Option<String>> {
    if let Some(path) = requested {
        return Ok(Some(path.into()));
    }
    work.check()?;
    let path = work
        .shared
        .app
        .state::<crate::Host>()
        .root
        .join("hook-workspaces")
        .join(&work.rule.id);
    std::fs::create_dir_all(&path).map_err(|_| "无法创建规则工作目录")?;
    work.check()?;
    Ok(Some(path.to_string_lossy().into_owned()))
}
fn shell_command(
    shell: &Shell,
    code: &str,
    cwd: Option<String>,
) -> Result<(
    mewu_core::McpCommand,
    Option<tempfile::NamedTempFile>,
    std::collections::BTreeMap<String, String>,
)> {
    let mut temp = None;
    let mut environment = std::collections::BTreeMap::new();
    let (executable, args) = match shell {
        Shell::Cmd => {
            if !cfg!(windows) {
                return Err("CMD 仅适用于 Windows".into());
            }
            use std::io::Write;
            let mut file = tempfile::Builder::new()
                .prefix("mewu-hook-")
                .suffix(".cmd")
                .tempfile()
                .map_err(|_| "无法准备命令文件")?;
            file.write_all(b"@echo off\r\nchcp 65001 >nul\r\n")
                .and_then(|_| file.write_all(code.as_bytes()))
                .and_then(|_| file.write_all(b"\r\n"))
                .map_err(|_| "无法写入命令文件")?;
            file.as_file().sync_all().map_err(|_| "无法保存命令文件")?;
            let path = file.path().to_string_lossy().into_owned();
            temp = Some(file);
            environment.insert("MEWU_HOOK_SCRIPT".into(), format!("\"{path}\""));
            (
                windows_system_program("cmd.exe")?,
                vec![
                    "/D".into(),
                    "/S".into(),
                    "/C".into(),
                    "%MEWU_HOOK_SCRIPT%".into(),
                ],
            )
        }
        Shell::Powershell => {
            let executable = if cfg!(windows) {
                windows_system_program("WindowsPowerShell/v1.0/powershell.exe")?
            } else {
                find_program("pwsh")?
            };
            let source=format!("[Console]::InputEncoding = [Text.UTF8Encoding]::new($false); [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); $OutputEncoding = [Text.UTF8Encoding]::new($false);\n{code}");
            let encoded = base64::engine::general_purpose::STANDARD.encode(
                source
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>(),
            );
            #[cfg(windows)]
            if encoded.len() > 24000 {
                return Err("命令过长，请使用程序动作运行脚本文件".into());
            }
            (
                executable,
                vec![
                    "-NoLogo".into(),
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-EncodedCommand".into(),
                    encoded,
                ],
            )
        }
        Shell::Sh => {
            if cfg!(windows) {
                return Err("此运行环境请使用 PowerShell 或 CMD".into());
            }
            (PathBuf::from("/bin/sh"), vec!["-c".into(), code.into()])
        }
    };
    Ok((
        mewu_core::McpCommand {
            executable: executable.to_string_lossy().into_owned(),
            args,
            cwd,
        },
        temp,
        environment,
    ))
}
fn windows_system_program(name: &str) -> Result<PathBuf> {
    let root = std::env::var_os("SystemRoot").ok_or("系统目录不可用")?;
    let path = PathBuf::from(root).join("System32").join(name);
    if !path.is_file() {
        return Err("系统运行程序不存在".into());
    }
    Ok(path)
}
fn find_program(name: &str) -> Result<PathBuf> {
    let paths = std::env::var_os("PATH").ok_or("未找到运行程序")?;
    for parent in std::env::split_paths(&paths) {
        if !parent.is_absolute() {
            continue;
        }
        let candidate = parent.join(name);
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .map_err(|_| "运行程序不可用".into());
        }
    }
    Err("未安装 PowerShell，请使用 Shell".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_execution_does_not_relax_mcp_executable_admission() {
        let shell = if cfg!(windows) {
            Shell::Powershell
        } else {
            Shell::Sh
        };
        let (command, _file, _environment) = shell_command(&shell, "exit 0", None).unwrap();
        assert!(
            crate::mcp_runtime::validate_command(&command).is_err(),
            "MCP must still reject shells"
        );
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn real_cmd_handles_spaces_percent_signs_and_unicode_in_script_path() {
        let folder = tempfile::Builder::new()
            .prefix("mewu hooks %NO_EXPANSION% & 中文 ")
            .tempdir()
            .unwrap();
        let (mut command, file, mut environment) = shell_command(
            &Shell::Cmd,
            "echo success>cmd-result.txt\r\nexit /b 0",
            Some(folder.path().to_string_lossy().into()),
        )
        .unwrap();
        let script = folder.path().join("命令 %LITERAL% & 文件.cmd");
        std::fs::copy(file.as_ref().unwrap().path(), &script).unwrap();
        environment.insert(
            "MEWU_HOOK_SCRIPT".into(),
            format!("\"{}\"", script.display()),
        );
        command.cwd = Some(folder.path().to_string_lossy().into());
        crate::mcp_runtime::run_user_command(
            &command,
            b"{}".to_vec(),
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(10),
            environment,
            Box::new(|| Ok(())),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(folder.path().join("cmd-result.txt"))
                .unwrap()
                .trim(),
            "success"
        );
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn real_command_drains_output_and_passes_event_as_data_not_source() {
        let folder = tempfile::tempdir().unwrap();
        let code="[Console]::Out.WriteLine(('x' * 200000)); $raw = [Console]::In.ReadToEnd(); [IO.File]::WriteAllText((Join-Path (Get-Location).Path 'event.json'), $raw, [Text.UTF8Encoding]::new($false)); exit 0";
        let (command, _file, environment) = shell_command(
            &Shell::Powershell,
            code,
            Some(folder.path().to_string_lossy().into()),
        )
        .unwrap();
        let event =
            json!({"text":"$(New-Item SHOULD_NOT_EXIST) & 中文🙂", "padding":"p".repeat(128000)});
        let bytes = serde_json::to_vec(&event).unwrap();
        crate::mcp_runtime::run_user_command(
            &command,
            bytes.clone(),
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(15),
            environment,
            Box::new(|| Ok(())),
        )
        .await
        .unwrap();
        assert!(
            std::fs::read(folder.path().join("event.json")).unwrap() == bytes,
            "UTF-8 event bytes must survive stdin"
        );
        assert!(!folder.path().join("SHOULD_NOT_EXIST").exists());
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn native_command_cancellation_prevents_the_delayed_effect() {
        let folder = tempfile::tempdir().unwrap();
        let (command,_file,environment)=shell_command(&Shell::Powershell,"[Console]::In.ReadToEnd() | Out-Null; [IO.File]::WriteAllText('started','1'); Start-Sleep -Seconds 10; [IO.File]::WriteAllText('late','1')",Some(folder.path().to_string_lossy().into())).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let marker = folder.path().join("started");
        let stop = tokio::spawn(async move {
            let until = Instant::now() + Duration::from_secs(10);
            while !marker.exists() {
                assert!(Instant::now() < until);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            flag.store(true, Ordering::Release);
        });
        let result = crate::mcp_runtime::run_user_command(
            &command,
            b"{}".to_vec(),
            cancel,
            Duration::from_secs(15),
            environment,
            Box::new(|| Ok(())),
        )
        .await;
        stop.await.unwrap();
        assert!(result.is_err());
        assert!(!folder.path().join("late").exists());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn real_unix_command_drains_output_and_keeps_utf8_event_separate_from_code() {
        let folder = tempfile::tempdir().unwrap();
        let code = "head -c 200000 /dev/zero; cat > event.json";
        let (command, _file, environment) = shell_command(
            &Shell::Sh,
            code,
            Some(folder.path().to_string_lossy().into()),
        )
        .unwrap();
        let bytes = serde_json::to_vec(
            &json!({"text":"$(touch SHOULD_NOT_EXIST) & 中文🙂", "padding":"p".repeat(128000)}),
        )
        .unwrap();
        crate::mcp_runtime::run_user_command(
            &command,
            bytes.clone(),
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(15),
            environment,
            Box::new(|| Ok(())),
        )
        .await
        .unwrap();
        assert!(std::fs::read(folder.path().join("event.json")).unwrap() == bytes);
        assert!(!folder.path().join("SHOULD_NOT_EXIST").exists());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn real_unix_command_cancellation_stops_its_delayed_process_group() {
        let folder = tempfile::tempdir().unwrap();
        let (command, _file, environment) = shell_command(
            &Shell::Sh,
            "cat >/dev/null; printf 1 > started; sleep 10; printf 1 > late",
            Some(folder.path().to_string_lossy().into()),
        )
        .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let marker = folder.path().join("started");
        let stop = tokio::spawn(async move {
            let until = Instant::now() + Duration::from_secs(10);
            while !marker.exists() {
                assert!(Instant::now() < until);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            flag.store(true, Ordering::Release);
        });
        let result = crate::mcp_runtime::run_user_command(
            &command,
            b"{}".to_vec(),
            cancel,
            Duration::from_secs(15),
            environment,
            Box::new(|| Ok(())),
        )
        .await;
        stop.await.unwrap();
        assert!(result.is_err());
        assert!(!folder.path().join("late").exists());
    }
}
