// SPDX-License-Identifier: MPL-2.0
//! The official GitHub Release is the update metadata, as in the .NET client.
use super::release::ReleaseTarget;
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const LATEST_RELEASE: &str = "https://api.github.com/repos/abnste/mewu_ai/releases/latest";
pub const MAX_METADATA: usize = 1024 * 1024;
pub const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
const INVALID: &str = "update_invalid";

#[derive(Deserialize)]
struct Release {
    id: u64,
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    #[serde(default)]
    assets: serde_json::Value,
    body: Option<String>,
}
#[derive(Deserialize)]
struct Asset {
    id: u64,
    name: String,
    state: String,
    size: u64,
    browser_download_url: String,
    digest: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub release_id: u64,
    pub asset_id: u64,
    pub version: String,
    pub name: String,
    pub url: url::Url,
    pub bytes: u64,
    pub sha256: Option<[u8; 32]>,
    pub release_body: Option<String>,
}

fn asset_digest(value: Option<&str>) -> Result<Option<[u8; 32]>, &'static str> {
    let Some(value) = value else { return Ok(None) };
    let encoded = value.strip_prefix("sha256:").ok_or(INVALID)?;
    if encoded.len() != 64
        || !encoded
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(INVALID);
    }
    let mut bytes = [0; 32];
    for (i, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
        let digit = |b: u8| {
            if b.is_ascii_digit() {
                b - b'0'
            } else {
                b - b'a' + 10
            }
        };
        bytes[i] = digit(pair[0]) * 16 + digit(pair[1]);
    }
    Ok(Some(bytes))
}

