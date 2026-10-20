// SPDX-License-Identifier: MPL-2.0
use super::*;
use std::collections::VecDeque;

fn arguments(values: &[&str]) -> Vec<OsString> {
    std::iter::once("mewu")
        .chain(values.iter().copied())
        .map(OsString::from)
        .collect()
}
fn invocation(values: &[&str]) -> Invocation {
    parse(arguments(values), &mut io::empty()).unwrap()
}
struct NoInput;
impl Read for NoInput {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("offline command must not read input")
    }
}

#[test]
fn offline_help_and_version_require_no_input_or_host() {
    for values in [&[][..], &["help"][..], &["--help"][..], &["-h"][..]] {
        assert!(matches!(
            parse(arguments(values), &mut NoInput).unwrap().action,
            Action::Help
        ));
    }
    for values in [&["version"][..], &["--version"][..], &["-V"][..]] {
        assert!(matches!(
            parse(arguments(values), &mut NoInput).unwrap().action,
            Action::Version
        ));
    }
}

#[test]
fn human_help_is_concise_offline_and_supports_real_subcommands() {
    for values in [
        vec![],
        vec!["--help"],
        vec!["help", "scene"],
        vec!["scene"],
        vec!["scene", "list", "--help"],
        vec!["scene", "get", "--help"],
        vec!["run", "get", "example", "-h"],
        vec!["call"],
        vec!["call", "--help"],
        vec!["call", "capture.open", "--help"],
        vec!["help", "status"],
    ] {
        let call = parse(arguments(&values), &mut NoInput).unwrap();
        assert!(matches!(call.action, Action::Help));
        assert_eq!(
            presentation::output_format(&call, false),
            OutputFormat::Text
        );
        let usage = presentation::help(call.help_topic);
        let response = Response::success("offline", json!({"usage":usage}));
        let mut output = Vec::new();
        presentation::emit_text(&mut output, "help", &response).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.starts_with("MewuAI ") && text.contains("用法\n"));
        assert!(!text.contains("\"result\"") && !text.contains("\\n"));
        assert!(text.lines().count() <= 32);
    }
    assert!(
        presentation::help(invocation(&["scene", "--help"]).help_topic).contains("mewu scene get")
    );
    assert!(presentation::help(invocation(&["call", "--help"]).help_topic).contains("--file PATH"));
    for values in [
        vec!["help", "not-a-command"],
        vec!["scene", "bad", "--help"],
        vec!["call", "--help", "--stdin"],
        vec!["--help", "-h"],
    ] {
        assert!(parse(arguments(&values), &mut NoInput).is_err());
    }
}

#[test]
fn terminal_views_and_machine_output_have_explicit_and_compatible_selection() {
    for command in [
        vec!["status"],
        vec!["scene", "list"],
        vec!["scene", "get", "id"],
        vec!["run", "get", "id"],
        vec!["capabilities"],
    ] {
        let call = invocation(&command);
        assert_eq!(presentation::output_format(&call, true), OutputFormat::Text);
        assert_eq!(
            presentation::output_format(&call, false),
            OutputFormat::Json
        );
    }
    for command in [
        vec!["schema"],
        vec!["call", "capture.open"],
        vec!["call", "app.status"],
        vec!["help", "--output", "json"],
        vec!["status", "--jsonl"],
    ] {
        let call = invocation(&command);
        assert_eq!(presentation::output_format(&call, true), OutputFormat::Json);
        assert_eq!(
            presentation::output_format(&call, false),
            OutputFormat::Json
        );
    }
    for terminal in [true, false] {
        let call = invocation(&["status", "--output", "text"]);
        assert_eq!(
            presentation::output_format(&call, terminal),
            OutputFormat::Text
        );
        let call = invocation(&["status", "--output", "json"]);
        assert_eq!(
            presentation::output_format(&call, terminal),
            OutputFormat::Json
        );
    }
    for args in [
        vec!["status", "--output", "yaml"],
        vec!["status", "--output"],
        vec!["status", "--output", "text", "--jsonl"],
        vec!["status", "--output", "json", "--output", "json"],
    ] {
        assert!(parse(arguments(&args), &mut io::empty()).is_err());
    }
    // --json remains the input object; selecting output must not consume it.
    let call = invocation(&[
        "call",
        "chat.send",
        "--json",
        r#"{"text":"你好"}"#,
        "--output",
        "text",
    ]);
    assert_eq!(call.request.params, json!({"text":"你好"}));
    assert_eq!(
        presentation::output_format(&call, false),
        OutputFormat::Text
    );
}

