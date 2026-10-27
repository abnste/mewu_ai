// SPDX-License-Identifier: MPL-2.0
use crate::{Asset, MessageRole, Snapshot};
use serde_json::Value;

/// Shared by the execution loop and durable journal validation. A normal
/// inspect → generate → validate → correct → package workflow needs more than
/// six model turns. Keep the event budget bounded (12 model + 48 tool receipts).
pub const MAX_AGENT_MODEL_ROUNDS: u32 = 12;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, try_from = "String")]
pub enum RecordedRunKind {
    Chat,
    Workflow,
    Continuation,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, try_from = "String")]
pub enum RecordedRunStatus {
    Running,
    Completed,
    Failed,
    Canceled,
    Interrupted,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, try_from = "String")]
pub enum ReceiptPhase {
    Prepared,
    Dispatched,
    Responded,
    NotSent,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, try_from = "String")]
pub enum ReceiptKind {
    Model,
    Tool,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, try_from = "String")]
pub enum ContentUnavailable {
    Oversized,
    UnsupportedContent,
    MemoryPolicy,
    HistoryPolicy,
    NotReceived,
    InvalidResponse,
    LegacyNotRecorded,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, try_from = "String")]
pub enum ContinueBlockedReason {
    Running,
    SceneClosed,
    SceneFrozen,
    SceneInactive,
    ConversationChanged,
    NoConfirmedContent,
    LegacyNotRecorded,
    ConnectionUnselected,
    MandatoryContextTooLarge,
    SelectionTooLarge,
    ContextUnavailable,
}

