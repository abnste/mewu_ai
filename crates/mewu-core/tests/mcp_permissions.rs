// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection as SqlConnection;
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb {
    directory: PathBuf,
    path: PathBuf,
}
impl TestDb {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("mewu-mcp-test-{}", Uuid::new_v4()));
        assert!(!directory
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&directory).unwrap();
        Self {
            path: directory.join("scenes.sqlite3"),
            directory,
        }
    }
}
impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}
fn command() -> McpCommand {
    McpCommand {
        executable: "/fixture/server".into(),
        args: vec!["--stdio".into()],
        cwd: None,
    }
}
fn tool(name: &str) -> McpTool {
    McpTool {
        name: name.into(),
        description: "本地测试工具".into(),
        input_schema: json!({"type":"object","properties":{"text":{"type":"string"}}}),
    }
}
fn add(store: &mut Store, names: &[&str]) -> McpServer {
    store
        .upsert_mcp_server(
            None,
            None,
            "本地工具".into(),
            command(),
            names.iter().map(|name| tool(name)).collect(),
        )
        .unwrap()
        .mcp_servers
        .last()
        .unwrap()
        .clone()
}
fn grant(store: &mut Store, server: &McpServer, agent: &str, names: &[&str]) -> McpServer {
    store
        .apply(SceneCommand::SetMcpGrants {
            server_id: server.id.clone(),
            expected_revision: server.revision,
            agent_id: agent.into(),
            tool_names: names.iter().map(|name| name.to_string()).collect(),
        })
        .unwrap()
        .mcp_servers
        .into_iter()
        .find(|value| value.id == server.id)
        .unwrap()
}
fn start(store: &mut Store, scene: &str) -> RunContext {
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.into(),
            draft: "执行工具".into(),
        })
        .unwrap();
    store.begin_run(scene).unwrap()
}
fn other_agent(store: &mut Store) -> String {
    let agent = AgentProfile {
        id: Uuid::new_v4().to_string(),
        name: "其他 Agent".into(),
        instructions: String::new(),
        memory: String::new(),
        memory_enabled: false,
        memory_provider: Default::default(),
        default_connection_id: None,
    };
    let id = agent.id.clone();
    store.apply(SceneCommand::SaveAgent { agent }).unwrap();
    id
}
fn payload(db: &SqlConnection) -> String {
    db.query_row("SELECT payload FROM app_state WHERE id = 1", [], |row| {
        row.get(0)
    })
    .unwrap()
}

#[test]
fn tool_permission_is_agent_scoped_and_model_index_stays_stable() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = store.snapshot().agents[0].id.clone();
    let other = other_agent(&mut store);
    let server = add(&mut store, &["not_granted", "read", "write"]);
    assert_eq!(server.revision, 1);
    assert!(server.grants.is_empty());
    let server = grant(&mut store, &server, &agent, &["read"]);
    let server = grant(&mut store, &server, &other, &["write"]);
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene);
    assert_eq!(run.mcp_servers.len(), 1);
    assert_eq!(run.mcp_servers[0].tools.len(), 3);
    assert_eq!(
        run.mcp_servers[0].grants,
        vec![McpGrant {
            agent_id: agent.clone(),
            tool_names: vec!["read".into()]
        }]
    );
    assert_eq!(
        store
            .mcp_for_run(&scene, &run.run_id, &server.id, server.revision, "read")
            .unwrap()
            .grants
            .len(),
        1
    );
    for name in ["write", "not_granted", "missing"] {
        assert!(matches!(
            store.begin_mcp_tool(
                &scene,
                &run.run_id,
                "denied",
                &server.id,
                server.revision,
                name
            ),
            Err(CoreError::McpPermissionDenied)
        ));
    }
    assert!(store.snapshot().scenes[0]
        .run
        .as_ref()
        .unwrap()
        .steps
        .is_empty());
    let state = store
        .begin_mcp_tool(
            &scene,
            &run.run_id,
            "call",
            &server.id,
            server.revision,
            "read",
        )
        .unwrap();
    let expected = format!("mcp_{}_1", Uuid::parse_str(&server.id).unwrap().simple());
    assert_eq!(
        state.scenes[0].run.as_ref().unwrap().steps[0].name,
        expected
    );
    assert!(state.scenes[0].run.as_ref().unwrap().steps[0]
        .label
        .contains("read"));
    store
        .finish_tool(
            &scene,
            &run.run_id,
            "call",
            ToolStepStatus::Completed,
            "成功",
        )
        .unwrap();
    store.finish_run(&scene, &run.run_id, "已完成").unwrap();
    assert_eq!(store.snapshot().scenes[0].messages[0].tool_steps.len(), 1);
    let fresh = store.apply(SceneCommand::NewScene).unwrap().active_scene_id;
    store
        .apply(SceneCommand::SetAgent {
            scene_id: fresh.clone(),
            agent_id: other,
        })
        .unwrap();
    let second = start(&mut store, &fresh);
    assert!(matches!(
        store.mcp_for_run(&fresh, &second.run_id, &server.id, server.revision, "read"),
        Err(CoreError::McpPermissionDenied)
    ));
    assert!(store
        .mcp_for_run(&fresh, &second.run_id, &server.id, server.revision, "write")
        .is_ok());
}