#[test]
fn human_views_render_real_results_without_protocol_noise_or_terminal_injection() {
    let mut output = Vec::new();
    presentation::emit_text(
        &mut output,
        "app.status",
        &Response::success(
            "r",
            json!({"version":"1.0.3","platform":"windows","activeSceneId":null,"revision":8}),
        ),
    )
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("运行中") && text.contains("当前会话  —") && !text.contains("\"id\""));
    let mut output = Vec::new();
    presentation::emit_text(&mut output,"scene.list",&Response::success("r",json!({"scenes":[{"id":"scene-id","title":"标题\n\u{1b}[31m\r坏\u{202e}标题","frozen":true,"closed":false}],"nextOffset":20}))).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("已冻结") && text.contains("下一页: mewu scene list --offset 20"));
    assert!(!text.contains('\u{1b}') && !text.contains('\r') && !text.contains('\u{202e}'));
    assert_eq!(text.lines().count(), 4);
    let catalog: Value =
        serde_json::from_str(include_str!("../../apps/desktop/control/catalog.json")).unwrap();
    let mut output = Vec::new();
    presentation::emit_text(
        &mut output,
        "app.capabilities",
        &Response::success("r", catalog),
    )
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains("capture.open")
            && text.contains("recording.start")
            && text.contains("可调用方法（37）")
    );
    let mut output = Vec::new();
    presentation::emit_text(&mut output,"run.get",&Response::success("r",json!({"sceneId":"scene-id","run":{"status":"completed"},"messages":[{"role":"assistant","text":"第一行\n第二行","reasoning":"do not show this"}]}))).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains("已完成")
            && text.contains("AI\n第一行\n第二行")
            && !text.contains("do not show this")
    );
    let mut output = Vec::new();
    presentation::emit_text(
        &mut output,
        "app.status",
        &Response::error("r", unavailable()),
    )
    .unwrap();
    assert!(output.is_empty());
}

#[test]
fn convenience_queries_generate_the_actual_server_contract() {
    assert_eq!(invocation(&["status"]).request.method, "app.status");
    assert_eq!(invocation(&["schema"]).request.method, "app.schema");
    assert_eq!(
        invocation(&["capabilities"]).request.method,
        "app.capabilities"
    );
    let list = invocation(&["scene", "list", "--offset", "10", "--limit", "5"]);
    assert_eq!(list.request.method, "scene.list");
    assert_eq!(list.request.params, json!({"offset":10,"limit":5}));
    let scene = invocation(&["scene", "get", "scene-a", "--limit", "2"]);
    assert_eq!(scene.request.method, "scene.get");
    assert_eq!(scene.request.params, json!({"sceneId":"scene-a","limit":2}));
    let run = invocation(&["run", "get", "scene-a"]);
    assert_eq!(run.request.method, "run.get");
    assert_eq!(run.request.params, json!({"sceneId":"scene-a"}));
}

#[test]
fn context_fills_only_missing_fields_and_preserves_explicit_target_and_revision() {
    let call = invocation(&[
        "call",
        "chat.send",
        "--json",
        r#"{"text":"hello","context":{"sceneId":"original","revision":2}}"#,
        "--scene",
        "different",
        "--revision",
        "9",
        "--request-id",
        "agent-cycle-001",
    ]);
    assert_eq!(call.request.id, "agent-cycle-001");
    assert_eq!(
        call.request.params,
        json!({"text":"hello","context":{"sceneId":"original","revision":2}})
    );
    let partial = invocation(&[
        "call",
        "chat.send",
        "--json",
        r#"{"context":{"sceneId":"original"}}"#,
        "--revision",
        "3",
    ]);
    assert_eq!(
        partial.request.params,
        json!({"context":{"sceneId":"original","revision":3}})
    );
    assert_eq!(
        invocation(&["call", "scene.freeze", "--scene", "a", "--revision", "0"])
            .request
            .params,
        json!({"context":{"sceneId":"a","revision":0}})
    );
}