pub fn select(
    body: &[u8],
    current: &str,
    target: ReleaseTarget,
) -> Result<Option<Offer>, &'static str> {
    if body.is_empty() || body.len() > MAX_METADATA {
        return Err(INVALID);
    }
    let release: Release = serde_json::from_slice(body).map_err(|_| INVALID)?;
    let version = release.tag_name.strip_prefix('v').ok_or(INVALID)?;
    let latest = semver::Version::parse(version).map_err(|_| INVALID)?;
    let current = semver::Version::parse(current).map_err(|_| INVALID)?;
    if version.len() > 64
        || !latest.build.is_empty()
        || !latest.pre.is_empty()
        || release.id == 0
        || release.draft
        || release.prerelease
        || release.published_at.as_deref().is_none_or(str::is_empty)
        || release.html_url
            != format!(
                "https://github.com/abnste/mewu_ai/releases/tag/{}",
                release.tag_name
            )
    {
        return Err(INVALID);
    }
    // Existing older releases do not need a new updater payload or signature.
    // In particular the original 1.0.0 four-asset release remains untouched.
    if latest <= current {
        return Ok(None);
    }
    let assets: Vec<Asset> = serde_json::from_value(release.assets).map_err(|_| INVALID)?;
    if assets.len() > 4096 {
        return Err(INVALID);
    }
    let expected_path = target.payload_path(version);
    let name = expected_path.rsplit('/').next().ok_or(INVALID)?;
    let matches: Vec<_> = assets.iter().filter(|asset| asset.name == name).collect();
    if matches.len() != 1 {
        return Err(INVALID);
    }
    let asset = matches[0];
    let url = url::Url::parse(&asset.browser_download_url).map_err(|_| INVALID)?;
    super::release::validate_release_for(target, &current.to_string(), version, &url)?;
    if asset.id == 0 || asset.state != "uploaded" || !(4096..=MAX_PACKAGE).contains(&asset.size) {
        return Err(INVALID);
    }
    Ok(Some(Offer {
        release_id: release.id,
        asset_id: asset.id,
        version: version.into(),
        name: name.into(),
        url,
        bytes: asset.size,
        sha256: asset_digest(asset.digest.as_deref())?,
        release_body: release.body,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignatureBlock {
    schema_version: u32,
    version: String,
    assets: Vec<SignatureAsset>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignatureAsset {
    name: String,
    signature: String,
}

pub fn signature(offer: &Offer) -> Result<String, &'static str> {
    use base64::Engine;
    const PREFIX: &str = "<!-- mewu-update-signatures:";
    const START: &str = "<!-- mewu-update-signatures:v1\n";
    // GitHub can return edited release notes with CRLF line endings. Normalize
    // only actual line delimiters, just as the publisher does; JSON escapes and
    // the canonical base64 signature strings are left unchanged.
    let body = offer
        .release_body
        .as_deref()
        .ok_or(INVALID)?
        .replace("\r\n", "\n");
    if body.match_indices(PREFIX).count() != 1 {
        return Err(INVALID);
    }
    let (_, tail) = body.split_once(START).ok_or(INVALID)?;
    let (json, _) = tail.split_once("\n-->").ok_or(INVALID)?;
    if json.len() > 32 * 1024 {
        return Err(INVALID);
    }
    let block: SignatureBlock = serde_json::from_str(json).map_err(|_| INVALID)?;
    if block.schema_version != 1 || block.version != offer.version || block.assets.len() != 3 {
        return Err(INVALID);
    }
    let mut seen = std::collections::HashSet::new();
    let mut selected = None;
    for asset in block.assets {
        if !seen.insert(asset.name.clone())
            || ![
                ReleaseTarget::WindowsX64,
                ReleaseTarget::MacArm64,
                ReleaseTarget::LinuxX64,
            ]
            .iter()
            .any(|target| {
                target.payload_path(&offer.version).rsplit('/').next() == Some(asset.name.as_str())
            })
        {
            return Err(INVALID);
        }
        if asset.signature.is_empty() || asset.signature.len() > 8192 {
            return Err(INVALID);
        }
        let engine = base64::engine::general_purpose::STANDARD;
        let decoded = engine.decode(&asset.signature).map_err(|_| INVALID)?;
        if engine.encode(&decoded) != asset.signature {
            return Err(INVALID);
        }
        // The original SDK independently verifies the signed trusted-comment
        // version and Minisign over actual bytes during download.
        if asset.name == offer.name {
            selected = Some(asset.signature);
        }
    }
    selected.ok_or(INVALID)
}

pub async fn fetch(client: &reqwest::Client) -> Result<Vec<u8>, &'static str> {
    use futures_util::StreamExt;
    let response = client
        .get(LATEST_RELEASE)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", concat!("MewuAI/", env!("CARGO_PKG_VERSION")))
        .send()
        .await
        .map_err(network_error)?;
    if response.status().as_u16() == 403 || response.status().as_u16() == 429 {
        return Err("update_rate_limited");
    }
    if !response.status().is_success() {
        return Err("update_feed_unavailable");
    }
    if response.url().as_str() != LATEST_RELEASE
        || response
            .content_length()
            .is_some_and(|n| n > MAX_METADATA as u64)
    {
        return Err(INVALID);
    }
    let mut result = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(network_error)?;
        if result.len().saturating_add(chunk.len()) > MAX_METADATA {
            return Err(INVALID);
        }
        result.extend_from_slice(&chunk);
    }
    Ok(result)
}
pub fn network_error(error: reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "update_timeout"
    } else {
        "update_network"
    }
}