#[test]
fn configuration_cas_and_noop_grants_do_not_cancel_run() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = store.snapshot().agents[0].id.clone();
    let original = add(&mut store, &["a", "b"]);
    let server = grant(&mut store, &original, &agent, &["b", "a"]);
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene);
    let before = store.snapshot();
    for cmd in [
        SceneCommand::SetMcpEnabled {
            server_id: server.id.clone(),
            expected_revision: original.revision,
            enabled: false,
        },
        SceneCommand::SetMcpGrants {
            server_id: server.id.clone(),
            expected_revision: original.revision,
            agent_id: agent.clone(),
            tool_names: vec![],
        },
        SceneCommand::RemoveMcpServer {
            server_id: server.id.clone(),
            expected_revision: original.revision,
        },
    ] {
        assert!(matches!(store.apply(cmd), Err(CoreError::McpConflict)));
    }
    assert!(matches!(
        store.upsert_mcp_server(
            Some(server.id.clone()),
            Some(original.revision),
            server.label.clone(),
            command(),
            vec![tool("new")]
        ),
        Err(CoreError::McpConflict)
    ));
    assert_eq!(store.snapshot(), before);
    assert_eq!(grant(&mut store, &server, &agent, &["a", "b"]), server);
    store
        .apply(SceneCommand::SetMcpEnabled {
            server_id: server.id.clone(),
            expected_revision: server.revision,
            enabled: true,
        })
        .unwrap();
    assert_eq!(store.snapshot(), before);
    assert!(store
        .begin_mcp_tool(
            &scene,
            &run.run_id,
            "call",
            &server.id,
            server.revision,
            "a"
        )
        .is_ok());
}

