// SPDX-License-Identifier: MPL-2.0
//! The only background dispatcher. The Store chooses actions and persists every
//! transition; this module never keeps another queue or retries mutation bytes.
#[cfg(test)]
#[path = "memory_worker_http_tests.rs"]
mod http_tests;

use crate::{credentials, hindsight as http, memory_host, Host};
use mewu_core::{
    MemoryDispatch, MemoryReason as Reason, MemoryWorkAction as Action,
    MemoryWorkObservation as Observation, RemoteOperationState,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as u64
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        {
            let host = app.state::<Host>();
            let recovered = host.lock().and_then(|mut engine| {
                engine
                    .store
                    .recover_memory_work()
                    .map_err(crate::agent_tools::domain_error)
            });
            if recovered.is_err() {
                let _ = app.emit_to("settings", "host-error", "无法恢复记忆任务，请重新启动");
                return;
            }
        }
        loop {
            let result = dispatch_one(&app).await;
            if result == Ok(true) {
                continue;
            }
            if result.is_err() {
                // An unrecorded claim stays durable for restart recovery. Never
                // send it a second time or disguise a storage failure as success.
                let _ = app.emit_to("settings", "host-error", "无法保存记忆任务状态，请重新启动");
                return;
            }
            let host = app.state::<Host>();
            tokio::select! {
                _ = host.memory.wake.notified() => (),
                _ = tokio::time::sleep(Duration::from_secs(5)) => (),
            }
        }
    });
}

async fn dispatch_one(app: &AppHandle) -> Result<bool, String> {
    let host = app.state::<Host>();
    let work = {
        if !host.exit.is_running() {
            return Ok(false);
        }
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        let mut engine = host.lock()?;
        let pending = engine
            .store
            .pending_memory_work(now(), 32)
            .map_err(crate::agent_tools::domain_error)?;
        let mut selected = None;
        for candidate in pending {
            if memory_host::authorize(&plugins.store, &candidate.grant).is_err() {
                // Persist revocation so an abandoned front page cannot starve
                // authorized work belonging to later bindings. Only an explicit
                // user selection may grant the plugin's new revision again.
                let binding = engine
                    .store
                    .memory_binding(&candidate.grant.binding_id)
                    .map_err(crate::agent_tools::domain_error)?;
                engine
                    .store
                    .set_memory_binding_policy(
                        &binding.id,
                        binding.revision,
                        binding.plugin_revision,
                        false,
                        binding.policy,
                    )
                    .map_err(crate::agent_tools::domain_error)?;
                host.memory
                    .cancel_where(|task| task.binding_id == binding.id);
                let snapshot = engine.store.snapshot();
                crate::cancel_ended_runs(&mut engine, &snapshot);
                crate::publish(app, &snapshot);
                memory_host::changed(app, &binding.agent_id);
                continue;
            }
            let binding = engine
                .store
                .memory_binding(&candidate.grant.binding_id)
                .map_err(crate::agent_tools::domain_error)?;
            let (permit, receive) = match host.memory.register(memory_host::origin(&binding)) {
                Ok(value) => value,
                Err(_) => return Ok(false),
            };
            if !host.exit.is_running() {
                return Ok(false);
            }
            if let Some(dispatch) = engine
                .store
                .claim_memory_work(
                    &candidate.operation_id,
                    candidate.revision,
                    &candidate.grant,
                    now(),
                )
                .map_err(crate::agent_tools::domain_error)?
            {
                selected = Some((dispatch, permit, receive, binding.agent_id));
                break;
            }
        }
        selected
    };
    let Some((dispatch, permit, mut receive, agent_id)) = work else {
        return Ok(false);
    };
    let observation = match credentials::read_memory(
        &host.root,
        &dispatch.route.binding_id,
        dispatch.route.credential_id.as_deref(),
    ) {
        Ok(key) => {
            let mut control = http::Control {
                deadline: tokio::time::Instant::now() + Duration::from_secs(35),
                cancel: &mut receive,
            };
            execute(
                &dispatch,
                if key.is_empty() {
                    None
                } else {
                    Some(key.as_str())
                },
                &mut control,
                || authorized(app, &dispatch),
            )
            .await
        }
        Err(_) => Observation::NotDispatched {
            reason: Reason::ProviderUnavailable,
        },
    };
    // Receipts survive permission revocation and shutdown. They document what
    // happened to an existing claim; they do not authorize another request.
    host.lock()?
        .store
        .record_memory_work(dispatch.lease, observation, now())
        .map_err(crate::agent_tools::domain_error)?;
    memory_host::changed(app, &agent_id);
    drop(permit);
    Ok(true)
}

