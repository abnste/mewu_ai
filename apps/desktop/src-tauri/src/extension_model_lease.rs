// SPDX-License-Identifier: MPL-2.0
//! Host-only credentials for one native model run. Never serialized to workers.
//! Reuse the key already validated by run preflight instead of reopening the OS
//! keychain during each plugin model request. Live Store authority is checked
//! separately by Context before and throughout every request.
use mewu_core::ConnectionProfile;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use zeroize::Zeroizing;

pub(crate) struct ModelLease {
    scene_id: String,
    run_id: String,
    connection: ConnectionProfile,
    canceled: Arc<AtomicBool>,
    key: Arc<Zeroizing<String>>,
}

impl ModelLease {
    pub(crate) fn new(
        scene_id: String,
        run_id: String,
        connection: ConnectionProfile,
        canceled: Arc<AtomicBool>,
        key: Arc<Zeroizing<String>>,
    ) -> Self {
        Self {
            scene_id,
            run_id,
            connection,
            canceled,
            key,
        }
    }

    pub(crate) fn connection(&self) -> &ConnectionProfile {
        &self.connection
    }

    pub(crate) fn key(
        &self,
        scene_id: &str,
        native_run: Option<&(String, String)>,
        canceled: &Arc<AtomicBool>,
        connection: &ConnectionProfile,
    ) -> Result<&str, String> {
        if self.canceled.load(Ordering::SeqCst)
            || !Arc::ptr_eq(canceled, &self.canceled)
            || scene_id != self.scene_id
            || !native_run
                .is_some_and(|(scene, run)| scene == &self.scene_id && run == &self.run_id)
            || connection != &self.connection
        {
            return Err("本轮模型授权已变化".into());
        }
        Ok(self.key.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> ConnectionProfile {
        serde_json::from_value(serde_json::json!({
            "id":"connection", "name":"test", "providerId":"Custom",
            "baseUrl":"https://example.invalid/v1", "model":"test-model",
            "hasKey":true, "credentialId":"credential", "revision":1
        }))
        .unwrap()
    }

    #[test]
    fn model_key_is_shared_only_with_the_exact_native_run_and_cancel_token() {
        let canceled = Arc::new(AtomicBool::new(false));
        let secret = Arc::new(Zeroizing::new("test-only-key".to_owned()));
        let lease = ModelLease::new(
            "scene".into(),
            "run".into(),
            profile(),
            canceled.clone(),
            secret.clone(),
        );
        let owner = ("scene".into(), "run".into());
        for _ in 0..3 {
            assert_eq!(
                lease
                    .key("scene", Some(&owner), &canceled, &profile())
                    .unwrap(),
                "test-only-key"
            );
        }
        assert_eq!(
            Arc::strong_count(&secret),
            2,
            "requests borrow the preflight key without duplicating it"
        );
        assert!(lease.key("scene", None, &canceled, &profile()).is_err());
        assert!(lease
            .key("other", Some(&owner), &canceled, &profile())
            .is_err());
        assert!(lease
            .key(
                "scene",
                Some(&("scene".into(), "other".into())),
                &canceled,
                &profile()
            )
            .is_err());
        assert!(lease
            .key(
                "scene",
                Some(&owner),
                &Arc::new(AtomicBool::new(false)),
                &profile()
            )
            .is_err());
        let mut changed = profile();
        changed.revision += 1;
        assert!(lease
            .key("scene", Some(&owner), &canceled, &changed)
            .is_err());
        canceled.store(true, Ordering::SeqCst);
        assert!(lease
            .key("scene", Some(&owner), &canceled, &profile())
            .is_err());
        drop(lease);
        assert_eq!(Arc::strong_count(&secret), 1);
    }
}
