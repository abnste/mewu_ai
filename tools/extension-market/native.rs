// SPDX-License-Identifier: MPL-2.0
//! Console-only validation adapter. Reuses the host parsers without starting
//! Tauri, opening a registry, fetching a URL, or executing plugin contributions.
#[allow(dead_code)]
#[path = "../../apps/desktop/src-tauri/src/extension_market.rs"]
mod extension_market;
#[allow(dead_code)]
#[path = "../../apps/desktop/src-tauri/src/extension_project.rs"]
mod extension_project;
#[allow(dead_code)]
#[path = "../../apps/desktop/src-tauri/src/network_policy.rs"]
mod network_policy;
#[allow(dead_code)]
#[path = "../../apps/desktop/src-tauri/src/system_preferences.rs"]
mod system_preferences;

use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

const PROTOCOL: u32 = 1;
const MAX_CATALOG: usize = 2 * 1024 * 1024;

fn validate(kind: &str, bytes: &[u8]) -> Result<Value, String> {
    match kind {
        "extension-package"=>{
            let package=extension_market::decode_package(bytes)?;
            Ok(json!({"id":package.manifest.id,"version":package.manifest.version,"modules":package.modules.len()}))
        },
        "extension-project" => serde_json::to_value(extension_project::parse_project(bytes)?)
            .map_err(|e| e.to_string()),
        "extension-projects" => serde_json::to_value(extension_project::parse_projects(bytes)?)
            .map_err(|e| e.to_string()),
        "extension-project-image" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct ImageInput {
                project: extension_project::Project,
                index: usize,
                data: String,
            }
            let input: ImageInput =
                serde_json::from_slice(bytes).map_err(|_| "图片校验输入格式无效")?;
            extension_project::validate_project(&input.project)?;
            let shot = input
                .project
                .screenshots
                .get(input.index)
                .ok_or("截图序号无效")?;
            if input.data.len() > 2 * 1024 * 1024 * 4 / 3 + 4 {
                return Err("截图编码超过限制".into());
            }
            let image = base64::engine::general_purpose::STANDARD
                .decode(&input.data)
                .map_err(|_| "截图编码无效")?;
            extension_project::validate_image(shot, &image)?;
            Ok(
                json!({"width":shot.width,"height":shot.height,"size":shot.size,"sha256":shot.sha256}),
            )
        }
        "extension-manifest" => serde_json::to_value(extension_market::parse_manifest(bytes)?)
            .map_err(|e| e.to_string()),
        "extension-entry" => {
            serde_json::to_value(extension_market::parse_entry(bytes)?).map_err(|e| e.to_string())
        }
        "extension-catalog" => {
            serde_json::to_value(extension_market::parse_catalog(bytes)?).map_err(|e| e.to_string())
        }
        _ => Err("不支持的插件校验模式".into()),
    }
}

fn run() -> Result<Value, String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 1 {
        return Err("用法：mewu-plugin-validator <extension-manifest|extension-entry|extension-catalog|extension-package|extension-project|extension-projects|--version>；extension-package 从标准输入读取 ZIP，其他模式读取 JSON".into());
    }
    if args[0] == "--version" {
        return Ok(
            json!({"name": "mewu-plugin-validator", "hostVersion": env!("CARGO_PKG_VERSION"), "hostApi": 2}),
        );
    }
    let limit = match args[0].as_str() {
        "extension-project" => 64 * 1024,
        "extension-projects" => MAX_CATALOG,
        "extension-project-image" => 3 * 1024 * 1024,
        "extension-manifest" => mewu_extension::MAX_MANIFEST_BYTES,
        "extension-package" => extension_market::MAX_ARCHIVE,
        "extension-entry" | "extension-catalog" => MAX_CATALOG,
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
    let mut output = match result {
        Ok(value) => {
            json!({"protocolVersion": PROTOCOL, "ok": true, "value": value})
        }
        Err(error) => {
            json!({"protocolVersion": PROTOCOL, "ok": false, "error": error})
        }
    };
    let mut extension_digest = Sha256::new();
    for (index, source) in [
        include_bytes!("../../crates/mewu-extension/src/contract.rs").as_slice(),
        include_bytes!("../../crates/mewu-extension/src/package.rs").as_slice(),
        include_bytes!("../../crates/mewu-extension/src/sdk.js").as_slice(),
        include_bytes!("../../crates/mewu-extension/src/runtime.rs").as_slice(),
        include_bytes!("../../crates/mewu-extension/src/appearance.rs").as_slice(),
        include_bytes!("../../crates/mewu-extension/src/appearance-contract.json").as_slice(),
        include_bytes!("../../apps/desktop/src-tauri/src/extension_market.rs").as_slice(),
    ]
    .iter()
    .enumerate()
    {
        if index > 0 {
            extension_digest.update([0]);
        }
        extension_digest.update(source);
    }
    output["extensionContractVersion"] = json!(2);
    output["extensionContractSha256"] = json!(format!("{:x}", extension_digest.finalize()));
    output["projectContractVersion"] = json!(1);
    output["projectContractSha256"] = json!(format!(
        "{:x}",
        Sha256::digest(include_bytes!(
            "../../apps/desktop/src-tauri/src/extension_project.rs"
        ))
    ));
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
