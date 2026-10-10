// SPDX-License-Identifier: MPL-2.0
//! One-time, local WPF connection migration. No network or model calls.
//!
//! WPF evidence: Services/{CredentialService,ProviderProtocolPolicy}.cs and
//! Models/AppSettings.cs at f753501f1ac0253205ce1f6a8f3ffd3c7f521b96.
//! Each profile uses the normal credential-before-DB commit path. The intent
//! marks a profile attempted BEFORE that commit. After a crash a missing
//! attempted ID is never recreated: absent/removed cannot be distinguished
//! without a shared DB transaction. It is reported as interrupted instead.
use crate::{connection_host, credentials, provider_transport};
use mewu_core::{
    ConnectionAdvanced, ConnectionAuthMode, ConnectionParameters, ConnectionProfile,
    ConnectionProtocol, SceneCommand, Store, LEGACY_CONNECTION_ID,
};
use serde::{de, Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use zeroize::Zeroizing;

const MAX_SOURCE: usize = 2 * 1024 * 1024;
const MAX_PROFILES: usize = 128;
const MAX_MARKER: usize = 128 * 1024;
const MARKER: &str = "legacy-connections-import.json";
const LOCK: &str = "legacy-connections-import.lock";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    InvalidProfile,
    UnsupportedType,
    UnsupportedProtocol,
    UnsupportedAuthentication,
    UnsupportedHeaders,
    UnsupportedParameters,
    UnsupportedRoutingMetadata,
    UnsupportedProxy,
    InvalidEndpoint,
    InvalidCredential,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Pending {},
    Attempted {},
    Imported {},
    Existing {},
    Interrupted {},
    Skipped { reason: SkipReason },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportEntry {
    pub source_index: usize,
    pub connection_id: Option<String>,
    pub outcome: Outcome,
}
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub source_found: bool,
    pub already_completed: bool,
    pub entries: Vec<ImportEntry>,
    pub default_selected: bool,
    pub empty_seed_removed: bool,
    pub routing_interrupted: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Routing {
    expected_default: Option<String>,
    default_eligible: bool,
    active_scene: String,
    active_seed: bool,
    target: Option<String>,
    attempted: bool,
    finished: bool,
    default_selected: bool,
    empty_seed_removed: bool,
    interrupted: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Migration {
    version: u32,
    source_sha256: String,
    entries: Vec<ImportEntry>,
    routing: Routing,
    complete: bool,
}
impl Migration {
    fn can_retry_legacy_credentials(&self) -> bool {
        self.version == 1
            && self.complete
            && self.entries.iter().any(|entry| {
                entry.connection_id.is_some()
                    && matches!(
                        entry.outcome,
                        Outcome::Skipped {
                            reason: SkipReason::InvalidCredential
                        }
                    )
            })
    }
    fn report(&self, already_completed: bool) -> ImportReport {
        ImportReport {
            source_found: true,
            already_completed,
            entries: self.entries.clone(),
            default_selected: self.routing.default_selected,
            empty_seed_removed: self.routing.empty_seed_removed,
            routing_interrupted: self.routing.interrupted,
        }
    }
    fn validate(&self) -> Result<(), String> {
        let mut ids = HashSet::new();
        if !matches!(self.version, 1 | 2)
            || self.source_sha256.len() != 64
            || !self.source_sha256.bytes().all(|v| v.is_ascii_hexdigit())
            || self.entries.len() > MAX_PROFILES
            || self.entries.iter().enumerate().any(|(index, entry)| {
                entry.source_index != index
                    || entry.connection_id.as_ref().is_some_and(|id| {
                        canonical_id(id).as_ref() != Some(id) || !ids.insert(id.clone())
                    })
                    || entry.connection_id.is_none()
                        && !matches!(entry.outcome, Outcome::Skipped { .. })
                    || (self.complete
                        && matches!(entry.outcome, Outcome::Pending {} | Outcome::Attempted {}))
            })
            || self.complete && !self.routing.finished
            || self
                .routing
                .target
                .as_ref()
                .is_some_and(|id| canonical_id(id).as_ref() != Some(id))
            || uuid::Uuid::parse_str(&self.routing.active_scene).is_err()
        {
            return Err("旧连接导入记录无效；原资料未改动".into());
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct LegacyProfile {
    id: String,
    name: String,
    #[serde(rename = "Type")]
    provider_type: String,
    base_url: String,
    model: String,
    #[serde(default)]
    credential_id: String,
    #[serde(default = "auto")]
    api_format: String,
    #[serde(default = "auto")]
    auth_mode: String,
    #[serde(default)]
    request_path: String,
    #[serde(default)]
    region: String,
    #[serde(default)]
    plan: String,
    #[serde(default)]
    account_id_header: String,
    #[serde(default)]
    custom_headers: BTreeMap<String, String>,
    #[serde(default = "empty_object")]
    request_parameters: Value,
    #[serde(default)]
    sensitive_header_credential_ids: BTreeMap<String, String>,
}
fn auto() -> String {
    "auto".into()
}
fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}
fn canonical_id(value: &str) -> Option<String> {
    if !(value.len() == 32 && value.bytes().all(|c| c.is_ascii_hexdigit()) || value.len() == 36) {
        return None;
    }
    let id = uuid::Uuid::parse_str(value).ok()?;
    if id.is_nil() || id.to_string() == LEGACY_CONNECTION_ID {
        None
    } else {
        Some(id.to_string())
    }
}
struct Prepared {
    profile: ConnectionProfile,
    key: Option<Zeroizing<String>>,
}
struct Source {
    digest: String,
    entries: Vec<ImportEntry>,
    prepared: Vec<Option<Prepared>>,
    default_id: Option<String>,
}

// Value's standard visitor accepts duplicate object keys. A duplicate in an
// old credential/protocol field must not become silent last-value-wins.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Unique;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON")
            }
            fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Unique, E> {
                Ok(Unique(Value::Bool(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Unique, E> {
                serde_json::Number::from_f64(v)
                    .map(|v| Unique(Value::Number(v)))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Unique, E> {
                Ok(Unique(Value::String(v.into())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Unique, E> {
                Ok(Unique(Value::String(v)))
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                let mut values = Vec::new();
                while let Some(Unique(v)) = a.next_element()? {
                    values.push(v);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((k, Unique(v))) = a.next_entry::<String, Unique>()? {
                    if values.insert(k, v).is_some() {
                        return Err(de::Error::custom("duplicate field"));
                    }
                }
                Ok(Unique(Value::Object(values)))
            }
        }
        d.deserialize_any(Visitor)
    }
}

fn prepare_profile(value: Value, old_directory: &Path) -> Result<Prepared, SkipReason> {
    let old: LegacyProfile =
        serde_json::from_value(value).map_err(|_| SkipReason::InvalidProfile)?;
    let id = canonical_id(&old.id).ok_or(SkipReason::InvalidProfile)?;
    if !matches!(
        old.provider_type.to_ascii_lowercase().as_str(),
        "minimax" | "openaicompatible" | "anthropic"
    ) {
        return Err(SkipReason::UnsupportedType);
    }
    if !old.custom_headers.is_empty()
        || !old.sensitive_header_credential_ids.is_empty()
        || !old.account_id_header.is_empty()
    {
        return Err(SkipReason::UnsupportedHeaders);
    }
    if !old.region.is_empty() || !old.plan.is_empty() {
        return Err(SkipReason::UnsupportedRoutingMetadata);
    }
    let protocol = match old.api_format.trim().to_ascii_lowercase().as_str() {
        "" | "auto" if old.provider_type.eq_ignore_ascii_case("Anthropic") => {
            ConnectionProtocol::AnthropicMessages
        }
        "" | "auto" | "chat" | "openai_chat" | "openai-chat" | "completions" => {
            ConnectionProtocol::ChatCompletions
        }
        "responses" | "openai_responses" | "openai-responses" => {
            ConnectionProtocol::OpenAiResponses
        }
        "anthropic" | "messages" | "anthropic_messages" => ConnectionProtocol::AnthropicMessages,
        _ => return Err(SkipReason::UnsupportedProtocol),
    };
    let base = url::Url::parse(old.base_url.trim()).map_err(|_| SkipReason::InvalidEndpoint)?;
    let auth_mode = match old.auth_mode.trim().to_ascii_lowercase().as_str() {
        "" | "auto"
            if protocol == ConnectionProtocol::AnthropicMessages
                && base.host_str() == Some("api.anthropic.com") =>
        {
            ConnectionAuthMode::ApiKey
        }
        "" | "auto" | "bearer" | "authorization" | "anthropic_auth_token" => {
            ConnectionAuthMode::Bearer
        }
        "api_key" | "api-key" | "x-api-key" | "anthropic_api_key" => ConnectionAuthMode::ApiKey,
        "none" | "anonymous" => ConnectionAuthMode::None,
        _ => return Err(SkipReason::UnsupportedAuthentication),
    };
    let parameters: ConnectionParameters = serde_json::from_value(old.request_parameters)
        .map_err(|_| SkipReason::UnsupportedParameters)?;
    let mut profile = ConnectionProfile {
        id,
        name: old.name.trim().into(),
        provider_id: if old.provider_type.eq_ignore_ascii_case("MiniMax") {
            "MiniMax"
        } else {
            "custom"
        }
        .into(),
        base_url: old.base_url.trim().trim_end_matches('/').into(),
        model: old.model.trim().into(),
        has_key: false,
        credential_id: None,
        revision: 1,
        advanced: ConnectionAdvanced {
            protocol,
            auth_mode,
            request_path: None,
            request_parameters: parameters,
        },
    };
    // Convert legacy append/prefix routing to one explicit origin-root path.
    // This also preserves legacy bare-remote-host /v1 behavior.
    profile.advanced.request_path = Some(legacy_request_path(&base, &old.request_path, protocol)?);
    mewu_core::validate_connection_profile(&profile).map_err(|_| SkipReason::InvalidProfile)?;
    if profile.base_url.is_empty() || profile.model.is_empty() {
        return Err(SkipReason::InvalidProfile);
    }
    provider_transport::endpoint(&profile).map_err(|_| SkipReason::InvalidEndpoint)?;
    connection_host::validate_request_options(&profile)
        .map_err(|_| SkipReason::UnsupportedParameters)?;
    let key = if old.credential_id.is_empty() {
        if auth_mode != ConnectionAuthMode::None {
            return Err(SkipReason::InvalidCredential);
        }
        None
    } else {
        // Even anonymous routes retain an existing credential reference; an
        // authentication choice is not authorization to silently discard it.
        Some(
            credentials::read_legacy(old_directory, &old.credential_id)
                .map_err(|_| SkipReason::InvalidCredential)?,
        )
    };
    Ok(Prepared { profile, key })
}
fn legacy_request_path(
    base: &url::Url,
    custom: &str,
    protocol: ConnectionProtocol,
) -> Result<String, SkipReason> {
    let default = match protocol {
        ConnectionProtocol::ChatCompletions => "/chat/completions",
        ConnectionProtocol::AnthropicMessages => "/messages",
        ConnectionProtocol::OpenAiResponses => "/responses",
    };
    let raw = if custom.trim().is_empty() {
        default
    } else {
        custom.trim()
    };
    if raw.contains(['\\', '#', '?', ':']) || raw.contains("..") || raw.starts_with("//") {
        return Err(SkipReason::InvalidEndpoint);
    }
    let path = if raw.starts_with('/') {
        raw.to_string()
    } else {
        format!("/{raw}")
    };
    let bp = base.path().trim_end_matches('/');
    let low_bp = bp.to_ascii_lowercase();
    let low_path = path.to_ascii_lowercase();
    if low_bp.ends_with(low_path.trim_end_matches('/')) {
        return Ok(bp.into());
    }
    let local = match base.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if bp.is_empty()
        && !local
        && matches!(
            path.as_str(),
            "/chat/completions" | "/responses" | "/messages"
        )
    {
        return Ok(format!("/v1{path}"));
    }
    if low_path.starts_with(&format!("{low_bp}/")) {
        return Ok(path);
    }
    if low_bp.rfind("/v1").is_some() && low_path.starts_with("/v1/") {
        return Ok(format!("{bp}{}", &path[3..]));
    }
    Ok(format!("{bp}{path}"))
}
fn prepare_source(bytes: &[u8], old_directory: &Path) -> Result<Source, String> {
    if bytes.len() > MAX_SOURCE {
        return Err("旧版设置超过 2 MiB".into());
    }
    let content = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let Unique(value) =
        serde_json::from_slice::<Unique>(content).map_err(|_| "旧版设置格式无效；原资料未改动")?;
    let object = value.as_object().ok_or("旧版设置必须是对象")?;
    let providers = object
        .get("Providers")
        .and_then(Value::as_array)
        .ok_or("旧版连接列表无效")?;
    if providers.len() > MAX_PROFILES {
        return Err("旧版连接数量超过 128".into());
    }
    let default_id = match object.get("DefaultProviderId") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if value.is_empty() => None,
        Some(Value::String(value)) => Some(canonical_id(value).ok_or("旧版默认连接标识无效")?),
        _ => return Err("旧版默认连接标识无效".into()),
    };
    let unsupported_proxy = object
        .get("NetworkProxyMode")
        .is_some_and(|v| v.as_str() != Some("system"))
        || object
            .get("NetworkProxyUrl")
            .is_some_and(|v| v.as_str().is_none_or(|s| !s.is_empty()));
    let mut ids = HashSet::new();
    let mut entries = Vec::new();
    let mut prepared = Vec::new();
    for (index, value) in providers.iter().enumerate() {
        let id = value
            .get("Id")
            .and_then(Value::as_str)
            .and_then(canonical_id);
        if id.as_ref().is_some_and(|id| !ids.insert(id.clone())) {
            return Err("旧版连接标识重复；原资料未改动".into());
        }
        let outcome = if unsupported_proxy {
            Err(SkipReason::UnsupportedProxy)
        } else {
            prepare_profile(value.clone(), old_directory)
        };
        let (outcome, data) = match outcome {
            Ok(v) => (Outcome::Pending {}, Some(v)),
            Err(reason) => (Outcome::Skipped { reason }, None),
        };
        entries.push(ImportEntry {
            source_index: index,
            connection_id: id,
            outcome,
        });
        prepared.push(data);
    }
    if default_id.as_ref().is_some_and(|id| !ids.contains(id)) {
        return Err("旧版默认连接不存在；原资料未改动".into());
    }
    Ok(Source {
        digest: format!("{:x}", Sha256::digest(bytes)),
        entries,
        prepared,
        default_id,
    })
}

fn empty_seed(profile: &ConnectionProfile) -> bool {
    profile.id == LEGACY_CONNECTION_ID
        && profile.revision == 1
        && profile.name == "默认连接"
        && profile.provider_id == "custom"
        && profile.model.is_empty()
        && !profile.has_key
        && profile.credential_id.is_none()
        && profile.advanced == ConnectionAdvanced::default()
        && matches!(profile.base_url.as_str(), "" | "https://api.openai.com/v1")
}

/// Call once during setup after Store::open and before exposing the Host. A
/// returned error must be surfaced, but must not prevent normal app startup.
/// Neither this report nor the marker contains plaintext credentials.
pub fn import_into(
    store: &mut Store,
    new_root: &Path,
    old_settings_directory: &Path,
) -> Result<ImportReport, String> {
    if !cfg!(windows) {
        return Err("旧版连接导入仅支持 Windows".into());
    }
    let _new_pin = pin_directory(new_root)?;
    let root = fs::canonicalize(new_root).map_err(|_| "无法访问新版连接目录")?;
    let mut marker = read_optional(&root.join(MARKER), MAX_MARKER)?;
    if let Some(bytes) = &marker {
        let saved: Migration =
            serde_json::from_slice(bytes).map_err(|_| "旧连接导入记录无效；原资料未改动")?;
        saved.validate()?;
        if saved.complete && !saved.can_retry_legacy_credentials() {
            return Ok(saved.report(true));
        }
    }
    if !exists(old_settings_directory)? {
        return Ok(ImportReport::default());
    }
    let _old_pin = pin_directory(old_settings_directory)?;
    let old = fs::canonicalize(old_settings_directory).map_err(|_| "无法访问旧版设置目录")?;
    if old == root {
        return Err("新版与旧版连接目录不能相同".into());
    }
    let Some(bytes) = read_optional(&old.join("settings.json"), MAX_SOURCE)? else {
        return Ok(ImportReport::default());
    };
    if let Some(previous) = &marker {
        let saved: Migration =
            serde_json::from_slice(previous).map_err(|_| "旧连接导入记录无效")?;
        if saved.source_sha256 != format!("{:x}", Sha256::digest(&bytes)) {
            return Err("旧版设置在导入后改变；已保存连接与原资料均保留".into());
        }
    }
    // Complete parsing, endpoint validation AND every referenced key read occur
    // before either the migration intent or any profile is written.
    let mut source = prepare_source(&bytes, &old)?;
    if read_optional(&old.join("settings.json"), MAX_SOURCE)?.as_ref() != Some(&bytes) {
        return Err("旧版设置在导入准备时改变；原资料未改动".into());
    }
    let lock_path = root.join(LOCK);
    check_regular_if_present(&lock_path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    no_follow(&mut options);
    let lock = options.open(&lock_path).map_err(|_| "无法锁定旧连接导入")?;
    regular(&lock.metadata().map_err(|_| "无法检查导入锁")?)?;
    lock.try_lock().map_err(|_| "旧连接正在导入")?;
    if read_optional(&root.join(MARKER), MAX_MARKER)? != marker {
        return Err("旧连接导入记录已更改".into());
    }
    let mut migration = if let Some(data) = &marker {
        let value: Migration = serde_json::from_slice(data).map_err(|_| "旧连接导入记录无效")?;
        value.validate()?;
        if value.source_sha256 != source.digest
            || value.entries.len() != source.entries.len()
            || value
                .entries
                .iter()
                .zip(&source.entries)
                .any(|(a, b)| a.connection_id != b.connection_id)
        {
            return Err("旧版设置在未完成导入后改变；已导入连接与原资料均保留".into());
        }
        let mut value = value;
        if value.can_retry_legacy_credentials() {
            // A v1 Skipped credential has never entered Attempted/DB commit.
            // Imported/Existing/Interrupted entries are immutable tombstones.
            // Persist v2 before any retry so a failed or canceled retry cannot
            // become a repeating startup import or restore deleted connections.
            let retry_target = value.entries.iter().any(|entry| {
                entry.connection_id == value.routing.target
                    && matches!(
                        entry.outcome,
                        Outcome::Skipped {
                            reason: SkipReason::InvalidCredential
                        }
                    )
            });
            for (entry, prepared) in value.entries.iter_mut().zip(&source.entries) {
                if entry.connection_id.is_some()
                    && matches!(
                        entry.outcome,
                        Outcome::Skipped {
                            reason: SkipReason::InvalidCredential
                        }
                    )
                {
                    // Existing IDs are observed by the normal loop even when
                    // their old key is still unavailable, never overwritten.
                    entry.outcome = if store
                        .snapshot()
                        .connections
                        .iter()
                        .any(|p| Some(&p.id) == entry.connection_id.as_ref())
                    {
                        Outcome::Pending {}
                    } else {
                        prepared.outcome.clone()
                    };
                }
            }
            let snapshot = store.snapshot();
            let untouched_default = value.routing.default_eligible
                && value.routing.expected_default.as_deref() == Some(LEGACY_CONNECTION_ID)
                && snapshot.default_connection_id == value.routing.expected_default
                && snapshot.connections.iter().any(empty_seed);
            let untouched_scene = snapshot.active_scene_id == value.routing.active_scene
                && snapshot.scenes.iter().any(|scene| {
                    scene.id == value.routing.active_scene
                        && !scene.closed
                        && !scene.frozen
                        && scene.connection_id.as_deref() == Some(LEGACY_CONNECTION_ID)
                });
            if retry_target
                && untouched_default
                && !value.routing.default_selected
                && !value.routing.empty_seed_removed
                && !value.routing.interrupted
            {
                // Default selection and scene binding are independent. A scene
                // that was unbound/frozen, or was changed since the failed v1
                // pass, must not gain a connection during credential recovery.
                value.routing.active_seed &= untouched_scene;
                value.routing.attempted = false;
                value.routing.finished = false;
            }
            value.version = 2;
            value.complete = false;
        }
        value
    } else {
        let snapshot = store.snapshot();
        let seed = snapshot.connections.iter().any(empty_seed);
        let active = snapshot
            .scenes
            .iter()
            .find(|scene| scene.id == snapshot.active_scene_id);
        Migration {
            version: 2,
            source_sha256: source.digest.clone(),
            entries: source.entries.clone(),
            complete: false,
            routing: Routing {
                expected_default: snapshot.default_connection_id.clone(),
                default_eligible: snapshot.default_connection_id.is_none()
                    || seed
                        && snapshot.default_connection_id.as_deref() == Some(LEGACY_CONNECTION_ID),
                active_scene: snapshot.active_scene_id.clone(),
                active_seed: seed
                    && active.is_some_and(|s| {
                        !s.frozen
                            && !s.closed
                            && s.connection_id.as_deref() == Some(LEGACY_CONNECTION_ID)
                    }),
                target: source.default_id.clone(),
                attempted: false,
                finished: false,
                default_selected: false,
                empty_seed_removed: false,
                interrupted: false,
            },
        }
    };
    commit_marker(&root, &mut marker, &migration)?;
    for index in 0..migration.entries.len() {
        if !matches!(
            migration.entries[index].outcome,
            Outcome::Pending {} | Outcome::Attempted {}
        ) {
            continue;
        }
        let id = migration.entries[index]
            .connection_id
            .as_deref()
            .ok_or("旧连接导入记录缺少身份")?;
        if store.snapshot().connections.iter().any(|p| p.id == id) {
            migration.entries[index].outcome = Outcome::Existing {};
        } else if matches!(migration.entries[index].outcome, Outcome::Attempted {}) {
            migration.entries[index].outcome = Outcome::Interrupted {};
        } else {
            let Some(prepared) = source.prepared[index].take() else {
                migration.entries[index].outcome = source.entries[index].outcome.clone();
                commit_marker(&root, &mut marker, &migration)?;
                continue;
            };
            migration.entries[index].outcome = Outcome::Attempted {};
            commit_marker(&root, &mut marker, &migration)?;
            if connection_host::save_profile(store, &root, prepared.profile, None, prepared.key)
                .is_err()
            {
                return Err(
                    "旧连接导入未完成；已保存连接保留，未确认条目下次不会自动重建，原资料未改动"
                        .into(),
                );
            }
            migration.entries[index].outcome = Outcome::Imported {};
        }
        commit_marker(&root, &mut marker, &migration)?;
    }
    if !migration.routing.finished {
        if migration.routing.attempted {
            migration.routing.interrupted = true;
        } else {
            migration.routing.attempted = true;
            commit_marker(&root, &mut marker, &migration)?;
            apply_routing(store, &mut migration)?;
        }
        migration.routing.finished = true;
    }
    migration.complete = true;
    commit_marker(&root, &mut marker, &migration)?;
    Ok(migration.report(false))
}
fn apply_routing(store: &mut Store, migration: &mut Migration) -> Result<(), String> {
    let Some(target) = migration.routing.target.clone() else {
        return Ok(());
    };
    if !migration.entries.iter().any(|e| {
        e.connection_id.as_deref() == Some(&target) && matches!(e.outcome, Outcome::Imported {})
    }) {
        return Ok(());
    }
    let snapshot = store.snapshot();
    if !snapshot.connections.iter().any(|p| p.id == target) {
        return Ok(());
    }
    if migration.routing.default_eligible
        && snapshot.default_connection_id == migration.routing.expected_default
    {
        store
            .apply(SceneCommand::SetDefaultConnection {
                connection_id: Some(target.clone()),
            })
            .map_err(|_| "旧连接已导入，但默认连接未保存")?;
        migration.routing.default_selected = true;
    }
    // Remove only the exact untouched factory placeholder. Core removal clears
    // its references without changing message/history/real configured profiles.
    let seed = snapshot.connections.iter().any(empty_seed);
    if seed {
        store
            .remove_connection_profile(LEGACY_CONNECTION_ID, 1)
            .map_err(|_| "旧连接已导入，但空默认连接未清理")?;
        migration.routing.empty_seed_removed = true;
    }
    let snapshot = store.snapshot();
    if migration.routing.active_seed
        && snapshot.active_scene_id == migration.routing.active_scene
        && snapshot.scenes.iter().any(|s| {
            s.id == migration.routing.active_scene
                && !s.frozen
                && !s.closed
                && s.connection_id.is_none()
        })
    {
        store
            .apply(SceneCommand::SetSceneConnection {
                scene_id: migration.routing.active_scene.clone(),
                connection_id: Some(target),
            })
            .map_err(|_| "旧连接已导入，但当前会话连接未保存")?;
    }
    Ok(())
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
fn no_follow(options: &mut OpenOptions) {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
    #[cfg(not(windows))]
    {
        let _ = options;
    }
}
fn regular(metadata: &Metadata) -> Result<(), String> {
    if !metadata.is_file() || is_link(metadata) {
        Err("旧连接导入文件不能是链接或特殊文件".into())
    } else {
        Ok(())
    }
}
fn exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("无法检查旧连接导入文件".into()),
    }
}
fn check_regular_if_present(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(m) => {
            regular(&m)?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("无法检查旧连接导入文件".into()),
    }
}
fn pin_directory(path: &Path) -> Result<File, String> {
    if !path.is_absolute() {
        return Err("旧连接导入目录无效".into());
    }
    let meta = fs::symlink_metadata(path).map_err(|_| "无法访问旧连接导入目录")?;
    if !meta.is_dir() || is_link(&meta) {
        return Err("旧连接导入目录不能是链接".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(1 | 2)
            .custom_flags(0x0200_0000 | 0x0020_0000);
    }
    let file = options.open(path).map_err(|_| "无法锁定旧连接导入目录")?;
    let meta = file.metadata().map_err(|_| "无法检查旧连接导入目录")?;
    if !meta.is_dir() || is_link(&meta) {
        return Err("旧连接导入目录不能是链接".into());
    }
    Ok(file)
}
fn read_optional(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, String> {
    if !check_regular_if_present(path)? {
        return Ok(None);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let file = options.open(path).map_err(|_| "无法读取旧连接导入文件")?;
    let before = file.metadata().map_err(|_| "无法检查旧连接导入文件")?;
    regular(&before)?;
    if before.len() > limit as u64 {
        return Err("旧连接导入文件超过大小限制".into());
    }
    let mut bytes = Vec::new();
    (&file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取旧连接导入文件")?;
    let after = file.metadata().map_err(|_| "无法检查旧连接导入文件")?;
    if bytes.len() > limit
        || before.len() != bytes.len() as u64
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err("旧连接导入文件在读取时改变".into());
    }
    Ok(Some(bytes))
}
fn commit_marker(
    root: &Path,
    expected: &mut Option<Vec<u8>>,
    value: &Migration,
) -> Result<(), String> {
    value.validate()?;
    let bytes = serde_json::to_vec(value).map_err(|_| "无法保存旧连接导入记录")?;
    if bytes.len() > MAX_MARKER {
        return Err("旧连接导入记录超过大小限制".into());
    }
    if expected.as_ref() == Some(&bytes) {
        return Ok(());
    }
    let target = root.join(MARKER);
    if read_optional(&target, MAX_MARKER)? != *expected {
        return Err("旧连接导入记录已更改".into());
    }
    let temp = root.join(format!(".legacy-connections-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| "无法保存旧连接导入记录")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "无法保存旧连接导入记录")?;
        drop(file);
        if read_optional(&target, MAX_MARKER)? != *expected {
            return Err("旧连接导入记录已更改".into());
        }
        if fs::rename(&temp, &target).is_err()
            && read_optional(&target, MAX_MARKER)?.as_ref() != Some(&bytes)
        {
            return Err("无法提交旧连接导入记录；已保存连接和原资料保留".into());
        }
        *expected = Some(bytes);
        Ok(())
    })();
    if temp.exists() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Explicit opt-in diagnosis only; never uses an APPDATA fallback and never
    /// opens a database or writes a marker. Root runs this separately if needed.
    #[cfg(windows)]
    #[test]
    #[ignore = "explicit legacy directory required; reads only referenced credentials, no writes"]
    fn diagnose_explicit_legacy_credentials_without_disclosing_content() {
        let path = std::env::var_os("MEWU_LEGACY_DIAGNOSTIC_DIRECTORY")
            .map(std::path::PathBuf::from)
            .expect("explicit diagnostic directory required");
        let _pin = pin_directory(&path).expect("diagnostic directory rejected");
        let bytes = read_optional(&path.join("settings.json"), MAX_SOURCE)
            .expect("diagnostic settings unreadable")
            .expect("diagnostic settings missing");
        let Unique(value) = serde_json::from_slice::<Unique>(
            bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes),
        )
        .expect("diagnostic settings invalid");
        let providers = value
            .get("Providers")
            .and_then(Value::as_array)
            .expect("diagnostic provider list invalid");
        assert!(providers.len() <= MAX_PROFILES, "diagnostic provider limit");
        let mut succeeded = true;
        for (index, provider) in providers.iter().enumerate() {
            let id = provider
                .get("CredentialId")
                .and_then(Value::as_str)
                .expect("diagnostic credential reference invalid");
            if id.is_empty() {
                continue;
            }
            let report = match credentials::read_legacy_detailed(&path, id) {
                Ok(key) => {
                    json!({"sourceIndex":index,"success":true,"byteLength":key.len(),"visibleAscii":key.bytes().all(|b| (0x20..=0x7e).contains(&b))})
                }
                Err(error) => {
                    succeeded = false;
                    json!({"sourceIndex":index,"success":false,"diagnostic":error})
                }
            };
            println!("{}", serde_json::to_string(&report).unwrap());
        }
        assert!(succeeded, "one or more legacy credential stages failed");
    }

    fn profile(id: &str) -> Value {
        json!({"Id":id,"Name":"MiniMax M3","Type":"MiniMax","BaseUrl":"https://api.minimaxi.com/v1","Model":"MiniMax-M3",
            "CredentialId":"","ApiFormat":"auto","AuthMode":"none","CustomHeaders":{},"SensitiveHeaderCredentialIds":{},
            "RequestParameters":{"service_tier":"priority"}})
    }
    fn settings(providers: Vec<Value>, default: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({"Providers":providers,"DefaultProviderId":default,"NetworkProxyMode":"system","NetworkProxyUrl":""})).unwrap()
    }
    #[test]
    fn protocol_auth_and_legacy_paths_preserve_the_actual_endpoint_and_parameters() {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let prepared =
            prepare_profile(profile(&id), Path::new("unused")).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            prepared.profile.id,
            uuid::Uuid::parse_str(&id).unwrap().to_string()
        );
        assert_eq!(
            prepared.profile.advanced.protocol,
            ConnectionProtocol::ChatCompletions
        );
        assert_eq!(
            serde_json::to_value(&prepared.profile.advanced.request_parameters).unwrap(),
            json!({"service_tier":"priority"})
        );
        for (base, path, expected) in [
            ("https://example.test", "", "/v1/chat/completions"),
            ("http://127.0.0.1:9876", "", "/chat/completions"),
            (
                "https://example.test/v1/chat/completions",
                "",
                "/v1/chat/completions",
            ),
            (
                "https://example.test/api/v1",
                "/v1/responses",
                "/api/v1/responses",
            ),
            (
                "https://example.test/api/v3",
                "responses",
                "/api/v3/responses",
            ),
            (
                "https://example.test/api/v3",
                "/api/v3/responses",
                "/api/v3/responses",
            ),
        ] {
            assert_eq!(
                legacy_request_path(
                    &url::Url::parse(base).unwrap(),
                    path,
                    ConnectionProtocol::ChatCompletions
                )
                .unwrap(),
                expected
            );
        }
        let mut p = profile(&id);
        p["ApiFormat"] = json!("anthropic");
        p["RequestParameters"] = json!({"temperature":1.5});
        assert!(matches!(
            prepare_profile(p, Path::new("unused")),
            Err(SkipReason::UnsupportedParameters)
        ));
    }
    #[test]
    fn ambiguous_structured_input_is_not_silently_accepted() {
        let id = uuid::Uuid::new_v4().simple().to_string();
        assert!(prepare_source(
            br#"{"Providers":[],"Providers":[],"DefaultProviderId":null}"#,
            Path::new("unused")
        )
        .is_err());
        assert!(prepare_source(
            &settings(vec![profile(&id), profile(&id)], &id),
            Path::new("unused")
        )
        .is_err());
        let mut p = profile(&id);
        p["RequestParameters"] = Value::Null;
        assert!(matches!(
            prepare_profile(p, Path::new("unused")),
            Err(SkipReason::UnsupportedParameters)
        ));
        for (field, value, reason) in [
            (
                "ApiFormat",
                json!("future_wire"),
                SkipReason::UnsupportedProtocol,
            ),
            (
                "AuthMode",
                json!("oauth"),
                SkipReason::UnsupportedAuthentication,
            ),
            (
                "CustomHeaders",
                json!({"x-fixture":"preserve-me"}),
                SkipReason::UnsupportedHeaders,
            ),
            (
                "RequestParameters",
                json!({"max_tokens":4096}),
                SkipReason::UnsupportedParameters,
            ),
            (
                "Region",
                json!("cn-east"),
                SkipReason::UnsupportedRoutingMetadata,
            ),
            ("FutureField", json!(true), SkipReason::InvalidProfile),
        ] {
            let mut p = profile(&id);
            p[field] = value;
            assert!(matches!(prepare_profile(p,Path::new("unused")),Err(found) if found==reason));
        }
        let mut bytes: Value = serde_json::from_slice(&settings(vec![profile(&id)], &id)).unwrap();
        bytes["NetworkProxyMode"] = json!("direct");
        let source =
            prepare_source(&serde_json::to_vec(&bytes).unwrap(), Path::new("unused")).unwrap();
        assert_eq!(
            source.entries[0].outcome,
            Outcome::Skipped {
                reason: SkipReason::UnsupportedProxy
            }
        );
    }

    #[cfg(windows)]
    struct Fixture {
        root: std::path::PathBuf,
        new: std::path::PathBuf,
        old: std::path::PathBuf,
    }
    #[cfg(windows)]
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("mewu-legacy-import-{}", uuid::Uuid::new_v4()));
            assert!(!root
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with("c:\\hermes"));
            fs::create_dir(&root).unwrap();
            let new = root.join("new");
            let old = root.join("old");
            fs::create_dir(&new).unwrap();
            fs::create_dir(&old).unwrap();
            fs::create_dir(old.join("Credentials")).unwrap();
            Self { root, new, old }
        }
        fn store(&self) -> Store {
            Store::open(self.new.join("spaces.db")).unwrap()
        }
        fn source(&self, profiles: Vec<Value>, default: &str) -> Vec<u8> {
            let bytes = settings(profiles, default);
            fs::write(self.old.join("settings.json"), &bytes).unwrap();
            bytes
        }
        fn credential(&self, id: &str, key: &str) -> Vec<u8> {
            let dir = self.old.join("Credentials");
            credentials::write(&dir, key).unwrap();
            let path = dir.join(format!("{id}.bin"));
            fs::rename(dir.join("connection.bin"), &path).unwrap();
            fs::read(path).unwrap()
        }
        fn keyed(&self, id: &str, key: &str) -> (Value, Vec<u8>) {
            let mut p = profile(id);
            p["CredentialId"] = json!(id);
            p["AuthMode"] = json!("auto");
            let bytes = self.credential(id, key);
            (p, bytes)
        }
        fn marker(&self) -> Migration {
            serde_json::from_slice(&fs::read(self.new.join(MARKER)).unwrap()).unwrap()
        }
        fn mark_as_v1(&self) {
            let mut marker = self.marker();
            marker.version = 1;
            fs::write(self.new.join(MARKER), serde_json::to_vec(&marker).unwrap()).unwrap();
        }
        fn failed_v1(&self, store: &mut Store, id: &str) -> Vec<u8> {
            let (profile, ciphertext) = self.keyed(id, "fixture-repaired-key");
            self.source(vec![profile], id);
            fs::write(
                self.old.join("Credentials").join(format!("{id}.bin")),
                b"bad-dpapi",
            )
            .unwrap();
            let result = import_into(store, &self.new, &self.old).unwrap();
            assert!(matches!(
                result.entries[0].outcome,
                Outcome::Skipped {
                    reason: SkipReason::InvalidCredential
                }
            ));
            self.mark_as_v1();
            ciphertext
        }
    }
    #[cfg(windows)]
    impl Drop for Fixture {
        fn drop(&mut self) {
            let path = self.root.canonicalize().unwrap();
            let parent = std::env::temp_dir().canonicalize().unwrap();
            assert_eq!(path.parent(), Some(parent.as_path()));
            assert!(path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("mewu-legacy-import-"));
            let _ = fs::remove_dir_all(path);
        }
    }
    #[cfg(windows)]
    #[test]
    fn synthetic_dpapi_import_is_scoped_durable_and_preserves_old_files_and_history() {
        let f = Fixture::new();
        let a = uuid::Uuid::new_v4().simple().to_string();
        let b = uuid::Uuid::new_v4().simple().to_string();
        let (pa, ca) = f.keyed(&a, "fixture-A-never-user-key");
        let (pb, cb) = f.keyed(&b, "fixture-B-never-user-key");
        let bytes = f.source(vec![pa, pb], &b);
        let mut store = f.store();
        let first = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: first.clone(),
                draft: "历史草稿保留".into(),
            })
            .unwrap();
        store.apply(SceneCommand::NewScene).unwrap();
        let active = store.snapshot().active_scene_id;
        let result = import_into(&mut store, &f.new, &f.old).unwrap();
        assert_eq!(
            result
                .entries
                .iter()
                .filter(|e| matches!(e.outcome, Outcome::Imported {}))
                .count(),
            2
        );
        assert!(result.empty_seed_removed && result.default_selected);
        let snapshot = store.snapshot();
        let target = canonical_id(&b).unwrap();
        assert_eq!(
            snapshot.default_connection_id.as_deref(),
            Some(target.as_str())
        );
        assert_eq!(
            snapshot
                .scenes
                .iter()
                .find(|s| s.id == active)
                .unwrap()
                .connection_id
                .as_ref(),
            Some(&target)
        );
        let frozen = snapshot.scenes.iter().find(|s| s.id == first).unwrap();
        assert!(frozen.frozen && frozen.connection_id.is_none());
        assert_eq!(frozen.draft, "历史草稿保留");
        for (id, key, oldbytes) in [
            (&a, "fixture-A-never-user-key", ca),
            (&b, "fixture-B-never-user-key", cb),
        ] {
            let connection = snapshot
                .connections
                .iter()
                .find(|p| p.id == canonical_id(id).unwrap())
                .unwrap();
            assert_eq!(
                &**credentials::read_profile(&f.new, connection).unwrap(),
                key
            );
            let cipher = fs::read(f.new.join("connections").join(&connection.id).join(format!(
                "{}.bin",
                connection.credential_id.as_ref().unwrap()
            )))
            .unwrap();
            assert!(!cipher.windows(key.len()).any(|part| part == key.as_bytes()));
            assert_eq!(
                fs::read(f.old.join("Credentials").join(format!("{id}.bin"))).unwrap(),
                oldbytes
            );
            assert!(!serde_json::to_string(&snapshot).unwrap().contains(key));
            assert!(!fs::read_to_string(f.new.join(MARKER))
                .unwrap()
                .contains(key));
        }
        assert_eq!(fs::read(f.old.join("settings.json")).unwrap(), bytes);
        drop(store);
        let store = f.store();
        assert_eq!(store.snapshot(), snapshot);
    }
    #[cfg(windows)]
    #[test]
    fn completed_marker_does_not_restore_deleted_profile_or_default_even_if_old_settings_change() {
        let f = Fixture::new();
        let id = uuid::Uuid::new_v4().simple().to_string();
        f.source(vec![profile(&id)], &id);
        let mut store = f.store();
        import_into(&mut store, &f.new, &f.old).unwrap();
        store
            .remove_connection_profile(&canonical_id(&id).unwrap(), 1)
            .unwrap();
        let before = store.snapshot();
        drop(store);
        fs::write(f.old.join("settings.json"), b"not JSON now").unwrap();
        let mut store = f.store();
        let report = import_into(&mut store, &f.new, &f.old).unwrap();
        assert!(report.already_completed);
        assert_eq!(store.snapshot(), before);
    }
    #[cfg(windows)]
    #[test]
    fn repaired_v1_credentials_import_once_and_deleted_connections_do_not_return() {
        let f = Fixture::new();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut store = f.store();
        let ciphertext = f.failed_v1(&mut store, &id);
        let source = fs::read(f.old.join("settings.json")).unwrap();
        fs::write(
            f.old.join("Credentials").join(format!("{id}.bin")),
            &ciphertext,
        )
        .unwrap();
        let result = import_into(&mut store, &f.new, &f.old).unwrap();
        assert_eq!(result.entries[0].outcome, Outcome::Imported {});
        assert!(result.default_selected && result.empty_seed_removed);
        assert_eq!(f.marker().version, 2);
        assert_eq!(fs::read(f.old.join("settings.json")).unwrap(), source);
        assert_eq!(
            fs::read(f.old.join("Credentials").join(format!("{id}.bin"))).unwrap(),
            ciphertext
        );
        store
            .remove_connection_profile(&canonical_id(&id).unwrap(), 1)
            .unwrap();
        let before = store.snapshot();
        drop(store);
        let mut store = f.store();
        assert!(
            import_into(&mut store, &f.new, &f.old)
                .unwrap()
                .already_completed
        );
        assert_eq!(store.snapshot(), before);
    }
    #[cfg(windows)]
    #[test]
    fn v1_recovery_repairs_factory_default_without_rebinding_an_unselected_scene() {
        for changed_after_failure in [false, true] {
            let f = Fixture::new();
            let id = uuid::Uuid::new_v4().simple().to_string();
            let mut store = f.store();
            let scene = store.snapshot().active_scene_id;
            if !changed_after_failure {
                store
                    .apply(SceneCommand::SetSceneConnection {
                        scene_id: scene.clone(),
                        connection_id: None,
                    })
                    .unwrap();
            }
            let ciphertext = f.failed_v1(&mut store, &id);
            assert_eq!(f.marker().routing.active_seed, changed_after_failure);
            if changed_after_failure {
                store
                    .apply(SceneCommand::SetSceneConnection {
                        scene_id: scene.clone(),
                        connection_id: None,
                    })
                    .unwrap();
            }
            fs::write(
                f.old.join("Credentials").join(format!("{id}.bin")),
                ciphertext,
            )
            .unwrap();
            let report = import_into(&mut store, &f.new, &f.old).unwrap();
            assert!(report.default_selected && report.empty_seed_removed);
            let snapshot = store.snapshot();
            assert_eq!(snapshot.default_connection_id, canonical_id(&id));
            assert_eq!(
                snapshot
                    .scenes
                    .iter()
                    .find(|s| s.id == scene)
                    .unwrap()
                    .connection_id,
                None
            );
            assert!(!f.marker().routing.active_seed);
        }
    }
    #[cfg(windows)]
    #[test]
    fn v1_retry_preserves_successful_deleted_entries_and_user_routing() {
        let f = Fixture::new();
        let a = uuid::Uuid::new_v4().simple().to_string();
        let b = uuid::Uuid::new_v4().simple().to_string();
        let (pa, ciphertext) = f.keyed(&a, "fixture-retry-a");
        f.source(vec![pa, profile(&b)], &a);
        let bad_path = f.old.join("Credentials").join(format!("{a}.bin"));
        fs::write(&bad_path, b"bad-dpapi").unwrap();
        let mut store = f.store();
        import_into(&mut store, &f.new, &f.old).unwrap();
        f.mark_as_v1();
        store
            .remove_connection_profile(&canonical_id(&b).unwrap(), 1)
            .unwrap();
        let scene = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetSceneConnection {
                scene_id: scene.clone(),
                connection_id: None,
            })
            .unwrap();
        store
            .apply(SceneCommand::SetDefaultConnection {
                connection_id: None,
            })
            .unwrap();
        fs::write(bad_path, ciphertext).unwrap();
        let report = import_into(&mut store, &f.new, &f.old).unwrap();
        assert_eq!(report.entries[0].outcome, Outcome::Imported {});
        assert_eq!(report.entries[1].outcome, Outcome::Imported {});
        let snapshot = store.snapshot();
        assert!(!snapshot
            .connections
            .iter()
            .any(|p| p.id == canonical_id(&b).unwrap()));
        assert_eq!(snapshot.default_connection_id, None);
        assert_eq!(
            snapshot
                .scenes
                .iter()
                .find(|s| s.id == scene)
                .unwrap()
                .connection_id,
            None
        );
    }
    #[cfg(windows)]
    #[test]
    fn v1_retry_checks_source_before_keys_and_never_overwrites_existing_profile() {
        let f = Fixture::new();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut store = f.store();
        let ciphertext = f.failed_v1(&mut store, &id);
        let source = fs::read(f.old.join("settings.json")).unwrap();
        let marker = fs::read(f.new.join(MARKER)).unwrap();
        let before = store.snapshot();
        fs::write(f.old.join("settings.json"), b"{}").unwrap();
        assert!(import_into(&mut store, &f.new, &f.old).is_err());
        assert_eq!(store.snapshot(), before);
        assert_eq!(fs::read(f.new.join(MARKER)).unwrap(), marker);
        fs::write(f.old.join("settings.json"), source).unwrap();
        fs::write(
            f.old.join("Credentials").join(format!("{id}.bin")),
            ciphertext,
        )
        .unwrap();
        let mut custom = prepare_profile(profile(&id), &f.old)
            .unwrap_or_else(|_| panic!("fixture profile rejected"))
            .profile;
        custom.model = "user-edited-model".into();
        connection_host::save_profile(&mut store, &f.new, custom.clone(), None, None).unwrap();
        let result = import_into(&mut store, &f.new, &f.old).unwrap();
        assert_eq!(result.entries[0].outcome, Outcome::Existing {});
        assert_eq!(
            store
                .snapshot()
                .connections
                .iter()
                .find(|p| p.id == custom.id),
            Some(&custom)
        );
    }
    #[cfg(windows)]
    #[test]
    fn retry_sqlite_failure_is_attempted_and_restart_does_not_recreate() {
        let f = Fixture::new();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut store = f.store();
        let ciphertext = f.failed_v1(&mut store, &id);
        fs::write(
            f.old.join("Credentials").join(format!("{id}.bin")),
            ciphertext,
        )
        .unwrap();
        let before = store.snapshot();
        let db = rusqlite::Connection::open(f.new.join("spaces.db")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_retry BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(import_into(&mut store, &f.new, &f.old).is_err());
        assert_eq!(f.marker().version, 2);
        assert!(matches!(
            f.marker().entries[0].outcome,
            Outcome::Attempted {}
        ));
        assert_eq!(store.snapshot(), before);
        db.execute_batch("DROP TRIGGER reject_retry;").unwrap();
        drop(db);
        drop(store);
        let mut store = f.store();
        let report = import_into(&mut store, &f.new, &f.old).unwrap();
        assert_eq!(report.entries[0].outcome, Outcome::Interrupted {});
        assert_eq!(store.snapshot(), before);
    }
    #[cfg(windows)]
    #[test]
    fn failed_v2_retry_is_not_repeated_when_a_key_later_changes() {
        let f = Fixture::new();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut store = f.store();
        let ciphertext = f.failed_v1(&mut store, &id);
        import_into(&mut store, &f.new, &f.old).unwrap();
        assert_eq!(f.marker().version, 2);
        let before = store.snapshot();
        fs::write(
            f.old.join("Credentials").join(format!("{id}.bin")),
            ciphertext,
        )
        .unwrap();
        assert!(
            import_into(&mut store, &f.new, &f.old)
                .unwrap()
                .already_completed
        );
        assert_eq!(store.snapshot(), before);
    }
    #[cfg(windows)]
    #[test]
    fn existing_custom_profile_and_default_are_never_overwritten() {
        let f = Fixture::new();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let b = uuid::Uuid::new_v4().simple().to_string();
        f.source(vec![profile(&id), profile(&b)], &b);
        let mut store = f.store();
        let mut existing = prepare_profile(profile(&id), &f.old)
            .unwrap_or_else(|e| panic!("{e:?}"))
            .profile;
        existing.model = "user-chosen".into();
        existing.name = "用户连接".into();
        connection_host::save_profile(&mut store, &f.new, existing.clone(), None, None).unwrap();
        store
            .apply(SceneCommand::SetDefaultConnection {
                connection_id: Some(existing.id.clone()),
            })
            .unwrap();
        let scene = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetSceneConnection {
                scene_id: scene.clone(),
                connection_id: Some(existing.id.clone()),
            })
            .unwrap();
        let report = import_into(&mut store, &f.new, &f.old).unwrap();
        assert_eq!(report.entries[0].outcome, Outcome::Existing {});
        assert!(!report.default_selected);
        let s = store.snapshot();
        assert_eq!(
            s.connections.iter().find(|p| p.id == existing.id).unwrap(),
            &existing
        );
        assert_eq!(s.default_connection_id, Some(existing.id.clone()));
        assert_eq!(
            s.scenes
                .iter()
                .find(|s| s.id == scene)
                .unwrap()
                .connection_id,
            Some(existing.id)
        );
    }
    #[cfg(windows)]
    #[test]
    fn bad_key_unknown_headers_and_protocol_are_reported_without_touching_originals() {
        let f = Fixture::new();
        let ids = (0..4)
            .map(|_| uuid::Uuid::new_v4().simple().to_string())
            .collect::<Vec<_>>();
        let mut a = profile(&ids[0]);
        a["ApiFormat"] = json!("unrecognized");
        let mut b = profile(&ids[1]);
        b["CustomHeaders"] = json!({"Authorization":"synthetic-only"});
        let mut c = profile(&ids[2]);
        c["CredentialId"] = json!(ids[2]);
        c["AuthMode"] = json!("auto");
        fs::write(
            f.old.join("Credentials").join(format!("{}.bin", ids[2])),
            b"not-dpapi",
        )
        .unwrap();
        let source = f.source(vec![a, b, c, profile(&ids[3])], &ids[3]);
        let mut store = f.store();
        let report = import_into(&mut store, &f.new, &f.old).unwrap();
        assert_eq!(
            report.entries[0].outcome,
            Outcome::Skipped {
                reason: SkipReason::UnsupportedProtocol
            }
        );
        assert_eq!(
            report.entries[1].outcome,
            Outcome::Skipped {
                reason: SkipReason::UnsupportedHeaders
            }
        );
        assert_eq!(
            report.entries[2].outcome,
            Outcome::Skipped {
                reason: SkipReason::InvalidCredential
            }
        );
        assert_eq!(report.entries[3].outcome, Outcome::Imported {});
        assert_eq!(fs::read(f.old.join("settings.json")).unwrap(), source);
        assert_eq!(
            fs::read(f.old.join("Credentials").join(format!("{}.bin", ids[2]))).unwrap(),
            b"not-dpapi"
        );
    }
    #[cfg(windows)]
    #[test]
    fn sqlite_failure_does_not_change_snapshot_or_old_blob_and_restart_never_recreates_uncertain_id(
    ) {
        let f = Fixture::new();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let (p, old) = f.keyed(&id, "fixture-failed-db-key");
        f.source(vec![p], &id);
        let mut store = f.store();
        let before = store.snapshot();
        let db = rusqlite::Connection::open(f.new.join("spaces.db")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_import BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(import_into(&mut store, &f.new, &f.old).is_err());
        assert_eq!(store.snapshot(), before);
        assert!(matches!(
            f.marker().entries[0].outcome,
            Outcome::Attempted {}
        ));
        let credentials = f.new.join("connections").join(canonical_id(&id).unwrap());
        assert_eq!(fs::read_dir(credentials).unwrap().count(), 0);
        assert_eq!(
            fs::read(f.old.join("Credentials").join(format!("{id}.bin"))).unwrap(),
            old
        );
        db.execute_batch("DROP TRIGGER reject_import;").unwrap();
        drop(db);
        drop(store);
        let mut store = f.store();
        let report = import_into(&mut store, &f.new, &f.old).unwrap();
        assert_eq!(report.entries[0].outcome, Outcome::Interrupted {});
        assert_eq!(store.snapshot(), before);
    }
    #[cfg(windows)]
    #[test]
    fn bounded_and_nonregular_sources_fail_before_any_profile_or_intent_write() {
        let f = Fixture::new();
        let mut store = f.store();
        let before = store.snapshot();
        fs::create_dir(f.old.join("settings.json")).unwrap();
        assert!(import_into(&mut store, &f.new, &f.old).is_err());
        fs::remove_dir(f.old.join("settings.json")).unwrap();
        fs::write(f.old.join("settings.json"), vec![b' '; MAX_SOURCE + 1]).unwrap();
        assert!(import_into(&mut store, &f.new, &f.old).is_err());
        assert_eq!(store.snapshot(), before);
        assert!(!f.new.join(MARKER).exists());
        assert!(!f.new.join(LOCK).exists());
        let id = uuid::Uuid::new_v4().simple().to_string();
        assert!(credentials::read_legacy(&f.old, "../escape").is_err());
        fs::create_dir(f.old.join("Credentials").join(format!("{id}.bin"))).unwrap();
        assert!(credentials::read_legacy(&f.old, &id).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn legacy_reader_uses_directory_identity_across_case_spellings_and_rejects_junctions() {
        use std::os::windows::process::CommandExt;
        let f = Fixture::new();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let ciphertext = f.credential(&id, "fixture-case-sensitive-spelling");
        let directory = f.old.join("Credentials");
        let temporary = f.old.join("directory-move");
        let mixed = f.old.join("cReDeNtIaLs");
        fs::rename(&directory, &temporary).unwrap();
        fs::rename(&temporary, &mixed).unwrap();
        for old in [&f.old, &f.root.join("OLD")] {
            assert_eq!(
                &**credentials::read_legacy_detailed(old, &id).unwrap(),
                "fixture-case-sensitive-spelling"
            );
        }
        assert_eq!(
            fs::read(mixed.join(format!("{id}.bin"))).unwrap(),
            ciphertext
        );
        let real = f.root.join("relocated-credentials");
        fs::rename(&mixed, &real).unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&directory)
            .arg(&real)
            .creation_flags(0x0800_0000)
            .output()
            .expect("create synthetic junction");
        assert!(
            output.status.success(),
            "synthetic junction creation failed"
        );
        let error = credentials::read_legacy_detailed(&f.old, &id)
            .err()
            .unwrap();
        assert_eq!(
            error.stage,
            credentials::LegacyCredentialStage::CredentialDirectory
        );
        assert_eq!(
            fs::read(real.join(format!("{id}.bin"))).unwrap(),
            ciphertext
        );
        fs::remove_dir(&directory).unwrap();
    }
}
