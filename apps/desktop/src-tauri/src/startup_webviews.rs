// SPDX-License-Identifier: MPL-2.0
//! Tauri 2.12.1 creates configured webviews before calling the setup hook.
//! On Linux their default WebKit data directory overlaps the legacy business
//! root. Defer creation until storage resolution and Host installation succeed.
//! https://docs.rs/tauri/2.12.1/src/tauri/app.rs.html
use tauri::utils::config::{Config, WindowConfig};

pub(crate) fn defer(config: &mut Config) -> Vec<WindowConfig> {
    config
        .app
        .windows
        .iter_mut()
        .filter_map(|window| {
            if !window.create {
                return None;
            }
            let original = window.clone();
            window.create = false;
            Some(original)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_production_space_preserves_the_complete_native_configuration() {
        let mut config: Config = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let original = config.clone();
        let deferred = defer(&mut config);
        assert_eq!(deferred, original.app.windows);
        assert_eq!(deferred.len(), 1);
        assert_eq!(deferred[0].label, "space");
        assert!(!deferred[0].visible);
        assert!(deferred[0].transparent && deferred[0].fullscreen);
        assert!(config.app.windows.iter().all(|window| !window.create));
        for (window, original) in config.app.windows.iter_mut().zip(&original.app.windows) {
            window.create = original.create;
        }
        assert_eq!(config, original);
    }

    #[test]
    fn manual_window_templates_and_windowless_quit_are_not_created() {
        let mut config = Config::default();
        config.app.windows = vec![WindowConfig {
            label: "manual-template".into(),
            create: false,
            ..WindowConfig::default()
        }];
        let original = config.clone();
        assert!(defer(&mut config).is_empty());
        assert_eq!(config, original);
        config.app.windows.clear();
        assert!(defer(&mut config).is_empty());
        assert!(config.app.windows.is_empty());
    }
}
