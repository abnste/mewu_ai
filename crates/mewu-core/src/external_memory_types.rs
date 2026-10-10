// SPDX-License-Identifier: MPL-2.0
use serde::{Deserialize, Serialize};

// Add to AgentProfile with default {revision:0,bindingId:null}. SaveAgent must
// preserve this host-owned field, even when updating unrelated profile fields.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryProviderSelection {
    pub revision: u64,
    pub binding_id: Option<String>, // None = existing local SQLite provider.
}

// Public settings views contain no key, credential ID, arbitrary document URL,
// serialized HTTP request, or raw provider error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryPolicy {
    pub recall: bool,
    pub explicit_retain: bool,
    pub completed_turn_sync: bool, // default false
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingStatus {
    Provisioning,
    Ready,
    Suspended,
    Retired,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryBindingView {
    pub id: String,
    pub agent_id: String,
    pub revision: u64,        // permission/policy changes only
    pub ledger_revision: u64, // evidence/operation page invalidation
    pub provider_id: String,  // first implementation: hindsight
    pub plugin_id: String,
    pub contribution_id: String,
    pub plugin_revision: u64,
    pub endpoint: String, // normalized user-provided URL, no query/userinfo
    pub enabled: bool,
    pub status: BindingStatus,
    pub policy: MemoryPolicy,
    pub configured: bool, // never a key or a credential file path
    pub evidence_count: u64,
    pub pending_count: u64,
    pub blocked_count: u64,
    pub deletion_pending_count: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentMemoryStatus {
    pub selection: MemoryProviderSelection,
    pub bindings: Vec<MemoryBindingView>, // bounded page, not unlimited history
    pub next_cursor: Option<String>,
}

// Host-owned config: native generates binding_id and credential_id, writes its
// immutable encrypted memory/<binding>/<credential> blob, then calls reserve.
// On failed reserve native deletes only the new blob it just created.
// The core derives remote bank ID from binding UUID. Renderer/model never choose it.
pub struct HostNewMemoryBinding {
    pub id: String,
    pub agent_id: String,
    pub endpoint: String,
    pub credential_id: Option<String>,
    pub policy: MemoryPolicy,
    pub plugin_id: String,
    pub contribution_id: String,
    pub plugin_revision: u64,
}
// Host-only immutable route. No Serialize/Deserialize.
pub struct MemoryRoute {
    pub binding_id: String,
    pub endpoint: String,
    pub bank_id: String,
    pub credential_id: Option<String>,
    pub adapter_contract_version: u32,
}
// Native verified current plugin package/revision under plugins -> Engine.
// Core verifies the remaining binding, selection, Agent and run predicates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryAuthorityGrant {
    pub binding_id: String,
    pub binding_revision: u64,
    pub plugin_id: String,
    pub contribution_id: String,
    pub plugin_revision: u64,
}
// Persist on Run and copy into RunContext. No URL/credential reference required:
// immutable binding route remains in its SQL row. Workflow and old Run default None.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RunMemoryAuthority {
    Local {
        selection_revision: u64,
    },
    External {
        selection_revision: u64,
        grant: MemoryAuthorityGrant,
        completed_turn_sync: bool,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReserveBindingReceipt {
    pub binding: MemoryBindingView,
    pub operation_id: String,
    pub selection_revision: u64, // captured for later explicit selection CAS
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingMutationReceipt {
    pub binding: MemoryBindingView,
    pub selection: MemoryProviderSelection,
}

// Evidence is immutable. A correction is new evidence plus forgetting the old.
// Repeated source event/request UUID never re-enters a forgotten document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceOrigin {
    Manual,
    UserQuote,
    CompletedTurn,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceSource {
    pub origin: EvidenceOrigin,
    pub scene_id: Option<String>,
    pub user_message_id: Option<String>,
    pub assistant_message_id: Option<String>,
    pub run_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceState {
    Queued,
    Accepted,
    Committed,
    Blocked,
    Unknown,
    Failed,
    Suppressed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteDeleteState {
    Pending,
    CurrentlyAbsent,
    ConfirmedDeleted,
    Unknown,
    Failed,
    NeedsAuthorization,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quiescence {
    Proven,
    Unproven,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForgetReceipt {
    pub evidence_id: String,
    pub revision: u64,
    pub local_suppressed: bool,
    pub remote_state: RemoteDeleteState,
    pub quiescence: Quiescence,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteReceipt {
    pub evidence_id: String,
    pub revision: u64,
    pub operation_id: Option<String>, // blocked-before-queue has no HTTP operation
    pub state: EvidenceState,
    pub reason: Option<MemoryReason>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryEvidenceView {
    pub id: String,
    pub binding_id: String,
    pub revision: u64,
    pub source: EvidenceSource,
    pub preview: String, // at most 800 Unicode scalars
    pub state: EvidenceState,
    pub reason: Option<MemoryReason>,
    pub forgotten: Option<ForgetReceipt>,
    pub created_at: u64,
    pub updated_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryEvidencePage {
    pub entries: Vec<MemoryEvidenceView>,
    pub total: u64,
    pub revision: u64, // binding ledger_revision
    pub next_cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryEvidenceDetail {
    pub summary: MemoryEvidenceView,
    pub attributed_text: Option<String>, // <=64KiB; None after local suppression
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryReason {
    QueueFull,
    EvidenceTooLarge,
    AuthorityChanged,
    ProviderUnavailable,
    ProviderRejected,
    InvalidResponse,
    OutcomeUnknown,
    CancelledBeforeDispatch,
    CleanupNeedsAuthorization,
}

// One durable outbox. These are database rows/claims, not a second native queue.
// Host polls bounded pending rows, verifies plugin permission, then claims ONE.
#[derive(Clone, Debug)]
pub struct PendingMemoryWork {
    pub operation_id: String,
    pub revision: u64,
    pub grant: MemoryAuthorityGrant,
    pub next_attempt_at: u64,
}
// Native passes this directly to its Hindsight adapter, matching the action.
// The Store chooses action from durable state. Caller cannot force a resend.
pub enum MemoryWorkAction {
    CreateBank,
    InspectBank,
    Retain {
        document_id: String,
        operation_id: String,
        attributed_text: String,
        payload_sha256: String,
    },
    PollRetain {
        document_id: String,
        operation_id: String,
    },
    CancelRetain {
        document_id: String,
        operation_id: String,
    },
    DeleteDocument {
        document_id: String,
    },
    InspectDocument {
        document_id: String,
    },
}
pub struct MemoryDispatch {
    pub lease: MemoryWorkLease,
    pub grant: MemoryAuthorityGrant,
    pub route: MemoryRoute,
    pub action: MemoryWorkAction,
}
// Opaque host-only token; actual fields private to core. Native keeps it until
// one record call. It identifies row + claim nonce + fixed route/payload hash.
pub struct MemoryWorkLease {
    pub(crate) operation_id: String,
    pub(crate) nonce: String,
    pub(crate) revision: u64,
    pub(crate) binding_id: String,
    pub(crate) action: String,
}

// Adapter DTOs do not decide ledger Committed/Deleted states. Host validates
// bounded wire + exact identities, then passes an observation to the ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteOperationState {
    Pending,
    Processing,
    Completed,
    Failed,
    Cancelled,
    NotFound,
}
pub enum MemoryWorkObservation {
    BankReady {
        bank_id: String,
        service_version: String,
        observations_enabled: bool, // must be false for this adapter contract
    },
    BankAbsent {
        bank_id: String,
    },
    RetainAccepted {
        bank_id: String,
        operation_id: String,
    },
    Operation {
        bank_id: String,
        operation_id: String,
        state: RemoteOperationState,
    },
    CancelIntentObserved {
        bank_id: String,
        operation_id: String,
    },
    DocumentDeleteObserved {
        bank_id: String,
        document_id: String,
        absent: bool,
    },
    DocumentObserved {
        bank_id: String,
        document_id: String,
        present: bool,
    },
    NotDispatched {
        reason: MemoryReason,
    },
    MutationOutcomeUnknown {
        reason: MemoryReason,
    },
    ReadUnavailable {
        reason: MemoryReason,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryWorkReceipt {
    pub operation_id: String,
    pub operation_revision: u64,
    pub evidence: Option<WriteReceipt>,
    pub deletion: Option<ForgetReceipt>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllowedMemoryDocument {
    pub document_id: String,
    pub evidence_id: String,
    pub evidence_revision: u64,
    pub source: EvidenceSource,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryRecoverySummary {
    pub uncertain_operations: u64,
    pub blocked_operations: u64,
}

impl MemoryProviderSelection {
    pub fn is_default(&self) -> bool {
        self.revision == 0 && self.binding_id.is_none()
    }
}
impl Default for MemoryPolicy {
    fn default() -> Self {
        Self {
            recall: true,
            explicit_retain: true,
            completed_turn_sync: false,
        }
    }
}