/// Additional GitHub metadata integrity check after the native SDK has verified
/// the mandatory Minisign signature and its trusted-comment release version.
pub fn validate_download(offer: &Offer, bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.len() as u64 != offer.bytes || bytes.len() as u64 > MAX_PACKAGE {
        return Err("update_download_failed");
    }
    if offer
        .sha256
        .is_some_and(|expected| <[u8; 32]>::from(Sha256::digest(bytes)) != expected)
    {
        return Err("update_download_failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(version: &str, target: ReleaseTarget) -> serde_json::Value {
        let path = target.payload_path(version);
        serde_json::json!({"id":1,"tag_name":format!("v{version}"),"html_url":format!("https://github.com/abnste/mewu_ai/releases/tag/v{version}"),"draft":false,"prerelease":false,"published_at":"2026-10-16T00:00:00Z","body":"Release notes","assets":[{"id":2,"name":path.rsplit('/').next().unwrap(),"state":"uploaded","size":4096,"browser_download_url":format!("https://github.com{path}"),"digest":format!("sha256:{:x}",Sha256::digest([42;4096]))}]})
    }
    fn select_value(
        value: &serde_json::Value,
        target: ReleaseTarget,
    ) -> Result<Option<Offer>, &'static str> {
        select(&serde_json::to_vec(value).unwrap(), "1.0.0", target)
    }
    #[test]
    fn unchanged_or_older_release_needs_no_feed_or_new_assets() {
        let mut value = fixture("1.0.0", ReleaseTarget::WindowsX64);
        value["assets"] = serde_json::json!([]);
        let body = serde_json::to_vec(&value).unwrap();
        for target in [
            ReleaseTarget::WindowsX64,
            ReleaseTarget::MacArm64,
            ReleaseTarget::LinuxX64,
        ] {
            assert!(select(&body, "1.0.1", target).unwrap().is_none());
            assert!(select(&body, "1.0.0", target).unwrap().is_none());
        }
    }
    #[test]
    fn direct_release_selects_exact_platform_asset_and_checks_github_sha256() {
        for target in [
            ReleaseTarget::WindowsX64,
            ReleaseTarget::MacArm64,
            ReleaseTarget::LinuxX64,
        ] {
            let offer = select_value(&fixture("1.0.1", target), target)
                .unwrap()
                .unwrap();
            assert!(validate_download(&offer, &[42; 4096]).is_ok());
            assert_eq!(
                validate_download(&offer, &[43; 4096]),
                Err("update_download_failed")
            );
            assert_eq!(
                validate_download(&offer, &[42; 4095]),
                Err("update_download_failed")
            );
            assert_eq!(
                validate_download(&offer, &[42; 4097]),
                Err("update_download_failed")
            );
            let mut missing = fixture("1.0.1", target);
            missing["assets"][0]["digest"] = serde_json::Value::Null;
            assert!(select_value(&missing, target)
                .unwrap()
                .unwrap()
                .sha256
                .is_none());
            for bad in [
                "",
                "md5:abcd",
                &format!("sha256:{}", "A".repeat(64)),
                &format!("sha256:{}", "0".repeat(63)),
            ] {
                missing["assets"][0]["digest"] = bad.into();
                assert_eq!(select_value(&missing, target), Err(INVALID));
            }
        }
    }
    #[test]
    fn malformed_ambiguous_and_cross_repository_release_metadata_is_rejected() {
        let target = ReleaseTarget::WindowsX64;
        let valid = fixture("1.0.1", target);
        for mutation in 0..11 {
            let mut v = valid.clone();
            match mutation {
                0 => v["draft"] = true.into(),
                1 => v["prerelease"] = true.into(),
                2 => v["published_at"] = serde_json::Value::Null,
                3 => v["html_url"] = "https://github.com/other/repo/releases/tag/v1.0.1".into(),
                4 => { let duplicate=v["assets"][0].clone();v["assets"].as_array_mut().unwrap().push(duplicate); },
                5 => v["assets"][0]["size"] = (MAX_PACKAGE+1).into(),
                6 => v["assets"][0]["browser_download_url"] = "https://github.com/other/repo/releases/download/v1.0.1/MewuAI-Remake-Setup-1.0.1-win-x64.exe".into(),
                7 => v["assets"][0]["state"] = "new".into(),
                8 => v["tag_name"] = "v1.0.1+unsigned".into(),
                9 => v["tag_name"] = "v1.0.1-rc.1".into(),
                _ => v["assets"][0]["name"] = "MewuAI-Remake-Portable-1.0.1-win-x64.zip".into(),
            }
            assert_eq!(
                select_value(&v, target),
                Err(INVALID),
                "mutation {mutation}"
            );
        }
        assert!(select(b"not-json", "1.0.0", target).is_err());
        assert!(select(&vec![b' '; MAX_METADATA + 1], "1.0.0", target).is_err());
    }

    fn signature_fixture() -> (Offer, serde_json::Value) {
        use base64::Engine;
        let offer = select_value(
            &fixture("1.0.1", ReleaseTarget::WindowsX64),
            ReleaseTarget::WindowsX64,
        )
        .unwrap()
        .unwrap();
        let assets = [ReleaseTarget::WindowsX64, ReleaseTarget::MacArm64, ReleaseTarget::LinuxX64].map(|target| {
            serde_json::json!({"name":target.payload_path("1.0.1").rsplit('/').next().unwrap(),
                "signature":base64::engine::general_purpose::STANDARD.encode(b"parser-fixture-not-a-valid-Minisign-signature")})
        });
        (
            offer,
            serde_json::json!({"schemaVersion":1,"version":"1.0.1","assets":assets}),
        )
    }
    fn with_block(mut offer: Offer, block: &serde_json::Value) -> Offer {
        offer.release_body = Some(format!(
            "Visible release notes\n\n<!-- mewu-update-signatures:v1\n{}\n-->",
            serde_json::to_string(block).unwrap()
        ));
        offer
    }
    #[test]
    fn exactly_one_typed_hidden_signature_block_selects_only_the_exact_updater_asset() {
        let (offer, block) = signature_fixture();
        let selected = signature(&with_block(offer.clone(), &block)).unwrap();
        assert_eq!(selected, block["assets"][0]["signature"].as_str().unwrap());
        for mutation in 0..8 {
            let mut b = block.clone();
            match mutation {
                0 => b["version"] = "1.0.2".into(),
                1 => b["schemaVersion"] = 2.into(),
                2 => b["assets"][1] = b["assets"][0].clone(),
                3 => {
                    b["assets"].as_array_mut().unwrap().pop();
                }
                4 => b["assets"][0]["url"] = "https://evil.test/".into(),
                5 => b["assets"][0]["signature"] = "not-base64".into(),
                6 => b["assets"][0]["name"] = "MewuAI-Portable-1.0.1-win-x64.zip".into(),
                _ => b["url"] = "https://evil.test/".into(),
            }
            assert_eq!(
                signature(&with_block(offer.clone(), &b)),
                Err(INVALID),
                "mutation {mutation}"
            );
        }
        let mut duplicate = with_block(offer.clone(), &block);
        duplicate.release_body = Some(duplicate.release_body.as_ref().unwrap().repeat(2));
        assert_eq!(signature(&duplicate), Err(INVALID));
        assert_eq!(signature(&offer), Err(INVALID));
    }

    #[test]
    fn hidden_signature_block_accepts_crlf_without_changing_signature_bytes() {
        let (offer, block) = signature_fixture();
        let lf = with_block(offer, &block);
        let mut crlf = lf.clone();
        crlf.release_body = Some(lf.release_body.as_ref().unwrap().replace('\n', "\r\n"));
        let expected = block["assets"][0]["signature"].as_str().unwrap();
        assert_eq!(signature(&lf).unwrap(), expected);
        assert_eq!(signature(&crlf).unwrap(), expected);
    }

    #[test]
    fn mixed_line_endings_cannot_hide_duplicate_signature_markers() {
        let (offer, block) = signature_fixture();
        let lf = with_block(offer, &block);
        let crlf = lf.release_body.as_ref().unwrap().replace('\n', "\r\n");
        for body in [
            format!("{}{crlf}", lf.release_body.as_ref().unwrap()),
            format!("{crlf}{}", lf.release_body.as_ref().unwrap()),
            format!("{crlf}\r\n<!-- mewu-update-signatures:v2\r\n{{}}\r\n-->"),
        ] {
            let mut duplicate = lf.clone();
            duplicate.release_body = Some(body);
            assert_eq!(signature(&duplicate), Err(INVALID));
        }
    }

    #[tokio::test]
    #[ignore = "Explicit read-only public GitHub check; never downloads or installs a package"]
    async fn live_github_latest_uses_real_release_api_and_native_selection() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(25))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let bytes = fetch(&client).await.unwrap();
        for target in [
            ReleaseTarget::WindowsX64,
            ReleaseTarget::MacArm64,
            ReleaseTarget::LinuxX64,
        ] {
            let selected = select(&bytes, env!("CARGO_PKG_VERSION"), target).unwrap();
            if let Some(offer) = selected {
                signature(&offer).unwrap();
                assert!(offer.bytes <= MAX_PACKAGE);
            }
        }
    }
}
