// SPDX-License-Identifier: MPL-2.0
use crate::{MemoryProviderSelection, RunMemoryAuthority};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Image,
    Text,
    Video,
    Html,
    Svg,
    File,
}

/// Host-attested desktop geometry for an entire captured image. Imported,
/// derived and legacy images carry no coordinate claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureCoordinateSpace {
    QuartzPoints,
    PhysicalPixels,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureCoordinates {
    pub space: CaptureCoordinateSpace,
    pub origin_x: f64,
    pub origin_y: f64,
    pub width: f64,
    pub height: f64,
}

impl CaptureCoordinates {
    pub fn valid_for(&self, width: u32, height: u32) -> bool {
        let minimum = f64::from(i32::MIN);
        let maximum = f64::from(i32::MAX);
        [self.origin_x, self.origin_y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && width > 0
            && height > 0
            && width <= 16384
            && height <= 16384
            && u64::from(width) * u64::from(height) <= 33_554_432
            && self.width >= 1.
            && self.height >= 1.
            && self.width <= 65536.
            && self.height <= 65536.
            && (minimum..=maximum).contains(&self.origin_x)
            && (minimum..=maximum).contains(&self.origin_y)
            && (minimum..=maximum + 65536.).contains(&(self.origin_x + self.width))
            && (minimum..=maximum + 65536.).contains(&(self.origin_y + self.height))
            && (self.space != CaptureCoordinateSpace::PhysicalPixels
                || (self.width == f64::from(width)
                    && self.height == f64::from(height)
                    && self.origin_x.fract() == 0.
                    && self.origin_y.fract() == 0.))
    }

    pub fn point(&self, width: u32, height: u32, x: u32, y: u32) -> Option<(f64, f64)> {
        if !self.valid_for(width, height) || x >= width || y >= height {
            return None;
        }
        Some((
            self.origin_x + f64::from(x) * self.width / f64::from(width),
            self.origin_y + f64::from(y) * self.height / f64::from(height),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub kind: AssetKind,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_y: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_factor: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_coordinates: Option<CaptureCoordinates>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Region {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Host-registered replacement pixels. Geometry above remains the display
    /// rectangle in background coordinates; drawings now use this image's
    /// pixel coordinates with origin (0,0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_override: Option<Asset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drawings: Vec<Drawing>,
    #[serde(default)]
    pub drawing_revision: u64,
    #[serde(default, skip_serializing_if = "DrawingHistory::is_empty")]
    pub drawing_history: DrawingHistory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr: Option<RegionOcr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<SavedTranslation>,
}

/// Display bounds in background pixels, without host-owned region contents.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegionGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl From<&Region> for RegionGeometry {
    fn from(region: &Region) -> Self {
        Self {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        }
    }
}

/// Geometry facts only; no old image, derived layer, drawing or conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegionGeometryEdit {
    pub id: String,
    pub region_id: String,
    pub background_id: String,
    pub source_id: String,
    pub from: RegionGeometry,
    pub to: RegionGeometry,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegionGeometryHistory {
    pub revision: u64,
    pub undo: Vec<RegionGeometryEdit>,
    pub redo: Vec<RegionGeometryEdit>,
}

impl RegionGeometryHistory {
    pub fn is_default(&self) -> bool {
        self.revision == 0 && self.undo.is_empty() && self.redo.is_empty()
    }
}

/// Captured before recognition starts; a result must match the complete target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrTarget {
    pub scene_id: String,
    pub region_id: String,
    pub background_id: String,
    pub drawing_revision: u64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Translation uses the same complete source/geometry compare-and-swap fence.
pub type TranslationTarget = OcrTarget;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranslationBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranslationLine {
    pub id: String,
    pub source: String,
    pub text: String,
    pub r#box: TranslationBox,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranslationDocument {
    pub version: u32,
    pub target_language: String,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_angle: Option<f64>,
    pub lines: Vec<TranslationLine>,
}

/// Immutable host-rendered pixels and their matching native text selection.
/// source_id is derived by Store, never supplied by the model or renderer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedTranslation {
    pub background_id: String,
    pub drawing_revision: u64,
    pub source_id: String,
    pub document: TranslationDocument,
    pub overlay: Asset,
    pub selection: OcrDocument,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegionOcr {
    pub background_id: String,
    pub drawing_revision: u64,
    pub document: OcrDocument,
}

/// Dimensions are the original composed crop, before engine-specific resizing.
/// Word coordinates remain in the engine's text-angle rotation space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrDocument {
    pub engine: String,
    pub language: String,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_angle: Option<f64>,
    pub lines: Vec<OcrLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrLine {
    pub text: String,
    pub words: Vec<OcrWord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrWord {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DrawingKind {
    Pen,
    Line,
    Highlighter,
    Mark,
    Arrow,
    Rect,
    Ellipse,
    Text,
    Number,
    Mosaic,
    Rich,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DrawingPoint {
    pub x: f64,
    pub y: f64,
}

/// Editable manual markup in original background pixels, never executable SVG.
/// Two shape points are endpoints/opposite corners; text/number use a top-left
/// anchor. Number font_size is its circle diameter; mosaic stroke_width is its
/// integer background-aligned pixel block size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Drawing {
    pub id: String,
    pub kind: DrawingKind,
    pub color: String,
    pub stroke_width: f64,
    pub points: Vec<DrawingPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<crate::VisualDrawingOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich: Option<crate::RichDrawingRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DrawingEdit {
    pub index: usize,
    pub before: Option<Drawing>,
    pub after: Option<Drawing>,
}

/// Old single edits retain their exact wire representation. A batch is one
/// history action, replayed forward in order and backward in reverse order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DrawingStep {
    Single(DrawingEdit),
    Batch(DrawingBatchEdit),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DrawingBatchEdit {
    pub batch: Vec<DrawingEdit>,
}
impl From<DrawingEdit> for DrawingStep {
    fn from(value: DrawingEdit) -> Self {
        Self::Single(value)
    }
}
impl DrawingStep {
    pub fn edits(&self) -> &[DrawingEdit] {
        match self {
            Self::Single(edit) => std::slice::from_ref(edit),
            Self::Batch(batch) => &batch.batch,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DrawingHistory {
    #[serde(default)]
    pub undo: Vec<DrawingStep>,
    #[serde(default)]
    pub redo: Vec<DrawingStep>,
}
impl DrawingHistory {
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty() && self.redo.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpaceItem {
    pub id: String,
    pub asset: Asset,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_edit: Option<VideoEdit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_annotations: Option<crate::VideoAnnotationDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReferenceKind {
    Region,
    Item,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reference {
    pub kind: ReferenceKind,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Message {
    pub id: String,
    pub role: MessageRole,
    pub text: String,
    /// Only public provider reasoning text, never signatures or encrypted state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Present only on the first user message of a non-initial conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_start: Option<usize>,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refs: Option<Vec<Reference>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Asset>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_steps: Vec<ToolStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Run {
    pub id: String,
    #[serde(default)]
    pub kind: crate::RecordedRunKind,
    pub status: RunStatus,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub connection_revision: Option<u64>,
    /// Executable provider identity; external runs have no invented model connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_runtime: Option<ExternalRunOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_authority: Option<RunMemoryAuthority>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub steps: Vec<ToolStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalRunOrigin {
    pub plugin_id: String,
    pub service_id: String,
    pub revision: u64,
    pub sha256: String,
}

impl ExternalRunOrigin {
    pub(crate) fn validate(&self) -> bool {
        let valid_id = |id: &str| !id.is_empty() && id.len() <= 128
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
        valid_id(&self.plugin_id) && valid_id(&self.service_id) && self.revision > 0
            && self.sha256.len() == 64 && self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolStepStatus {
    Running,
    Completed,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolStep {
    pub id: String,
    pub name: String,
    pub label: String,
    pub status: ToolStepStatus,
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlackboardLink {
    pub parent_scene_id: String,
    pub item_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_title_attempted: bool,
    #[serde(default, skip_serializing_if = "title_revision_is_zero")]
    pub title_revision: u64,
    pub agent_id: String,
    #[serde(default)]
    pub connection_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub frozen: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub minimized: bool,
    #[serde(default)]
    pub closed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Asset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blackboard_link: Option<BlackboardLink>,
    pub regions: Vec<Region>,
    #[serde(default, skip_serializing_if = "RegionGeometryHistory::is_default")]
    pub geometry_history: RegionGeometryHistory,
    pub items: Vec<SpaceItem>,
    pub refs: Vec<Reference>,
    pub draft: String,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_start: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<Run>,
}

fn title_revision_is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub instructions: String,
    pub memory: String,
    #[serde(default = "default_memory_enabled")]
    pub memory_enabled: bool,
    #[serde(default, skip_serializing_if = "MemoryProviderSelection::is_default")]
    pub memory_provider: MemoryProviderSelection,
    #[serde(default)]
    pub default_connection_id: Option<String>,
}

fn default_memory_enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemorySource {
    pub scene_id: String,
    pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryEntry {
    pub id: String,
    pub agent_id: String,
    pub text: String,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MemorySource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<MemoryOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOrigin {
    LegacyMemory,
    LegacyReference,
    Manual,
    Conversation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryStats {
    pub agent_id: String,
    pub count: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryPage {
    pub entries: Vec<MemoryEntry>,
    pub total: u64,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Agent and provenance are intentionally absent: the running scene supplies both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MemoryMutation {
    Save {
        id: Option<String>,
        text: String,
        expected_revision: Option<u64>,
    },
    Delete {
        id: String,
        expected_revision: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMatch {
    pub scene_id: String,
    pub message_id: String,
    pub role: MessageRole,
    pub text: String,
    pub created_at: u64,
}

/// Public connection metadata only. Credentials belong to the host's secret store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Connection {
    pub base_url: String,
    pub model: String,
    pub has_key: bool,
}

pub const LEGACY_CONNECTION_ID: &str = "00000000-0000-4000-8000-000000000001";
pub const LEGACY_CREDENTIAL_ID: &str = "00000000-0000-4000-8000-000000000002";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionProtocol {
    #[default]
    ChatCompletions,
    AnthropicMessages,
    #[serde(rename = "openai_responses")]
    OpenAiResponses,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionAuthMode {
    #[default]
    Bearer,
    ApiKey,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionServiceTier {
    Standard,
    Priority,
}

/// Only supported generation options; this cannot replace messages, tools or auth.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionParameters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<ConnectionServiceTier>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionAdvanced {
    #[serde(default)]
    pub protocol: ConnectionProtocol,
    #[serde(default)]
    pub auth_mode: ConnectionAuthMode,
    #[serde(default)]
    pub request_path: Option<String>,
    #[serde(default)]
    pub request_parameters: ConnectionParameters,
}

/// Secret references are opaque host-owned UUIDs, never credential values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    pub provider_id: String,
    pub base_url: String,
    pub model: String,
    pub has_key: bool,
    pub credential_id: Option<String>,
    pub revision: u64,
    #[serde(default)]
    pub advanced: ConnectionAdvanced,
}

impl ConnectionProfile {
    pub fn connection(&self) -> Connection {
        Connection {
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            has_key: self.has_key,
        }
    }
}

/// Public launch configuration. Credentials must never be stored in these fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpCommand {
    pub executable: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpGrant {
    pub agent_id: String,
    pub tool_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServer {
    pub id: String,
    pub label: String,
    pub revision: u64,
    pub command: McpCommand,
    pub tools: Vec<McpTool>,
    pub grants: Vec<McpGrant>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u32,
    pub active_scene_id: String,
    pub scenes: Vec<Scene>,
    pub agents: Vec<AgentProfile>,
    #[serde(default)]
    pub memories: Vec<MemoryEntry>,
    #[serde(default)]
    pub memory_stats: Vec<MemoryStats>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    pub connection: Connection,
    #[serde(default)]
    pub connections: Vec<ConnectionProfile>,
    #[serde(default)]
    pub default_connection_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SceneCommand {
    NewScene,
    NewConversation {
        scene_id: String,
    },
    SetSceneConnection {
        scene_id: String,
        connection_id: Option<String>,
    },
    SetDefaultConnection {
        connection_id: Option<String>,
    },
    FreezeScene {
        scene_id: String,
    },
    ActivateScene {
        scene_id: String,
    },
    CloseScene {
        scene_id: String,
    },
    SetDraft {
        scene_id: String,
        draft: String,
    },
    RenameScene {
        scene_id: String,
        title: String,
    },
    SetRefs {
        scene_id: String,
        refs: Vec<Reference>,
    },
    AddRegion {
        scene_id: String,
        region: Region,
    },
    UpdateRegion {
        scene_id: String,
        region: Region,
    },
    SetRegionGeometry {
        scene_id: String,
        region_id: String,
        background_id: String,
        source_id: String,
        expected_revision: u64,
        from: RegionGeometry,
        to: RegionGeometry,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        edit_id: Option<String>,
    },
    FinishRegionGeometryEdit {
        scene_id: String,
        edit_id: String,
    },
    UndoRegionGeometry {
        scene_id: String,
        expected_history_revision: u64,
        expected_operation_id: String,
        region_id: String,
        background_id: String,
        source_id: String,
        expected_revision: u64,
        from: RegionGeometry,
    },
    RedoRegionGeometry {
        scene_id: String,
        expected_history_revision: u64,
        expected_operation_id: String,
        region_id: String,
        background_id: String,
        source_id: String,
        expected_revision: u64,
        from: RegionGeometry,
    },
    RemoveRegion {
        scene_id: String,
        region_id: String,
    },
    AddDrawing {
        scene_id: String,
        region_id: String,
        background_id: String,
        expected_revision: u64,
        drawing: Drawing,
    },
    UpdateDrawing {
        scene_id: String,
        region_id: String,
        background_id: String,
        expected_revision: u64,
        drawing: Drawing,
    },
    UpdateDrawings {
        scene_id: String,
        region_id: String,
        background_id: String,
        expected_revision: u64,
        drawings: Vec<Drawing>,
    },
    RemoveDrawing {
        scene_id: String,
        region_id: String,
        background_id: String,
        expected_revision: u64,
        drawing_id: String,
    },
    UndoDrawing {
        scene_id: String,
        region_id: String,
        background_id: String,
        expected_revision: u64,
    },
    RedoDrawing {
        scene_id: String,
        region_id: String,
        background_id: String,
        expected_revision: u64,
    },
    UpdateItem {
        scene_id: String,
        item: SpaceItem,
    },
    RemoveItem {
        scene_id: String,
        item_id: String,
    },
    SaveAgent {
        agent: AgentProfile,
    },
    SetAgent {
        scene_id: String,
        agent_id: String,
    },
    SaveMemory {
        agent_id: String,
        id: Option<String>,
        text: String,
        expected_revision: Option<u64>,
    },
    DeleteMemory {
        agent_id: String,
        id: String,
        expected_revision: u64,
    },
    SetMcpGrants {
        server_id: String,
        expected_revision: u64,
        agent_id: String,
        tool_names: Vec<String>,
    },
    SetMcpEnabled {
        server_id: String,
        expected_revision: u64,
        enabled: bool,
    },
    RemoveMcpServer {
        server_id: String,
        expected_revision: u64,
    },
}

/// Immutable input captured when a run starts; later scene edits cannot rewrite it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunContext {
    pub scene_id: String,
    pub run_id: String,
    pub memory_authority: Option<RunMemoryAuthority>,
    pub agent: AgentProfile,
    pub memories: Vec<MemoryEntry>,
    pub mcp_servers: Vec<McpServer>,
    pub connection: Connection,
    pub connection_profile: ConnectionProfile,
    pub messages: Vec<Message>,
    pub background: Option<Asset>,
    pub regions: Vec<Region>,
    pub items: Vec<SpaceItem>,
    pub refs: Vec<Reference>,
    #[serde(skip)]
    pub projection: crate::RunProjection,
}

/// A plugin-owned Agent receives conversation input without host credentials or
/// a fabricated connection. Its own runtime decides which services to invoke.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalRunContext {
    pub scene_id: String,
    pub run_id: String,
    pub agent: AgentProfile,
    pub messages: Vec<Message>,
    pub background: Option<Asset>,
    pub regions: Vec<Region>,
    pub items: Vec<SpaceItem>,
    pub refs: Vec<Reference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoRange {
    pub start_ticks: u64,
    pub end_ticks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoRangeOperation {
    pub id: String,
    #[serde(deserialize_with = "required_video_range")]
    pub from: Option<VideoRange>,
    #[serde(deserialize_with = "required_video_range")]
    pub to: Option<VideoRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoEdit {
    pub source_id: String,
    pub source_duration_ticks: u64,
    pub revision: u64,
    #[serde(deserialize_with = "required_video_range")]
    pub range: Option<VideoRange>,
    pub undo: Vec<VideoRangeOperation>,
    pub redo: Vec<VideoRangeOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoTarget {
    pub scene_id: String,
    pub item_id: String,
    pub source_id: String,
    pub expected_revision: u64,
}

// Host-only evidence, intentionally not a renderer-deserializable DTO.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedVideoSource {
    pub asset: Asset,
    pub duration_ticks: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoSourceView {
    pub asset: Asset,
    pub edit: Option<VideoEdit>,
    pub annotations: Option<crate::VideoAnnotationDocument>,
}

// Native keeps this entire value private; frontend sends only VideoTarget.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoExportOrigin {
    pub scene_id: String,
    pub item_id: String,
    pub source: VideoSourceView,
}

// A saved nullable range must be present (explicit null means full original).
// Deserialize-with without default avoids silently treating a missing field as
// full source. The optional SpaceItem.videoEdit itself still defaults to None.
fn required_video_range<'de, D>(deserializer: D) -> Result<Option<VideoRange>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<VideoRange>::deserialize(deserializer)
}
