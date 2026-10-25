// SPDX-License-Identifier: MPL-2.0
use super::*;
use chrono::{TimeZone, Utc};
use serde_json::json;
use std::sync::{atomic::AtomicBool, Arc};
fn rule() -> Rule {
    Rule {
        id: uuid::Uuid::new_v4().to_string(),
        revision: 0,
        name: "问号".into(),
        enabled: true,
        trigger: Trigger::Prompt {
            matcher: Matcher {
                mode: MatchMode::Contains,
                value: "？".into(),
                ignore_case: false,
            },
        },
        condition: String::new(),
        actions: vec![Action::InjectPrompt {
            text: "回答前先核对问题".into(),
        }],
        cooldown_ms: 500,
        timeout_ms: 1000,
        stop_on_error: true,
    }
}
#[test]
fn literal_question_marks_are_not_regex_and_input_is_unchanged() {
    let r = rule();
    let mut e = Event::new("prompt.beforeSend");
    e.text = "这是什么？".into();
    assert!(r.matches(&e).unwrap());
    assert_eq!(e.text, "这是什么？");
    e.text = "abc?".into();
    assert!(!r.matches(&e).unwrap());
    let mut ascii = r.clone();
    ascii.trigger = Trigger::Prompt {
        matcher: Matcher {
            mode: MatchMode::Contains,
            value: "?".into(),
            ignore_case: false,
        },
    };
    assert!(ascii.matches(&e).unwrap());
    e.name = "agent.replyCompleted".into();
    assert!(!ascii.matches(&e).unwrap());
}
#[test]
fn case_exact_regex_and_recursive_event_boundaries() {
    let mut r = rule();
    r.trigger = Trigger::Reply {
        matcher: Matcher {
            mode: MatchMode::Regex,
            value: r"^done\b".into(),
            ignore_case: true,
        },
    };
    r.actions = vec![Action::OpenUrl {
        url: "https://example.com".into(),
    }];
    r.validate().unwrap();
    let mut e = Event::new("agent.replyCompleted");
    e.text = "DONE ok".into();
    assert!(r.matches(&e).unwrap());
    e.ancestors.push(r.id.clone());
    assert!(!r.matches(&e).unwrap());
    e.ancestors = vec!["ancestor".into(); 12];
    assert!(
        r.matches(&e).unwrap(),
        "sibling rules do not consume recursion depth"
    );
    e.depth = 8;
    assert!(!r.matches(&e).unwrap());
}
#[test]
fn unsafe_or_ambiguous_configuration_is_rejected_before_saving() {
    let mut r = rule();
    r.actions = vec![Action::OpenUrl {
        url: "javascript:alert(1)".into(),
    }];
    assert!(r.validate().is_err());
    r.actions = vec![Action::Mcp {
        server_id: "server".into(),
        tool: "tool".into(),
        arguments: json!("raw shell"),
    }];
    assert!(r.validate().is_err());
    r.actions = vec![Action::Emit {
        name: "agent.replyCompleted".into(),
        payload: json!({}),
    }];
    assert!(r.validate().is_err());
    r = rule();
    r.trigger = Trigger::Event {
        name: "app.started".into(),
    };
    assert!(
        r.validate().is_err(),
        "injection cannot silently disappear on background events"
    );
    r = rule();
    if let Trigger::Prompt { matcher } = &mut r.trigger {
        matcher.mode = MatchMode::Regex;
        matcher.value = "[".into();
    }
    assert!(r.validate().is_err());
}
#[test]
fn cron_uses_real_timezone_and_does_not_catch_up_missed_work() {
    let now = Utc.with_ymd_and_hms(2026, 10, 26, 0, 30, 0).unwrap();
    assert_eq!(
        next_occurrence("0 9 * * *", "Asia/Singapore", now).unwrap(),
        Utc.with_ymd_and_hms(2026, 10, 26, 1, 0, 0).unwrap()
    );
    let late = Utc.with_ymd_and_hms(2026, 10, 26, 2, 0, 0).unwrap();
    assert_eq!(
        next_occurrence("0 9 * * *", "Asia/Singapore", late).unwrap(),
        Utc.with_ymd_and_hms(2026, 10, 27, 1, 0, 0).unwrap()
    );
    assert!(next_occurrence("0 9 * * *", "made-up-timezone", now).is_err());
    assert!(next_occurrence("not a cron", "UTC", now).is_err());
}
#[test]
fn dst_calendar_is_evaluated_by_the_timezone_library() {
    let before = Utc.with_ymd_and_hms(2026, 3, 7, 14, 0, 0).unwrap();
    assert_eq!(
        next_occurrence("0 9 * * *", "America/New_York", before).unwrap(),
        Utc.with_ymd_and_hms(2026, 3, 8, 13, 0, 0).unwrap()
    );
}
#[test]
fn event_text_is_data_and_script_has_no_io() {
    let mut e = Event::new("custom.test");
    e.text = "'); throw new Error('not executed'); //".into();
    let c = Arc::new(AtomicBool::new(false));
    assert_eq!(
        evaluate("return event.text;", &e, c.clone()).unwrap(),
        json!(e.text)
    );
    assert_eq!(
        evaluate(
            "return [typeof process, typeof require, typeof fetch, typeof setTimeout];",
            &e,
            c.clone()
        )
        .unwrap(),
        json!(["undefined", "undefined", "undefined", "undefined"])
    );
    assert!(evaluate("while(true) {}", &e, c).is_err());
    assert!(evaluate("return true;", &e, Arc::new(AtomicBool::new(true))).is_err());
}
#[test]
fn persistent_cas_and_rule_revisions_prevent_lost_edits() {
    let mut store = Store::memory().unwrap();
    let r = rule();
    let saved = store.save(0, r.clone()).unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.rules[0].revision, 1);
    assert!(store.save(0, r.clone()).is_err());
    assert!(store.save(1, r.clone()).is_err());
    assert!(store.remove(1, &r.id, 0).is_err());
    let mut update = saved.rules[0].clone();
    update.name = "新名称".into();
    let next = store.save(1, update).unwrap();
    assert_eq!(next.rules[0].revision, 2);
    assert_eq!(next.rules[0].name, "新名称");
    assert!(store.remove(1, &r.id, 1).is_err());
    assert_eq!(store.remove(2, &r.id, 2).unwrap().rules.len(), 0);
}
#[test]
fn script_validation_parses_the_body_without_running_or_escaping_it() {
    assert!(validate_script("throw new Error('must not run at save');").is_ok());
    assert!(validate_script("while(true) {}").is_ok());
    assert!(validate_script("}); while(true) {}; (function(event){").is_err());
    assert!(validate_script("return event.payload.value === true;").is_ok());
    assert!(validate_script("return (").is_err());
    assert!(
        validate_script(
            "}; throw new Error('must never run during save'); function unused(event){"
        )
        .is_ok(),
        "compile-only must never evaluate even module-level statements"
    );
    let e = Event::new("custom.test");
    assert!(evaluate(
        "}); throw new Error('outside'); (function(event){",
        &e,
        Arc::new(AtomicBool::new(false))
    )
    .is_err());
}
#[test]
fn queued_shortcuts_and_timers_cannot_use_a_newer_rule_revision() {
    for (trigger, name) in [
        (
            Trigger::Shortcut {
                accelerator: "Control+F8".into(),
            },
            "shortcut.pressed",
        ),
        (
            Trigger::Cron {
                expression: "* * * * *".into(),
                timezone: "UTC".into(),
            },
            "schedule.due",
        ),
    ] {
        let mut r = rule();
        r.revision = 7;
        r.trigger = trigger;
        r.actions = vec![Action::Delay { milliseconds: 1 }];
        let mut e = Event::new(name);
        e.payload = json!({"ruleId":r.id,"ruleRevision":7});
        assert!(r.matches(&e).unwrap());
        r.revision = 8;
        assert!(!r.matches(&e).unwrap());
        e.payload["ruleRevision"] = json!(8);
        assert!(r.matches(&e).unwrap());
    }
}
#[test]
fn run_progress_and_terminal_state_cannot_be_forged_or_overwritten() {
    let mut s = Store::memory().unwrap();
    let r = s.save(0, rule()).unwrap().rules.remove(0);
    let run = s
        .claim(&r, &Event::new("prompt.beforeSend"), 1000, true)
        .unwrap()
        .unwrap();
    s.progress(&run.id, 1).unwrap();
    s.finish(&run.id, "completed", None).unwrap();
    s.finish(&run.id, "canceled", Some("late cleanup")).unwrap();
    s.progress(&run.id, 0).unwrap();
    let actual = s.snapshot().unwrap().runs.remove(0);
    assert_eq!(actual.status, "completed");
    assert_eq!(actual.completed_actions, 1);
    assert_eq!(actual.error, None);
    assert!(s.progress(&run.id, 17).is_err());
    assert!(s.finish(&run.id, "made-up-success", None).is_err());
}
#[test]
fn execution_ledger_has_no_replay_or_simultaneous_rule_execution() {
    let mut s = Store::memory().unwrap();
    let r = s.save(0, rule()).unwrap().rules.remove(0);
    let e = Event::new("prompt.beforeSend");
    let run = s.claim(&r, &e, 1000, false).unwrap().unwrap();
    assert!(s.claim(&r, &e, 1500, true).unwrap().is_none());
    let later = Event::new("prompt.beforeSend");
    assert!(s.claim(&r, &later, 2000, true).unwrap().is_none());
    s.progress(&run.id, 1).unwrap();
    s.finish(&run.id, "completed", None).unwrap();
    assert!(s.claim(&r, &e, 3000, true).unwrap().is_none());
    assert!(s.claim(&r, &later, 1200, false).unwrap().is_none());
    assert!(s.claim(&r, &later, 2000, false).unwrap().is_some());
}
#[test]
fn disable_edit_and_delete_revoke_an_old_rule_identity() {
    let mut s = Store::memory().unwrap();
    let r = s.save(0, rule()).unwrap().rules.remove(0);
    let e = Event::new("prompt.beforeSend");
    let mut disabled = r.clone();
    disabled.enabled = false;
    let snap = s.save(1, disabled).unwrap();
    assert!(s.claim(&r, &e, 1000, true).unwrap().is_none());
    assert!(s.claim(&snap.rules[0], &e, 1000, true).unwrap().is_none());
    s.remove(2, &r.id, 2).unwrap();
    assert!(s.claim(&r, &e, 1000, true).unwrap().is_none());
}
#[test]
fn reopening_records_uncertainty_without_reexecuting_or_storing_prompt_content() {
    let p = std::env::temp_dir().join(format!("mewu-hooks-test-{}.db", uuid::Uuid::new_v4()));
    let id;
    {
        let mut s = Store::open(&p).unwrap();
        let r = s.save(0, rule()).unwrap().rules.remove(0);
        let mut e = Event::new("prompt.beforeSend");
        e.text = "private prompt sentinel".into();
        id = s.claim(&r, &e, 1000, true).unwrap().unwrap().id;
    }
    {
        let s = Store::open(&p).unwrap();
        let runs = s.snapshot().unwrap().runs;
        assert_eq!(runs[0].id, id);
        assert_eq!(runs[0].status, "interrupted");
        assert!(!serde_json::to_string(&runs)
            .unwrap()
            .contains("private prompt sentinel"));
    }
    std::fs::remove_file(p).unwrap();
}