#[test]
fn disabling_server_cancels_all_granted_agent_runs_including_frozen_only() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = store.snapshot().agents[0].id.clone();
    let other = other_agent(&mut store);
    let server = add(&mut store, &["read"]);
    let server = grant(&mut store, &server, &agent, &["read"]);
    let a = store.snapshot().active_scene_id;
    let run_a = start(&mut store, &a);
    store
        .begin_mcp_tool(&a, &run_a.run_id, "a", &server.id, server.revision, "read")
        .unwrap();
    let b = store
        .apply(SceneCommand::FreezeScene {
            scene_id: a.clone(),
        })
        .unwrap()
        .active_scene_id;
    let run_b = start(&mut store, &b);
    let c = store.apply(SceneCommand::NewScene).unwrap().active_scene_id;
    store
        .apply(SceneCommand::SetAgent {
            scene_id: c.clone(),
            agent_id: other,
        })
        .unwrap();
    start(&mut store, &c);
    let state = store
        .apply(SceneCommand::SetMcpEnabled {
            server_id: server.id.clone(),
            expected_revision: server.revision,
            enabled: false,
        })
        .unwrap();
    assert_eq!(state.active_scene_id, c);
    assert!(state.scenes[..2]
        .iter()
        .all(|scene| scene.frozen && scene.run.as_ref().unwrap().status == RunStatus::Canceled));
    assert_eq!(
        state.scenes[0].run.as_ref().unwrap().error.as_deref(),
        Some("工具配置已更改")
    );
    assert_eq!(
        state.scenes[0].messages[0].tool_steps[0].status,
        ToolStepStatus::Failed
    );
    assert_eq!(
        state.scenes[2].run.as_ref().unwrap().status,
        RunStatus::Running
    );
    assert!(matches!(
        store.finish_tool(&a, &run_a.run_id, "a", ToolStepStatus::Completed, "迟到"),
        Err(CoreError::StaleRun)
    ));
    assert!(matches!(
        store.mcp_for_run(&b, &run_b.run_id, &server.id, server.revision, "read"),
        Err(CoreError::StaleRun)
    ));
    let next = start(&mut store, &a);
    assert!(next.mcp_servers.is_empty());
    assert!(matches!(
        store.mcp_for_run(&a, &next.run_id, &server.id, server.revision + 1, "read"),
        Err(CoreError::McpPermissionDenied)
    ));
}

#[test]
fn rediscovery_clears_grants_preserves_catalog_order_and_rejects_old_run() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = store.snapshot().agents[0].id.clone();
    let server = add(&mut store, &["read"]);
    let server = grant(&mut store, &server, &agent, &["read"]);
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene);
    let state = store
        .upsert_mcp_server(
            Some(server.id.clone()),
            Some(server.revision),
            server.label.clone(),
            command(),
            vec![tool("read")],
        )
        .unwrap();
    assert!(state.mcp_servers[0].grants.is_empty());
    assert_eq!(state.mcp_servers[0].revision, server.revision + 1);
    assert_eq!(
        state.scenes[0].run.as_ref().unwrap().status,
        RunStatus::Canceled
    );
    assert_eq!(run.mcp_servers[0], server);
    assert!(matches!(
        store.begin_mcp_tool(
            &scene,
            &run.run_id,
            "late",
            &server.id,
            server.revision,
            "read"
        ),
        Err(CoreError::StaleRun)
    ));
    let latest = state.mcp_servers[0].clone();
    let reordered = store
        .upsert_mcp_server(
            Some(latest.id.clone()),
            Some(latest.revision),
            latest.label,
            command(),
            vec![tool("write"), tool("read")],
        )
        .unwrap()
        .mcp_servers[0]
        .clone();
    let granted = grant(&mut store, &reordered, &agent, &["read"]);
    let next = start(&mut store, &scene);
    assert!(matches!(
        store.mcp_for_run(&scene, &next.run_id, &server.id, server.revision, "read"),
        Err(CoreError::McpConflict)
    ));
    let state = store
        .begin_mcp_tool(
            &scene,
            &next.run_id,
            "next",
            &server.id,
            granted.revision,
            "read",
        )
        .unwrap();
    assert!(state.scenes[0].run.as_ref().unwrap().steps[0]
        .name
        .ends_with("_1"));
}

