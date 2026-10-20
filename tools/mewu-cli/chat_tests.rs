// SPDX-License-Identifier: MPL-2.0
use super::*;
#[derive(Default)]
struct FakeClock(Duration);
impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.0
    }
    fn sleep(&mut self, d: Duration) {
        self.0 += d;
    }
}
struct FakeHost {
    calls: Vec<Request>,
    scene: String,
    run: String,
    polls: usize,
    turns: usize,
    cancelled: bool,
    old_host: bool,
    disconnect: bool,
    foreign: bool,
    draft: String,
}
impl Default for FakeHost {
    fn default() -> Self {
        Self {
            calls: vec![],
            scene: "original".into(),
            run: String::new(),
            polls: 0,
            turns: 0,
            cancelled: false,
            old_host: false,
            disconnect: false,
            foreign: false,
            draft: String::new(),
        }
    }
}
impl Control for FakeHost {
    fn call(&mut self, r: &Request, _: Duration) -> Result<Response, ControlError> {
        self.calls.push(r.clone());
        let value = match r.method.as_str() {
            "app.status" => json!({"activeSceneId":self.scene}),
            "scene.new" => {
                self.scene = "cli-scene".into();
                json!({"operationId":r.id,"status":"completed","result":{"sceneId":self.scene}})
            }
            "scene.activate" => {
                self.scene = r.params["sceneId"].as_str().unwrap().into();
                json!({"operationId":r.id,"status":"completed","result":{"sceneId":self.scene}})
            }
            "scene.get" => json!({"sceneId":self.scene,"draft":self.draft,"refs":[],"run":null}),
            "chat.send" => {
                self.turns += 1;
                self.polls = 0;
                self.cancelled = false;
                self.run = format!("run-{}", self.turns);
                json!({"operationId":r.id,"status":"completed","result":{"runId":self.run}})
            }
            "chat.cancel" => {
                assert_eq!(r.params["expectedRunId"], self.run);
                assert_eq!(r.params["context"]["sceneId"], self.scene);
                self.cancelled = true;
                json!({"operationId":r.id,"status":"completed","result":{"runId":self.run,"canceled":true}})
            }
            "run.get" => {
                if self.disconnect {
                    return Err(unavailable());
                }
                self.polls += 1;
                let status = if self.cancelled {
                    "canceled"
                } else if self.polls < 3 {
                    "running"
                } else {
                    "completed"
                };
                let mut v = json!({"sceneId":self.scene,"run":{"id":if self.foreign{"other-run"}else{&self.run},"status":status,"steps":[]},"messages":[]});
                if status == "completed" {
                    v["messages"] =
                        json!([{"role":"assistant","runId":self.run,"text":"Hello world"}]);
                } else if !self.old_host {
                    v["stream"] = json!({"text":if self.polls==1{"Hello"}else{"Hello wor"},"truncated":false});
                }
                v
            }
            _ => panic!("unexpected method {}", r.method),
        };
        Ok(Response::success(r.id.clone(), value))
    }
}
#[derive(Default)]
struct Ui {
    text: String,
    updates: Vec<String>,
    cancel: bool,
}
impl ChatUi for Ui {
    fn progress(&mut self, label: &str) -> io::Result<()> {
        self.updates.push(label.into());
        Ok(())
    }
    fn delta(&mut self, text: &str, replacement: bool) -> io::Result<()> {
        if replacement {
            self.text.clear();
        }
        self.text.push_str(text);
        Ok(())
    }
    fn interrupted(&mut self) -> io::Result<bool> {
        Ok(std::mem::take(&mut self.cancel))
    }
}
fn call(args: &[&str]) -> Invocation {
    parse(
        std::iter::once("mewu")
            .chain(args.iter().copied())
            .map(OsString::from)
            .collect(),
        &mut io::empty(),
    )
    .unwrap()
}

