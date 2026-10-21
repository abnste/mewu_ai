// SPDX-License-Identifier: MPL-2.0
//! Parsing historical tool receipts, not a provider implementation.
use serde::Deserialize;
use serde_json::Value;
use tauri::AppHandle;

pub async fn recall(_: &AppHandle, _: &str, _: &str, _: &str) -> Result<Value,String> {
    Err("旧记忆连接已停用，请在独立记忆插件中迁移连接".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Save {
    source_quote: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Forget {
    id: String,
    expected_revision: u64,
}
fn parse<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, String> {
    crate::agent_tools::parse(value)
}

pub(crate) enum RecordedEffect {
    Retain { quote: String },
    Forget { id: String, expected_revision: u64 },
}
pub(crate) fn recorded_effect(name: &str, arguments: &str) -> Result<RecordedEffect, String> {
    match name {
        "memory_save" => Ok(RecordedEffect::Retain {
            quote: parse::<Save>(arguments)?.source_quote,
        }),
        "memory_forget" => {
            let args: Forget = parse(arguments)?;
            Ok(RecordedEffect::Forget {
                id: args.id,
                expected_revision: args.expected_revision,
            })
        }
        _ => Err("此工具不能修改外接记忆".into()),
    }
}