#[test]
fn changing_grants_or_removing_server_revokes_original_grantees() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = store.snapshot().agents[0].id.clone();
    let server = add(&mut store, &["a", "b"]);
    let server = grant(&mut store, &server, &agent, &["a", "b"]);
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene);
    let changed = grant(&mut store, &server, &agent, &["a"]);
    assert!(matches!(
        store.mcp_for_run(&scene, &run.run_id, &changed.id, changed.revision, "a"),
        Err(CoreError::StaleRun)
    ));
    let next = start(&mut store, &scene);
    let removed = store
        .apply(SceneCommand::RemoveMcpServer {
            server_id: changed.id.clone(),
            expected_revision: changed.revision,
        })
        .unwrap();
    assert!(removed.mcp_servers.is_empty());
    assert_eq!(
        removed.scenes[0].run.as_ref().unwrap().status,
        RunStatus::Canceled
    );
    assert!(matches!(
        store.begin_mcp_tool(
            &scene,
            &next.run_id,
            "late",
            &changed.id,
            changed.revision,
            "a"
        ),
        Err(CoreError::StaleRun)
    ));
}

#[test]
fn grant_limit_counts_tools_across_servers_and_agents_independently() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = store.snapshot().agents[0].id.clone();
    let other = other_agent(&mut store);
    let tools: Vec<_> = (0..64).map(|index| tool(&format!("tool{index}"))).collect();
    let server = store
        .upsert_mcp_server(None, None, "64个工具".into(), command(), tools)
        .unwrap()
        .mcp_servers[0]
        .clone();
    let names: Vec<_> = server.tools[..60]
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    let server = grant(&mut store, &server, &agent, &names);
    let extra = add(&mut store, &["extra"]);
    let before = store.snapshot();
    assert!(store
        .apply(SceneCommand::SetMcpGrants {
            server_id: extra.id.clone(),
            expected_revision: extra.revision,
            agent_id: agent,
            tool_names: vec!["extra".into()]
        })
        .is_err());
    assert_eq!(store.snapshot(), before);
    let authorized = grant(&mut store, &extra, &other, &["extra"]);
    assert_eq!(authorized.grants[0].agent_id, other);
    assert_eq!(server.grants[0].tool_names.len(), 60);
}

#[test]
fn invalid_catalog_command_and_unknown_fields_cannot_enter_snapshot() {
    let mut store = Store::open_in_memory().unwrap();
    let before = store.snapshot();
    let mut deep = json!("value");
    for _ in 0..32 {
        deep = json!({"nested":deep});
    }
    let mut bad_tools = vec![
        vec![tool("same"), tool("same")],
        vec![tool("bad\nname")],
        vec![tool(&"n".repeat(129))],
        (0..65).map(|index| tool(&format!("t{index}"))).collect(),
    ];
    for schema in [
        json!({"type":"array"}),
        json!({}),
        json!(false),
        json!({"type":"object","nested":deep}),
        json!({"type":"object","description":"中".repeat(5500)}),
    ] {
        let mut invalid = tool("bad");
        invalid.input_schema = schema;
        bad_tools.push(vec![invalid]);
    }
    let mut description = tool("bad");
    description.description = "中".repeat(4001);
    bad_tools.push(vec![description]);
    let large_catalog: Vec<_> = (0..64)
        .map(|index| McpTool {
            name: format!("t{index}"),
            description: "中".repeat(1500),
            input_schema: json!({"type":"object"}),
        })
        .collect();
    bad_tools.push(large_catalog);
    for tools in bad_tools {
        assert!(store
            .upsert_mcp_server(None, None, "非法目录".into(), command(), tools)
            .is_err());
        assert_eq!(store.snapshot(), before);
    }
    let mut invalid_commands = vec![];
    for executable in [" ".into(), "bad\npath".into(), "x".repeat(4097)] {
        invalid_commands.push(McpCommand {
            executable,
            args: vec![],
            cwd: None,
        });
    }
    invalid_commands.push(McpCommand {
        args: vec!["a".into(); 65],
        ..command()
    });
    invalid_commands.push(McpCommand {
        args: vec!["中".repeat(5500)],
        ..command()
    });
    invalid_commands.push(McpCommand {
        args: vec!["--x\0secret".into()],
        ..command()
    });
    invalid_commands.push(McpCommand {
        cwd: Some(" ".into()),
        ..command()
    });
    for config in invalid_commands {
        assert!(store
            .upsert_mcp_server(None, None, "坏配置".into(), config, vec![tool("read")])
            .is_err());
        assert_eq!(store.snapshot(), before);
    }
    assert!(serde_json::from_value::<McpCommand>(
        json!({"executable":"/safe","args":[],"env":{"TOKEN":"secret"}})
    )
    .is_err());
    assert!(serde_json::from_value::<SceneCommand>(
        json!({"type":"upsert_mcp_server","command":{"executable":"/unsafe","args":[]}})
    )
    .is_err());
    assert!(store
        .upsert_mcp_server(None, None, "名".repeat(81), command(), vec![])
        .is_err());
}

