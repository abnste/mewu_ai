// SPDX-License-Identifier: MPL-2.0
//! Chat is an orchestrator of the same authenticated desktop operations, never
//! another Agent engine or an independent credential/history store.
use super::*;
use rustyline::{error::ReadlineError, Config, DefaultEditor};

pub(super) fn validate_prompt(text: &str) -> Result<(), ControlError> {
    if text.trim().is_empty()
        || text.len() > 128 * 1024
        || text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
    {
        return Err(invalid());
    }
    Ok(())
}
pub(super) trait ChatUi {
    fn progress(&mut self, label: &str) -> io::Result<()>;
    fn delta(&mut self, text: &str, replacement: bool) -> io::Result<()>;
    fn interrupted(&mut self) -> io::Result<bool>;
}
fn failure(code: ErrorCode, message: &'static str) -> Response {
    Response::error("chat", ControlError::not_sent(code, message))
}
fn io_failure(_: io::Error) -> Response {
    failure(ErrorCode::Internal, "Cannot write chat output")
}

pub(super) struct Session {
    pub scene: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    first: bool,
    activate: bool,
}
impl Session {
    fn new(call: &Invocation) -> Self {
        let scene = call
            .request
            .params
            .pointer("/context/sceneId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Self {
            activate: scene.is_some(),
            scene,
            agent: call.agent.clone(),
            model: call.model.clone(),
            first: true,
        }
    }
}
fn query(
    control: &mut impl Control,
    call: &Invocation,
    method: &str,
    params: Value,
) -> Result<Value, Response> {
    let request = Request::new(uuid::Uuid::new_v4().to_string(), method, params);
    let response = control
        .call(&request, call.timeout)
        .unwrap_or_else(|e| Response::error(request.id, e));
    if exit_code(&response) != 0 {
        return Err(response);
    }
    response
        .result()
        .cloned()
        .ok_or_else(|| failure(ErrorCode::InvalidResponse, "Missing control result"))
}
fn operation(
    control: &mut impl Control,
    clock: &mut impl Clock,
    call: &Invocation,
    method: &str,
    params: Value,
    id: Option<String>,
) -> Result<Value, Response> {
    let mut request = call.clone();
    request.action = Action::Request;
    request.wait = true;
    request.jsonl = false;
    request.request = Request::new(
        id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        method,
        params,
    );
    // The operation is submitted exactly once. dispatch only polls its receipt.
    let response = dispatch(&request, control, clock, |_| Ok(())).map_err(io_failure)?;
    if exit_code(&response) != 0 {
        return Err(response);
    }
    response
        .result()
        .and_then(|v| v.get("result"))
        .cloned()
        .ok_or_else(|| {
            Response::error(
                request.request.id,
                ControlError::unknown(ErrorCode::InvalidResponse, "Missing operation completion"),
            )
        })
}
fn choose_profile(list: &Value, choice: &str) -> Result<String, Response> {
    let rows = list
        .as_array()
        .ok_or_else(|| failure(ErrorCode::InvalidResponse, "Invalid profiles"))?;
    if let Some(found) = rows.iter().find(|row| row["id"].as_str() == Some(choice)) {
        return Ok(found["id"].as_str().unwrap().to_owned());
    }
    let hits: Vec<_> = rows
        .iter()
        .filter(|v| {
            v["id"].as_str() == Some(choice)
                || v["name"]
                    .as_str()
                    .is_some_and(|n| n.to_lowercase() == choice.to_lowercase())
                || v["model"].as_str() == Some(choice)
        })
        .collect();
    if hits.len() != 1 {
        return Err(failure(
            ErrorCode::InvalidRequest,
            "Profile not found or ambiguous; use its exact ID",
        ));
    }
    hits[0]["id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| failure(ErrorCode::InvalidResponse, "Invalid profile identity"))
}
fn configure(
    control: &mut impl Control,
    clock: &mut impl Clock,
    call: &Invocation,
    session: &mut Session,
) -> Result<(), Response> {
    let Some(scene) = session.scene.clone() else {
        return Ok(());
    };
    for (method, selected, kind, field) in [
        ("agent.list", session.agent.take(), "set_agent", "agentId"),
        (
            "model.list",
            session.model.take(),
            "set_scene_connection",
            "connectionId",
        ),
    ] {
        if let Some(selected) = selected {
            let list = query(control, call, method, json!({}))?;
            let id = choose_profile(&list, &selected)?;
            let mut command = json!({"type":kind,"sceneId":scene});
            command[field] = id.into();
            operation(
                control,
                clock,
                call,
                "scene.command",
                json!({"context":{"sceneId":scene},"command":command}),
                None,
            )?;
        }
    }
    Ok(())
}
fn ensure_scene(
    control: &mut impl Control,
    clock: &mut impl Clock,
    call: &Invocation,
    session: &mut Session,
) -> Result<String, Response> {
    let current = query(control, call, "app.status", json!({}))?;
    if let Some(scene) = session.scene.clone() {
        if current["activeSceneId"].as_str() != Some(&scene) {
            if !session.activate {
                return Err(failure(
                    ErrorCode::Conflict,
                    "Desktop scene changed; use /use to select this scene explicitly",
                ));
            }
            operation(
                control,
                clock,
                call,
                "scene.activate",
                json!({"sceneId":scene}),
                None,
            )?;
        }
    } else {
        let result = operation(
            control,
            clock,
            call,
            "scene.new",
            json!({"context":{"sceneId":current["activeSceneId"]}}),
            None,
        )?;
        session.scene = Some(
            result["sceneId"]
                .as_str()
                .filter(|s| identifier(s))
                .ok_or_else(|| failure(ErrorCode::InvalidResponse, "Missing scene identity"))?
                .into(),
        );
    }
    session.activate = false;
    configure(control, clock, call, session)?;
    Ok(session.scene.clone().unwrap())
}

#[derive(Debug)]
pub(super) struct Answer {
    pub scene: String,
    pub run: String,
    pub status: String,
    pub text: String,
}
impl Answer {
    fn value(&self) -> Value {
        json!({"sceneId":self.scene,"runId":self.run,"status":self.status,"text":self.text})
    }
}

pub(super) fn turn(
    control: &mut impl Control,
    clock: &mut impl Clock,
    call: &Invocation,
    session: &mut Session,
    text: &str,
    ui: &mut impl ChatUi,
) -> Result<Answer, Response> {
    validate_prompt(text).map_err(|e| Response::error("chat", e))?;
    let scene = ensure_scene(control, clock, call, session)?;
    let state = query(
        control,
        call,
        "scene.get",
        json!({"sceneId":scene,"limit":1}),
    )?;
    if state["draft"].as_str().is_some_and(|s| !s.is_empty()) {
        return Err(failure(
            ErrorCode::Busy,
            "Scene has an unsent draft; finish it in Mewu or start a new chat",
        ));
    }
    if state["run"]["status"] == "running" {
        return Err(failure(
            ErrorCode::Busy,
            "Scene already has a running answer",
        ));
    }
    let mut context = json!({"sceneId":scene});
    if session.first {
        if let Some(revision) = call.request.params.pointer("/context/revision") {
            context["revision"] = revision.clone();
        }
    }
    let id = if session.first {
        call.request.id.clone()
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    session.first = false;
    ui.progress("正在发送").map_err(io_failure)?;
    let sent = operation(
        control,
        clock,
        call,
        "chat.send",
        json!({"text":text,"refs":[],"expectedDraft":"","expectedRefs":state["refs"],"context":context}),
        Some(id),
    )?;
    let run = sent["runId"]
        .as_str()
        .filter(|s| identifier(s))
        .ok_or_else(|| {
            Response::error(
                call.request.id.clone(),
                ControlError::unknown(ErrorCode::InvalidResponse, "Missing accepted run identity"),
            )
        })?
        .to_owned();
    let started = clock.now();
    let mut printed = String::new();
    let mut canceled = false;
    loop {
        if clock.now().saturating_sub(started) >= call.wait_timeout {
            return Err(Response::error(
                run.clone(),
                ControlError::unknown(
                    ErrorCode::Timeout,
                    "Answer still running; inspect this scene/run without resending the message",
                ),
            ));
        }
        if !canceled
            && ui.interrupted().map_err(|_| {
                Response::error(
                    run.clone(),
                    ControlError::unknown(
                        ErrorCode::Internal,
                        "Input interrupted after message submission; inspect the original run",
                    ),
                )
            })?
        {
            // Cancellation is also a single operation and is fenced to this run.
            let cancellation = operation(
                control,
                clock,
                call,
                "chat.cancel",
                json!({"expectedRunId":run,"context":{"sceneId":scene}}),
                None,
            );
            match cancellation {
                Ok(value) if value["canceled"] == true && value["runId"].as_str() == Some(&run) => {
                    canceled = true
                }
                result => {
                    let check = query(control, call, "run.get", json!({"sceneId":scene,"limit":2}));
                    if !check.as_ref().is_ok_and(|v| {
                        v["run"]["id"].as_str() == Some(&run)
                            && matches!(
                                v["run"]["status"].as_str(),
                                Some("completed" | "canceled" | "failed")
                            )
                    }) {
                        return Err(Response::error(
                            run.clone(),
                            ControlError::unknown(
                                result
                                    .err()
                                    .and_then(|r| r.error_ref().map(|e| e.code))
                                    .unwrap_or(ErrorCode::InvalidResponse),
                                "Cancellation not confirmed; inspect the original run",
                            ),
                        ));
                    }
                }
            }
        }
        let view = query(control, call, "run.get", json!({"sceneId":scene,"limit":2})).map_err(
            |response| {
                Response::error(
                    run.clone(),
                    ControlError::unknown(
                        response.error_ref().map_or(ErrorCode::Internal, |e| e.code),
                        "Message was submitted; query the original scene/run without resending",
                    ),
                )
            },
        )?;
        if view["sceneId"].as_str() != Some(&scene) || view["run"]["id"].as_str() != Some(&run) {
            return Err(Response::error(
                run.clone(),
                ControlError::unknown(
                    ErrorCode::Conflict,
                    "Run was replaced; the message was not repeated",
                ),
            ));
        }
        let status = view["run"]["status"].as_str().ok_or_else(|| {
            Response::error(
                run.clone(),
                ControlError::unknown(ErrorCode::InvalidResponse, "Missing run status"),
            )
        })?;
        let final_text = view["messages"]
            .as_array()
            .and_then(|rows| {
                rows.iter()
                    .rev()
                    .find(|m| m["role"] == "assistant" && m["runId"].as_str() == Some(&run))
            })
            .and_then(|m| m["text"].as_str());
        // Old hosts omit stream; they still return the authoritative final reply.
        let visible = if status == "running" {
            view.pointer("/stream/text")
                .and_then(Value::as_str)
                .or(final_text)
        } else {
            final_text
        };
        if let Some(visible) = visible {
            if visible != printed {
                let (delta, replacement) = match visible.strip_prefix(&printed) {
                    Some(delta) => (delta, false),
                    None => (visible, true),
                };
                ui.delta(delta, replacement).map_err(|_| {
                    Response::error(
                        run.clone(),
                        ControlError::unknown(
                            ErrorCode::Internal,
                            "Reply output interrupted; inspect the original run",
                        ),
                    )
                })?;
                printed = visible.to_owned();
            }
        }
        if call.jsonl {
            let progress = Response::success(
                call.request.id.clone(),
                json!({"sceneId":scene,"runId":run,"status":status,"text":printed}),
            );
            emit(&mut io::stdout().lock(), &progress).map_err(|_| {
                Response::error(
                    run.clone(),
                    ControlError::unknown(
                        ErrorCode::Internal,
                        "Progress output interrupted; inspect the original run",
                    ),
                )
            })?;
        }
        if status != "running" {
            if !matches!(status, "completed" | "failed" | "canceled") {
                return Err(Response::error(
                    run.clone(),
                    ControlError::unknown(ErrorCode::InvalidResponse, "Invalid run status"),
                ));
            }
            if status == "completed" && final_text.is_none() {
                return Err(Response::error(
                    run.clone(),
                    ControlError::unknown(ErrorCode::InvalidResponse, "Missing final answer"),
                ));
            }
            return Ok(Answer {
                scene,
                run,
                status: status.into(),
                text: printed,
            });
        }
        let label = view["run"]["steps"]
            .as_array()
            .and_then(|steps| steps.iter().rev().find(|s| s["status"] == "running"))
            .and_then(|s| s["label"].as_str())
            .unwrap_or("思考中");
        ui.progress(if canceled { "正在停止" } else { label })
            .map_err(|_| {
                Response::error(
                    run.clone(),
                    ControlError::unknown(
                        ErrorCode::Internal,
                        "Progress display interrupted; inspect the original run",
                    ),
                )
            })?;
        clock.sleep(POLL_INTERVAL);
    }
}

fn report(response: Response, call: &Invocation) -> i32 {
    let code = exit_code(&response);
    if call.output == Some(OutputFormat::Json) || call.jsonl {
        if emit(&mut io::stdout().lock(), &response).is_err() {
            eprintln!("{}", stderr_message(74));
            return 74;
        }
    } else {
        terminal::error(presentation::error_message(code));
        if code == 6 {
            eprintln!("请求 / 任务 ID: {}", presentation::safe_text(&response.id));
        } else if response
            .error_ref()
            .is_some_and(|e| e.code == ErrorCode::Busy)
        {
            eprintln!(
                "请检查 Mewu 的 Agent / 模型配置，或该会话是否有未发送草稿、正在进行的操作。"
            );
        }
    }
    code
}
fn print_profiles(
    control: &mut impl Control,
    call: &Invocation,
    method: &str,
) -> Result<(), Response> {
    let rows = query(control, call, method, json!({}))?;
    if let Some(rows) = rows.as_array() {
        for row in rows {
            println!(
                "  {}  {}",
                presentation::safe_text(row["id"].as_str().unwrap_or_default()),
                presentation::safe_text(
                    row["name"]
                        .as_str()
                        .or(row["model"].as_str())
                        .unwrap_or_default()
                )
            );
        }
    }
    Ok(())
}
fn identity(
    control: &mut impl Control,
    call: &Invocation,
    scene: Option<&str>,
) -> (String, String) {
    let found = (|| -> Result<(String, String), Response> {
        let status = query(control, call, "app.status", json!({}))?;
        let id = scene
            .or_else(|| status["activeSceneId"].as_str())
            .ok_or_else(|| failure(ErrorCode::Unavailable, "No active scene"))?;
        let state = query(control, call, "scene.get", json!({"sceneId":id,"limit":1}))?;
        let agents = query(control, call, "agent.list", json!({}))?;
        let agent = agents
            .as_array()
            .and_then(|rows| rows.iter().find(|v| v["id"] == state["agentId"]));
        let models = query(control, call, "model.list", json!({}))?;
        let model = models.as_array().and_then(|rows| {
            rows.iter()
                .find(|v| state["connectionId"].is_string() && v["id"] == state["connectionId"])
        });
        Ok((
            agent
                .and_then(|v| v["name"].as_str())
                .unwrap_or("Mewu Agent")
                .into(),
            model
                .and_then(|v| v["model"].as_str())
                .unwrap_or("Mewu 当前连接")
                .into(),
        ))
    })();
    found.unwrap_or_else(|_| ("Mewu Agent".into(), "Mewu 当前连接".into()))
}

pub(super) fn run(call: &Invocation, server: Option<&Path>) -> i32 {
    let interactive = call.prompt.is_none();
    if interactive && (!io::stdin().is_terminal() || !io::stdout().is_terminal()) {
        terminal::error("连续对话需要终端。可用 mewu chat \"问题\"，或通过 --stdin 读取文字。");
        return 2;
    }
    let mut control = match connect_control(server, call.timeout) {
        Ok(c) => c,
        Err(e) => return report(Response::error("chat", e), call),
    };
    let mut clock = RealClock(Instant::now());
    let mut session = Session::new(call);
    if !interactive {
        let mut ui =
            match terminal::Turn::new(call.output != Some(OutputFormat::Json) && !call.jsonl) {
                Ok(ui) => ui,
                Err(e) => return report(io_failure(e), call),
            };
        let answer = turn(
            &mut control,
            &mut clock,
            call,
            &mut session,
            call.prompt.as_ref().unwrap(),
            &mut ui,
        );
        drop(ui);
        return match answer {
            Err(e) => report(e, call),
            Ok(a) => {
                if call.output == Some(OutputFormat::Json) {
                    if emit(
                        &mut io::stdout().lock(),
                        &Response::success(call.request.id.clone(), a.value()),
                    )
                    .is_err()
                    {
                        eprintln!("{}", stderr_message(74));
                        return 74;
                    }
                } else if !call.jsonl && io::stdout().is_terminal() {
                    terminal::resume(&a.scene);
                }
                match a.status.as_str() {
                    "completed" => 0,
                    "canceled" => 130,
                    _ => {
                        terminal::error("AI 回复失败，可在 Mewu 中查看该会话。");
                        1
                    }
                }
            }
        };
    }
    let (agent, model) = identity(&mut control, call, session.scene.as_deref());
    terminal::welcome(&agent, &model);
    // The server intentionally limits each frame to 10 s. Do not keep an idle
    // connection/worker while a human is composing; verify a fresh peer for
    // each explicit input instead of extending deadlines or replaying a call.
    drop(control);
    let config = Config::builder()
        .max_history_size(256)
        .unwrap()
        .history_ignore_space(true)
        .build();
    let mut editor = match DefaultEditor::with_config(config) {
        Ok(e) => e,
        Err(_) => {
            terminal::error("无法初始化终端输入。");
            return 1;
        }
    };
    loop {
        let prompt = ("› ".to_owned(), terminal::prompt());
        let line = match editor.readline(&prompt) {
            Ok(s) => s,
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
            Err(_) => {
                terminal::error("无法读取终端输入。");
                break;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let _ = editor.add_history_entry(line);
        let command = line
            .split_once(' ')
            .map_or((line, ""), |(a, b)| (a, b.trim()));
        match command {
            ("/exit" | "/quit", "") => break,
            ("/help", "") => {
                terminal::commands();
                continue;
            }
            ("/new", "") => {
                session.scene = None;
                session.activate = false;
                terminal::notice("新会话，下条消息开始。");
                continue;
            }
            _ => {}
        }
        let mut control = match connect_control(server, call.timeout) {
            Ok(c) => c,
            Err(e) => {
                report(Response::error("chat", e), call);
                continue;
            }
        };
        match command {
            ("/clear", "") => {
                terminal::clear();
                let (a, m) = identity(&mut control, call, session.scene.as_deref());
                terminal::welcome(&a, &m);
                continue;
            }
            ("/session", "") => {
                println!("{}", session.scene.as_deref().unwrap_or("尚未发送消息"));
                continue;
            }
            ("/agent", "") | ("/model", "") => {
                if let Err(e) = print_profiles(
                    &mut control,
                    call,
                    if command.0 == "/agent" {
                        "agent.list"
                    } else {
                        "model.list"
                    },
                ) {
                    report(e, call);
                }
                continue;
            }
            ("/agent", name) | ("/model", name) => {
                if command.0 == "/agent" {
                    session.agent = Some(name.into());
                } else {
                    session.model = Some(name.into());
                }
                match ensure_scene(&mut control, &mut clock, call, &mut session) {
                    Ok(_) => {
                        let (a, m) = identity(&mut control, call, session.scene.as_deref());
                        terminal::notice(&format!("{a} · {m}"));
                    }
                    Err(e) => {
                        report(e, call);
                    }
                }
                continue;
            }
            ("/use", id) if identifier(id) => {
                session.scene = Some(id.into());
                session.activate = true;
                session.agent = None;
                session.model = None;
                match ensure_scene(&mut control, &mut clock, call, &mut session) {
                    Ok(_) => {
                        let (a, m) = identity(&mut control, call, session.scene.as_deref());
                        terminal::notice(&format!("{a} · {m}"));
                    }
                    Err(e) => {
                        report(e, call);
                    }
                }
                continue;
            }
            _ if line.starts_with('/') => {
                terminal::notice("未知命令，输入 /help 查看。");
                continue;
            }
            _ => {}
        }
        let mut ui = match terminal::Turn::new(true) {
            Ok(ui) => ui,
            Err(e) => {
                report(io_failure(e), call);
                continue;
            }
        };
        let answer = turn(&mut control, &mut clock, call, &mut session, line, &mut ui);
        drop(ui);
        match answer {
            Ok(a) if a.status == "failed" => {
                terminal::error("AI 回复失败，可在 Mewu 中查看该会话。")
            }
            Ok(a) if a.status == "canceled" => terminal::notice("已停止。"),
            Ok(_) => {}
            Err(e) => {
                report(e, call);
            }
        }
    }
    0
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
