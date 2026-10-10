// SPDX-License-Identifier: MPL-2.0
use crate::*;
use std::collections::HashSet;
use uuid::Uuid;

type Result<T> = std::result::Result<T, CoreError>;

fn invalid(message: &str) -> CoreError {
    CoreError::Invalid(message.into())
}

pub fn validate_connection_profile(profile: &ConnectionProfile) -> Result<()> {
    if Uuid::parse_str(&profile.id)
        .map(|id| id.to_string() != profile.id)
        .unwrap_or(true)
        || profile.revision == 0
    {
        return Err(invalid("连接身份或修订号无效"));
    }
    if profile.name.trim().is_empty()
        || profile.name.chars().count() > 80
        || profile.name.chars().any(char::is_control)
    {
        return Err(invalid("连接名称须为 1–80 个字符且不含控制字符"));
    }
    if profile.provider_id.is_empty()
        || profile.provider_id.len() > 80
        || !profile
            .provider_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        return Err(invalid("供应商标识无效"));
    }
    let url = &profile.base_url;
    if url.chars().count() > 4096
        || (!url.is_empty() && !(url.starts_with("https://") || url.starts_with("http://")))
        || url.contains(['@', '?', '#'])
        || url.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(invalid(
            "连接地址须为不含凭据、查询参数、片段或空白的 HTTP(S) 地址",
        ));
    }
    if profile.model.chars().count() > 256
        || profile
            .model
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(invalid("模型名称过长或包含空白/控制字符"));
    }
    if profile.has_key != profile.credential_id.is_some()
        || profile.credential_id.as_deref().is_some_and(|value| {
            Uuid::parse_str(value)
                .map(|id| id.to_string() != value)
                .unwrap_or(true)
        })
        || (profile.credential_id.as_deref() == Some(LEGACY_CREDENTIAL_ID)
            && profile.id != LEGACY_CONNECTION_ID)
    {
        return Err(invalid("连接凭据引用无效"));
    }
    if let Some(path) = &profile.advanced.request_path {
        if path.is_empty()
            || path.len() > 512
            || path.starts_with("//")
            || !path
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/_-.".contains(&c))
            || path.split('/').any(|part| part == "." || part == "..")
        {
            return Err(invalid("请求路径须为安全的相对 API 路径"));
        }
    }
    let parameters = &profile.advanced.request_parameters;
    if parameters
        .temperature
        .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
        || parameters
            .top_p
            .is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 1.0)
    {
        return Err(invalid("temperature 须为 0–2，top_p 须大于 0 且不超过 1"));
    }
    Ok(())
}

pub(crate) fn empty_connection() -> Connection {
    Connection {
        base_url: String::new(),
        model: String::new(),
        has_key: false,
    }
}

pub(crate) fn sync_compatibility(state: &mut Snapshot) {
    state.connection = state
        .default_connection_id
        .as_ref()
        .and_then(|id| state.connections.iter().find(|profile| &profile.id == id))
        .map(ConnectionProfile::connection)
        .unwrap_or_else(empty_connection);
}

pub(crate) fn migrate_legacy(state: &mut Snapshot) {
    let legacy = &state.connection;
    let profile = ConnectionProfile {
        id: LEGACY_CONNECTION_ID.into(),
        name: "默认连接".into(),
        provider_id: "custom".into(),
        base_url: legacy.base_url.clone(),
        model: legacy.model.clone(),
        has_key: legacy.has_key,
        credential_id: legacy.has_key.then(|| LEGACY_CREDENTIAL_ID.into()),
        revision: 1,
        advanced: ConnectionAdvanced::default(),
    };
    state.connections = vec![profile];
    state.default_connection_id = Some(LEGACY_CONNECTION_ID.into());
    for scene in &mut state.scenes {
        scene.connection_id = Some(LEGACY_CONNECTION_ID.into());
        // Old running requests are immediately settled by Store::open. Do not
        // invent which historical connection completed earlier messages.
        if let Some(run) = &mut scene.run {
            if run.status == RunStatus::Running {
                run.connection_id = Some(LEGACY_CONNECTION_ID.into());
                run.connection_revision = Some(1);
            }
        }
    }
}

pub(crate) fn validate(state: &Snapshot) -> Result<()> {
    let mut ids = HashSet::new();
    for profile in &state.connections {
        validate_connection_profile(profile)?;
        if !ids.insert(profile.id.as_str()) {
            return Err(invalid("连接身份重复"));
        }
    }
    let exists = |id: &Option<String>| id.as_deref().is_none_or(|id| ids.contains(id));
    if !exists(&state.default_connection_id)
        || state
            .agents
            .iter()
            .any(|agent| !exists(&agent.default_connection_id))
        || state
            .scenes
            .iter()
            .any(|scene| !exists(&scene.connection_id))
    {
        return Err(invalid("默认或会话连接不存在"));
    }
    let expected = state
        .default_connection_id
        .as_deref()
        .and_then(|id| state.connections.iter().find(|profile| profile.id == id))
        .map(ConnectionProfile::connection)
        .unwrap_or_else(empty_connection);
    if state.connection != expected {
        return Err(invalid("旧连接投影与默认连接不一致"));
    }
    for scene in &state.scenes {
        if let Some(run) = &scene.run {
            match (&run.connection_id, run.connection_revision) {
                (None, None) if run.status != RunStatus::Running => (),
                (Some(id), Some(revision)) if Uuid::parse_str(id).is_ok() && revision > 0 => {
                    if run.status == RunStatus::Running
                        && (scene.connection_id.as_ref() != Some(id)
                            || !state
                                .connections
                                .iter()
                                .any(|p| p.id == *id && p.revision == revision))
                    {
                        return Err(invalid("运行绑定的连接版本已失效"));
                    }
                }
                _ => return Err(invalid("运行连接身份不完整")),
            }
        }
    }
    Ok(())
}
