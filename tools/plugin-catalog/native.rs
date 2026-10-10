// SPDX-License-Identifier: MPL-2.0
//! Console-only validation adapter. Reuses the host parsers without starting
//! Tauri, opening a registry, fetching a URL, or executing plugin contributions.
#[allow(dead_code)]
#[path = "../../apps/desktop/src-tauri/src/system_preferences.rs"]
mod system_preferences;
#[allow(dead_code)]
#[path = "../../apps/desktop/src-tauri/src/network_policy.rs"]
mod network_policy;
#[allow(dead_code)]
#[path = "../../apps/desktop/src-tauri/src/plugin_sources.rs"]
mod plugin_sources;
#[allow(dead_code)]
#[path = "../../apps/desktop/src-tauri/src/plugins.rs"]
mod plugins;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

const PROTOCOL: u32 = 1;
const MAX_CATALOG: usize = 2 * 1024 * 1024;

fn fingerprint() -> String {
    let mut digest = Sha256::new();
    digest.update(include_bytes!(
        "../../apps/desktop/src-tauri/src/plugins.rs"
    ));
    digest.update([0]);
    digest.update(include_bytes!(
        "../../apps/desktop/src-tauri/src/plugin_sources.rs"
    ));
    digest.update([0]);
    digest.update(include_bytes!("../../crates/mewu-core/src/model.rs"));
    digest.update([0]);
    digest.update(include_bytes!("../../crates/mewu-core/src/connections.rs"));
    format!("{:x}", digest.finalize())
}

fn validate(kind: &str, bytes: &[u8]) -> Result<Value, String> {
    match kind {
        "manifest" | "bundled-manifest" => {
            let manifest = plugins::parse_manifest(bytes)?;
            if kind == "manifest" && manifest.id.starts_with("mewu.") {
                return Err("mewu. 标识仅供宿主内置包使用".into());
            }
            serde_json::to_value(manifest).map_err(|e| e.to_string())
        }
        "catalog" => {
            let entries = plugin_sources::parse_catalog(bytes)?;
            Ok(json!({"schemaVersion": 1, "plugins": entries}))
        }
        "entry" => {
            // Deserialize raw bytes directly into the *same* strict host type.
            // Parsing first as Value would discard duplicate object keys.
            let entry: plugin_sources::CatalogEntry = serde_json::from_slice(bytes)
                .map_err(|e| format!("目录条目 JSON 格式不正确：{e}"))?;
            let catalog = serde_json::to_vec(&json!({"schemaVersion": 1, "plugins": [entry]}))
                .map_err(|e| e.to_string())?;
            let mut entries = plugin_sources::parse_catalog(&catalog)?;
            serde_json::to_value(entries.remove(0)).map_err(|e| e.to_string())
        }
        _ => Err("模式必须为 manifest、bundled-manifest、catalog 或 entry".into()),
    }
}

fn run() -> Result<Value, String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 1 {
        return Err("用法：mewu-plugin-validator <manifest|bundled-manifest|catalog|entry|--version>，从标准输入读取 JSON".into());
    }
    if args[0] == "--version" {
        return Ok(
            json!({"name": "mewu-plugin-validator", "hostVersion": env!("CARGO_PKG_VERSION"), "hostApi": 1}),
        );
    }
    let limit = match args[0].as_str() {
        "manifest" | "bundled-manifest" => plugins::MAX_MANIFEST_BYTES,
        "catalog" | "entry" => MAX_CATALOG,
        _ => return Err("不支持的校验模式".into()),
    };
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取标准输入".to_string())?;
    if bytes.len() > limit {
        return Err(format!("输入超过 {limit} 字节"));
    }
    validate(&args[0], &bytes)
}

fn main() {
    let result = run();
    let status = if result.is_ok() { 0 } else { 1 };
    let output = match result {
        Ok(value) => {
            json!({"protocolVersion": PROTOCOL, "contractSha256": fingerprint(), "ok": true, "value": value})
        }
        Err(error) => {
            json!({"protocolVersion": PROTOCOL, "contractSha256": fingerprint(), "ok": false, "error": error})
        }
    };
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    if serde_json::to_writer(&mut stdout, &output).is_err()
        || stdout.write_all(b"\n").is_err()
        || stdout.flush().is_err()
    {
        std::process::exit(2);
    }
    std::process::exit(status);
}