fn authorized(app: &AppHandle, dispatch: &MemoryDispatch) -> bool {
    let host = app.state::<Host>();
    let Ok(plugins) = host.plugins.lock() else {
        return false;
    };
    let Ok(engine) = host.lock() else {
        return false;
    };
    host.exit.is_running()
        && memory_host::authorize(&plugins.store, &dispatch.grant).is_ok()
        && engine
            .store
            .memory_work_is_authorized(&dispatch.lease, &dispatch.grant)
            .unwrap_or(false)
}

fn preflight_failure(mut error: http::AdapterError) -> Observation {
    // A GET has reached the service, but the claimed PUT/POST has not.
    error.delivery = http::Delivery::NotDispatched;
    unavailable(error, false)
}

fn revoked(mutated: bool) -> Observation {
    if mutated {
        Observation::MutationOutcomeUnknown {
            reason: Reason::OutcomeUnknown,
        }
    } else {
        Observation::NotDispatched {
            reason: Reason::AuthorityChanged,
        }
    }
}

fn unavailable(error: http::AdapterError, mutation_started: bool) -> Observation {
    let reason = match error.code {
        http::ErrorCode::Cancelled => Reason::CancelledBeforeDispatch,
        http::ErrorCode::Http(401 | 403) | http::ErrorCode::UnsupportedVersion => {
            Reason::ProviderRejected
        }
        http::ErrorCode::InvalidInput
        | http::ErrorCode::InvalidResponse
        | http::ErrorCode::IdentityMismatch
        | http::ErrorCode::BodyTooLarge => Reason::InvalidResponse,
        _ => Reason::ProviderUnavailable,
    };
    if mutation_started || error.delivery == http::Delivery::MutationMayHaveReachedServer {
        Observation::MutationOutcomeUnknown {
            reason: Reason::OutcomeUnknown,
        }
    } else if error.delivery == http::Delivery::NotDispatched {
        Observation::NotDispatched { reason }
    } else {
        Observation::ReadUnavailable { reason }
    }
}