#[test]
fn maximum_server_count_and_schema_byte_budget_are_enforced() {
    let mut store = Store::open_in_memory().unwrap();
    for _ in 0..8 {
        add(&mut store, &["tool"]);
    }
    let before = store.snapshot();
    assert!(store
        .upsert_mcp_server(None, None, "第9个".into(), command(), vec![tool("tool")])
        .is_err());
    assert_eq!(store.snapshot(), before);
}

#[test]
fn mcp_configuration_and_grants_survive_restart_and_old_snapshot_defaults_empty() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    let agent = store.snapshot().agents[0].id.clone();
    let server = add(&mut store, &["read"]);
    grant(&mut store, &server, &agent, &["read"]);
    let state = store.snapshot();
    drop(store);
    assert_eq!(Store::open(&temp.path).unwrap().snapshot(), state);
    let db = SqlConnection::open(&temp.path).unwrap();
    let mut old: serde_json::Value = serde_json::from_str(&payload(&db)).unwrap();
    old.as_object_mut().unwrap().remove("mcpServers");
    db.execute(
        "UPDATE app_state SET payload = ?1",
        [serde_json::to_string(&old).unwrap()],
    )
    .unwrap();
    let restored = Store::open(&temp.path).unwrap().snapshot();
    assert!(restored.mcp_servers.is_empty());
    assert_eq!(restored.agents, state.agents);
    assert_eq!(restored.scenes, state.scenes);
}

#[test]
fn mcp_revocation_and_step_registration_roll_back_on_database_error() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    let agent = store.snapshot().agents[0].id.clone();
    let server = add(&mut store, &["read"]);
    let server = grant(&mut store, &server, &agent, &["read"]);
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene);
    let db = SqlConnection::open(&temp.path).unwrap();
    let before = store.snapshot();
    let before_payload = payload(&db);
    db.execute_batch("CREATE TRIGGER fail_mcp BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
    assert!(matches!(
        store.begin_mcp_tool(
            &scene,
            &run.run_id,
            "call",
            &server.id,
            server.revision,
            "read"
        ),
        Err(CoreError::Database(_))
    ));
    assert!(matches!(
        store.apply(SceneCommand::SetMcpEnabled {
            server_id: server.id.clone(),
            expected_revision: server.revision,
            enabled: false
        }),
        Err(CoreError::Database(_))
    ));
    assert!(matches!(
        store.upsert_mcp_server(
            Some(server.id.clone()),
            Some(server.revision),
            server.label.clone(),
            command(),
            vec![tool("new")]
        ),
        Err(CoreError::Database(_))
    ));
    assert_eq!(store.snapshot(), before);
    assert_eq!(payload(&db), before_payload);
    db.execute_batch("DROP TRIGGER fail_mcp;").unwrap();
    store
        .begin_mcp_tool(
            &scene,
            &run.run_id,
            "call",
            &server.id,
            server.revision,
            "read",
        )
        .unwrap();
    store
        .apply(SceneCommand::SetMcpEnabled {
            server_id: server.id,
            expected_revision: server.revision,
            enabled: false,
        })
        .unwrap();
    assert_eq!(
        store.snapshot().scenes[0].messages[0].tool_steps[0].status,
        ToolStepStatus::Failed
    );
}