#[test]
fn invalid_flags_extra_arguments_and_ambiguous_json_are_rejected_without_echoing_input() {
    for values in [
        vec!["call", "chat.send", "--wait", "--wait"],
        vec!["status", "--request-id", "one", "--request-id", "two"],
        vec!["status", "--unknown"],
        vec!["status", "extra"],
        vec!["call", "chat.send", "--json", "{}", "--stdin"],
        vec!["call", "chat.send", "--file", "x", "--json", "{}"],
        vec![
            "call",
            "chat.send",
            "--json",
            r#"{"context":{"revision":1,"revision":2}}"#,
        ],
        vec!["call", "chat.send", "--json", r#"{"a":[{"x":1,"x":2}]}"#],
        vec![
            "call",
            "chat.send",
            "--json",
            r#"{"context":null}"#,
            "--scene",
            "a",
        ],
        vec!["call", "chat.send", "--json", "[]"],
        vec!["call", "chat.send", "--json", "{} {}"],
        vec!["status", "--timeout-ms", "0"],
        vec!["status", "--timeout-ms", "3600001"],
        vec!["status", "--request-id", "invalid id"],
        vec!["status", "--revision", "9007199254740992"],
        vec!["status", "--revision", "-1"],
        vec!["status", "--revision", "1.0"],
        vec!["status", "--wait-timeout-ms", "10"],
        vec!["help", "--wait"],
        vec!["scene", "get"],
        vec!["scene", "list", "extra"],
    ] {
        let error = parse(arguments(&values), &mut io::empty()).unwrap_err();
        assert_eq!(error.outcome, ExecutionOutcome::NotSent);
    }
    let error = parse(
        arguments(&["call", "agent.list", "--file", "PRIVATE-NOT-A-REAL-FILE"]),
        &mut io::empty(),
    )
    .unwrap_err();
    assert!(!error.message.contains("PRIVATE"));
    assert!(!error.message.contains("os error"));
}

struct InputFile(PathBuf);
impl InputFile {
    fn new(bytes: &[u8]) -> Self {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.private/remake-review/mewu-cli-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&parent).unwrap();
        let path = parent.join("request.json");
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }
}
impl Drop for InputFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_dir(self.0.parent().unwrap());
    }
}

#[test]
fn stdin_and_utf8_file_use_bounded_json_and_enforce_the_final_frame_limit() {
    let call = parse(
        arguments(&["call", "chat.send", "--stdin"]),
        &mut r#"{"text":"你好"}"#.as_bytes(),
    )
    .unwrap();
    assert_eq!(call.request.params, json!({"text":"你好"}));
    let file = InputFile::new(br#"{"context":{"revision":8}}"#);
    let call = invocation(&["call", "scene.freeze", "--file", file.0.to_str().unwrap()]);
    assert_eq!(call.request.params, json!({"context":{"revision":8}}));
    let large = InputFile::new(&vec![b' '; MAX_FRAME_BYTES + 1]);
    assert_eq!(
        parse(
            arguments(&["call", "app.status", "--file", large.0.to_str().unwrap()]),
            &mut io::empty()
        )
        .unwrap_err()
        .code,
        ErrorCode::FrameTooLarge
    );
    let full_json = format!(r#"{{"x":"{}"}}"#, "a".repeat(MAX_FRAME_BYTES - 8));
    assert_eq!(full_json.len(), MAX_FRAME_BYTES);
    assert!(bounded_json(full_json.as_bytes()).is_ok());
    let full = InputFile::new(full_json.as_bytes());
    assert_eq!(
        parse(
            arguments(&["call", "app.status", "--file", full.0.to_str().unwrap()]),
            &mut io::empty()
        )
        .unwrap_err()
        .code,
        ErrorCode::FrameTooLarge
    );
}

#[derive(Default)]
struct FakeClock(Duration);
impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.0
    }
    fn sleep(&mut self, duration: Duration) {
        self.0 += duration;
    }
}
struct LocalDispatch {
    requests: Vec<Request>,
    replies: VecDeque<Result<Value, ControlError>>,
}
impl LocalDispatch {
    fn new(replies: Vec<Result<Value, ControlError>>) -> Self {
        Self {
            requests: vec![],
            replies: replies.into(),
        }
    }
}
impl Control for LocalDispatch {
    fn call(&mut self, request: &Request, _: Duration) -> Result<Response, ControlError> {
        request.validate().unwrap();
        self.requests.push(request.clone());
        self.replies
            .pop_front()
            .expect("unexpected extra dispatch")
            .map(|value| Response::success(request.id.clone(), value))
    }
}

#[test]
fn wait_follows_real_receipts_with_queries_and_never_replays_the_mutation() {
    let mut control = LocalDispatch::new(vec![
        Ok(json!({"operationId":"operation-a","status":"queued"})),
        Ok(json!({"operationId":"operation-a","status":"running"})),
        Ok(json!({"operationId":"operation-a","status":"completed","result":{"sent":true}})),
    ]);
    let mut output = Vec::new();
    let result = dispatch(
        &invocation(&[
            "call",
            "chat.send",
            "--wait",
            "--jsonl",
            "--json",
            r#"{"text":"hello"}"#,
        ]),
        &mut control,
        &mut FakeClock::default(),
        |response| emit(&mut output, response),
    )
    .unwrap();
    assert_eq!(exit_code(&result), 0);
    assert_eq!(
        control
            .requests
            .iter()
            .map(|request| request.method.as_str())
            .collect::<Vec<_>>(),
        ["chat.send", "operation.get", "operation.get"]
    );
    assert_eq!(control.requests[1].params, json!({"id":"operation-a"}));
    assert_eq!(control.requests[2].params, json!({"id":"operation-a"}));
    let rows = std::str::from_utf8(&output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["result"]["status"], "queued");
    assert_eq!(rows[1]["result"]["status"], "running");
    assert_eq!(rows[2]["result"]["status"], "completed");
}

