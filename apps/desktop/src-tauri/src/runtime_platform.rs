// SPDX-License-Identifier: MPL-2.0
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimePlatform {
    Windows,
    Macos,
    Linux,
    Other,
}

impl RuntimePlatform {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "windows" => Self::Windows,
            "macos" => Self::Macos,
            "linux" => Self::Linux,
            _ => Self::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_platform_matches_the_compiled_target_and_wire_contract() {
        let actual = serde_json::to_value(RuntimePlatform::current()).unwrap();
        let expected = match std::env::consts::OS {
            "windows" | "macos" | "linux" => std::env::consts::OS,
            _ => "other",
        };
        assert_eq!(actual, expected);
        assert_eq!(serde_json::to_value(RuntimePlatform::Macos).unwrap(), "macos");
    }
}