async fn execute(
    dispatch: &MemoryDispatch,
    key: Option<&str>,
    control: &mut http::Control<'_>,
    authorized: impl Fn() -> bool,
) -> Observation {
    if !authorized() {
        return revoked(false);
    }
    let adapter = match http::HindsightHttp::new(&dispatch.route.endpoint) {
        Ok(value) => value,
        Err(error) => return unavailable(error, false),
    };
    if dispatch.route.adapter_contract_version != 1 {
        return Observation::NotDispatched {
            reason: Reason::InvalidResponse,
        };
    }
    let bank = &dispatch.route.bank_id;
    match &dispatch.action {
        Action::CreateBank => {
            let version = match adapter.probe(key, control).await {
                Ok(value) => value.api_version,
                Err(error) => return preflight_failure(error),
            };
            if !authorized() {
                return revoked(false);
            }
            match adapter.inspect_bank(bank, key, control).await {
                Ok(http::BankLookup::Missing) => (),
                Ok(http::BankLookup::Present(_)) => {
                    return Observation::NotDispatched {
                        reason: Reason::ProviderRejected,
                    }
                }
                Err(error) => return preflight_failure(error),
            }
            if !authorized() {
                return revoked(false);
            }
            if let Err(error) = adapter.create_bank_once(bank, key, control).await {
                return unavailable(error, false);
            }
            if !authorized() {
                return revoked(true);
            }
            match adapter.inspect_bank(bank, key, control).await {
                Ok(http::BankLookup::Present(value)) if !value.observations_enabled => {
                    Observation::BankReady {
                        bank_id: value.bank_id,
                        service_version: version,
                        observations_enabled: value.observations_enabled,
                    }
                }
                Ok(_) => Observation::MutationOutcomeUnknown {
                    reason: Reason::OutcomeUnknown,
                },
                Err(error) => unavailable(error, true),
            }
        }
        Action::InspectBank => {
            let version = match adapter.probe(key, control).await {
                Ok(value) => value.api_version,
                Err(error) => return unavailable(error, false),
            };
            if !authorized() {
                return revoked(false);
            }
            match adapter.inspect_bank(bank, key, control).await {
                Ok(http::BankLookup::Missing) => Observation::BankAbsent {
                    bank_id: bank.clone(),
                },
                Ok(http::BankLookup::Present(value)) if !value.observations_enabled => {
                    Observation::BankReady {
                        bank_id: value.bank_id,
                        service_version: version,
                        observations_enabled: value.observations_enabled,
                    }
                }
                Ok(_) => Observation::ReadUnavailable {
                    reason: Reason::InvalidResponse,
                },
                Err(error) => unavailable(error, false),
            }
        }
        Action::Retain {
            document_id,
            operation_id,
            attributed_text,
            payload_sha256,
        } => {
            match adapter.inspect_bank(bank, key, control).await {
                Ok(http::BankLookup::Present(value)) if !value.observations_enabled => (),
                Ok(_) => {
                    return Observation::NotDispatched {
                        reason: Reason::ProviderRejected,
                    }
                }
                Err(error) => return preflight_failure(error),
            }
            if !authorized() {
                return revoked(false);
            }
            match adapter
                .retain_once(
                    bank,
                    http::RetainInput {
                        operation_id,
                        document_id,
                        attributed_text,
                        payload_sha256,
                    },
                    key,
                    control,
                )
                .await
            {
                Ok(value) => Observation::RetainAccepted {
                    bank_id: value.bank_id,
                    operation_id: value.operation_id,
                },
                Err(error) => unavailable(error, false),
            }
        }
        Action::PollRetain { operation_id, .. } => match adapter
            .operation_status(bank, operation_id, key, control)
            .await
        {
            Ok(value) => Observation::Operation {
                bank_id: bank.clone(),
                operation_id: value.operation_id,
                state: match value.state {
                    http::OperationState::Pending => RemoteOperationState::Pending,
                    http::OperationState::Processing => RemoteOperationState::Processing,
                    http::OperationState::Completed => RemoteOperationState::Completed,
                    http::OperationState::Failed => RemoteOperationState::Failed,
                    http::OperationState::Cancelled => RemoteOperationState::Cancelled,
                    http::OperationState::NotFound => RemoteOperationState::NotFound,
                },
            },
            Err(error) => unavailable(error, false),
        },
        Action::CancelRetain { operation_id, .. } => match adapter
            .cancel_operation(bank, operation_id, key, control)
            .await
        {
            Ok(http::CancelObservation::Acknowledged) => Observation::CancelIntentObserved {
                bank_id: bank.clone(),
                operation_id: operation_id.clone(),
            },
            Ok(http::CancelObservation::NotFound) => Observation::Operation {
                bank_id: bank.clone(),
                operation_id: operation_id.clone(),
                state: RemoteOperationState::NotFound,
            },
            Ok(http::CancelObservation::AlreadyTerminal) => {
                if !authorized() {
                    return revoked(true);
                }
                match adapter
                    .operation_status(bank, operation_id, key, control)
                    .await
                {
                    Ok(value) => Observation::Operation {
                        bank_id: bank.clone(),
                        operation_id: value.operation_id,
                        state: match value.state {
                            http::OperationState::Pending => RemoteOperationState::Pending,
                            http::OperationState::Processing => RemoteOperationState::Processing,
                            http::OperationState::Completed => RemoteOperationState::Completed,
                            http::OperationState::Failed => RemoteOperationState::Failed,
                            http::OperationState::Cancelled => RemoteOperationState::Cancelled,
                            http::OperationState::NotFound => RemoteOperationState::NotFound,
                        },
                    },
                    Err(error) => unavailable(error, true),
                }
            }
            Err(error) => unavailable(error, false),
        },
        Action::DeleteDocument { document_id } => match adapter
            .delete_document_once(bank, document_id, key, control)
            .await
        {
            Ok(_) => Observation::DocumentDeleteObserved {
                bank_id: bank.clone(),
                document_id: document_id.clone(),
                absent: true,
            },
            Err(error) => unavailable(error, false),
        },
        Action::InspectDocument { document_id } => match adapter
            .inspect_document(bank, document_id, key, control)
            .await
        {
            Ok(value) => Observation::DocumentObserved {
                bank_id: bank.clone(),
                document_id: document_id.clone(),
                present: matches!(value, http::DocumentObservation::Present { .. }),
            },
            Err(error) => unavailable(error, false),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_late_read_failure_cannot_turn_a_dispatched_mutation_into_safe_retry() {
        let read = http::AdapterError {
            code: http::ErrorCode::Deadline,
            delivery: http::Delivery::ReadOnly,
        };
        assert!(matches!(
            unavailable(read, true),
            Observation::MutationOutcomeUnknown { .. }
        ));
        assert!(matches!(
            unavailable(read, false),
            Observation::ReadUnavailable { .. }
        ));
        let canceled = http::AdapterError {
            code: http::ErrorCode::Cancelled,
            delivery: http::Delivery::MutationMayHaveReachedServer,
        };
        assert!(matches!(
            unavailable(canceled, false),
            Observation::MutationOutcomeUnknown { .. }
        ));
    }
}