#[test]
fn ordinary_output_is_one_response_and_queued_receipt_does_not_poll_without_wait() {
    let mut control = LocalDispatch::new(vec![Ok(
        json!({"operationId":"operation-a","status":"queued"}),
    )]);
    let mut output = Vec::new();
    let result = dispatch(
        &invocation(&["call", "capture.open"]),
        &mut control,
        &mut FakeClock::default(),
        |response| emit(&mut output, response),
    )
    .unwrap();
    assert!(output.is_empty());
    emit(&mut output, &result).unwrap();
    assert_eq!(std::str::from_utf8(&output).unwrap().lines().count(), 1);
    assert_eq!(control.requests.len(), 1);
    assert_eq!(result.result().unwrap()["status"], "queued");
}

#[test]
fn accepted_duplicate_operation_is_polled_but_failure_is_a_nonzero_exit() {
    let mut control = LocalDispatch::new(vec![
        Ok(json!({"operationId":"operation-a","status":"accepted","duplicate":true})),
        Ok(
            json!({"operationId":"operation-a","status":"failed","error":{"code":"OPERATION_FAILED","message":"not done"}}),
        ),
    ]);
    let result = dispatch(
        &invocation(&["call", "capture.open", "--wait"]),
        &mut control,
        &mut FakeClock::default(),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(exit_code(&result), 1);
    assert_eq!(control.requests.len(), 2);
    assert_eq!(control.requests[1].method, "operation.get");
}

#[test]
fn wait_timeout_or_poll_disconnection_preserves_unknown_and_never_replays() {
    let mut timeout_control = LocalDispatch::new(vec![Ok(
        json!({"operationId":"operation-a","status":"queued"}),
    )]);
    let result = dispatch(
        &invocation(&["call", "chat.send", "--wait", "--wait-timeout-ms", "1"]),
        &mut timeout_control,
        &mut FakeClock::default(),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(exit_code(&result), 6);
    assert_eq!(result.id, "operation-a");
    assert_eq!(result.error_ref().unwrap().code, ErrorCode::Timeout);
    assert_eq!(timeout_control.requests.len(), 1);
    let mut disconnected = LocalDispatch::new(vec![
        Ok(json!({"operationId":"operation-a","status":"queued"})),
        Err(ControlError::not_sent(
            ErrorCode::Unavailable,
            "poll was not sent",
        )),
    ]);
    let result = dispatch(
        &invocation(&["call", "chat.send", "--wait"]),
        &mut disconnected,
        &mut FakeClock::default(),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(exit_code(&result), 6);
    assert_eq!(
        result.error_ref().unwrap().outcome,
        ExecutionOutcome::Unknown
    );
    assert_eq!(
        disconnected
            .requests
            .iter()
            .filter(|request| request.method == "chat.send")
            .count(),
        1
    );
}

#[test]
fn bad_poll_identity_or_incomplete_terminal_receipt_does_not_claim_success() {
    for invalid_result in [
        json!({"operationId":"different","status":"completed","result":null}),
        json!({"operationId":"operation-a","status":"completed"}),
    ] {
        let mut control = LocalDispatch::new(vec![
            Ok(json!({"operationId":"operation-a","status":"queued"})),
            Ok(invalid_result),
        ]);
        let result = dispatch(
            &invocation(&["call", "chat.send", "--wait"]),
            &mut control,
            &mut FakeClock::default(),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(exit_code(&result), 6);
        assert_eq!(result.error_ref().unwrap().code, ErrorCode::InvalidResponse);
    }
}

#[test]
fn real_protocol_errors_have_fixed_exit_classes_and_stdout_write_failures_stop_dispatch() {
    for (error, code) in [
        (
            ControlError::not_sent(ErrorCode::Unavailable, "not running"),
            3,
        ),
        (
            ControlError::not_sent(ErrorCode::PermissionDenied, "denied"),
            4,
        ),
        (ControlError::not_sent(ErrorCode::Timeout, "before send"), 5),
        (ControlError::unknown(ErrorCode::Timeout, "after send"), 6),
    ] {
        assert_eq!(exit_code(&Response::error("request", error)), code);
    }
    let mut control = LocalDispatch::new(vec![Ok(
        json!({"operationId":"operation-a","status":"queued"}),
    )]);
    let failed = dispatch(
        &invocation(&["call", "chat.send", "--wait", "--jsonl"]),
        &mut control,
        &mut FakeClock::default(),
        |_| Err(io::Error::from(io::ErrorKind::BrokenPipe)),
    );
    assert!(failed.is_err());
    assert_eq!(control.requests.len(), 1);
}