#[test]
fn chat_parses_plain_prompts_stdin_and_explicit_profiles_without_json_input() {
    let c = call(&[
        "chat", "你好", "世界", "--agent", "助理", "--model", "Flash",
    ]);
    assert!(matches!(c.action, Action::Chat));
    assert_eq!(c.prompt.as_deref(), Some("你好 世界"));
    assert_eq!(c.agent.as_deref(), Some("助理"));
    assert_eq!(presentation::output_format(&c, false), OutputFormat::Text);
    let c = call(&["chat", "--", "--help"]);
    assert_eq!(c.prompt.as_deref(), Some("--help"));
    let c = parse(
        vec!["mewu".into(), "chat".into(), "--stdin".into()],
        &mut "不是 JSON\n第二行".as_bytes(),
    )
    .unwrap();
    assert_eq!(c.prompt.as_deref(), Some("不是 JSON\n第二行"));
    for args in [
        vec!["chat", "--output", "json"],
        vec!["chat", "message", "--stdin"],
        vec!["chat", "--json", "{}"],
        vec!["status", "--agent", "id"],
        vec!["chat", "--revision", "1"],
    ] {
        assert!(parse(
            std::iter::once("mewu")
                .chain(args)
                .map(OsString::from)
                .collect(),
            &mut io::empty()
        )
        .is_err());
    }
    assert!(validate_prompt(&"x".repeat(128 * 1024 + 1)).is_err());
    assert!(validate_prompt("\u{0000}").is_err());
    assert!(matches!(call(&["chat", "--help"]).action, Action::Help));
}
#[test]
fn actual_orchestration_uses_one_scene_once_per_turn_and_streams_the_owned_run() {
    let c = call(&["chat", "hi"]);
    let mut h = FakeHost::default();
    let mut s = Session::new(&c);
    let mut clock = FakeClock::default();
    let mut ui = Ui::default();
    let answer = turn(&mut h, &mut clock, &c, &mut s, "hi", &mut ui).unwrap();
    assert_eq!(answer.text, "Hello world");
    assert_eq!(ui.text, "Hello world");
    assert_eq!(answer.status, "completed");
    assert_eq!(
        h.calls.iter().filter(|r| r.method == "scene.new").count(),
        1
    );
    assert_eq!(
        h.calls.iter().filter(|r| r.method == "chat.send").count(),
        1
    );
    let sent = h.calls.iter().find(|r| r.method == "chat.send").unwrap();
    assert_eq!(sent.id, c.request.id);
    assert_eq!(sent.params["refs"], json!([]));
    assert_eq!(sent.params["expectedDraft"], "");
    assert_eq!(sent.params["context"]["sceneId"], "cli-scene");
    turn(&mut h, &mut clock, &c, &mut s, "second", &mut Ui::default()).unwrap();
    assert_eq!(
        h.calls.iter().filter(|r| r.method == "scene.new").count(),
        1
    );
    let ids: Vec<_> = h
        .calls
        .iter()
        .filter(|r| r.method == "chat.send")
        .map(|r| &r.id)
        .collect();
    assert_ne!(ids[0], ids[1]);
}
#[test]
fn old_hosts_return_final_text_and_do_not_require_a_new_stream_api() {
    let c = call(&["chat", "hi"]);
    let mut h = FakeHost {
        old_host: true,
        ..Default::default()
    };
    let mut s = Session::new(&c);
    let mut ui = Ui::default();
    let a = turn(&mut h, &mut FakeClock::default(), &c, &mut s, "hi", &mut ui).unwrap();
    assert_eq!(a.text, "Hello world");
    assert_eq!(ui.text, "Hello world");
}
#[test]
fn post_submission_failures_and_foreign_runs_are_unknown_and_never_resubmitted() {
    for foreign in [false, true] {
        let c = call(&["chat", "hi"]);
        let mut h = FakeHost {
            foreign,
            disconnect: !foreign,
            ..Default::default()
        };
        let mut s = Session::new(&c);
        let err = turn(
            &mut h,
            &mut FakeClock::default(),
            &c,
            &mut s,
            "hi",
            &mut Ui::default(),
        )
        .unwrap_err();
        assert_eq!(err.error_ref().unwrap().outcome, ExecutionOutcome::Unknown);
        assert_eq!(err.id, "run-1");
        assert_eq!(
            h.calls.iter().filter(|r| r.method == "chat.send").count(),
            1
        );
    }
}
#[test]
fn cancellation_targets_only_the_confirmed_run_and_drafts_are_not_overwritten() {
    let c = call(&["chat", "hi"]);
    let mut h = FakeHost::default();
    let mut s = Session::new(&c);
    let a = turn(
        &mut h,
        &mut FakeClock::default(),
        &c,
        &mut s,
        "hi",
        &mut Ui {
            cancel: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(a.status, "canceled");
    assert_eq!(
        h.calls.iter().filter(|r| r.method == "chat.cancel").count(),
        1
    );
    let mut h = FakeHost {
        draft: "user's unsubmitted input".into(),
        ..Default::default()
    };
    let mut s = Session::new(&c);
    assert!(turn(
        &mut h,
        &mut FakeClock::default(),
        &c,
        &mut s,
        "hi",
        &mut Ui::default()
    )
    .is_err());
    assert!(!h.calls.iter().any(|r| r.method == "chat.send"));
}
#[test]
fn desktop_scene_changes_are_not_silently_activated_on_later_turns() {
    let c = call(&["chat", "hi"]);
    let mut h = FakeHost::default();
    let mut s = Session::new(&c);
    turn(
        &mut h,
        &mut FakeClock::default(),
        &c,
        &mut s,
        "hi",
        &mut Ui::default(),
    )
    .unwrap();
    h.scene = "human-selected".into();
    let err = turn(
        &mut h,
        &mut FakeClock::default(),
        &c,
        &mut s,
        "next",
        &mut Ui::default(),
    )
    .unwrap_err();
    assert_eq!(err.error_ref().unwrap().code, ErrorCode::Conflict);
    assert!(!h.calls.iter().any(|r| r.method == "scene.activate"));
    assert_eq!(h.turns, 1);
}
#[test]
fn ambiguous_profile_names_are_not_guessed() {
    let rows = json!([{"id":"one","name":"Test"},{"id":"two","name":"Test"}]);
    assert!(choose_profile(&rows, "test").is_err());
    assert_eq!(choose_profile(&rows, "one").unwrap(), "one");
}

#[test]
fn terminal_failure_after_sending_preserves_unknown_execution() {
    struct BrokenUi;
    impl ChatUi for BrokenUi {
        fn progress(&mut self, _: &str) -> io::Result<()> {
            Ok(())
        }
        fn delta(&mut self, _: &str, _: bool) -> io::Result<()> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn interrupted(&mut self) -> io::Result<bool> {
            Ok(false)
        }
    }
    let c = call(&["chat", "hi"]);
    let mut h = FakeHost::default();
    let mut s = Session::new(&c);
    let err = turn(
        &mut h,
        &mut FakeClock::default(),
        &c,
        &mut s,
        "hi",
        &mut BrokenUi,
    )
    .unwrap_err();
    assert_eq!(err.error_ref().unwrap().outcome, ExecutionOutcome::Unknown);
    assert_eq!(
        h.calls.iter().filter(|r| r.method == "chat.send").count(),
        1
    );
}

#[test]
fn final_render_failure_is_unknown_without_repeating_the_message() {
    struct BrokenFinish;
    impl ChatUi for BrokenFinish {
        fn progress(&mut self, _: &str) -> io::Result<()> {
            Ok(())
        }
        fn delta(&mut self, _: &str, _: bool) -> io::Result<()> {
            Ok(())
        }
        fn interrupted(&mut self) -> io::Result<bool> {
            Ok(false)
        }
        fn finish(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }
    let c = call(&["chat", "hi"]);
    let mut h = FakeHost::default();
    let mut s = Session::new(&c);
    let err = turn(
        &mut h,
        &mut FakeClock::default(),
        &c,
        &mut s,
        "hi",
        &mut BrokenFinish,
    )
    .unwrap_err();
    assert_eq!(err.error_ref().unwrap().outcome, ExecutionOutcome::Unknown);
    assert_eq!(
        h.calls.iter().filter(|r| r.method == "chat.send").count(),
        1
    );
}
