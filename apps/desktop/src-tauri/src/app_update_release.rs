// SPDX-License-Identifier: MPL-2.0
//! First-party payload contracts. Installer media and updater media are distinct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseTarget {
    WindowsX64,
    MacArm64,
    LinuxX64,
}

impl ReleaseTarget {
    pub fn manifest_key(self) -> &'static str {
        match self {
            Self::WindowsX64 => "windows-x86_64",
            Self::MacArm64 => "darwin-aarch64",
            Self::LinuxX64 => "linux-x86_64",
        }
    }

    pub fn payload_path(self, version: &str) -> String {
        match self {
            Self::WindowsX64 => format!("/abnste/mewu_ai/releases/download/v{version}/MewuAI-Setup-{version}-win-x64.exe"),
            Self::MacArm64 => format!("/abnste/mewu_ai/releases/download/v{version}/MewuAI-Update-{version}-mac-arm64.app.tar.gz"),
            Self::LinuxX64 => format!("/abnste/mewu_ai/releases/download/v{version}/MewuAI-{version}-linux-x64.AppImage"),
        }
    }

    pub fn for_platform(os: &str, arch: &str) -> Option<Self> {
        match (os, arch) {
            ("windows", "x86_64") => Some(Self::WindowsX64),
            ("macos", "aarch64") => Some(Self::MacArm64),
            ("linux", "x86_64") => Some(Self::LinuxX64),
            _ => None,
        }
    }
}

pub fn validate_release_for(
    target: ReleaseTarget,
    current: &str,
    version: &str,
    url: &url::Url,
) -> Result<(), &'static str> {
    if version.len() > 64 {
        return Err("update_invalid");
    }
    let current = semver::Version::parse(current).map_err(|_| "update_invalid")?;
    let candidate = semver::Version::parse(version).map_err(|_| "update_invalid")?;
    if candidate <= current || !candidate.build.is_empty() || candidate.major < 1 {
        return Err("update_invalid");
    }
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != target.payload_path(version)
    {
        return Err("update_invalid");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_updater_uses_latest_release_without_a_legacy_endpoint_fallback() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let updater = &config["plugins"]["updater"];
        assert_eq!(
            updater["endpoints"],
            serde_json::json!([
                "https://api.github.com/repos/abnste/mewu_ai/releases/latest"
            ])
        );
        assert_eq!(updater["requireSignedVersion"], true);
    }

    fn url(target: ReleaseTarget, version: &str) -> url::Url {
        url::Url::parse(&format!(
            "https://github.com{}",
            target.payload_path(version)
        ))
        .unwrap()
    }
    #[test]
    fn target_contracts_never_substitute_installer_architecture_or_other_platform() {
        let targets = [
            ReleaseTarget::WindowsX64,
            ReleaseTarget::MacArm64,
            ReleaseTarget::LinuxX64,
        ];
        for target in targets {
            for candidate in targets {
                assert_eq!(
                    validate_release_for(
                        target,
                        "1.0.0-preview.1",
                        "1.0.0",
                        &url(candidate, "1.0.0")
                    )
                    .is_ok(),
                    target == candidate
                );
            }
            let valid = url(target, "1.0.0");
            for wrong in [
                valid.as_str().replace("https:", "http:"),
                valid.as_str().replace("abnste/", "someone/"),
                valid.as_str().replace("mewu_ai/", "other-repository/"),
                valid.as_str().replace("github.com/", "github.com.evil/"),
                format!("{valid}?token=x"),
                format!("{valid}#asset"),
                valid.as_str().replace("github.com/", "user@github.com/"),
                valid.as_str().replace("github.com/", "github.com:444/"),
                valid.as_str().replace("1.0.0", "1.0.1"),
                valid
                    .as_str()
                    .replace("mac-arm64", "mac-x64")
                    .replace("linux-x64", "linux-arm64")
                    .replace("win-x64", "win-arm64"),
            ] {
                assert!(
                    validate_release_for(
                        target,
                        "1.0.0-preview.1",
                        "1.0.0",
                        &url::Url::parse(&wrong).unwrap()
                    )
                    .is_err(),
                    "{wrong}"
                );
            }
        }
    }

    #[test]
    fn stable_mac_update_requires_its_versioned_tar_not_a_dmg_or_legacy_channel() {
        let valid = url::Url::parse("https://github.com/abnste/mewu_ai/releases/download/v1.0.1/MewuAI-Update-1.0.1-mac-arm64.app.tar.gz").unwrap();
        assert!(validate_release_for(ReleaseTarget::MacArm64, "1.0.0", "1.0.1", &valid).is_ok());
        for bad in [
            "https://github.com/abnste/mewu_ai/releases/download/update-channel/MewuAI-Update-1.0.1-mac-arm64.app.tar.gz",
            "https://github.com/abnste/mewu_ai/releases/download/v1.0.1/MewuAI-1.0.1-mac-arm64.dmg",
            "https://github.com/abnste/mewu_ai/releases/latest/download/MewuAI-Update-1.0.1-mac-arm64.app.tar.gz",
            "https://github.com/abnste/mewu_ai/releases/download/v1.0.0/MewuAI-Update-1.0.1-mac-arm64.app.tar.gz",
            "https://github.com/abnste/mewu_ai/releases/download/v1.0.1/MewuAI-Update-1.0.2-mac-arm64.app.tar.gz",
        ] {
            assert!(
                validate_release_for(
                    ReleaseTarget::MacArm64,
                    "1.0.0",
                    "1.0.1",
                    &url::Url::parse(bad).unwrap()
                )
                .is_err(),
                "{bad}"
            );
        }
    }
    #[test]
    fn release_version_is_bound_to_payload_and_only_supported_platform_pairs_exist() {
        for target in [
            ReleaseTarget::WindowsX64,
            ReleaseTarget::MacArm64,
            ReleaseTarget::LinuxX64,
        ] {
            for version in [
                "0.9.9",
                "1.0.0-alpha.1",
                "1.0.0-preview.1",
                "1.0.0+unsigned",
                "invalid",
            ] {
                assert!(validate_release_for(
                    target,
                    "1.0.0-preview.1",
                    version,
                    &url(target, version)
                )
                .is_err());
            }
            assert!(validate_release_for(
                target,
                "1.0.0",
                "1.0.0-preview.2",
                &url(target, "1.0.0-preview.2")
            )
            .is_err());
        }
        assert_eq!(
            ReleaseTarget::for_platform("macos", "aarch64")
                .unwrap()
                .manifest_key(),
            "darwin-aarch64"
        );
        assert_eq!(
            ReleaseTarget::for_platform("linux", "x86_64")
                .unwrap()
                .manifest_key(),
            "linux-x86_64"
        );
        assert_eq!(
            ReleaseTarget::for_platform("windows", "x86_64")
                .unwrap()
                .manifest_key(),
            "windows-x86_64"
        );
        for pair in [
            ("macos", "x86_64"),
            ("linux", "aarch64"),
            ("windows", "aarch64"),
            ("darwin", "aarch64"),
        ] {
            assert_eq!(ReleaseTarget::for_platform(pair.0, pair.1), None);
        }
    }
}