// Output only: stable ownership is looked up from Store, never trusted from IPC.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunJournalSummary {
    pub run_id: String,
    pub scene_id: String,
    pub agent_id: String,
    pub user_message_id: String,
    pub kind: RecordedRunKind,
    pub status: RecordedRunStatus,
    pub revision: u64,
    pub checkpoint_seq: u64,
    pub created_at: u64,
    pub updated_at: u64,
    pub can_continue: bool,
    pub continue_blocked_reason: Option<ContinueBlockedReason>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JournalEntrySummary {
    pub id: String,
    pub sequence: u64,
    pub kind: ReceiptKind,
    pub phase: ReceiptPhase,
    pub round: u32,
    pub tool_call_id: Option<String>,
    pub label: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub returned_error: Option<bool>,
    pub content_bytes: u64,
    pub content_sha256: Option<String>,
    pub content_unavailable: Option<ContentUnavailable>,
    pub response_kind: Option<ToolResponseKind>,
    pub observed_encoding: Option<String>,
    pub observed_bytes: Option<u64>,
    pub observed_sha256: Option<String>,
    pub selectable: bool,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JournalPage {
    pub summary: RunJournalSummary,
    pub entries: Vec<JournalEntrySummary>,
    pub continuation: ContinuationDecision,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunJournalPage {
    pub runs: Vec<RunJournalSummary>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "type",
    rename_all_fields = "camelCase",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum JournalContent {
    Text { text: String },
    Json { value: Value },
    Omitted { reason: ContentUnavailable },
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JournalEntryDetail {
    pub run_id: String,
    pub journal_revision: u64,
    pub entry: JournalEntrySummary,
    pub binding: Option<ToolBinding>,
    pub arguments_wire_sha256: Option<String>,
    pub content: JournalContent,
    pub references: Vec<EvidenceReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_sent_reason: Option<NotSentReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_diagnostic: Option<ArgumentDiagnostic>,
}

/// Host-produced argument metadata, never raw JSON, values or unknown keys.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ArgumentDiagnosticReason {
    Required,
    Type,
    AdditionalProperties,
    Const,
    Enum,
    Minimum,
    Maximum,
    ExclusiveMinimum,
    ExclusiveMaximum,
    MinLength,
    MaxLength,
    MinItems,
    MaxItems,
    OneOf,
    Pattern,
    Syntax,
    Eof,
    Data,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum ArgumentExpectedType {
    Null,
    Boolean,
    Number,
    Integer,
    String,
    Array,
    Object,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArgumentDiagnostic {
    pub field_path: String,
    pub reason: ArgumentDiagnosticReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_type: Option<ArgumentExpectedType>,
}

impl ArgumentDiagnostic {
    pub fn new(
        field_path: String,
        reason: ArgumentDiagnosticReason,
        expected_type: Option<ArgumentExpectedType>,
    ) -> Option<Self> {
        let value = Self {
            field_path,
            reason,
            expected_type,
        };
        value.is_valid().then_some(value)
    }

    pub(crate) fn is_valid(&self) -> bool {
        if self.field_path.len() > 256
            || (self.expected_type.is_some() && self.reason != ArgumentDiagnosticReason::Type)
        {
            return false;
        }
        let mut parts = self.field_path.split('/');
        if parts.next() != Some("$") {
            return false;
        }
        let mut previous = "";
        let mut array = "";
        let mut numeric_depth = 0;
        let mut count = 0;
        for part in parts {
            count += 1;
            if count > 16 {
                return false;
            }
            if part.bytes().all(|b| b.is_ascii_digit()) && !part.is_empty() {
                let limit = match previous {
                    "groups" => 6,
                    "objects" => 48,
                    "points" => 128,
                    "header" | "align" => 32,
                    "rows" => 199,
                    _ if array == "rows" && numeric_depth == 1 => 32,
                    _ => return false,
                };
                if part.len() > 3
                    || (part.len() > 1 && part.starts_with('0'))
                    || part.parse::<usize>().map_or(true, |index| index >= limit)
                {
                    return false;
                }
                numeric_depth += 1;
            } else {
                if !matches!(
                    part,
                    "coordinateSpace"
                        | "styleUnit"
                        | "timeSpace"
                        | "objects"
                        | "targetHandle"
                        | "interval"
                        | "startFrameHandle"
                        | "endHandle"
                        | "primitive"
                        | "kind"
                        | "bounds"
                        | "x"
                        | "y"
                        | "width"
                        | "height"
                        | "style"
                        | "color"
                        | "strokeWidth"
                        | "opacity"
                        | "topLeft"
                        | "anchor"
                        | "text"
                        | "fontSize"
                        | "groups"
                        | "target"
                        | "points"
                        | "table"
                        | "version"
                        | "header"
                        | "rows"
                        | "align"
                        | "tex"
                ) {
                    return false;
                }
                array = part;
                numeric_depth = 0;
            }
            previous = part;
        }
        true
    }
}

// Values are constructed ONLY by the host. These metadata fields may be exposed
// in JournalEntryDetail; none grants authority to execute. No endpoint,
// executable/argv, API key, environment, credential ID or raw arguments.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "type",
    rename_all_fields = "camelCase",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ToolBinding {
    Extension {
        plugin_id: String,
        plugin_revision: u64,
        version: String,
        sha256: String,
        service_id: String,
        advertised_alias: String,
        name: String,
    },
    VideoAnnotation {
        plugin_id: String,
        plugin_revision: u64,
        contribution_id: String,
        name: String,
        target_manifest_sha256: String,
    },
    VisualAnnotation {
        plugin_id: String,
        plugin_revision: u64,
        contribution_id: String,
        name: String,
        target_manifest_sha256: String,
    },
    Mcp {
        server_id: String,
        revision: u64,
        name: String,
        advertised_alias: String,
    },
    Local {
        name: String,
    },
    ExternalMemory {
        binding_id: String,
        binding_revision: u64,
        plugin_id: String,
        plugin_revision: u64,
        contribution_id: String,
        name: String,
    },
    Rejected {
        advertised_name: String,
    },
}

/// A deterministic model-safe alias. Identity fields remain separately recorded
/// and checked; a model-visible name is never an execution capability.
pub fn extension_tool_alias(plugin_id: &str, service_id: &str) -> String {
    let digest = crate::journal::digest(format!("{plugin_id}\0{service_id}").as_bytes());
    format!("ext_{}", &digest[..60])
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "type",
    rename_all_fields = "camelCase",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EvidenceReference {
    LocalMemory {
        id: String,
        revision: u64,
    },
    ExternalMemory {
        binding_id: String,
        evidence_id: String,
        operation_id: Option<String>,
        revision: u64,
    },
    History {
        scene_id: String,
        message_id: String,
    },
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedTool {
    pub call_id: String,
    pub binding: ToolBinding,
    pub arguments_wire_sha256: String,
}
// All fields are private in product; not Deserialize and no IPC serialisation.
// Nonce fences a specific prepared receipt, not permission to execute a tool.
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchLease {
    pub(crate) run_id: String,
    pub(crate) entry_id: String,
    pub(crate) nonce: String,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelDispatchInput {
    pub round: u32,
    pub projected_request_sha256: String,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompletePublicTurn {
    pub text: String,
    pub calls: Vec<ProposedTool>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedTool {
    pub call_id: String,
    pub lease: DispatchLease,
}
// Tools retain model array order. Repeated call IDs can be recorded but must be
// marked NotSent(DuplicateCallId); native must never dispatch them.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnCommit {
    pub journal_revision: u64,
    pub checkpoint_seq: u64,
    pub tools: Vec<PreparedTool>,
}

// Created by native normalisation before result is handed to the model.
// Digest is over the stated deterministic JSON encoding, NOT claimed wire bytes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedToolResult {
    pub response_kind: ToolResponseKind,
    pub returned_error: Option<bool>,
    pub observed_encoding: String,
    pub observed_bytes: u64,
    pub observed_sha256: String,
    pub retained_content: JournalContent,
    pub references: Vec<EvidenceReference>,
    // Transient; NEVER included when serializing a journal receipt. In-memory
    // memory/history projections may be present while retained_content=Omitted.
    #[serde(skip)]
    pub model_visible_json: Option<String>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, try_from = "String")]
pub enum ToolResponseKind {
    Complete,
    RpcError,
    InputRequired,
    Task,
    LocalCommit,
    LocalRead,
}
// MCP Complete can set returned_error; LocalRead may also report its safe local
// error result. InputRequired/Task are received
// intermediate replies, not a confirmed final tool outcome. A missing model
// projection halts the loop AFTER committing the observation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, try_from = "String")]
pub enum NoResponseReason {
    IncompleteResponse,
    Canceled,
    TransportLost,
    TimedOut,
    ProtocolInvalid,
    ResultNotPersisted,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, try_from = "String")]
pub enum NotSentReason {
    PreparationFailed,
    InvalidArguments,
    NotAdvertised,
    AuthorityChanged,
    CatalogChanged,
    RunEnded,
    BudgetExhausted,
    DuplicateCallId,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiptCommit {
    pub journal_revision: u64,
    pub checkpoint_seq: u64,
}

// Typed local-effect entry points are internal Rust APIs, never renderer IPC.
// Store validates quote against a real user message before mutation. Earlier
// messages are eligible only within this run's current conversation and bound.
pub const LOCAL_MEMORY_SOURCE_MESSAGE_LIMIT: usize = 8;

pub fn local_memory_sources(messages: &[crate::Message]) -> impl Iterator<Item = &crate::Message> {
    messages.iter().rev()
        .filter(|message| message.role == crate::MessageRole::User)
        .take(LOCAL_MEMORY_SOURCE_MESSAGE_LIMIT)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "type",
    rename_all_fields = "camelCase",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum LocalMemoryEffect {
    Save {
        id: Option<String>,
        text: String,
        expected_revision: Option<u64>,
        source_quote: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_message_id: Option<String>,
    },
    Forget {
        id: String,
        expected_revision: u64,
    },
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalToolCommit {
    pub snapshot: Snapshot,
    pub result: ObservedToolResult,
    pub receipt: ReceiptCommit,
}

// Sole renderer mutation. Evidence text/tool params are deliberately absent.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContinueFromRecord {
    pub scene_id: String,
    pub source_run_id: String,
    pub expected_journal_revision: u64,
    pub expected_checkpoint_seq: u64,
    pub expected_connection_id: String,
    pub expected_connection_revision: u64,
    // None means all eligible evidence. It succeeds in one action if it fits.
    // Some([]) is distinct and still must leave at least one confirmed body.
    pub selected_event_ids: Option<Vec<String>>,
}
// Read-only, no credentials/model call/draft consumption. The same selection
// validation and serialization are reused by begin_recorded_continuation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContinuationSelection {
    pub scene_id: String,
    pub source_run_id: String,
    pub expected_journal_revision: u64,
    pub expected_checkpoint_seq: u64,
    pub selected_event_ids: Option<Vec<String>>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContinuationDecision {
    // Canonical source selected by core. For a retry of an interrupted
    // continuation this resolves to its original source (no UI recursion).
    pub source_run_id: String,
    pub journal_revision: u64,
    pub checkpoint_seq: u64,
    pub budget_bytes: u64,
    pub projection_bytes: u64,
    pub default_projection_bytes: u64,
    pub default_event_ids: Vec<String>,
    pub selected_event_ids: Vec<String>,
    // Always describes the DEFAULT set. Remains true for a valid smaller set.
    pub selection_required: bool,
    pub ready: bool,
    pub blocked_reason: Option<ContinueBlockedReason>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContinuationProvenance {
    pub source_run_id: String,
    pub source_revision: u64,
    pub checkpoint_seq: u64,
    pub selected_event_ids: Vec<String>,
    pub projection_sha256: String,
    pub projection_version: u32,
    // References only. Bodies/assets are resolved from immutable messages on
    // reads; do not duplicate the complete conversation for each continuation.
    pub original_message_ids: Vec<String>,
}
// RunContext adds a host-only, serde(skip) typed projection. Persistent Run adds
// kind(default Chat) only. Detailed provenance is in the journal run record.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "type",
    rename_all_fields = "camelCase",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RunProjection {
    Conversation,
    RecordedContinuation(ContinuationProjection),
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordedMessage {
    pub id: String,
    pub role: MessageRole,
    pub text: String,
    pub attachments: Vec<Asset>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContinuationProjection {
    pub provenance: ContinuationProvenance,
    // Immutable original conversation prefix THROUGH source user_message_id.
    // Excludes later messages, current refs/draft, ToolSteps and live pixels.
    pub original_messages: Vec<RecordedMessage>,
    pub quoted_evidence_json: String,
}

impl Default for RunProjection {
    fn default() -> Self {
        Self::Conversation
    }
}
impl Default for RecordedRunKind {
    fn default() -> Self {
        Self::Chat
    }
}

impl TryFrom<String> for RecordedRunKind {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        match value.as_str() {
            "chat" => Ok(Self::Chat),
            "workflow" => Ok(Self::Workflow),
            "continuation" => Ok(Self::Continuation),
            _ => Err("invalid journal enum".into()),
        }
    }
}

impl TryFrom<String> for RecordedRunStatus {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        match value.as_str() {
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "canceled" => Ok(Self::Canceled),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err("invalid journal enum".into()),
        }
    }
}

impl TryFrom<String> for ReceiptPhase {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        match value.as_str() {
            "prepared" => Ok(Self::Prepared),
            "dispatched" => Ok(Self::Dispatched),
            "responded" => Ok(Self::Responded),
            "not_sent" => Ok(Self::NotSent),
            "unknown" => Ok(Self::Unknown),
            _ => Err("invalid journal enum".into()),
        }
    }
}

impl TryFrom<String> for ReceiptKind {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        match value.as_str() {
            "model" => Ok(Self::Model),
            "tool" => Ok(Self::Tool),
            _ => Err("invalid journal enum".into()),
        }
    }
}

impl TryFrom<String> for ContentUnavailable {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        match value.as_str() {
            "oversized" => Ok(Self::Oversized),
            "unsupported_content" => Ok(Self::UnsupportedContent),
            "memory_policy" => Ok(Self::MemoryPolicy),
            "history_policy" => Ok(Self::HistoryPolicy),
            "not_received" => Ok(Self::NotReceived),
            "invalid_response" => Ok(Self::InvalidResponse),
            "legacy_not_recorded" => Ok(Self::LegacyNotRecorded),
            _ => Err("invalid journal enum".into()),
        }
    }
}

impl TryFrom<String> for ContinueBlockedReason {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        match value.as_str() {
            "running" => Ok(Self::Running),
            "scene_closed" => Ok(Self::SceneClosed),
            "scene_frozen" => Ok(Self::SceneFrozen),
            "scene_inactive" => Ok(Self::SceneInactive),
            "no_confirmed_content" => Ok(Self::NoConfirmedContent),
            "legacy_not_recorded" => Ok(Self::LegacyNotRecorded),
            "connection_unselected" => Ok(Self::ConnectionUnselected),
            "mandatory_context_too_large" => Ok(Self::MandatoryContextTooLarge),
            "selection_too_large" => Ok(Self::SelectionTooLarge),
            "context_unavailable" => Ok(Self::ContextUnavailable),
            "conversation_changed" => Ok(Self::ConversationChanged),
            _ => Err("invalid journal enum".into()),
        }
    }
}

impl TryFrom<String> for ToolResponseKind {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        match value.as_str() {
            "complete" => Ok(Self::Complete),
            "rpc_error" => Ok(Self::RpcError),
            "input_required" => Ok(Self::InputRequired),
            "task" => Ok(Self::Task),
            "local_commit" => Ok(Self::LocalCommit),
            "local_read" => Ok(Self::LocalRead),
            _ => Err("invalid journal enum".into()),
        }
    }
}

impl TryFrom<String> for NoResponseReason {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        match value.as_str() {
            "incomplete_response" => Ok(Self::IncompleteResponse),
            "canceled" => Ok(Self::Canceled),
            "transport_lost" => Ok(Self::TransportLost),
            "timed_out" => Ok(Self::TimedOut),
            "protocol_invalid" => Ok(Self::ProtocolInvalid),
            "result_not_persisted" => Ok(Self::ResultNotPersisted),
            _ => Err("invalid journal enum".into()),
        }
    }
}

impl TryFrom<String> for NotSentReason {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        match value.as_str() {
            "preparation_failed" => Ok(Self::PreparationFailed),
            "invalid_arguments" => Ok(Self::InvalidArguments),
            "not_advertised" => Ok(Self::NotAdvertised),
            "authority_changed" => Ok(Self::AuthorityChanged),
            "catalog_changed" => Ok(Self::CatalogChanged),
            "run_ended" => Ok(Self::RunEnded),
            "budget_exhausted" => Ok(Self::BudgetExhausted),
            "duplicate_call_id" => Ok(Self::DuplicateCallId),
            _ => Err("invalid journal enum".into()),
        }
    }
}
