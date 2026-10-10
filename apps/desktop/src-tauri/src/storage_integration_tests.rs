// SPDX-License-Identifier: MPL-2.0
//! Run this single ignored test in a fresh process: the native asset locator is
//! deliberately startup-only and cannot be reset between unrelated test roots.
use crate::{
    asset_locations, assets, plugin_host::PluginRuntime, plugins::PluginState, storage_root,
};
use image::{Rgba, RgbaImage};
use mewu_core::{
    Asset, AssetKind, CompletePublicTurn, ConnectionAdvanced, ConnectionAuthMode,
    ConnectionProfile, ModelDispatchInput, Reference, ReferenceKind, SceneCommand, Store,
};
use rusqlite::{types::Value, Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

struct Fixture {
    base: PathBuf,
    paths: storage_root::StoragePaths,
    target: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base =
            std::env::temp_dir().join(format!("mewu-storage-real-stores-{}", uuid::Uuid::new_v4()));
        assert!(base.is_absolute());
        assert!(!base
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("c:\\hermes"));
        fs::create_dir(&base).unwrap();
        let target = base.join("new-root");
        fs::create_dir(&target).unwrap();
        let paths = storage_root::StoragePaths {
            bootstrap: base.join("bootstrap"),
            legacy_root: base.join("legacy-unused"),
            default_root: base.join(".mewu"),
        };
        Self {
            base,
            paths,
            target,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // This is the exact test-created UUID directory, never a migrated user root.
        assert!(self
            .base
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("mewu-storage-real-stores-"));
        let _ = fs::remove_dir_all(&self.base);
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn png_asset(root: &Path, name: &str, pixel: [u8; 4]) -> Asset {
    let id = uuid::Uuid::new_v4().to_string();
    let path = root.join(format!("{id}.png"));
    RgbaImage::from_pixel(3, 2, Rgba(pixel))
        .save(&path)
        .unwrap();
    Asset {
        id,
        name: name.into(),
        kind: AssetKind::Image,
        path: path.to_str().unwrap().into(),
        width: Some(3),
        height: Some(2),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    }
}
#[derive(Debug, PartialEq)]
struct SqlProof {
    application_id: i64,
    version: i64,
    schema: Vec<(String, String, String, Option<String>)>,
    tables: BTreeMap<String, Vec<Vec<Value>>>,
}
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn sql_proof(path: &Path) -> SqlProof {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let schema = tx
        .prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name")
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut tables = BTreeMap::new();
    for (kind, name, _, _) in &schema {
        if kind != "table" {
            continue;
        }
        let without_rowid: i64 = tx
            .query_row(
                "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .unwrap();
        assert!((0..=1).contains(&without_rowid));
        let (projection, order) = if without_rowid == 1 {
            let pk = tx
                .prepare("SELECT name FROM pragma_table_xinfo(?1) WHERE pk>0 ORDER BY pk")
                .unwrap()
                .query_map([name], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(!pk.is_empty());
            (
                "*",
                pk.iter()
                    .map(|n| quote_identifier(n))
                    .collect::<Vec<_>>()
                    .join(","),
            )
        } else {
            ("rowid,*", "rowid".into())
        };
        let mut statement = tx
            .prepare(&format!(
                "SELECT {projection} FROM {} ORDER BY {order}",
                quote_identifier(name)
            ))
            .unwrap();
        let count = statement.column_count();
        let rows = statement
            .query_map([], |r| {
                (0..count)
                    .map(|i| r.get::<_, Value>(i))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        tables.insert(name.clone(), rows);
    }
    SqlProof {
        application_id: tx
            .pragma_query_value(None, "application_id", |r| r.get(0))
            .unwrap(),
        version: tx
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap(),
        schema,
        tables,
    }
}
fn file_tree(root: &Path, exclude: Option<&str>) -> BTreeMap<PathBuf, Option<(u64, String)>> {
    fn visit(
        root: &Path,
        current: &Path,
        exclude: Option<&str>,
        out: &mut BTreeMap<PathBuf, Option<(u64, String)>>,
    ) {
        for entry in fs::read_dir(current).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if relative.components().count() == 1
                && exclude.is_some_and(|s| relative == Path::new(s))
            {
                continue;
            }
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(!metadata.file_type().is_symlink());
            if metadata.is_dir() {
                out.insert(relative, None);
                visit(root, &path, None, out);
            } else {
                assert!(metadata.is_file());
                let bytes = fs::read(&path).unwrap();
                out.insert(relative, Some((bytes.len() as u64, digest(&bytes))));
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, exclude, &mut out);
    out
}

#[test]
#[ignore = "requires isolated global asset context"]
fn real_stores_cold_migration_preserves_history_and_uses_only_new_asset_root() {
    let f = Fixture::new();
    let runtime = storage_root::acquire_runtime_lock(&f.paths, Duration::ZERO).unwrap();
    let initial = storage_root::resolve_startup(&f.paths).unwrap();
    let source = initial.root.clone();
    let source_assets = source.join("assets");
    fs::create_dir(&source_assets).unwrap();
    fs::create_dir(source.join("empty-unknown-directory")).unwrap();
    let original = png_asset(&source_assets, "synthetic old picture", [17, 91, 203, 255]);
    let credential = source.join("connections/synthetic-id");
    fs::create_dir_all(&credential).unwrap();
    fs::write(credential.join("synthetic.bin"), [0, 255, 27, 11, 41]).unwrap();
    fs::write(
        source.join("unknown-plugin-data.bin"),
        b"test-created unknown bytes",
    )
    .unwrap();
    let mut store = Store::open(source.join("spaces.db")).unwrap();
    let scene = store.snapshot().active_scene_id.clone();
    let agent = store.snapshot().agents[0].id.clone();
    let snapshot = store.add_asset(&scene, original.clone()).unwrap();
    let item = snapshot
        .scenes
        .iter()
        .find(|s| s.id == scene)
        .unwrap()
        .items[0]
        .id
        .clone();
    store
        .apply(SceneCommand::SaveMemory {
            agent_id: agent,
            id: None,
            text: "synthetic durable memory for data-root migration".into(),
            expected_revision: None,
        })
        .unwrap();
    let connection_id = uuid::Uuid::new_v4().to_string();
    store
        .save_connection_profile(
            ConnectionProfile {
                id: connection_id.clone(),
                name: "synthetic offline connection".into(),
                provider_id: "custom".into(),
                base_url: "https://example.invalid/v1".into(),
                model: "synthetic-model".into(),
                has_key: false,
                credential_id: None,
                revision: 1,
                advanced: ConnectionAdvanced {
                    auth_mode: ConnectionAuthMode::None,
                    ..ConnectionAdvanced::default()
                },
            },
            None,
        )
        .unwrap();
    store
        .apply(SceneCommand::SetDefaultConnection {
            connection_id: Some(connection_id.clone()),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetSceneConnection {
            scene_id: scene.clone(),
            connection_id: Some(connection_id),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetRefs {
            scene_id: scene.clone(),
            refs: vec![Reference {
                kind: ReferenceKind::Item,
                id: item,
            }],
        })
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: "synthetic question with the old PNG".into(),
        })
        .unwrap();
    let run = store.begin_run(&scene).unwrap();
    store
        .attach_run_assets(&scene, &run.run_id, vec![original.clone()])
        .unwrap();
    let lease = store
        .begin_model_request(
            &scene,
            &run.run_id,
            ModelDispatchInput {
                round: 0,
                projected_request_sha256: digest(b"synthetic request, never sent"),
            },
        )
        .unwrap();
    store
        .record_model_turn(
            &lease,
            CompletePublicTurn {
                text: "synthetic local receipt, no model invocation".into(),
                calls: vec![],
            },
        )
        .unwrap();
    store
        .finish_run(&scene, &run.run_id, "synthetic completed reply")
        .unwrap();
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: scene.clone(),
        })
        .unwrap();
    let saved = store.snapshot();
    let mut plugins = PluginRuntime::open(&source).unwrap();
    let ocr = plugins
        .store
        .list()
        .unwrap()
        .into_iter()
        .find(|p| p.manifest.id == "mewu.ocr")
        .expect("actual official bundle seeded");
    plugins
        .store
        .set_enabled(&ocr.manifest.id, ocr.revision, false)
        .unwrap();
    let saved_plugins = plugins.store.snapshot().unwrap();
    assert!(!saved_plugins.plugins.is_empty());
    assert_eq!(
        saved_plugins
            .plugins
            .iter()
            .find(|p| p.manifest.id == "mewu.ocr")
            .unwrap()
            .state,
        PluginState::Disabled
    );
    storage_root::mark_initialized(&f.paths, &initial).unwrap();
    drop(plugins);
    drop(store);
    drop(initial);

    // Both real writers are closed before the complete, fixed cold file proof.
    let before_core = sql_proof(&source.join("spaces.db"));
    assert_eq!(before_core.version, 9);
    assert!(!before_core.tables["agent_run_records"].is_empty());
    assert!(!before_core.tables["agent_run_events"].is_empty());
    assert!(!before_core.tables["memory_records"].is_empty());
    let before_plugins = sql_proof(&source.join("plugins/plugins.db"));
    let before_files = file_tree(&source, None);
    let generation = storage_root::readonly_state(&f.paths)
        .unwrap()
        .unwrap()
        .generation;
    let id = storage_root::request_migration(&f.paths, &source, generation, &f.target).unwrap();
    let migrated = storage_root::resolve_startup(&f.paths).unwrap();
    assert_eq!(migrated.root, f.target);
    assert_eq!(
        file_tree(&f.target, Some(&format!(".mewu-transfer-{id}"))),
        before_files
    );
    assert_eq!(
        file_tree(&source, Some(&format!(".mewu-migrated-{id}.json"))),
        before_files
    );
    assert_eq!(sql_proof(&f.target.join("spaces.db")), before_core);
    assert_eq!(
        sql_proof(&f.target.join("plugins/plugins.db")),
        before_plugins
    );
    asset_locations::install_context(migrated.locations.clone())
        .expect("isolated test process required");
    let old_image = assets::image(&original, &f.target.join("assets"))
        .unwrap()
        .to_rgba8();
    assert_eq!(old_image.dimensions(), (3, 2));
    assert!(old_image.pixels().all(|p| p.0 == [17, 91, 203, 255]));
    storage_root::commit_startup(&f.paths, migrated.trial.as_ref().unwrap()).unwrap();
    drop(migrated);
    let isolated_source = f.base.join("retained-source-isolated");
    fs::rename(&source, &isolated_source).unwrap();
    assert!(!source.exists());
    assert_eq!(
        assets::image(&original, &f.target.join("assets"))
            .unwrap()
            .to_rgba8(),
        old_image
    );

    // Open actual owners from the new root. Opening cannot rewrite old paths,
    // receipts, memory rows or the official disabled choice.
    let mut store = Store::open(f.target.join("spaces.db")).unwrap();
    let plugins = PluginRuntime::open(&f.target).unwrap();
    assert_eq!(store.snapshot(), saved);
    assert_eq!(plugins.store.snapshot().unwrap(), saved_plugins);
    assert_eq!(sql_proof(&f.target.join("spaces.db")), before_core);
    assert_eq!(
        sql_proof(&f.target.join("plugins/plugins.db")),
        before_plugins
    );
    let old_scene = store
        .snapshot()
        .scenes
        .into_iter()
        .find(|s| s.id == scene)
        .unwrap();
    assert_eq!(old_scene.items[0].asset.path, original.path);
    assert!(old_scene
        .messages
        .iter()
        .flat_map(|m| m.attachments.iter().flatten())
        .any(|a| a.path == original.path));
    let mut next = store.apply(SceneCommand::NewScene).unwrap();
    let fresh_scene = next.active_scene_id.clone();
    let fresh = png_asset(
        &f.target.join("assets"),
        "synthetic new picture",
        [99, 52, 11, 255],
    );
    store.add_asset(&fresh_scene, fresh.clone()).unwrap();
    next = store
        .apply(SceneCommand::SetDraft {
            scene_id: fresh_scene.clone(),
            draft: "new content after committed relocation".into(),
        })
        .unwrap();
    assert!(assets::image(&fresh, &f.target.join("assets")).is_ok());
    assert!(!isolated_source
        .join("assets")
        .join(format!("{}.png", fresh.id))
        .exists());
    drop(plugins);
    drop(store);
    assert_eq!(sql_proof(&isolated_source.join("spaces.db")), before_core);
    assert_eq!(
        sql_proof(&isolated_source.join("plugins/plugins.db")),
        before_plugins
    );
    let reopened = Store::open(f.target.join("spaces.db")).unwrap();
    assert_eq!(reopened.snapshot(), next);
    assert_eq!(
        assets::resolve(&reopened.snapshot(), &original.id)
            .unwrap()
            .path,
        original.path
    );
    drop(reopened);
    // Missing new bytes cannot fall back, even when the retained old bytes exist.
    let physical = f.target.join("assets").join(format!("{}.png", original.id));
    fs::rename(&physical, f.target.join("assets/removed-test-image.png")).unwrap();
    assert!(assets::image(&original, &f.target.join("assets")).is_err());
    assert!(isolated_source
        .join("assets")
        .join(format!("{}.png", original.id))
        .exists());
    drop(runtime);
}
