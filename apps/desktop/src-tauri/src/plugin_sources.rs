// SPDX-License-Identifier: MPL-2.0
//! Public GitHub JSON packages. Resolve once, download by commit, install reviewed bytes.
use crate::plugins::{self, PluginManifest, PluginRecord, PluginSource};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubFile {
    pub repository: String,
    pub reference: String,
    pub path: String,
}

fn segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 160
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
pub fn parse_url(input: &str, default_file: &str) -> Result<GithubFile, String> {
    if input.len() > 2048 || input.contains(['%', '\\']) {
        return Err("请使用 GitHub 仓库或 JSON 文件链接".into());
    }
    let url = url::Url::parse(input.trim()).map_err(|_| "GitHub 链接格式不正确")?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("请使用 https://github.com 上的公开仓库链接".into());
    }
    // Inspect original path too: Url normalizes dot segments before validation.
    if input.split('/').any(|s| matches!(s, "." | "..")) {
        return Err("GitHub 文件路径不正确".into());
    }
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    if parts.len() < 2 || !segment(parts[0]) || !segment(parts[1]) {
        return Err("GitHub 仓库地址不正确".into());
    }
    let repository = format!(
        "{}/{}",
        parts[0],
        parts[1].strip_suffix(".git").unwrap_or(parts[1])
    );
    if parts.len() == 2 {
        return Ok(GithubFile {
            repository,
            reference: "HEAD".into(),
            path: default_file.into(),
        });
    }
    if parts.len() < 5 || parts[2] != "blob" || !parts[3..].iter().all(|s| segment(s)) {
        return Err("请使用仓库地址或 blob 文件链接；含斜杠的分支请改用提交链接".into());
    }
    let path = parts[4..].join("/");
    if !path.ends_with(".json") || path.len() > 512 {
        return Err("插件文件必须是 JSON".into());
    }
    Ok(GithubFile {
        repository,
        reference: parts[3].into(),
        path,
    })
}
fn client() -> Result<reqwest::Client, String> {
    crate::network_policy::apply(reqwest::Client::builder(), None)?
        .user_agent("Mewu/1.0 plugin-catalog")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法初始化插件下载".into())
}
async fn read(
    client: &reqwest::Client,
    url: url::Url,
    accept: &str,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .header("Accept", accept)
        .header("X-GitHub-Api-Version", "2026-03-10")
        .send()
        .await
        .map_err(|_| "GitHub 连接失败，请稍后重试")?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            403 | 429 => "GitHub 请求额度已用完，请稍后重试",
            404 => "GitHub 仓库或插件文件不存在",
            301 | 302 | 307 | 308 => "仓库地址已变更，请使用最新的 GitHub 链接",
            _ => "GitHub 下载失败",
        }
        .into());
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("插件下载超过大小限制".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "GitHub 下载中断")?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err("插件下载超过大小限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub async fn download(file: GithubFile, limit: usize) -> Result<(Vec<u8>, PluginSource), String> {
    // All network addresses are constructed from the already validated GitHub path.
    let parsed = parse_url(
        &format!(
            "https://github.com/{}/blob/{}/{}",
            file.repository, file.reference, file.path
        ),
        "",
    )?;
    let client = client()?;
    let commit = if parsed.reference.len() == 40
        && parsed.reference.bytes().all(|b| b.is_ascii_hexdigit())
    {
        parsed.reference.to_ascii_lowercase()
    } else {
        let endpoint = url::Url::parse(&format!(
            "https://api.github.com/repos/{}/commits/{}",
            parsed.repository, parsed.reference
        ))
        .map_err(|_| "GitHub 地址不正确")?;
        let raw = read(&client, endpoint, "application/vnd.github.sha", 256).await?;
        let commit = String::from_utf8(raw)
            .map_err(|_| "GitHub 提交信息不正确")?
            .trim()
            .to_string();
        if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("GitHub 未返回完整提交标识".into());
        }
        commit.to_ascii_lowercase()
    };
    let mut endpoint = url::Url::parse(&format!(
        "https://api.github.com/repos/{}/contents/{}",
        parsed.repository, parsed.path
    ))
    .map_err(|_| "GitHub 文件地址不正确")?;
    endpoint.query_pairs_mut().append_pair("ref", &commit);
    let bytes = read(&client, endpoint, "application/vnd.github.raw+json", limit).await?;
    Ok((
        bytes,
        PluginSource::Github {
            repository: parsed.repository,
            commit,
            path: parsed.path,
        },
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogEntry {
    pub manifest: PluginManifest,
    pub source: PluginSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<PluginRecord>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub entries: Vec<CatalogEntry>,
    pub source: Option<String>,
    pub error: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogFile {
    schema_version: u32,
    plugins: Vec<CatalogEntry>,
}

pub fn parse_catalog(bytes: &[u8]) -> Result<Vec<CatalogEntry>, String> {
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("插件目录超过 2 MiB".into());
    }
    let file: CatalogFile =
        serde_json::from_slice(bytes).map_err(|_| "插件目录 JSON 格式不正确")?;
    if file.schema_version != 1 || file.plugins.len() > 256 {
        return Err("插件目录格式不支持或超过 256 项".into());
    }
    let mut ids = HashSet::new();
    for entry in &file.plugins {
        plugins::parse_manifest(
            &serde_json::to_vec(&entry.manifest).map_err(|_| "目录插件格式错误")?,
        )?;
        if entry.manifest.id.starts_with("mewu.")
            || entry.installed.is_some()
            || !ids.insert(&entry.manifest.id)
        {
            return Err("目录包含保留标识、安装状态或重复插件".into());
        }
        let PluginSource::Github {
            repository,
            commit,
            path,
        } = &entry.source
        else {
            return Err("社区目录的插件必须来自 GitHub".into());
        };
        if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("目录插件必须绑定完整提交".into());
        }
        parse_url(
            &format!("https://github.com/{repository}/blob/{commit}/{path}"),
            "",
        )?;
    }
    Ok(file.plugins)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Explicit public-network check; never installs a package or opens app data.
    /// The upstream repository's default branch and package.json were checked on
    /// GitHub before adding this fixture (2026-10-08).
    #[tokio::test]
    #[ignore = "explicit public GitHub download verification; no installation"]
    async fn live_github_download_is_json_and_repeatable_at_resolved_commit() {
        use sha2::{Digest, Sha256};
        let file = parse_url(
            "https://github.com/dsh-market/dsh-market/blob/main/package.json",
            "package.json",
        )
        .unwrap();
        assert_eq!(file.repository, "dsh-market/dsh-market");
        assert_eq!(file.reference, "main");
        let (first, source) = download(file, 64 * 1024)
            .await
            .expect("public GitHub branch download");
        assert!(first.len() <= 64 * 1024);
        assert!(serde_json::from_slice::<serde_json::Value>(&first)
            .unwrap()
            .is_object());
        let PluginSource::Github {
            repository,
            commit,
            path,
        } = &source
        else {
            panic!("GitHub source expected")
        };
        assert_eq!(commit.len(), 40);
        assert!(commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(path, "package.json");
        let pinned = GithubFile {
            repository: repository.clone(),
            reference: commit.clone(),
            path: path.clone(),
        };
        let (second, second_source) = download(pinned, 64 * 1024)
            .await
            .expect("public GitHub pinned download");
        assert_eq!(source, second_source);
        assert_eq!(first, second);
        println!("GitHub verified: repository={repository}, commit={commit}, path={path}, bytes={}, sha256={:x}", first.len(), Sha256::digest(&first));
    }

    fn community_entry() -> CatalogEntry {
        let mut manifest =
            plugins::parse_manifest(include_bytes!("../../plugins/official-table.json")).unwrap();
        manifest.id = "example.table".into();
        CatalogEntry {
            manifest,
            source: PluginSource::Github {
                repository: "example/plugins".into(),
                commit: "0123456789abcdef0123456789abcdef01234567".into(),
                path: "packages/table/mewu-plugin.json".into(),
            },
            installed: None,
        }
    }

    #[test]
    fn community_catalog_preserves_pinned_source_and_rejects_mutable_or_unsafe_sources() {
        let entry = community_entry();
        let json = serde_json::json!({"schemaVersion":1,"plugins":[entry]});
        let parsed = parse_catalog(&serde_json::to_vec(&json).unwrap()).unwrap();
        assert_eq!(parsed[0].source, entry.source);
        assert_eq!(parsed[0].manifest, entry.manifest);
        for source in [
            serde_json::json!({"type":"github","repository":"example/plugins","commit":"main","path":"mewu-plugin.json"}),
            serde_json::json!({"type":"github","repository":"example/plugins","commit":"0123456789abcdef0123456789abcdef01234567","path":"../mewu-plugin.json"}),
            serde_json::json!({"type":"github","repository":"example/plugins","commit":"0123456789abcdef0123456789abcdef01234567","path":"%2e%2e/mewu-plugin.json"}),
            serde_json::json!({"type":"github","repository":"example/plugins?redirect=evil","commit":"0123456789abcdef0123456789abcdef01234567","path":"mewu-plugin.json"}),
            serde_json::json!({"type":"local","path":"D:/private/plugin.json"}),
            serde_json::json!({"type":"official"}),
        ] {
            let mut bad = json.clone();
            bad["plugins"][0]["source"] = source;
            assert!(parse_catalog(&serde_json::to_vec(&bad).unwrap()).is_err());
        }
    }

    #[test]
    fn remote_catalog_cannot_inject_local_install_status_or_duplicate_identity() {
        let entry = community_entry();
        let mut json = serde_json::json!({"schemaVersion":1,"plugins":[entry]});
        json["plugins"][0]["installed"] = serde_json::to_value(PluginRecord {
            manifest: entry.manifest.clone(),
            source: entry.source.clone(),
            revision: 99,
            state: plugins::PluginState::Enabled,
            has_rollback: true,
            error: None,
        })
        .unwrap();
        assert!(parse_catalog(&serde_json::to_vec(&json).unwrap()).is_err());
        let duplicates = serde_json::json!({"schemaVersion":1,"plugins":[entry,entry]});
        assert!(parse_catalog(&serde_json::to_vec(&duplicates).unwrap()).is_err());
        let future = serde_json::json!({"schemaVersion":2,"plugins":[entry]});
        assert!(parse_catalog(&serde_json::to_vec(&future).unwrap()).is_err());
    }
    #[test]
    fn github_urls_are_confined_and_commit_paths_stable() {
        let result = parse_url("https://github.com/example/demo", "mewu-plugin.json").unwrap();
        assert_eq!(result.repository, "example/demo");
        assert_eq!(result.reference, "HEAD");
        assert_eq!(
            parse_url("https://github.com/example/demo/blob/main/plugin.json", "")
                .unwrap()
                .path,
            "plugin.json"
        );
        for bad in [
            "http://github.com/a/b",
            "https://github.com.evil/a/b",
            "https://127.0.0.1/a/b",
            "https://user@github.com/a/b",
            "https://github.com/a/b?x=1",
            "https://github.com/a/b/blob/main/../p.json",
            "https://github.com/a/b/blob/main/%2e%2e/p.json",
            "https://github.com/a/b/blob/main/p.exe",
        ] {
            assert!(parse_url(bad, "mewu-plugin.json").is_err(), "{bad}");
        }
    }
    #[test]
    fn remote_catalog_cannot_claim_official_or_installed() {
        let manifest =
            plugins::parse_manifest(include_bytes!("../../plugins/official-table.json")).unwrap();
        let entry = CatalogEntry {
            manifest,
            source: PluginSource::Official,
            installed: None,
        };
        let value = serde_json::json!({"schemaVersion":1,"plugins":[entry]});
        assert!(parse_catalog(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}
