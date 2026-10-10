// SPDX-License-Identifier: MPL-2.0
//! Declarative host capabilities. No executable plugin code is loaded.
//! The application must own its single-instance lease before opening this store.
//! SQLite is authoritative; a durable intent record recovers file changes that
//! were interrupted before the registry transaction committed.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const OFFICIAL_TABLE: &[u8] = include_bytes!("../../plugins/official-table.json");
const OFFICIAL_DRAWING: &[u8] =
    include_bytes!("../../plugins/migrations/official-drawing-1.2.0.json");
// Immutable migration evidence, not a second current catalog entry.
const OFFICIAL_DRAWING_V1_0_0: &[u8] =
    include_bytes!("../../plugins/migrations/official-drawing-1.0.0.json");
const OFFICIAL_DRAWING_V1_1_0: &[u8] =
    include_bytes!("../../plugins/migrations/official-drawing-1.1.0.json");
const OFFICIAL_OCR: &[u8] = include_bytes!("../../plugins/official-ocr.json");
const OFFICIAL_SCROLL: &[u8] = include_bytes!("../../plugins/official-scroll.json");
const OFFICIAL_TRANSLATION: &[u8] = include_bytes!("../../plugins/official-translation.json");
const OFFICIAL_PIN: &[u8] = include_bytes!("../../plugins/official-pin.json");
const OFFICIAL_MEMORY: &[u8] = include_bytes!("../../plugins/official-memory-hindsight.json");
const OFFICIAL_VOICE: &[u8] = include_bytes!("../../plugins/official-voice.json");
const OFFICIAL_CODES: &[u8] = include_bytes!("../../plugins/official-codes.json");
const OFFICIAL_VIDEO_TRIM: &[u8] =
    include_bytes!("../../plugins/migrations/official-video-trim-1.0.0.json");
const OFFICIAL_RECORDING_AUDIO: &[u8] =
    include_bytes!("../../plugins/migrations/official-recording-audio-1.0.0.json");
const OFFICIAL_GIF: &[u8] = include_bytes!("../../plugins/migrations/official-gif-1.0.0.json");
const OFFICIAL_RECORDING: &[u8] =
    include_bytes!("../../plugins/migrations/official-recording-1.0.0.json");
#[cfg(test)]
const OFFICIAL_ANNOTATIONS: &[u8] =
    include_bytes!("../../plugins/migrations/official-annotations-1.0.0.json");
const RECORDING_MIGRATION_KEY: &str = "migration.recording-bundle.v1";
const LEGACY_RECORDING_IDS: [&str; 3] = ["mewu.video-trim", "mewu.recording-audio", "mewu.gif"];
const BUNDLED: [&[u8]; 26] = [
    OFFICIAL_TABLE,
    OFFICIAL_OCR,
    OFFICIAL_SCROLL,
    OFFICIAL_TRANSLATION,
    OFFICIAL_PIN,
    OFFICIAL_MEMORY,
    OFFICIAL_VOICE,
    OFFICIAL_CODES,
    include_bytes!("../../plugins/official-provider-minimax.json"),
    include_bytes!("../../plugins/official-provider-volcengine.json"),
    include_bytes!("../../plugins/official-provider-dashscope.json"),
    include_bytes!("../../plugins/official-provider-deepseek.json"),
    include_bytes!("../../plugins/official-provider-moonshot.json"),
    include_bytes!("../../plugins/official-provider-zhipu.json"),
    include_bytes!("../../plugins/official-provider-tencent.json"),
    include_bytes!("../../plugins/official-provider-baidu.json"),
    include_bytes!("../../plugins/official-provider-siliconflow.json"),
    include_bytes!("../../plugins/official-provider-openai.json"),
    include_bytes!("../../plugins/official-provider-anthropic.json"),
    include_bytes!("../../plugins/official-provider-google.json"),
    include_bytes!("../../plugins/official-provider-xai.json"),
    include_bytes!("../../plugins/official-provider-openrouter.json"),
    include_bytes!("../../plugins/official-provider-groq.json"),
    include_bytes!("../../plugins/official-provider-mistral.json"),
    include_bytes!("../../plugins/official-provider-together.json"),
    include_bytes!("../../plugins/official-provider-custom.json"),
];

pub(crate) const CORE_DRAWING: &str = "mewu.core.drawing";
pub(crate) const CORE_RECORDING: &str = "mewu.core.recording";

/// Exact built-in host authority; a package cannot change this revision or kind.
pub(crate) fn core_recording_grant(id: &str, revision: u64, contribution: &str) -> bool {
    id == CORE_RECORDING
        && revision == 1
        && matches!(contribution, "record" | "audio" | "trim" | "gif")
}
fn core_drawing_grant(id: &str, revision: u64, contribution: &str) -> bool {
    id == CORE_DRAWING
        && revision == 1
        && matches!(contribution, "drawing-tools" | "video-drawing-tools")
}

/// Only these exact historical official manifests can preserve a saved choice.
/// This reads approved metadata, never edits or resurrects a legacy installation.
pub(crate) fn legacy_official_audio_record(record: &PluginRecord) -> bool {
    let canonical = match record.manifest.id.as_str() {
        "mewu.recording" => OFFICIAL_RECORDING,
        "mewu.recording-audio" => OFFICIAL_RECORDING_AUDIO,
        _ => return false,
    };
    record.source == PluginSource::Official
        && record.error.is_none()
        && parse_manifest(canonical).is_ok_and(|manifest| manifest == record.manifest)
}
const DB_VERSION: i64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginManifest {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    pub license: String,
    pub host_api: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<PluginTag>,
    pub contributions: Vec<PluginContribution>,
}

/// Discovery metadata never grants a host capability or executable permission.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum PluginTag {
    Agi,
    UiEnhancement,
    Billing,
    Themes,
    ModelProviders,
    Messaging,
    Memory,
    Tools,
    System,
    Vision,
    Audio,
    Documents,
    Skills,
    Automation,
    Privacy,
    Entertainment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PluginContribution {
    #[serde(rename = "model.connection")]
    ModelConnection {
        id: String,
        title: String,
        template: ModelConnectionTemplate,
    },
    #[serde(rename = "agent.visual-annotations")]
    AgentVisualAnnotations {
        id: String,
        title: String,
        engine: VisualAnnotationEngine,
    },
    #[serde(rename = "selection.recording")]
    SelectionRecording {
        id: String,
        title: String,
        engine: RecordingEngine,
    },
    #[serde(rename = "artifact.video-gif")]
    ArtifactVideoGif {
        id: String,
        title: String,
        engine: VideoGifEngine,
    },
    #[serde(rename = "recording.audio")]
    RecordingAudio {
        id: String,
        title: String,
        engine: RecordingAudioEngine,
    },
    #[serde(rename = "artifact.video-trim")]
    ArtifactVideoTrim {
        id: String,
        title: String,
        engine: VideoTrimEngine,
    },
    #[serde(rename = "selection.codes")]
    Codes {
        id: String,
        title: String,
        engine: CodeEngine,
    },
    #[serde(rename = "input.speech-to-text")]
    InputSpeechToText {
        id: String,
        title: String,
        engine: SpeechEngine,
    },
    #[serde(rename = "memory.provider")]
    MemoryProvider {
        id: String,
        title: String,
        adapter: MemoryAdapter,
    },
    #[serde(rename = "selection.pin")]
    Pin { id: String, title: String },
    #[serde(rename = "selection.translation")]
    Translation { id: String, title: String },
    #[serde(rename = "selection.scroll")]
    Scroll { id: String, title: String },
    #[serde(rename = "selection.workflow")]
    Workflow {
        id: String,
        title: String,
        prompt: String,
        accepts: Vec<WorkflowInput>,
    },
    #[serde(rename = "selection.drawing-tools")]
    DrawingTools {
        id: String,
        title: String,
        tools: Vec<DrawingTool>,
    },
    #[serde(rename = "artifact.video-drawing-tools")]
    VideoDrawingTools {
        id: String,
        title: String,
        tools: Vec<VideoDrawingTool>,
    },
    #[serde(rename = "selection.ocr")]
    Ocr {
        id: String,
        title: String,
        engine: OcrEngine,
    },
}

/// Declarative connection defaults, not executable adapters. Only the host's
/// supported protocols/authentication/options can be selected by a package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionTemplate {
    pub provider_id: String,
    pub group: ModelConnectionGroup,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub search_terms: String,
    pub advanced: ModelConnectionAdvanced,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelConnectionGroup { China, Global, Custom }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionAdvanced {
    pub protocol: mewu_core::ConnectionProtocol,
    pub auth_mode: mewu_core::ConnectionAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_path: Option<String>,
    pub request_parameters: ModelConnectionParameters,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelConnectionParameters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<mewu_core::ConnectionServiceTier>,
}

impl ModelConnectionTemplate {
    pub(crate) fn profile(&self, name: &str) -> Result<mewu_core::ConnectionProfile, String> {
        let advanced = serde_json::from_value(serde_json::to_value(&self.advanced).map_err(|e| e.to_string())?)
            .map_err(|_| "模型接入参数无效".to_string())?;
        Ok(mewu_core::ConnectionProfile {
            id: "00000000-0000-4000-8000-000000000001".into(), name: name.into(),
            provider_id: self.provider_id.clone(), base_url: self.base_url.clone(), model: self.model.clone(),
            has_key: false, credential_id: None, revision: 1, advanced,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionPreset {
    pub id: String,
    pub name: String,
    pub group: ModelConnectionGroup,
    pub base_url: String,
    pub model: String,
    pub advanced: ModelConnectionAdvanced,
    pub search_terms: String,
    pub plugin_id: String,
    pub plugin_revision: u64,
    pub contribution_id: String,
}

fn validate_model_template(template: &ModelConnectionTemplate, title: &str) -> Result<(), String> {
    let profile = template.profile(title)?;
    mewu_core::validate_connection_profile(&profile).map_err(|e| e.to_string())?;
    if template.search_terms.chars().count() > 500 || template.search_terms.chars().any(char::is_control) {
        return Err("模型接入搜索词过长或包含控制字符".into());
    }
    if template.base_url.is_empty() {
        if template.group != ModelConnectionGroup::Custom { return Err("只有自定义接入允许空地址".into()); }
    } else {
        let url = url::Url::parse(&template.base_url).map_err(|_| "模型接入地址无效")?;
        let local = match url.host() {
            Some(url::Host::Domain("localhost")) => true,
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        if url.host().is_none() || !(url.scheme() == "https" || (url.scheme() == "http" && local))
            || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
            return Err("模型接入需要无凭据的 HTTPS 地址，只有本机服务允许 HTTP".into());
        }
    }
    if profile.advanced.request_path.as_deref().is_some_and(|path| !path.starts_with('/')) {
        return Err("模型接入请求路径必须以 / 开头".into());
    }
    if profile.advanced.protocol == mewu_core::ConnectionProtocol::AnthropicMessages
        && (profile.advanced.request_parameters.temperature.is_some_and(|v| v > 1.0)
            || profile.advanced.request_parameters.service_tier == Some(mewu_core::ConnectionServiceTier::Priority)) {
        return Err("Anthropic 接入不支持此 temperature 或 service_tier".into());
    }
    Ok(())
}

/// A declaration chooses a host implementation. It carries no executable code,
/// endpoint, credentials or permission to read any Agent's history.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryAdapter {
    Hindsight,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OcrEngine {
    Windows,
}

/// A fixed host adapter, not permission to load arbitrary recognizers or code.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
pub enum SpeechEngine {
    #[serde(rename = "windows.sapi")]
    WindowsSapi,
}

/// A fixed local decoder, never a URL, command or automatic external action.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
pub enum CodeEngine {
    #[serde(rename = "rxing")]
    Rxing,
}

/// Selects the host's local media editor, not an executable or codec package.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
pub enum VideoTrimEngine {
    #[serde(rename = "windows.media-editing")]
    WindowsMediaEditing,
}

/// A recording session entry point. Audio, trim and GIF remain typed capabilities
/// in the same package; this grant is required even for a silent recording.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
pub enum RecordingEngine {
    #[serde(rename = "windows.wgc-mf")]
    WindowsWgcMf,
}
impl TryFrom<String> for RecordingEngine {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "windows.wgc-mf" => Ok(Self::WindowsWgcMf),
            _ => Err("不支持此屏幕录制引擎"),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingGrant {
    pub plugin_id: String,
    pub revision: u64,
    pub contribution_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
pub enum VisualAnnotationEngine {
    #[serde(rename = "host.vector-v1")]
    HostVectorV1,
}
impl TryFrom<String> for VisualAnnotationEngine {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "host.vector-v1" => Ok(Self::HostVectorV1),
            _ => Err("不支持此原位作答引擎"),
        }
    }
}

/// Fixed host export capability, not a codec, executable or automatic action.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
pub enum VideoGifEngine {
    #[serde(rename = "windows.media-editing-gif")]
    WindowsMediaEditingGif,
}

impl TryFrom<String> for VideoGifEngine {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "windows.media-editing-gif" => Ok(Self::WindowsMediaEditingGif),
            _ => Err("不支持此 GIF 导出引擎"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GifGrant {
    pub plugin_id: String,
    pub revision: u64,
    pub contribution_id: String,
}

/// Fixed native recording source support, not permission to start a microphone.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
pub enum RecordingAudioEngine {
    #[serde(rename = "windows.wasapi")]
    WindowsWasapi,
}
impl TryFrom<String> for RecordingAudioEngine {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "windows.wasapi" => Ok(Self::WindowsWasapi),
            _ => Err("不支持此录屏声音引擎"),
        }
    }
}

impl TryFrom<String> for VideoTrimEngine {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "windows.media-editing" => Ok(Self::WindowsMediaEditing),
            _ => Err("不支持此视频裁剪引擎"),
        }
    }
}

impl TryFrom<String> for CodeEngine {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "rxing" => Ok(Self::Rxing),
            _ => Err("不支持此二维码或条码识别引擎"),
        }
    }
}

impl TryFrom<String> for SpeechEngine {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "windows.sapi" => Ok(Self::WindowsSapi),
            _ => Err("不支持此语音识别引擎"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum DrawingTool {
    Pen,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Text,
    Highlighter,
    Number,
    Mosaic,
}

/// A distinct capability: screenshot tools never grant video authoring.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase", try_from = "String")]
pub enum VideoDrawingTool {
    Pen,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Text,
    Number,
}
impl TryFrom<String> for VideoDrawingTool {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "pen" => Ok(Self::Pen),
            "line" => Ok(Self::Line),
            "arrow" => Ok(Self::Arrow),
            "rect" => Ok(Self::Rect),
            "ellipse" => Ok(Self::Ellipse),
            "text" => Ok(Self::Text),
            "number" => Ok(Self::Number),
            _ => Err("不支持此视频绘制工具"),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoDrawingGrant {
    pub plugin_id: String,
    pub revision: u64,
    pub contribution_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectionWorkflow {
    pub id: String,
    pub title: String,
    pub kind: WorkflowKind,
    pub prompt: String,
    pub accepts: Vec<WorkflowInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkflowKind {
    #[serde(rename = "selection.workflow")]
    SelectionWorkflow,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowInput {
    Image,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginSource {
    Official,
    Local {
        path: String,
    },
    Github {
        repository: String,
        commit: String,
        path: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    Enabled,
    Disabled,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginRecord {
    pub manifest: PluginManifest,
    pub source: PluginSource,
    pub revision: u64,
    pub state: PluginState,
    pub has_rollback: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginSnapshot {
    pub revision: u64,
    pub plugins: Vec<PluginRecord>,
}

/// A revoke hook over an already integrity-checked snapshot. Admission and
/// publication must additionally call PluginStore::video_gif with the exact grant.
/// This function takes no host locks and performs no I/O.
pub fn gif_grant_is_current(grant: &GifGrant, snapshot: &PluginSnapshot) -> bool {
    if grant.plugin_id == CORE_RECORDING {
        return core_recording_grant(&grant.plugin_id, grant.revision, &grant.contribution_id)
            && grant.contribution_id == "gif";
    }
    grant.revision > 0
        && snapshot.plugins.iter().any(|record| {
            record.manifest.id == grant.plugin_id
                && record.revision == grant.revision
                && record.state == PluginState::Enabled
                && record.error.is_none()
                && record.manifest.contributions.iter().any(|item| {
                    matches!(item, PluginContribution::ArtifactVideoGif {
                        id,
                        engine: VideoGifEngine::WindowsMediaEditingGif,
                        ..
                    } if id == &grant.contribution_id)
                })
        })
}

/// Pure snapshot revocation check. Real admission/acceptance additionally use
/// PluginStore::video_drawing_tools, including complete package integrity.
pub fn video_drawing_grant_is_current(
    grant: &VideoDrawingGrant,
    tool: VideoDrawingTool,
    snapshot: &PluginSnapshot,
) -> bool {
    if grant.plugin_id == CORE_DRAWING {
        return core_drawing_grant(&grant.plugin_id, grant.revision, &grant.contribution_id)
            && grant.contribution_id == "video-drawing-tools";
    }
    grant.revision>0 && snapshot.plugins.iter().any(|record|record.manifest.id==grant.plugin_id
        && record.revision==grant.revision && record.state==PluginState::Enabled && record.error.is_none()
        && record.manifest.contributions.iter().any(|item|matches!(item,
            PluginContribution::VideoDrawingTools{id,tools,..} if id==&grant.contribution_id && tools.contains(&tool))))
}

pub fn recording_grant_is_current(grant: &RecordingGrant, snapshot: &PluginSnapshot) -> bool {
    if grant.plugin_id == CORE_RECORDING {
        return core_recording_grant(&grant.plugin_id, grant.revision, &grant.contribution_id)
            && grant.contribution_id == "record";
    }
    grant.revision > 0 && snapshot.plugins.iter().any(|record| {
        record.manifest.id == grant.plugin_id && record.revision == grant.revision
            && record.state == PluginState::Enabled && record.error.is_none()
            && record.manifest.contributions.iter().any(|item| matches!(item,
                PluginContribution::SelectionRecording { id, engine: RecordingEngine::WindowsWgcMf, .. }
                    if id == &grant.contribution_id))
    })
}

pub fn visual_annotation_grant_is_current(
    grant: &mewu_core::VisualAnnotationGrant,
    snapshot: &PluginSnapshot,
) -> bool {
    if grant.plugin_id == "mewu.core.annotations" {
        return core_annotation_grant_is_current(grant);
    }
    grant.plugin_revision > 0 && snapshot.plugins.iter().any(|record| {
        record.manifest.id == grant.plugin_id && record.revision == grant.plugin_revision
            && record.state == PluginState::Enabled && record.error.is_none()
            && record.manifest.contributions.iter().any(|item| matches!(item,
                PluginContribution::AgentVisualAnnotations { id, engine: VisualAnnotationEngine::HostVectorV1, .. }
                    if id == &grant.contribution_id))
    })
}

fn core_annotation_grant_is_current(grant: &mewu_core::VisualAnnotationGrant) -> bool {
    grant.plugin_id == "mewu.core.annotations"
        && grant.plugin_revision == 1
        && grant.contribution_id == "annotate"
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Package {
    manifest: PluginManifest,
    source: PluginSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredRecord {
    package: Package,
    revision: u64,
    state: PluginState,
    previous: Option<Package>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileIntent {
    before: Option<StoredRecord>,
    after: StoredRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recording_migration: Option<RecordingMigration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchivedRecordingRecord {
    id: String,
    // Preserve the database payload verbatim, including its original whitespace.
    payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecordingMigration {
    version: u32,
    records: Vec<ArchivedRecordingRecord>,
}

pub struct PluginStore {
    connection: Connection,
    root: PathBuf,
}

/// These manifests are compiled from first-party files, never remote input.
pub fn bundled_manifests() -> Vec<PluginManifest> {
    BUNDLED
        .into_iter()
        .map(|bytes| parse_manifest(bytes).expect("bundled plugin must pass manifest validation"))
        .collect()
}

pub fn read_validated_local(path: &Path) -> Result<Vec<u8>, String> {
    if !path.is_absolute() {
        return Err("请选择绝对路径的插件 JSON 文件".into());
    }
    let bytes = read_bounded(path)?;
    parse_manifest(&bytes)?;
    Ok(bytes)
}

/// Parsing is also available to the host's preview/download path. It performs
/// no I/O, registration, execution or network access.
pub fn parse_manifest(bytes: &[u8]) -> Result<PluginManifest, String> {
    if bytes.is_empty() || bytes.len() > MAX_MANIFEST_BYTES {
        return Err("插件文件为空或超过 64 KiB".into());
    }
    let manifest: PluginManifest =
        serde_json::from_slice(bytes).map_err(|error| format!("插件 JSON 格式不正确：{error}"))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn text(value: &str, max: usize, multiline: bool) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= max
        && !value
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
}

fn identifier(value: &str) -> bool {
    let reserved = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    !value.is_empty()
        && value.len() <= 80
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'-'))
        && value
            .split('.')
            .all(|segment| !segment.is_empty() && !segment.ends_with('-'))
        && !reserved.contains(&value.split('.').next().unwrap_or_default())
}

fn validate_manifest(manifest: &PluginManifest) -> Result<(), String> {
    if manifest.schema_version != 1 || manifest.host_api != 1 {
        return Err("此插件需要不受支持的格式或宿主 API".into());
    }
    if !identifier(&manifest.id)
        || manifest.version.len() > 64
        || semver::Version::parse(&manifest.version).is_err()
        || !manifest.version.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'+')
        })
        || !text(&manifest.name, 80, false)
        || !text(&manifest.description, 2000, true)
        || manifest
            .publisher
            .as_ref()
            .is_some_and(|p| !text(p, 120, false))
    {
        return Err("插件名称、标识、版本或说明不符合要求".into());
    }
    // This first declarative format deliberately accepts a small reviewed
    // license set; a free-text claim cannot silently authorize GPL/AGPL code.
    if ![
        "MIT",
        "Apache-2.0",
        "BSD-2-Clause",
        "BSD-3-Clause",
        "ISC",
        "CC0-1.0",
        "MPL-2.0",
    ]
    .contains(&manifest.license.as_str())
    {
        return Err("此插件声明的许可证暂不支持".into());
    }
    if manifest.tags.len() > 16 || manifest.tags.iter().collect::<BTreeSet<_>>().len() != manifest.tags.len() {
        return Err("插件模块标签不能重复，最多 16 个".into());
    }
    if manifest.contributions.is_empty() || manifest.contributions.len() > 16 {
        return Err("插件必须包含 1 至 16 项功能".into());
    }
    let mut ids = BTreeSet::new();
    for contribution in &manifest.contributions {
        let (id, title) = match contribution {
            PluginContribution::ModelConnection { id, title, template } => {
                validate_model_template(template, title)?;
                (id, title)
            }
            PluginContribution::Workflow {
                id,
                title,
                prompt,
                accepts,
            } => {
                if !text(prompt, 8000, true) || accepts != &[WorkflowInput::Image] {
                    return Err("选区工作流必须接受图片并包含不超过 8000 字的指令".into());
                }
                (id, title)
            }
            PluginContribution::DrawingTools { id, title, tools } => {
                if tools.is_empty()
                    || tools.len() > 9
                    || tools.iter().collect::<BTreeSet<_>>().len() != tools.len()
                {
                    return Err("绘制工具必须包含 1 至 9 个不重复的工具".into());
                }
                (id, title)
            }
            PluginContribution::VideoDrawingTools { id, title, tools } => {
                if tools.is_empty()
                    || tools.len() > 7
                    || tools.iter().collect::<BTreeSet<_>>().len() != tools.len()
                {
                    return Err("视频绘制工具必须包含 1 至 7 个不重复的工具".into());
                }
                (id, title)
            }
            PluginContribution::SelectionRecording { id, title, .. }
            | PluginContribution::AgentVisualAnnotations { id, title, .. }
            | PluginContribution::ArtifactVideoGif { id, title, .. }
            | PluginContribution::RecordingAudio { id, title, .. }
            | PluginContribution::ArtifactVideoTrim { id, title, .. }
            | PluginContribution::Codes { id, title, .. }
            | PluginContribution::InputSpeechToText { id, title, .. }
            | PluginContribution::MemoryProvider { id, title, .. }
            | PluginContribution::Ocr { id, title, .. }
            | PluginContribution::Scroll { id, title }
            | PluginContribution::Pin { id, title }
            | PluginContribution::Translation { id, title } => (id, title),
        };
        if !identifier(id) || !ids.insert(id) || !text(title, 80, false) {
            return Err("插件功能标识重复或格式不正确".into());
        }
    }
    if serde_json::to_vec_pretty(manifest)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_MANIFEST_BYTES
    {
        return Err("规范化后的插件包超过 64 KiB".into());
    }
    Ok(())
}

fn validate_source(source: &PluginSource) -> Result<(), String> {
    match source {
        PluginSource::Official => Ok(()),
        PluginSource::Local { path } if text(path, 4096, false) => Ok(()),
        PluginSource::Github {
            repository,
            commit,
            path,
        } => {
            if repository.split('/').count() != 2
                || !repository.split('/').all(|p| {
                    !p.is_empty()
                        && p != "."
                        && p != ".."
                        && !p.to_ascii_lowercase().ends_with(".git")
                        && p.bytes()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
                })
                || !text(repository, 512, false)
                || commit.len() != 40
                || !commit.bytes().all(|c| c.is_ascii_hexdigit())
                || !text(path, 512, false)
                || path.contains('\\')
                || path.starts_with('/')
                || path
                    .split('/')
                    .any(|p| p.is_empty() || matches!(p, "." | ".."))
            {
                return Err("GitHub 来源必须绑定仓库、完整提交和相对文件路径".into());
            }
            Ok(())
        }
        _ => Err("插件来源不正确".into()),
    }
}

fn same_origin(a: &PluginSource, b: &PluginSource) -> bool {
    match (a, b) {
        (PluginSource::Official, PluginSource::Official) => true,
        (PluginSource::Local { path: a }, PluginSource::Local { path: b }) => a == b,
        (
            PluginSource::Github {
                repository: a,
                path: ap,
                ..
            },
            PluginSource::Github {
                repository: b,
                path: bp,
                ..
            },
        ) => a.eq_ignore_ascii_case(b) && ap == bp,
        _ => false,
    }
}

fn validate_record(record: &StoredRecord) -> Result<(), String> {
    validate_manifest(&record.package.manifest)?;
    validate_source(&record.package.source)?;
    if record.revision == 0 || record.revision > i64::MAX as u64 {
        return Err("插件修订号不正确".into());
    }
    if let Some(previous) = &record.previous {
        validate_manifest(&previous.manifest)?;
        validate_source(&previous.source)?;
        if previous.manifest.id != record.package.manifest.id
            || previous.manifest.version == record.package.manifest.version
            || record.state == PluginState::Removed
            || !same_origin(&previous.source, &record.package.source)
        {
            return Err("插件回退记录不正确".into());
        }
    }
    if record.package.manifest.id.starts_with("mewu.")
        && record.package.source != PluginSource::Official
    {
        return Err("第三方插件不能使用 Mewu 官方标识".into());
    }
    Ok(())
}

fn err_db(error: rusqlite::Error) -> String {
    format!("插件数据库操作失败：{error}")
}
fn json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

impl PluginStore {
    pub fn open(app_data: &Path) -> Result<Self, String> {
        if !app_data.is_absolute() {
            return Err("应用数据目录必须为绝对路径".into());
        }
        ordinary_directory(app_data)?;
        let app_data = fs::canonicalize(app_data).map_err(|e| e.to_string())?;
        let root = safe_child_dir(&app_data, "plugins", true)?;
        safe_child_dir(&root, "packages", true)?;
        let db_path = root.join("plugins.db");
        if db_path.try_exists().map_err(|e| e.to_string())? {
            ordinary_file(&db_path)?;
        }
        let mut connection = Connection::open(&db_path).map_err(err_db)?;
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(err_db)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(err_db)?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(err_db)?;
        if version == 0 {
            let tables: i64 = connection
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |r| r.get(0),
                )
                .map_err(err_db)?;
            if tables != 0 {
                return Err("插件数据库格式未知，未覆盖现有内容".into());
            }
            let tx = connection.transaction().map_err(err_db)?;
            tx.execute_batch("CREATE TABLE registry(id TEXT PRIMARY KEY, payload TEXT NOT NULL); CREATE TABLE metadata(slot INTEGER PRIMARY KEY CHECK(slot=1), revision INTEGER NOT NULL CHECK(revision>=0)); INSERT INTO metadata VALUES(1,0); CREATE TABLE file_intent(slot INTEGER PRIMARY KEY CHECK(slot=1), payload TEXT NOT NULL); PRAGMA user_version=1;").map_err(err_db)?;
            tx.commit().map_err(err_db)?;
        } else if version != DB_VERSION {
            return Err("插件数据库版本不受支持，未覆盖现有内容".into());
        }
        connection.execute_batch("CREATE TABLE IF NOT EXISTS preferences(key TEXT PRIMARY KEY, value TEXT NOT NULL);").map_err(err_db)?;
        let mut store = Self { connection, root };
        store.global_revision()?;
        for record in store.records()? {
            validate_record(&record)?;
        }
        store.recover()?;
        // A tombstone is still a record. Ordinary seeding never overwrites an
        // existing installation, a disabled choice, or an explicit removal.
        for seed in BUNDLED {
            let manifest = parse_manifest(seed)?;
            if store.read(&manifest.id)?.is_none() {
                store.install_package(manifest, PluginSource::Official, None, true)?;
            }
        }
        Ok(store)
    }

    /// Deliberately recognize two exact committed states, not semver ranges.
    /// revision2 + exact1.0 previous is compatible with the old one-step upgrade;
    /// persisted state cannot distinguish a historically identical explicit update.
    fn upgrade_canonical_drawing_v1_1_0(&mut self) -> Result<(), String> {
        let previous = parse_manifest(OFFICIAL_DRAWING_V1_0_0)?;
        let original = parse_manifest(OFFICIAL_DRAWING_V1_1_0)?;
        let target = parse_manifest(OFFICIAL_DRAWING)?;
        if target.id != original.id || target.version != "1.2.0" {
            return Ok(());
        }
        let Some(record) = self.read(&original.id)? else {
            return Ok(());
        };
        if record.package.source != PluginSource::Official
            || record.package.manifest != original
            || record.state != PluginState::Enabled
            || self.check_package(&record.package).is_err()
        {
            return Ok(());
        }
        let eligible = match (record.revision, &record.previous) {
            (1, None) => true,
            (2, Some(package)) => {
                package.source == PluginSource::Official
                    && package.manifest == previous
                    && self.check_package(package).is_ok()
            }
            _ => false,
        };
        if eligible {
            self.install_package(target, PluginSource::Official, Some(record.revision), true)?;
        }
        Ok(())
    }

    fn migrate_recording_bundle(&mut self) -> Result<(), String> {
        let marker: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM preferences WHERE key=?1",
                [RECORDING_MIGRATION_KEY],
                |r| r.get(0),
            )
            .optional()
            .map_err(err_db)?;
        let records = self.legacy_recording_rows()?;
        if let Some(marker) = marker {
            if marker.len() > MAX_MANIFEST_BYTES * 12 {
                return Err("屏幕录制迁移记录过大，未覆盖内容".into());
            }
            let archived: RecordingMigration =
                serde_json::from_str(&marker).map_err(|_| "屏幕录制迁移记录损坏，未覆盖内容")?;
            validate_recording_migration(&archived)?;
            if !records.is_empty() || self.read("mewu.recording")?.is_none() {
                return Err("屏幕录制迁移记录与插件状态不一致，未重新启用".into());
            }
            return Ok(());
        }
        if self.read("mewu.recording")?.is_some() {
            return Err("屏幕录制迁移记录缺失，未覆盖现有插件".into());
        }
        let migration = RecordingMigration {
            version: 1,
            records,
        };
        validate_recording_migration(&migration)?;
        if self
            .records()?
            .len()
            .saturating_sub(migration.records.len())
            >= 512
        {
            return Err("插件数量已达到上限，未归并屏幕录制".into());
        }
        let mut state = if !migration.records.is_empty() && migration.records.len() != 3 {
            PluginState::Disabled
        } else {
            PluginState::Enabled
        };
        for archived in &migration.records {
            let record = decode_record(&archived.id, &archived.payload)?;
            if record.state == PluginState::Removed {
                state = PluginState::Removed;
            } else if state != PluginState::Removed
                && (record.state == PluginState::Disabled
                    || record.package.source != PluginSource::Official
                    || !legacy_recording_is_canonical(&record.package.manifest)?
                    || self.check_package(&record.package).is_err())
            {
                state = PluginState::Disabled;
            }
        }
        let after = StoredRecord {
            package: Package {
                manifest: parse_manifest(OFFICIAL_RECORDING)?,
                source: PluginSource::Official,
            },
            revision: 1,
            state,
            previous: None,
        };
        // Old files are immutable migration evidence, never executable authority.
        // Only their registry rows move to the archive, atomically with the new row.
        self.change_with_migration(None, after, true, Some(migration))?;
        Ok(())
    }

    fn legacy_recording_rows(&self) -> Result<Vec<ArchivedRecordingRecord>, String> {
        let mut result = Vec::new();
        for id in LEGACY_RECORDING_IDS {
            let payload: Option<String> = self
                .connection
                .query_row("SELECT payload FROM registry WHERE id=?1", [id], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(err_db)?;
            if let Some(payload) = payload {
                result.push(ArchivedRecordingRecord {
                    id: id.into(),
                    payload,
                });
            }
        }
        Ok(result)
    }

    pub fn snapshot(&self) -> Result<PluginSnapshot, String> {
        self.ready()?;
        Ok(PluginSnapshot {
            revision: self.global_revision()?,
            plugins: self.list()?,
        })
    }

    /// Only intact, enabled packages provide defaults for *new* user profiles.
    /// Existing profiles are user-owned; no credentials, DB writes or network
    /// calls occur here, and disabling a template never changes saved profiles.
    pub fn model_connection_presets(&self) -> Result<Vec<ModelConnectionPreset>, String> {
        let mut result = Vec::new();
        for record in self.list()? {
            if record.state != PluginState::Enabled || record.error.is_some() { continue; }
            for contribution in record.manifest.contributions {
                let PluginContribution::ModelConnection { id, title, template } = contribution else { continue; };
                let provider_id = if record.source == PluginSource::Official {
                    template.provider_id.clone()
                } else {
                    // Stable across updates; community IDs cannot shadow the
                    // original provider names or another package's templates.
                    let mut digest = Sha256::new();
                    digest.update(record.manifest.id.as_bytes()); digest.update([0]); digest.update(id.as_bytes());
                    format!("plugin.{:x}", digest.finalize())
                };
                result.push(ModelConnectionPreset {
                    id: provider_id, name: title, group: template.group, base_url: template.base_url,
                    model: template.model, advanced: template.advanced, search_terms: template.search_terms,
                    plugin_id: record.manifest.id.clone(), plugin_revision: record.revision, contribution_id: id,
                });
            }
        }
        Ok(result)
    }

    pub fn list(&self) -> Result<Vec<PluginRecord>, String> {
        self.ready()?;
        self.records()?
            .into_iter()
            .map(|r| self.public_record(r))
            .collect()
    }

    /// Relocation preflight needs typed provenance, not package-file validation.
    /// Include rollback provenance so a later rollback cannot restore an old-root
    /// local update source. This performs only reads on the owned SQLite handle.
    pub fn migration_sources(&self) -> Result<Vec<PluginSource>, String> {
        self.ready()?;
        let mut sources = Vec::new();
        for record in self.records()? {
            if record.state == PluginState::Removed {
                continue;
            }
            sources.push(record.package.source);
            if let Some(previous) = record.previous {
                sources.push(previous.source);
            }
        }
        Ok(sources)
    }

    /// Host validates its network policy before saving this preference.
    pub fn get_catalog_source(&self) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT value FROM preferences WHERE key='catalog_source'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err_db)
    }

    pub fn set_catalog_source(&mut self, source: Option<&str>) -> Result<(), String> {
        match source {
            Some(source) => {
                if !text(source, 2048, false) {
                    return Err("插件目录地址不正确".into());
                }
                self.connection.execute("INSERT INTO preferences(key,value) VALUES('catalog_source',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [source]).map_err(err_db)?;
            }
            None => {
                self.connection
                    .execute("DELETE FROM preferences WHERE key='catalog_source'", [])
                    .map_err(err_db)?;
            }
        }
        Ok(())
    }

    pub fn install_local(
        &mut self,
        path: &Path,
        expected_revision: Option<u64>,
    ) -> Result<PluginRecord, String> {
        let bytes = read_validated_local(path)?;
        let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
        self.install_bytes(
            &bytes,
            PluginSource::Local {
                path: path.to_string_lossy().into_owned(),
            },
            expected_revision,
        )
    }

    pub fn install_bytes(
        &mut self,
        bytes: &[u8],
        mut source: PluginSource,
        expected_revision: Option<u64>,
    ) -> Result<PluginRecord, String> {
        if source == PluginSource::Official {
            return Err("官方来源仅允许安装程序提供".into());
        }
        if let PluginSource::Local { path } = &mut source {
            let file = Path::new(path);
            if !file.is_absolute() {
                return Err("本地插件来源必须为绝对路径".into());
            }
            ordinary_file(file)?;
            *path = fs::canonicalize(file)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .into_owned();
        }
        let manifest = parse_manifest(bytes)?;
        self.install_package(manifest, source, expected_revision, false)
    }

    /// Explicit user reinstall; unlike startup seeding this can clear a tombstone.
    pub fn install_official(
        &mut self,
        id: &str,
        expected_revision: Option<u64>,
    ) -> Result<PluginRecord, String> {
        let manifest = BUNDLED
            .into_iter()
            .map(parse_manifest)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .find(|m| m.id == id)
            .ok_or("未找到此官方插件")?;
        self.install_package(manifest, PluginSource::Official, expected_revision, true)
    }

    fn install_package(
        &mut self,
        manifest: PluginManifest,
        source: PluginSource,
        expected: Option<u64>,
        official: bool,
    ) -> Result<PluginRecord, String> {
        self.ready()?;
        validate_manifest(&manifest)?;
        validate_source(&source)?;
        if manifest.id.starts_with("mewu.") && !official {
            return Err("第三方插件不能覆盖 Mewu 官方插件".into());
        }
        let before = self.read(&manifest.id)?;
        cas(before.as_ref(), expected)?;
        if before.is_none() && self.records()?.len() >= 512 {
            return Err("插件记录已达到本版本上限".into());
        }
        let package = Package { manifest, source };
        if let Some(old) = &before {
            if old.state != PluginState::Removed
                && !same_origin(&old.package.source, &package.source)
            {
                return Err("此标识已由其他来源安装；更换来源前请先卸载旧插件".into());
            }
            if old.package.manifest.version == package.manifest.version
                && old.package.manifest != package.manifest
            {
                return Err("同版本插件内容发生变化，请为修改后的包使用新版本号".into());
            }
            if old.package == package
                && old.state != PluginState::Removed
                && self.check_package(&package).is_ok()
            {
                return self.public_record(old.clone());
            }
        }
        let after = StoredRecord {
            revision: next_revision(before.as_ref().map_or(0, |r| r.revision))?,
            state: before
                .as_ref()
                .filter(|r| r.state != PluginState::Removed)
                .map_or(PluginState::Enabled, |r| r.state),
            previous: before
                .as_ref()
                .filter(|r| r.state != PluginState::Removed)
                .and_then(|r| {
                    if r.package.manifest.version != package.manifest.version {
                        Some(r.package.clone())
                    } else {
                        r.previous.clone()
                    }
                }),
            package,
        };
        self.change(before, after, true)
    }

    pub fn set_enabled(
        &mut self,
        id: &str,
        expected_revision: u64,
        enabled: bool,
    ) -> Result<PluginRecord, String> {
        self.ready()?;
        let before = self.required(id, expected_revision)?;
        if before.state == PluginState::Removed {
            return Err("插件已卸载，请先重新安装".into());
        }
        if enabled {
            self.check_package(&before.package)?;
        }
        let state = if enabled {
            PluginState::Enabled
        } else {
            PluginState::Disabled
        };
        if before.state == state {
            return self.public_record(before);
        }
        let mut after = before.clone();
        after.state = state;
        after.revision = next_revision(after.revision)?;
        self.change(Some(before), after, false)
    }

    pub fn uninstall(&mut self, id: &str, expected_revision: u64) -> Result<PluginRecord, String> {
        self.ready()?;
        let before = self.required(id, expected_revision)?;
        if before.state == PluginState::Removed {
            return self.public_record(before);
        }
        let mut after = before.clone();
        after.state = PluginState::Removed;
        after.previous = None;
        after.revision = next_revision(after.revision)?;
        self.change(Some(before), after, true)
    }

    pub fn rollback(&mut self, id: &str, expected_revision: u64) -> Result<PluginRecord, String> {
        self.ready()?;
        let before = self.required(id, expected_revision)?;
        if before.state == PluginState::Removed {
            return Err("已卸载插件不能回退".into());
        }
        let package = before.previous.clone().ok_or("此插件没有可回退版本")?;
        let after = StoredRecord {
            package,
            revision: next_revision(before.revision)?,
            state: before.state,
            previous: Some(before.package.clone()),
        };
        self.change(Some(before), after, true)
    }

    /// Call again immediately before beginning a run. A cached menu item is not
    /// authority after an update, disable or uninstall changed its revision.
    pub fn workflow(
        &self,
        id: &str,
        revision: u64,
        contribution_id: &str,
    ) -> Result<SelectionWorkflow, String> {
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此插件未启用".into());
        }
        self.check_package(&record.package)?;
        for item in record.package.manifest.contributions {
            if let PluginContribution::Workflow {
                id,
                title,
                prompt,
                accepts,
            } = item
            {
                if id == contribution_id {
                    return Ok(SelectionWorkflow {
                        id,
                        title,
                        kind: WorkflowKind::SelectionWorkflow,
                        prompt,
                        accepts,
                    });
                }
            }
        }
        Err("未找到此选区工作流".into())
    }

    pub fn drawing_tools(
        &self,
        id: &str,
        revision: u64,
        contribution_id: &str,
    ) -> Result<Vec<DrawingTool>, String> {
        if id == CORE_DRAWING {
            return (core_drawing_grant(id, revision, contribution_id)
                && contribution_id == "drawing-tools")
                .then(|| {
                    vec![
                        DrawingTool::Pen,
                        DrawingTool::Line,
                        DrawingTool::Arrow,
                        DrawingTool::Rect,
                        DrawingTool::Ellipse,
                        DrawingTool::Text,
                        DrawingTool::Highlighter,
                        DrawingTool::Number,
                        DrawingTool::Mosaic,
                    ]
                })
                .ok_or_else(|| "绘制能力已变化".into());
        }
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此插件未启用".into());
        }
        self.check_package(&record.package)?;
        for item in record.package.manifest.contributions {
            if let PluginContribution::DrawingTools { id, tools, .. } = item {
                if id == contribution_id {
                    return Ok(tools);
                }
            }
        }
        Err("未找到此绘制工具集".into())
    }

    pub fn video_drawing_tools(
        &self,
        id: &str,
        revision: u64,
        contribution_id: &str,
    ) -> Result<Vec<VideoDrawingTool>, String> {
        if id == CORE_DRAWING {
            return (core_drawing_grant(id, revision, contribution_id)
                && contribution_id == "video-drawing-tools")
                .then(|| {
                    vec![
                        VideoDrawingTool::Pen,
                        VideoDrawingTool::Line,
                        VideoDrawingTool::Arrow,
                        VideoDrawingTool::Rect,
                        VideoDrawingTool::Ellipse,
                        VideoDrawingTool::Text,
                        VideoDrawingTool::Number,
                    ]
                })
                .ok_or_else(|| "视频绘制能力已变化".into());
        }
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此插件未启用".into());
        }
        self.check_package(&record.package)?;
        record
            .package
            .manifest
            .contributions
            .into_iter()
            .find_map(|item| match item {
                PluginContribution::VideoDrawingTools { id, tools, .. }
                    if id == contribution_id =>
                {
                    Some(tools)
                }
                _ => None,
            })
            .ok_or_else(|| "未找到此视频绘制工具集".into())
    }

    pub fn ocr(&self, id: &str, revision: u64, contribution_id: &str) -> Result<OcrEngine, String> {
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此插件未启用".into());
        }
        self.check_package(&record.package)?;
        for item in record.package.manifest.contributions {
            if let PluginContribution::Ocr { id, engine, .. } = item {
                if id == contribution_id {
                    return Ok(engine);
                }
            }
        }
        Err("未找到此文字识别功能".into())
    }

    pub fn scroll(&self, id: &str, revision: u64, contribution_id: &str) -> Result<(), String> {
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此插件未启用".into());
        }
        self.check_package(&record.package)?;
        if record.package.manifest.contributions.iter().any(
            |item| matches!(item, PluginContribution::Scroll { id, .. } if id == contribution_id),
        ) {
            Ok(())
        } else {
            Err("未找到此长截图功能".into())
        }
    }

    /// Translation is a host-owned text-only operation. The declaration cannot
    /// add prompts, tools, endpoints, credentials or executable code.
    pub fn translation(
        &self,
        id: &str,
        revision: u64,
        contribution_id: &str,
    ) -> Result<(), String> {
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此插件未启用".into());
        }
        self.check_package(&record.package)?;
        if record.package.manifest.contributions.iter().any(
            |item| matches!(item, PluginContribution::Translation { id, .. } if id == contribution_id),
        ) {
            Ok(())
        } else {
            Err("未找到此原位翻译功能".into())
        }
    }

    /// Authorizes a new host-owned image window. Existing pinned images are
    /// independent of the package; this declaration carries no window URL or code.
    pub fn pin(
        &self,
        id: &str,
        revision: u64,
        contribution_id: &str,
    ) -> Result<PluginContribution, String> {
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此插件未启用".into());
        }
        self.check_package(&record.package)?;
        record
            .package
            .manifest
            .contributions
            .into_iter()
            .find(
                |item| matches!(item, PluginContribution::Pin { id, .. } if id == contribution_id),
            )
            .ok_or_else(|| "未找到此贴图功能".into())
    }

    pub fn visual_annotations(
        &self,
        grant: &mewu_core::VisualAnnotationGrant,
    ) -> Result<VisualAnnotationEngine, String> {
        if grant.plugin_id == "mewu.core.annotations" {
            return core_annotation_grant_is_current(grant)
                .then_some(VisualAnnotationEngine::HostVectorV1)
                .ok_or_else(|| "标注能力已变化".into());
        }
        self.ready()?;
        let record = self.required(&grant.plugin_id, grant.plugin_revision)?;
        if record.state != PluginState::Enabled {
            return Err("原位作答插件未启用".into());
        }
        self.check_package(&record.package)?;
        record
            .package
            .manifest
            .contributions
            .into_iter()
            .find_map(|c| match c {
                PluginContribution::AgentVisualAnnotations { id, engine, .. }
                    if id == grant.contribution_id =>
                {
                    Some(engine)
                }
                _ => None,
            })
            .ok_or_else(|| "未找到此原位作答功能".into())
    }

    pub fn recording_candidate(&self) -> Result<Option<RecordingGrant>, String> {
        Ok(Some(RecordingGrant {
            plugin_id: CORE_RECORDING.into(),
            revision: 1,
            contribution_id: "record".into(),
        }))
    }

    pub fn recording(
        &self,
        id: &str,
        revision: u64,
        contribution_id: &str,
    ) -> Result<RecordingEngine, String> {
        if id == CORE_RECORDING {
            return (core_recording_grant(id, revision, contribution_id)
                && contribution_id == "record")
                .then_some(RecordingEngine::WindowsWgcMf)
                .ok_or_else(|| "录屏能力已变化".into());
        }
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("屏幕录制插件未启用".into());
        }
        self.check_package(&record.package)?;
        record
            .package
            .manifest
            .contributions
            .into_iter()
            .find_map(|c| match c {
                PluginContribution::SelectionRecording { id, engine, .. }
                    if id == contribution_id =>
                {
                    Some(engine)
                }
                _ => None,
            })
            .ok_or_else(|| "未找到此屏幕录制功能".into())
    }

    /// A picker candidate, not authority bound to an entire video export.
    /// Prefer the official package, then choose deterministically by identity.
    pub fn gif_candidate(&self) -> Result<Option<GifGrant>, String> {
        Ok(Some(GifGrant {
            plugin_id: CORE_RECORDING.into(),
            revision: 1,
            contribution_id: "gif".into(),
        }))
    }

    /// Exact admission after choosing GIF and before accepting file publication.
    /// MP4 saving does not depend on this grant.
    pub fn video_gif(
        &self,
        id: &str,
        revision: u64,
        contribution_id: &str,
    ) -> Result<VideoGifEngine, String> {
        if id == CORE_RECORDING {
            return (core_recording_grant(id, revision, contribution_id)
                && contribution_id == "gif")
                .then_some(VideoGifEngine::WindowsMediaEditingGif)
                .ok_or_else(|| "GIF导出能力已变化".into());
        }
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此 GIF 导出插件未启用".into());
        }
        self.check_package(&record.package)?;
        record
            .package
            .manifest
            .contributions
            .into_iter()
            .find_map(|item| match item {
                PluginContribution::ArtifactVideoGif { id, engine, .. }
                    if id == contribution_id =>
                {
                    Some(engine)
                }
                _ => None,
            })
            .ok_or_else(|| "未找到此 GIF 导出功能".into())
    }

    /// Installing this declaration never starts capture. An explicit preference
    /// and this exact current grant are both required by the recording host.
    pub fn recording_audio(
        &self,
        id: &str,
        revision: u64,
        contribution_id: &str,
    ) -> Result<RecordingAudioEngine, String> {
        if id == CORE_RECORDING {
            return (core_recording_grant(id, revision, contribution_id)
                && contribution_id == "audio")
                .then_some(RecordingAudioEngine::WindowsWasapi)
                .ok_or_else(|| "录屏声音能力已变化".into());
        }
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此录屏声音插件未启用".into());
        }
        self.check_package(&record.package)?;
        record
            .package
            .manifest
            .contributions
            .into_iter()
            .find_map(|item| match item {
                PluginContribution::RecordingAudio { id, engine, .. } if id == contribution_id => {
                    Some(engine)
                }
                _ => None,
            })
            .ok_or_else(|| "未找到此录屏声音功能".into())
    }

    /// Only range edits need this grant. Reading or exporting an already saved
    /// range must remain faithful to that document after the plugin is removed.
    pub fn video_trim(
        &self,
        id: &str,
        revision: u64,
        contribution_id: &str,
    ) -> Result<PluginContribution, String> {
        if id == CORE_RECORDING {
            return (core_recording_grant(id, revision, contribution_id)
                && contribution_id == "trim")
                .then(|| PluginContribution::ArtifactVideoTrim {
                    id: "trim".into(),
                    title: "裁剪".into(),
                    engine: VideoTrimEngine::WindowsMediaEditing,
                })
                .ok_or_else(|| "视频裁剪能力已变化".into());
        }
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此视频裁剪插件未启用".into());
        }
        self.check_package(&record.package)?;
        record.package.manifest.contributions.into_iter().find(|item| {
            matches!(item, PluginContribution::ArtifactVideoTrim { id, .. } if id == contribution_id)
        }).ok_or_else(|| "未找到此视频裁剪功能".into())
    }

    /// Authorize each explicitly started dictation and its final result again.
    /// This declaration never starts listening at installation or enumeration.
    pub fn speech_to_text(
        &self,
        id: &str,
        revision: u64,
        contribution_id: &str,
    ) -> Result<SpeechEngine, String> {
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此语音插件未启用".into());
        }
        self.check_package(&record.package)?;
        record
            .package
            .manifest
            .contributions
            .into_iter()
            .find_map(|item| match item {
                PluginContribution::InputSpeechToText { id, engine, .. }
                    if id == contribution_id =>
                {
                    Some(engine)
                }
                _ => None,
            })
            .ok_or_else(|| "未找到此语音输入功能".into())
    }

    /// Reauthorize both scan results and explicit copy/open actions. Merely
    /// declaring this contribution never grants general clipboard/URL access.
    pub fn codes(
        &self,
        id: &str,
        revision: u64,
        contribution_id: &str,
    ) -> Result<CodeEngine, String> {
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此二维码与条码插件未启用".into());
        }
        self.check_package(&record.package)?;
        record
            .package
            .manifest
            .contributions
            .into_iter()
            .find_map(|item| match item {
                PluginContribution::Codes { id, engine, .. } if id == contribution_id => {
                    Some(engine)
                }
                _ => None,
            })
            .ok_or_else(|| "未找到此二维码或条码识别功能".into())
    }

    pub fn memory_provider(
        &self,
        id: &str,
        revision: u64,
        contribution_id: &str,
    ) -> Result<MemoryAdapter, String> {
        self.ready()?;
        let record = self.required(id, revision)?;
        if record.state != PluginState::Enabled {
            return Err("此记忆插件未启用".into());
        }
        self.check_package(&record.package)?;
        record
            .package
            .manifest
            .contributions
            .into_iter()
            .find_map(|item| match item {
                PluginContribution::MemoryProvider { id, adapter, .. } if id == contribution_id => {
                    Some(adapter)
                }
                _ => None,
            })
            .ok_or_else(|| "未找到此记忆服务".into())
    }

    fn global_revision(&self) -> Result<u64, String> {
        let revision: i64 = self
            .connection
            .query_row("SELECT revision FROM metadata WHERE slot=1", [], |r| {
                r.get(0)
            })
            .map_err(err_db)?;
        u64::try_from(revision).map_err(|_| "插件数据库修订号不正确".into())
    }

    fn ready(&self) -> Result<(), String> {
        let pending: bool = self
            .connection
            .query_row("SELECT EXISTS(SELECT 1 FROM file_intent)", [], |r| r.get(0))
            .map_err(err_db)?;
        if pending {
            Err("插件文件操作尚未恢复，请重新启动应用后重试".into())
        } else {
            Ok(())
        }
    }

    fn read(&self, id: &str) -> Result<Option<StoredRecord>, String> {
        let value: Option<String> = self
            .connection
            .query_row("SELECT payload FROM registry WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err_db)?;
        value.map(|value| decode_record(id, &value)).transpose()
    }

    fn records(&self) -> Result<Vec<StoredRecord>, String> {
        let mut query = self
            .connection
            .prepare("SELECT id,payload FROM registry ORDER BY id")
            .map_err(err_db)?;
        let rows = query
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(err_db)?;
        rows.map(|row| {
            let (id, payload) = row.map_err(err_db)?;
            decode_record(&id, &payload)
        })
        .collect()
    }

    fn required(&self, id: &str, expected: u64) -> Result<StoredRecord, String> {
        let record = self.read(id)?.ok_or("未找到此插件")?;
        cas(Some(&record), Some(expected))?;
        Ok(record)
    }

    fn public_record(&self, record: StoredRecord) -> Result<PluginRecord, String> {
        validate_record(&record)?;
        let error = if record.state == PluginState::Removed {
            None
        } else {
            self.check_package(&record.package).err()
        };
        Ok(PluginRecord {
            manifest: record.package.manifest,
            source: record.package.source,
            revision: record.revision,
            state: record.state,
            has_rollback: record.previous.is_some(),
            error,
        })
    }

    fn check_package(&self, package: &Package) -> Result<(), String> {
        let path = self.package_path(package, false)?;
        let found = parse_manifest(&read_bounded(&path)?)?;
        if found != package.manifest {
            return Err("插件文件与批准的安装记录不一致，请重新安装".into());
        }
        Ok(())
    }

    fn package_path(&self, package: &Package, create: bool) -> Result<PathBuf, String> {
        validate_manifest(&package.manifest)?;
        let packages = safe_child_dir(&self.root, "packages", false)?;
        let id = safe_child_dir(&packages, &package.manifest.id, create)?;
        let version = safe_child_dir(&id, &package.manifest.version, create)?;
        Ok(version.join("plugin.json"))
    }

    fn change(
        &mut self,
        before: Option<StoredRecord>,
        after: StoredRecord,
        files: bool,
    ) -> Result<PluginRecord, String> {
        self.change_with_migration(before, after, files, None)
    }

    fn change_with_migration(
        &mut self,
        before: Option<StoredRecord>,
        after: StoredRecord,
        files: bool,
        recording_migration: Option<RecordingMigration>,
    ) -> Result<PluginRecord, String> {
        validate_record(&after)?;
        let intent = FileIntent {
            before: before.clone(),
            after: after.clone(),
            recording_migration,
        };
        validate_migration_intent(&intent)?;
        if files {
            // Reject a conflicting pre-existing target before journaling. The
            // rollback must never mistake somebody else's file for our output.
            for package in packages_of(Some(&after)) {
                self.preflight_package(package)?;
            }
            self.connection
                .execute(
                    "INSERT INTO file_intent(slot,payload) VALUES(1,?1)",
                    [json(&intent)?],
                )
                .map_err(err_db)?;
            if let Err(error) = self.apply_files(before.as_ref(), Some(&after)) {
                return Err(self.undo(&intent, error));
            }
        }
        let commit = (|| {
            let tx = self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(err_db)?;
            let current: Option<String> = tx
                .query_row(
                    "SELECT payload FROM registry WHERE id=?1",
                    [&after.package.manifest.id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err_db)?;
            let actual = current
                .map(|s| decode_record(&after.package.manifest.id, &s))
                .transpose()?;
            if actual != before {
                return Err("插件已被修改，请刷新后重试".into());
            }
            if let Some(migration) = &intent.recording_migration {
                verify_recording_migration_before(&tx, migration)?;
                tx.execute(
                    "INSERT INTO preferences(key,value) VALUES(?1,?2)",
                    params![RECORDING_MIGRATION_KEY, json(migration)?],
                )
                .map_err(err_db)?;
                for archived in &migration.records {
                    if tx
                        .execute(
                            "DELETE FROM registry WHERE id=?1 AND payload=?2",
                            params![archived.id, archived.payload],
                        )
                        .map_err(err_db)?
                        != 1
                    {
                        return Err("旧录屏插件已变更，未归并".into());
                    }
                }
            }
            tx.execute("INSERT INTO registry(id,payload) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", params![after.package.manifest.id, json(&after)?]).map_err(err_db)?;
            if tx.execute("UPDATE metadata SET revision=revision+1 WHERE slot=1 AND revision<9223372036854775807", []).map_err(err_db)? != 1 {
                return Err("插件全局修订号已达到上限".into());
            }
            if files {
                tx.execute("DELETE FROM file_intent WHERE slot=1", [])
                    .map_err(err_db)?;
            }
            tx.commit().map_err(err_db)
        })();
        if let Err(error) = commit {
            return Err(if files {
                self.undo(&intent, error)
            } else {
                error
            });
        }
        self.public_record(after)
    }

    fn undo(&mut self, intent: &FileIntent, error: String) -> String {
        match self
            .apply_files(Some(&intent.after), intent.before.as_ref())
            .and_then(|()| {
                self.connection
                    .execute("DELETE FROM file_intent WHERE slot=1", [])
                    .map(|_| ())
                    .map_err(err_db)
            }) {
            Ok(()) => error,
            Err(recovery) => format!("{error}；文件恢复尚未完成：{recovery}"),
        }
    }

    fn recover(&mut self) -> Result<(), String> {
        let pending: Option<String> = self
            .connection
            .query_row("SELECT payload FROM file_intent WHERE slot=1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err_db)?;
        let Some(pending) = pending else {
            return Ok(());
        };
        if pending.len() > MAX_MANIFEST_BYTES * 16 {
            return Err("插件恢复日志过大，未覆盖文件".into());
        }
        let intent: FileIntent =
            serde_json::from_str(&pending).map_err(|_| "插件恢复日志损坏，未覆盖文件")?;
        validate_migration_intent(&intent)?;
        validate_record(&intent.after)?;
        if let Some(before) = &intent.before {
            validate_record(before)?;
            if before.package.manifest.id != intent.after.package.manifest.id {
                return Err("插件恢复日志身份不一致".into());
            }
        }
        if self.read(&intent.after.package.manifest.id)? != intent.before {
            return Err("插件恢复日志与数据库不一致，未覆盖文件".into());
        }
        if let Some(migration) = &intent.recording_migration {
            verify_recording_migration_before(&self.connection, migration)?;
        }
        self.apply_files(Some(&intent.after), intent.before.as_ref())?;
        self.connection
            .execute("DELETE FROM file_intent WHERE slot=1", [])
            .map_err(err_db)?;
        Ok(())
    }

    fn apply_files(
        &self,
        from: Option<&StoredRecord>,
        to: Option<&StoredRecord>,
    ) -> Result<(), String> {
        let wanted = packages_of(to);
        // New versions are complete before any previous version is removed.
        for package in &wanted {
            let path = self.package_path(package, true)?;
            write_manifest(&path, &package.manifest)?;
        }
        for package in packages_of(from) {
            if wanted
                .iter()
                .any(|p| p.manifest.version == package.manifest.version)
            {
                continue;
            }
            self.remove_package(package)?;
        }
        Ok(())
    }

    fn preflight_package(&self, package: &Package) -> Result<(), String> {
        let mut parent = safe_child_dir(&self.root, "packages", false)?;
        for component in [&package.manifest.id, &package.manifest.version] {
            match fs::symlink_metadata(parent.join(component)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.to_string()),
                Ok(_) => parent = safe_child_dir(&parent, component, false)?,
            }
        }
        let path = parent.join("plugin.json");
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
            Ok(_) => {
                if parse_manifest(&read_bounded(&path)?)? != package.manifest {
                    return Err("目标版本文件已存在且内容不同，未覆盖文件".into());
                }
                Ok(())
            }
        }
    }

    fn remove_package(&self, package: &Package) -> Result<(), String> {
        let packages = safe_child_dir(&self.root, "packages", false)?;
        let id_path = packages.join(&package.manifest.id);
        if !id_path.try_exists().map_err(|e| e.to_string())? {
            return Ok(());
        }
        let id_path = safe_child_dir(&packages, &package.manifest.id, false)?;
        let version = id_path.join(&package.manifest.version);
        if !version.try_exists().map_err(|e| e.to_string())? {
            return Ok(());
        }
        let version = safe_child_dir(&id_path, &package.manifest.version, false)?;
        for filename in ["plugin.json", "plugin.json.pending"] {
            let file = version.join(filename);
            if file.try_exists().map_err(|e| e.to_string())? {
                ordinary_file(&file)?;
                fs::remove_file(&file).map_err(|e| format!("无法删除插件文件：{e}"))?;
            }
        }
        // Never recursively delete: an unexpected file is not ours to remove.
        if fs::read_dir(&version)
            .map_err(|e| e.to_string())?
            .next()
            .is_none()
        {
            fs::remove_dir(&version).map_err(|e| e.to_string())?;
        }
        if fs::read_dir(&id_path)
            .map_err(|e| e.to_string())?
            .next()
            .is_none()
        {
            fs::remove_dir(&id_path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn legacy_recording_is_canonical(manifest: &PluginManifest) -> Result<bool, String> {
    let bytes = match manifest.id.as_str() {
        "mewu.video-trim" => OFFICIAL_VIDEO_TRIM,
        "mewu.recording-audio" => OFFICIAL_RECORDING_AUDIO,
        "mewu.gif" => OFFICIAL_GIF,
        _ => return Ok(false),
    };
    Ok(*manifest == parse_manifest(bytes)?)
}

fn validate_recording_migration(value: &RecordingMigration) -> Result<(), String> {
    if value.version != 1 || value.records.len() > 3 || json(value)?.len() > MAX_MANIFEST_BYTES * 12
    {
        return Err("屏幕录制迁移记录无效".into());
    }
    let mut seen = BTreeSet::new();
    for archived in &value.records {
        if !LEGACY_RECORDING_IDS.contains(&archived.id.as_str()) || !seen.insert(&archived.id) {
            return Err("屏幕录制迁移身份不一致".into());
        }
        decode_record(&archived.id, &archived.payload)?;
    }
    Ok(())
}

fn validate_migration_intent(intent: &FileIntent) -> Result<(), String> {
    if let Some(migration) = &intent.recording_migration {
        validate_recording_migration(migration)?;
        if intent.before.is_some()
            || intent.after.revision != 1
            || intent.after.previous.is_some()
            || intent.after.package.source != PluginSource::Official
            || intent.after.package.manifest != parse_manifest(OFFICIAL_RECORDING)?
        {
            return Err("屏幕录制迁移日志身份无效".into());
        }
    }
    Ok(())
}

fn verify_recording_migration_before(
    db: &Connection,
    migration: &RecordingMigration,
) -> Result<(), String> {
    let marker: Option<String> = db
        .query_row(
            "SELECT value FROM preferences WHERE key=?1",
            [RECORDING_MIGRATION_KEY],
            |r| r.get(0),
        )
        .optional()
        .map_err(err_db)?;
    if marker.is_some() {
        return Err("屏幕录制已归并，未覆盖当前选择".into());
    }
    for id in LEGACY_RECORDING_IDS {
        let current: Option<String> = db
            .query_row("SELECT payload FROM registry WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err_db)?;
        let expected = migration
            .records
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.payload.as_str());
        if current.as_deref() != expected {
            return Err("旧录屏插件已变更，未归并".into());
        }
    }
    Ok(())
}

fn decode_record(id: &str, payload: &str) -> Result<StoredRecord, String> {
    if payload.len() > MAX_MANIFEST_BYTES * 3 {
        return Err("插件数据库记录过大".into());
    }
    let record: StoredRecord =
        serde_json::from_str(payload).map_err(|_| "插件数据库记录损坏，未覆盖内容")?;
    validate_record(&record)?;
    if record.package.manifest.id != id {
        return Err("插件数据库身份不一致".into());
    }
    Ok(record)
}

fn cas(record: Option<&StoredRecord>, expected: Option<u64>) -> Result<(), String> {
    if record.map(|r| r.revision) != expected {
        Err("插件已被修改，请刷新后重试".into())
    } else {
        Ok(())
    }
}
fn next_revision(value: u64) -> Result<u64, String> {
    value
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or_else(|| "插件修订号已达到上限".into())
}
fn packages_of(record: Option<&StoredRecord>) -> Vec<&Package> {
    let Some(record) = record.filter(|r| r.state != PluginState::Removed) else {
        return vec![];
    };
    std::iter::once(&record.package)
        .chain(record.previous.iter())
        .collect()
}

fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
fn ordinary_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("插件目录不可用：{e}"))?;
    if !metadata.is_dir() || linked(&metadata) {
        return Err("插件目录不能是链接或重解析点".into());
    }
    Ok(())
}
fn ordinary_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("插件文件不可用：{e}"))?;
    if !metadata.is_file() || linked(&metadata) {
        return Err("插件文件不能是链接或重解析点".into());
    }
    Ok(())
}
fn safe_child_dir(parent: &Path, name: &str, create: bool) -> Result<PathBuf, String> {
    ordinary_directory(parent)?;
    let path = parent.join(name);
    if create {
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    ordinary_directory(&path)?;
    let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
    if canonical.parent() != Some(parent) {
        return Err("插件目录超出允许范围".into());
    }
    Ok(canonical)
}
fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    ordinary_file(path)?;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || linked(&metadata) || metadata.len() > MAX_MANIFEST_BYTES as u64 {
        return Err("插件文件不符合大小或类型限制".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err("插件文件超过 64 KiB".into());
    }
    Ok(bytes)
}
fn write_manifest(path: &Path, manifest: &PluginManifest) -> Result<(), String> {
    let pending = path.with_file_name("plugin.json.pending");
    if pending.try_exists().map_err(|e| e.to_string())? {
        ordinary_file(&pending)?;
        fs::remove_file(&pending).map_err(|e| e.to_string())?;
    }
    if path.try_exists().map_err(|e| e.to_string())? {
        if parse_manifest(&read_bounded(path)?)? == *manifest {
            return Ok(());
        }
        return Err("已安装的同版本插件文件不一致，未覆盖文件".into());
    }
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&pending, path).map_err(|e| e.to_string())
    })();
    if result.is_err() && pending.exists() {
        let _ = fs::remove_file(pending);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("mewu-plugin-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            // Only this test-created UUID directory; never application data.
            if self.0.parent() == Some(std::env::temp_dir().as_path())
                && self
                    .0
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("mewu-plugin-test-")
            {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }
    fn package(version: &str) -> PluginManifest {
        let mut manifest = parse_manifest(OFFICIAL_TABLE).unwrap();
        manifest.id = "example.table".into();
        manifest.version = version.into();
        manifest.publisher = Some("Example".into());
        manifest
    }
    #[test]
    fn model_connections_validate_typed_protocols_addresses_and_secret_free_options() {
        let bytes = include_bytes!("../../plugins/examples/model-connection/mewu-plugin.json");
        let valid: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        for protocol in ["chat_completions", "anthropic_messages", "openai_responses"] {
            let mut value = valid.clone();
            value["contributions"][0]["template"]["advanced"]["protocol"] = serde_json::json!(protocol);
            value["contributions"][0]["template"]["advanced"]["requestParameters"] = serde_json::json!({"temperature":0.7,"top_p":0.8});
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_ok(), "{protocol}");
        }
        for (path, rejected) in [
            (vec!["protocol"], serde_json::json!("arbitrary")),
            (vec!["authMode"], serde_json::json!("oauth-script")),
            (vec!["requestPath"], serde_json::json!("//remote/messages")),
            (vec!["requestPath"], serde_json::json!("relative/messages")),
            (vec!["requestPath"], serde_json::json!("/../messages")),
            (vec!["requestParameters","temperature"], serde_json::json!("1")),
            (vec!["requestParameters","temperature"], serde_json::json!(2.1)),
            (vec!["requestParameters","top_p"], serde_json::json!(0)),
            (vec!["requestParameters","top_p"], serde_json::json!(1.1)),
            (vec!["requestParameters","service_tier"], serde_json::json!("default")),
            (vec!["requestParameters","messages"], serde_json::json!([])),
            (vec!["headers"], serde_json::json!({"Authorization":"secret"})),
            (vec!["apiKey"], serde_json::json!("secret")),
        ] {
            let mut value = valid.clone();
            let advanced = &mut value["contributions"][0]["template"]["advanced"];
            if path.len() == 1 { advanced[path[0]] = rejected; } else { advanced[path[0]][path[1]] = rejected; }
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err(), "{path:?}");
        }
        for address in ["http://192.0.2.1/v1", "ftp://localhost/v1", "https://user:secret@example.com/v1", "https://example.com/v1?key=secret", "https://example.com/#fragment", "https://", "https://example.com/ bad"] {
            let mut value = valid.clone(); value["contributions"][0]["template"]["baseUrl"] = serde_json::json!(address);
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err(), "{address}");
        }
        for field in ["script", "command", "apiKey", "credentialId", "autoRun", "prompt", "headers"] {
            for layer in ["contribution", "template"] {
                let mut value = valid.clone();
                let target = if layer == "template" { &mut value["contributions"][0]["template"] } else { &mut value["contributions"][0] };
                target[field] = serde_json::json!("injected");
                assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err(), "{layer}.{field}");
            }
        }
        for parameters in [serde_json::json!({"temperature":1.1}), serde_json::json!({"service_tier":"priority"})] {
            let mut value = valid.clone(); value["contributions"][0]["template"]["advanced"]["protocol"] = serde_json::json!("anthropic_messages");
            value["contributions"][0]["template"]["advanced"]["requestParameters"] = parameters;
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }

    #[test]
    fn model_connection_registry_lifecycle_keeps_saved_profiles_and_tombstones() {
        let dir = Directory::new(); let mut store = PluginStore::open(&dir.0).unwrap();
        let presets = store.model_connection_presets().unwrap(); assert_eq!(presets.len(),23);
        assert!(presets.iter().any(|value| value.id == "Anthropic" && value.advanced.protocol == mewu_core::ConnectionProtocol::AnthropicMessages));
        assert!(presets.iter().any(|value| value.id == "OpenAIResponses" && value.advanced.protocol == mewu_core::ConnectionProtocol::OpenAiResponses));
        // Actual core DB, with five saved user profiles and independent secret
        // references, in this owned temporary directory only.
        let profile_path = dir.0.join("space.db");
        let credential_path = dir.0.join("credential.fixture");
        let mut core = mewu_core::Store::open(&profile_path).unwrap();
        let mut expected = core.snapshot();
        // initial_snapshot intentionally calls migrate_legacy: its unconfigured
        // compatibility connection has this exact identity before our fixture
        // creates any user profiles. Keep that original profile unchanged.
        assert_eq!(expected.connections, vec![mewu_core::ConnectionProfile {
            id: mewu_core::LEGACY_CONNECTION_ID.into(), name: "默认连接".into(), provider_id: "custom".into(),
            base_url: "https://api.openai.com/v1".into(), model: String::new(), has_key: false, credential_id: None,
            revision: 1, advanced: mewu_core::ConnectionAdvanced::default(),
        }]);
        for provider in ["OpenAI", "Anthropic", "OpenAIResponses", "Custom", "MiniMax"] {
            let preset = presets.iter().find(|v| v.id == provider).unwrap();
            let advanced = serde_json::from_value(serde_json::to_value(&preset.advanced).unwrap()).unwrap();
            let user_profile = mewu_core::ConnectionProfile {
                id: uuid::Uuid::new_v4().to_string(), name: preset.name.clone(), provider_id: preset.id.clone(),
                base_url: preset.base_url.clone(), model: "synthetic-saved-model".into(), has_key: true,
                credential_id: Some(uuid::Uuid::new_v4().to_string()), revision: 1, advanced,
            };
            expected.connections.push(user_profile.clone());
            core.save_connection_profile(user_profile, None).unwrap();
        }
        let saved = core.snapshot(); assert_eq!(saved, expected); drop(core);
        let profile = fs::read(&profile_path).unwrap(); fs::write(&credential_path,b"immutable synthetic secret").unwrap();
        store.set_enabled("mewu.provider.anthropic",1,false).unwrap();
        assert_eq!(store.model_connection_presets().unwrap().len(),21);
        store.uninstall("mewu.provider.openai",1).unwrap();
        assert_eq!(store.model_connection_presets().unwrap().len(),19);
        let snapshot=store.snapshot().unwrap(); drop(store);
        let mut store=PluginStore::open(&dir.0).unwrap(); assert_eq!(store.snapshot().unwrap(),snapshot);
        assert_eq!(store.model_connection_presets().unwrap().len(),19);
        assert_eq!(fs::read(&profile_path).unwrap(),profile); assert_eq!(fs::read(credential_path).unwrap(),b"immutable synthetic secret");
        assert_eq!(mewu_core::Store::open(&profile_path).unwrap().snapshot(),saved);
        store.set_enabled("mewu.provider.anthropic",2,true).unwrap();
        assert_eq!(store.model_connection_presets().unwrap().len(),21);
        fs::write(path(&dir.0,"mewu.provider.anthropic","1.0.0"),b"{}").unwrap();
        assert_eq!(store.model_connection_presets().unwrap().len(),19);
    }

    #[test]
    fn community_model_templates_have_stable_namespaces_and_updates_only_change_defaults() {
        let dir=Directory::new(); let mut store=PluginStore::open(&dir.0).unwrap();
        let mut manifest=parse_manifest(include_bytes!("../../plugins/examples/model-connection/mewu-plugin.json")).unwrap();
        if let PluginContribution::ModelConnection{template,..}=&mut manifest.contributions[0] { template.provider_id="OpenAI".into(); }
        let bytes=serde_json::to_vec(&manifest).unwrap(); store.install_bytes(&bytes,source(),None).unwrap();
        let first=store.model_connection_presets().unwrap().into_iter().find(|value|value.plugin_id==manifest.id).unwrap();
        assert!(first.id.starts_with("plugin.")); assert_ne!(first.id,"OpenAI"); assert_eq!(first.id.len(),71);
        manifest.version="1.1.0".into();
        if let PluginContribution::ModelConnection{template,..}=&mut manifest.contributions[0] { template.base_url="https://changed.example/v1".into(); }
        store.install_bytes(&serde_json::to_vec(&manifest).unwrap(),source(),Some(1)).unwrap();
        let updated=store.model_connection_presets().unwrap().into_iter().find(|value|value.plugin_id==manifest.id).unwrap();
        assert_eq!(updated.id,first.id); assert_eq!(updated.plugin_revision,2); assert_ne!(updated.base_url,first.base_url);
        manifest.id="example.another-model".into(); store.install_bytes(&serde_json::to_vec(&manifest).unwrap(),source(),None).unwrap();
        let another=store.model_connection_presets().unwrap().into_iter().find(|value|value.plugin_id==manifest.id).unwrap();
        assert_ne!(another.id,first.id);
    }

    #[test]
    fn model_connection_seed_upgrade_only_adds_eighteen_exact_packages_once() {
        let dir=Directory::new(); let store=PluginStore::open(&dir.0).unwrap();
        // Recreate the previous eight-package registry in this owned fixture.
        for manifest in bundled_manifests().into_iter().filter(|value|value.id.starts_with("mewu.provider.")) {
            store.connection.execute("DELETE FROM registry WHERE id=?1",[&manifest.id]).unwrap();
            fs::remove_file(path(&dir.0,&manifest.id,&manifest.version)).unwrap();
            fs::remove_dir(path(&dir.0,&manifest.id,&manifest.version).parent().unwrap()).unwrap();
        }
        store.connection.execute("UPDATE metadata SET revision=8 WHERE slot=1",[]).unwrap();
        let old_rows:Vec<(String,String)>=store.connection.prepare("SELECT id,payload FROM registry ORDER BY id").unwrap().query_map([],|row|Ok((row.get(0)?,row.get(1)?))).unwrap().map(Result::unwrap).collect();
        let old_files:Vec<(PathBuf,Vec<u8>)>=store.records().unwrap().into_iter().map(|record| {let file=path(&dir.0,&record.package.manifest.id,&record.package.manifest.version);let bytes=fs::read(&file).unwrap();(file,bytes)}).collect();
        assert_eq!(old_rows.len(),8); drop(store);
        let store=PluginStore::open(&dir.0).unwrap(); assert_eq!(store.global_revision().unwrap(),26); assert_eq!(store.model_connection_presets().unwrap().len(),23);
        for (id,raw) in old_rows { let current:String=store.connection.query_row("SELECT payload FROM registry WHERE id=?1",[id],|row|row.get(0)).unwrap(); assert_eq!(current,raw); }
        for (file,raw) in old_files { assert_eq!(fs::read(file).unwrap(),raw); }
        let mut canonical = Vec::new();
        for record in store.records().unwrap().into_iter().filter(|value|value.package.manifest.id.starts_with("mewu.provider.")) {
            assert_eq!(record.revision,1);assert_eq!(record.state,PluginState::Enabled);assert_eq!(record.package.source,PluginSource::Official);assert!(record.previous.is_none());
            let manifest = &record.package.manifest;
            let pretty = serde_json::to_vec_pretty(manifest).unwrap();
            assert_eq!(fs::read(path(&dir.0,&manifest.id,&manifest.version)).unwrap(),pretty);
            let raw:String=store.connection.query_row("SELECT payload FROM registry WHERE id=?1",[&manifest.id],|row|row.get(0)).unwrap();
            assert_eq!(raw,json(&record).unwrap());
            canonical.push(serde_json::json!({
                "id": manifest.id, "version": manifest.version,
                "packagePath":format!("plugins/packages/{}/{}/plugin.json",manifest.id,manifest.version),
                "packageBytes":pretty.len(), "packageSha256":format!("{:x}",Sha256::digest(&pretty)),
                "manifestCompactSha256":format!("{:x}",Sha256::digest(serde_json::to_vec(manifest).unwrap())),
                "registryPayloadSha256":format!("{:x}",Sha256::digest(raw.as_bytes())),
            }));
        }
        assert_eq!(canonical.len(),18);
        println!("MEWU_PROVIDER_SEED_CANONICAL={}",serde_json::to_string(&canonical).unwrap());
        let snapshot=store.snapshot().unwrap(); drop(store); assert_eq!(PluginStore::open(&dir.0).unwrap().snapshot().unwrap(),snapshot);
    }

    #[test]
    fn discovery_tags_are_strict_metadata_and_legacy_bytes_stay_canonical() {
        let original = parse_manifest(OFFICIAL_TABLE).unwrap();
        let original_bytes = serde_json::to_vec(&original).unwrap();
        assert!(original.tags.is_empty());
        assert!(!serde_json::to_value(&original).unwrap().as_object().unwrap().contains_key("tags"));
        let mut value = serde_json::to_value(package("1.0.0")).unwrap();
        value["tags"] = serde_json::json!(["vision", "documents", "tools"]);
        let tagged = parse_manifest(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(tagged.tags, vec![PluginTag::Vision, PluginTag::Documents, PluginTag::Tools]);
        assert_eq!(tagged.contributions, original.contributions);
        value["tags"] = serde_json::json!(["vision", "vision"]);
        assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        value["tags"] = serde_json::json!(["execute-shell"]);
        assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        assert_eq!(original_bytes, serde_json::to_vec(&parse_manifest(OFFICIAL_TABLE).unwrap()).unwrap());
    }
    fn source() -> PluginSource {
        PluginSource::Github {
            repository: "example/table".into(),
            commit: "a".repeat(40),
            path: "plugin.json".into(),
        }
    }
    fn install(store: &mut PluginStore, version: &str, expected: Option<u64>) -> PluginRecord {
        store
            .install_bytes(
                &serde_json::to_vec(&package(version)).unwrap(),
                source(),
                expected,
            )
            .unwrap()
    }
    fn path(root: &Path, id: &str, version: &str) -> PathBuf {
        root.join("plugins")
            .join("packages")
            .join(id)
            .join(version)
            .join("plugin.json")
    }

    // Explicit historical fixture behavior. Current product startup must never
    // execute these retired seed/upgrade migrations.
    fn historical_open(root: &Path) -> Result<PluginStore, String> {
        let mut store = PluginStore::open(root)?;
        if store.read("mewu.drawing")?.is_none()
            || store.read("mewu.recording")?.is_some()
            || !store.legacy_recording_rows()?.is_empty()
        {
            store.migrate_recording_bundle()?;
        }
        store.upgrade_canonical_drawing_v1_1_0()?;
        Ok(store)
    }
    fn install_legacy_official(
        store: &mut PluginStore,
        id: &str,
        revision: Option<u64>,
    ) -> PluginRecord {
        let manifest = parse_manifest(match id {
            "mewu.drawing" => OFFICIAL_DRAWING,
            "mewu.recording" => OFFICIAL_RECORDING,
            _ => panic!("Not a retired official fixture"),
        })
        .unwrap();
        store
            .install_package(manifest, PluginSource::Official, revision, true)
            .unwrap()
    }

    #[test]
    fn core_drawing_and_recording_exact_authority_survives_legacy_disable_remove_and_tampering() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        let recording = store.recording_candidate().unwrap().unwrap();
        let gif = store.gif_candidate().unwrap().unwrap();
        let video = VideoDrawingGrant {
            plugin_id: CORE_DRAWING.into(),
            revision: 1,
            contribution_id: "video-drawing-tools".into(),
        };
        for empty in [
            PluginSnapshot {
                revision: 0,
                plugins: vec![],
            },
            store.snapshot().unwrap(),
        ] {
            assert!(recording_grant_is_current(&recording, &empty));
            assert!(gif_grant_is_current(&gif, &empty));
            assert!(video_drawing_grant_is_current(
                &video,
                VideoDrawingTool::Text,
                &empty
            ));
        }
        install_legacy_official(&mut store, "mewu.drawing", None);
        install_legacy_official(&mut store, "mewu.recording", None);
        store.set_enabled("mewu.drawing", 1, false).unwrap();
        store.uninstall("mewu.recording", 1).unwrap();
        fs::write(path(&dir.0, "mewu.drawing", "1.2.0"), b"{}").unwrap();
        let snapshot = store.snapshot().unwrap();
        assert!(recording_grant_is_current(&recording, &snapshot));
        assert!(gif_grant_is_current(&gif, &snapshot));
        assert!(video_drawing_grant_is_current(
            &video,
            VideoDrawingTool::Pen,
            &snapshot
        ));
        assert_eq!(
            store
                .drawing_tools(CORE_DRAWING, 1, "drawing-tools")
                .unwrap()
                .len(),
            9
        );
        assert_eq!(
            store
                .video_drawing_tools(CORE_DRAWING, 1, "video-drawing-tools")
                .unwrap()
                .len(),
            7
        );
        assert!(store.recording(CORE_RECORDING, 1, "record").is_ok());
        assert!(store.recording_audio(CORE_RECORDING, 1, "audio").is_ok());
        assert!(store.video_trim(CORE_RECORDING, 1, "trim").is_ok());
        assert!(store.video_gif(CORE_RECORDING, 1, "gif").is_ok());
        for revision in [0, 2, u64::MAX] {
            assert!(store
                .drawing_tools(CORE_DRAWING, revision, "drawing-tools")
                .is_err());
            assert!(store
                .video_drawing_tools(CORE_DRAWING, revision, "video-drawing-tools")
                .is_err());
            assert!(store.recording(CORE_RECORDING, revision, "record").is_err());
            assert!(store
                .recording_audio(CORE_RECORDING, revision, "audio")
                .is_err());
            assert!(store.video_trim(CORE_RECORDING, revision, "trim").is_err());
            assert!(store.video_gif(CORE_RECORDING, revision, "gif").is_err());
            assert!(!recording_grant_is_current(
                &RecordingGrant {
                    revision,
                    ..recording.clone()
                },
                &snapshot
            ));
            assert!(!gif_grant_is_current(
                &GifGrant {
                    revision,
                    ..gif.clone()
                },
                &snapshot
            ));
            assert!(!video_drawing_grant_is_current(
                &VideoDrawingGrant {
                    revision,
                    ..video.clone()
                },
                VideoDrawingTool::Text,
                &snapshot
            ));
        }
        assert!(store
            .drawing_tools(CORE_DRAWING, 1, "video-drawing-tools")
            .is_err());
        assert!(store
            .video_drawing_tools(CORE_DRAWING, 1, "drawing-tools")
            .is_err());
        for wrong in ["audio", "trim", "gif", "other"] {
            assert!(store.recording(CORE_RECORDING, 1, wrong).is_err());
        }
        for wrong in ["record", "trim", "gif", "other"] {
            assert!(store.recording_audio(CORE_RECORDING, 1, wrong).is_err());
        }
        assert!(store.video_trim(CORE_RECORDING, 1, "gif").is_err());
        assert!(store.video_gif(CORE_RECORDING, 1, "trim").is_err());
        assert!(store
            .drawing_tools("mewu.core.drawing-forged", 1, "drawing-tools")
            .is_err());
        assert!(store
            .recording("mewu.core.recording-forged", 1, "record")
            .is_err());
    }

    #[test]
    fn startup_preserves_every_legacy_record_package_and_preference_without_seed_or_migration() {
        fn files(
            root: &Path,
            relative: &Path,
            result: &mut std::collections::BTreeMap<PathBuf, Option<Vec<u8>>>,
        ) {
            for entry in fs::read_dir(root.join(relative)).unwrap() {
                let entry = entry.unwrap();
                let name = relative.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    result.insert(name.clone(), None);
                    files(root, &name, result);
                } else {
                    result.insert(name.clone(), Some(fs::read(root.join(name)).unwrap()));
                }
            }
        }
        let dir = Directory::new();
        let mut store = legacy_drawing_v11_store(&dir.0, true);
        install_legacy_official(&mut store, "mewu.recording", None);
        for bytes in [OFFICIAL_RECORDING_AUDIO, OFFICIAL_VIDEO_TRIM, OFFICIAL_GIF] {
            store
                .install_package(
                    parse_manifest(bytes).unwrap(),
                    PluginSource::Official,
                    None,
                    true,
                )
                .unwrap();
        }
        store.set_enabled("mewu.recording", 1, false).unwrap();
        store.uninstall("mewu.gif", 1).unwrap();
        let raw: Vec<(String, String)> = store
            .connection
            .prepare("SELECT id,payload FROM registry ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let snapshot = store.snapshot().unwrap();
        let revision = store.global_revision().unwrap();
        assert!(migration_marker(&store).is_none());
        drop(store);
        let mut before = std::collections::BTreeMap::new();
        files(&dir.0, Path::new(""), &mut before);
        let reopened = PluginStore::open(&dir.0).unwrap();
        let actual: Vec<(String, String)> = reopened
            .connection
            .prepare("SELECT id,payload FROM registry ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(actual, raw);
        assert_eq!(reopened.global_revision().unwrap(), revision);
        assert_eq!(reopened.snapshot().unwrap(), snapshot);
        assert!(migration_marker(&reopened).is_none());
        assert_eq!(
            reopened
                .read("mewu.drawing")
                .unwrap()
                .unwrap()
                .package
                .manifest
                .version,
            "1.1.0"
        );
        assert!(reopened.recording(CORE_RECORDING, 1, "record").is_ok());
        assert!(reopened
            .video_drawing_tools(CORE_DRAWING, 1, "video-drawing-tools")
            .is_ok());
        drop(reopened);
        let mut after = std::collections::BTreeMap::new();
        files(&dir.0, Path::new(""), &mut after);
        assert_eq!(after, before);
    }

    /// Recreate the previous release's committed default, including its real
    /// package file. Direct SQL is fixture setup, never a production downgrade.
    fn legacy_drawing_store(root: &Path) -> PluginStore {
        let mut store = PluginStore::open(root).unwrap();
        install_legacy_official(&mut store, "mewu.drawing", None);
        let current = store.read("mewu.drawing").unwrap().unwrap();
        let legacy = StoredRecord {
            package: Package {
                manifest: parse_manifest(OFFICIAL_DRAWING_V1_0_0).unwrap(),
                source: PluginSource::Official,
            },
            revision: 1,
            state: PluginState::Enabled,
            previous: None,
        };
        store.apply_files(Some(&current), Some(&legacy)).unwrap();
        store
            .connection
            .execute(
                "UPDATE registry SET payload=?1 WHERE id='mewu.drawing'",
                [json(&legacy).unwrap()],
            )
            .unwrap();
        store
    }

    fn legacy_drawing_v11_store(root: &Path, with_previous: bool) -> PluginStore {
        let mut store = PluginStore::open(root).unwrap();
        install_legacy_official(&mut store, "mewu.drawing", None);
        let current = store.read("mewu.drawing").unwrap().unwrap();
        let old = StoredRecord {
            package: Package {
                manifest: parse_manifest(OFFICIAL_DRAWING_V1_1_0).unwrap(),
                source: PluginSource::Official,
            },
            revision: if with_previous { 2 } else { 1 },
            state: PluginState::Enabled,
            previous: with_previous.then(|| Package {
                manifest: parse_manifest(OFFICIAL_DRAWING_V1_0_0).unwrap(),
                source: PluginSource::Official,
            }),
        };
        store.apply_files(Some(&current), Some(&old)).unwrap();
        store
            .connection
            .execute(
                "UPDATE registry SET payload=?1 WHERE id='mewu.drawing'",
                [json(&old).unwrap()],
            )
            .unwrap();
        store
    }

    #[test]
    fn core_drawing_has_nine_tools_without_seeding_an_installation() {
        let dir = Directory::new();
        let store = PluginStore::open(&dir.0).unwrap();
        assert!(store.read("mewu.drawing").unwrap().is_none());
        let tools = store
            .drawing_tools(CORE_DRAWING, 1, "drawing-tools")
            .unwrap();
        assert_eq!(tools.len(), 9);
        for tool in [
            DrawingTool::Line,
            DrawingTool::Highlighter,
            DrawingTool::Number,
            DrawingTool::Mosaic,
        ] {
            assert!(tools.contains(&tool));
        }
        assert!(!dir.0.join("plugins/packages/mewu.drawing").exists());
        let snapshot = store.snapshot().unwrap();
        drop(store);
        assert_eq!(
            PluginStore::open(&dir.0).unwrap().snapshot().unwrap(),
            snapshot
        );
    }

    #[test]
    fn canonical_v11_revision1_upgrade_commits_once_and_retains_previous() {
        let dir = Directory::new();
        let store = legacy_drawing_v11_store(&dir.0, false);
        let old = store.read("mewu.drawing").unwrap().unwrap();
        let snapshot = store.snapshot().unwrap();
        drop(store);
        let store = historical_open(&dir.0).unwrap();
        let upgraded = store.read("mewu.drawing").unwrap().unwrap();
        assert_eq!(upgraded.revision, 2);
        assert_eq!(upgraded.state, PluginState::Enabled);
        assert_eq!(
            upgraded.package.manifest,
            parse_manifest(OFFICIAL_DRAWING).unwrap()
        );
        assert_eq!(upgraded.previous, Some(old.package.clone()));
        assert_eq!(store.global_revision().unwrap(), snapshot.revision + 1);
        assert_eq!(
            parse_manifest(&fs::read(path(&dir.0, "mewu.drawing", "1.1.0")).unwrap()).unwrap(),
            old.package.manifest
        );
        assert!(store.check_package(&upgraded.package).is_ok());
        assert!(store
            .drawing_tools("mewu.drawing", 1, "drawing-tools")
            .is_err());
        assert_eq!(
            store
                .drawing_tools("mewu.drawing", 2, "drawing-tools")
                .unwrap()
                .len(),
            9
        );
        for unchanged in snapshot
            .plugins
            .iter()
            .filter(|p| p.manifest.id != "mewu.drawing")
        {
            assert!(store.snapshot().unwrap().plugins.contains(unchanged));
        }
        let snapshot = store.snapshot().unwrap();
        drop(store);
        assert_eq!(
            historical_open(&dir.0).unwrap().snapshot().unwrap(),
            snapshot
        );
    }

    #[test]
    fn drawing_migration_preserves_disabled_removed_and_explicit_version_choices() {
        for choice in [
            "disabled",
            "removed",
            "reenabled",
            "reinstalled",
            "manual-version",
        ] {
            let dir = Directory::new();
            let mut store = legacy_drawing_store(&dir.0);
            match choice {
                "disabled" => {
                    store.set_enabled("mewu.drawing", 1, false).unwrap();
                }
                "removed" => {
                    store.uninstall("mewu.drawing", 1).unwrap();
                }
                "reenabled" => {
                    store.set_enabled("mewu.drawing", 1, false).unwrap();
                    store.set_enabled("mewu.drawing", 2, true).unwrap();
                }
                "reinstalled" => {
                    store.uninstall("mewu.drawing", 1).unwrap();
                    store
                        .install_package(
                            parse_manifest(OFFICIAL_DRAWING_V1_0_0).unwrap(),
                            PluginSource::Official,
                            Some(2),
                            true,
                        )
                        .unwrap();
                }
                "manual-version" => {
                    let mut chosen = parse_manifest(OFFICIAL_DRAWING_V1_0_0).unwrap();
                    chosen.version = "1.0.1".into();
                    store
                        .install_package(chosen, PluginSource::Official, Some(1), true)
                        .unwrap();
                }
                _ => unreachable!(),
            }
            let snapshot = store.snapshot().unwrap();
            drop(store);
            let reopened = PluginStore::open(&dir.0).unwrap();
            assert_eq!(reopened.snapshot().unwrap(), snapshot, "{choice}");
            assert!(!path(&dir.0, "mewu.drawing", "1.1.0").exists(), "{choice}");
        }
    }

    #[test]
    fn drawing_migration_requires_exact_canonical_manifest_and_no_previous_version() {
        for fixture in ["changed-manifest", "previous-version"] {
            let dir = Directory::new();
            let store = legacy_drawing_store(&dir.0);
            let mut record = store.read("mewu.drawing").unwrap().unwrap();
            if fixture == "changed-manifest" {
                record.package.manifest.description = "用户保留的不同清单".into();
                fs::write(
                    path(&dir.0, "mewu.drawing", "1.0.0"),
                    serde_json::to_vec_pretty(&record.package.manifest).unwrap(),
                )
                .unwrap();
            } else {
                let mut previous = record.package.clone();
                previous.manifest.version = "0.9.0".into();
                record.previous = Some(previous);
                store.apply_files(None, Some(&record)).unwrap();
            }
            store
                .connection
                .execute(
                    "UPDATE registry SET payload=?1 WHERE id='mewu.drawing'",
                    [json(&record).unwrap()],
                )
                .unwrap();
            assert_eq!(record.revision, 1);
            let snapshot = store.snapshot().unwrap();
            assert!(snapshot.plugins.iter().all(|p| p.error.is_none()));
            drop(store);
            assert_eq!(
                PluginStore::open(&dir.0).unwrap().snapshot().unwrap(),
                snapshot,
                "{fixture}"
            );
            assert!(!path(&dir.0, "mewu.drawing", "1.1.0").exists());
        }
    }

    #[test]
    fn drawing_migration_does_not_repair_missing_or_changed_package_files() {
        for fixture in ["missing", "invalid-json", "changed-content"] {
            let dir = Directory::new();
            let store = legacy_drawing_store(&dir.0);
            let file = path(&dir.0, "mewu.drawing", "1.0.0");
            match fixture {
                "missing" => fs::remove_file(&file).unwrap(),
                "invalid-json" => fs::write(&file, b"{").unwrap(),
                "changed-content" => {
                    let mut manifest = parse_manifest(OFFICIAL_DRAWING_V1_0_0).unwrap();
                    manifest.name = "different content".into();
                    fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
                }
                _ => unreachable!(),
            }
            let bytes = fs::read(&file).ok();
            let snapshot = store.snapshot().unwrap();
            assert!(snapshot
                .plugins
                .iter()
                .find(|p| p.manifest.id == "mewu.drawing")
                .unwrap()
                .error
                .is_some());
            drop(store);
            let reopened = PluginStore::open(&dir.0).unwrap();
            assert_eq!(reopened.snapshot().unwrap(), snapshot, "{fixture}");
            assert_eq!(fs::read(&file).ok(), bytes);
            assert!(!path(&dir.0, "mewu.drawing", "1.1.0").exists());
        }
    }

    #[test]
    fn rolled_back_drawing_does_not_upgrade_again_on_restart() {
        let dir = Directory::new();
        drop(legacy_drawing_v11_store(&dir.0, false));
        let mut store = historical_open(&dir.0).unwrap();
        let record = store.rollback("mewu.drawing", 2).unwrap();
        assert_eq!(record.manifest.version, "1.1.0");
        assert_eq!(record.revision, 3);
        assert!(record.has_rollback);
        let snapshot = store.snapshot().unwrap();
        drop(store);
        let reopened = historical_open(&dir.0).unwrap();
        assert_eq!(reopened.snapshot().unwrap(), snapshot);
        assert_eq!(
            reopened
                .drawing_tools("mewu.drawing", 3, "drawing-tools")
                .unwrap()
                .len(),
            9
        );
    }

    #[test]
    fn explicit_drawing_upgrade_preserves_disabled_state() {
        let dir = Directory::new();
        let mut store = legacy_drawing_store(&dir.0);
        store.set_enabled("mewu.drawing", 1, false).unwrap();
        let updated = install_legacy_official(&mut store, "mewu.drawing", Some(2));
        assert_eq!(updated.state, PluginState::Disabled);
        assert_eq!(updated.manifest.version, "1.2.0");
        assert_eq!(updated.revision, 3);
        assert!(updated.has_rollback);
        assert!(store
            .drawing_tools("mewu.drawing", 3, "drawing-tools")
            .is_err());
        let snapshot = store.snapshot().unwrap();
        drop(store);
        assert_eq!(
            PluginStore::open(&dir.0).unwrap().snapshot().unwrap(),
            snapshot
        );
    }

    #[test]
    fn drawing_upgrade_database_failure_rolls_back_files_and_can_retry_after_repair() {
        let dir = Directory::new();
        let store = legacy_drawing_v11_store(&dir.0, false);
        let old = store.read("mewu.drawing").unwrap().unwrap();
        let revision = store.global_revision().unwrap();
        store.connection.execute_batch("CREATE TRIGGER fail_drawing_upgrade BEFORE UPDATE ON registry WHEN NEW.id='mewu.drawing' BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        drop(store);
        assert!(historical_open(&dir.0).is_err());
        let connection = Connection::open(dir.0.join("plugins/plugins.db")).unwrap();
        let payload: String = connection
            .query_row(
                "SELECT payload FROM registry WHERE id='mewu.drawing'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(decode_record("mewu.drawing", &payload).unwrap(), old);
        assert_eq!(
            connection
                .query_row("SELECT revision FROM metadata WHERE slot=1", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            revision as i64
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM file_intent", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(!path(&dir.0, "mewu.drawing", "1.2.0").exists());
        assert_eq!(
            parse_manifest(&fs::read(path(&dir.0, "mewu.drawing", "1.1.0")).unwrap()).unwrap(),
            old.package.manifest
        );
        connection
            .execute_batch("DROP TRIGGER fail_drawing_upgrade;")
            .unwrap();
        drop(connection);
        let recovered = historical_open(&dir.0).unwrap();
        assert_eq!(recovered.read("mewu.drawing").unwrap().unwrap().revision, 2);
        assert_eq!(recovered.global_revision().unwrap(), revision + 1);
    }

    #[test]
    fn drawing_manifest_accepts_legacy_external_tools_and_rejects_unknown_or_duplicate_tools() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        let mut legacy = parse_manifest(OFFICIAL_DRAWING_V1_0_0).unwrap();
        legacy.id = "example.drawing".into();
        store
            .install_bytes(&serde_json::to_vec(&legacy).unwrap(), source(), None)
            .unwrap();
        assert_eq!(
            store
                .drawing_tools("example.drawing", 1, "drawing-tools")
                .unwrap(),
            [
                DrawingTool::Pen,
                DrawingTool::Arrow,
                DrawingTool::Rect,
                DrawingTool::Ellipse,
                DrawingTool::Text
            ]
        );
        let mut current = parse_manifest(OFFICIAL_DRAWING).unwrap();
        current.id = legacy.id.clone();
        store
            .install_bytes(&serde_json::to_vec(&current).unwrap(), source(), Some(1))
            .unwrap();
        assert_eq!(
            store
                .drawing_tools("example.drawing", 2, "drawing-tools")
                .unwrap()
                .len(),
            9
        );
        for tools in [
            serde_json::json!([]),
            serde_json::json!(["pen", "pen"]),
            serde_json::json!(["script"]),
            serde_json::json!([
                "pen",
                "line",
                "arrow",
                "rect",
                "ellipse",
                "text",
                "highlighter",
                "number",
                "mosaic",
                "pen"
            ]),
        ] {
            let mut value = serde_json::to_value(&current).unwrap();
            value["contributions"][0]["tools"] = tools;
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }

    #[test]
    fn scroll_capability_is_revision_bound_and_removal_survives_restart() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert!(store.scroll("mewu.scroll", 1, "capture").is_ok());
        assert!(store.scroll("mewu.scroll", 1, "recognize").is_err());
        assert!(store.workflow("mewu.scroll", 1, "capture").is_err());
        store.set_enabled("mewu.scroll", 1, false).unwrap();
        assert!(store.scroll("mewu.scroll", 2, "capture").is_err());
        store.set_enabled("mewu.scroll", 2, true).unwrap();
        assert!(store.scroll("mewu.scroll", 1, "capture").is_err());
        assert!(store.scroll("mewu.scroll", 3, "capture").is_ok());
        store.uninstall("mewu.scroll", 3).unwrap();
        drop(store);
        let store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(
            store.read("mewu.scroll").unwrap().unwrap().state,
            PluginState::Removed
        );
        assert!(store.scroll("mewu.scroll", 4, "capture").is_err());
        let mut value: serde_json::Value = serde_json::from_slice(OFFICIAL_SCROLL).unwrap();
        value["contributions"][0]["command"] = serde_json::json!("program.exe");
        assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    #[test]
    fn translation_is_text_only_revision_bound_and_never_resurrects_removed_package() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert!(store
            .translation("mewu.translation", 1, "translate")
            .is_ok());
        assert!(store.workflow("mewu.translation", 1, "translate").is_err());
        assert!(store.translation("mewu.ocr", 1, "recognize").is_err());
        for name in ["prompt", "tools", "engine", "endpoint"] {
            let mut value: serde_json::Value =
                serde_json::from_slice(OFFICIAL_TRANSLATION).unwrap();
            value["contributions"][0][name] = serde_json::json!("unauthorized");
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        store.set_enabled("mewu.translation", 1, false).unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert!(store
            .translation("mewu.translation", 2, "translate")
            .is_err());
        store.set_enabled("mewu.translation", 2, true).unwrap();
        let package = path(&dir.0, "mewu.translation", "1.0.0");
        let original = fs::read(&package).unwrap();
        fs::write(&package, b"{}").unwrap();
        assert!(store
            .translation("mewu.translation", 3, "translate")
            .is_err());
        fs::write(&package, original).unwrap();
        store.uninstall("mewu.translation", 3).unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert!(!package.exists());
        assert!(store
            .translation("mewu.translation", 4, "translate")
            .is_err());
        store.install_official("mewu.translation", Some(4)).unwrap();
        assert!(store
            .translation("mewu.translation", 5, "translate")
            .is_ok());
    }

    fn legacy_recording_store(dir: &Directory) -> PluginStore {
        let mut store = PluginStore::open(&dir.0).unwrap();
        store
            .connection
            .execute(
                "DELETE FROM preferences WHERE key=?1",
                [RECORDING_MIGRATION_KEY],
            )
            .unwrap();
        for bytes in [OFFICIAL_VIDEO_TRIM, OFFICIAL_RECORDING_AUDIO, OFFICIAL_GIF] {
            store
                .install_package(
                    parse_manifest(bytes).unwrap(),
                    PluginSource::Official,
                    None,
                    true,
                )
                .unwrap();
        }
        store
    }

    fn migration_marker(store: &PluginStore) -> Option<RecordingMigration> {
        let raw: Option<String> = store
            .connection
            .query_row(
                "SELECT value FROM preferences WHERE key=?1",
                [RECORDING_MIGRATION_KEY],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        raw.map(|s| serde_json::from_str(&s).unwrap())
    }

    #[test]
    fn core_recording_has_four_exact_capabilities_without_seeding_a_package() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        let grant = store.recording_candidate().unwrap().unwrap();
        assert_eq!(grant.plugin_id, CORE_RECORDING);
        assert_eq!(store.snapshot().unwrap().plugins.len(), BUNDLED.len());
        assert!(migration_marker(&store).is_none());
        assert!(store.legacy_recording_rows().unwrap().is_empty());
        assert_eq!(
            store
                .recording(&grant.plugin_id, grant.revision, &grant.contribution_id)
                .unwrap(),
            RecordingEngine::WindowsWgcMf
        );
        assert!(recording_grant_is_current(
            &grant,
            &store.snapshot().unwrap()
        ));
        assert!(store.read("mewu.recording").unwrap().is_none());
        assert!(!dir.0.join("plugins/packages/mewu.recording").exists());
        for engine in [
            serde_json::json!("windows"),
            serde_json::json!({"windows.wgc-mf":null}),
            serde_json::Value::Null,
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(OFFICIAL_RECORDING).unwrap();
            value["contributions"][0]["engine"] = engine;
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        for field in ["autoStart", "microphone", "script", "path"] {
            let mut value: serde_json::Value = serde_json::from_slice(OFFICIAL_RECORDING).unwrap();
            value["contributions"][0][field] = serde_json::json!(true);
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        install_legacy_official(&mut store, "mewu.recording", None);
        store.set_enabled("mewu.recording", 1, false).unwrap();
        assert_eq!(store.recording_candidate().unwrap(), Some(grant.clone()));
        assert!(recording_grant_is_current(
            &grant,
            &store.snapshot().unwrap()
        ));
        for old in LEGACY_RECORDING_IDS {
            assert!(store.install_official(old, None).is_err());
        }
    }

    #[test]
    fn recording_migration_archives_exact_payloads_and_leaves_old_files_preferences_and_other_rows()
    {
        let dir = Directory::new();
        let mut store = legacy_recording_store(&dir);
        store
            .set_catalog_source(Some("https://example.test/catalog.json"))
            .unwrap();
        store.uninstall("mewu.voice", 1).unwrap();
        let raw = format!(
            "\n {} \n",
            serde_json::to_string_pretty(&store.read("mewu.gif").unwrap().unwrap()).unwrap()
        );
        store
            .connection
            .execute("UPDATE registry SET payload=?1 WHERE id='mewu.gif'", [&raw])
            .unwrap();
        let rows = store.legacy_recording_rows().unwrap();
        let others: Vec<_> = store
            .records()
            .unwrap()
            .into_iter()
            .filter(|r| !LEGACY_RECORDING_IDS.contains(&r.package.manifest.id.as_str()))
            .collect();
        let files: Vec<_> = rows
            .iter()
            .map(|r| {
                let file = path(&dir.0, &r.id, "1.0.0");
                (file.clone(), fs::read(file).unwrap())
            })
            .collect();
        let before = store.global_revision().unwrap();
        drop(store);
        let store = historical_open(&dir.0).unwrap();
        assert_eq!(store.global_revision().unwrap(), before + 1);
        assert_eq!(migration_marker(&store).unwrap().records, rows);
        assert!(store.legacy_recording_rows().unwrap().is_empty());
        assert_eq!(
            store.read("mewu.recording").unwrap().unwrap().state,
            PluginState::Enabled
        );
        for row in others {
            assert_eq!(store.read(&row.package.manifest.id).unwrap().unwrap(), row);
        }
        for (file, bytes) in files {
            assert_eq!(fs::read(file).unwrap(), bytes);
        }
        assert_eq!(
            store.get_catalog_source().unwrap().as_deref(),
            Some("https://example.test/catalog.json")
        );
        assert!(!dir.0.join("recording-preferences.json").exists());
        let snapshot = store.snapshot().unwrap();
        drop(store);
        assert_eq!(
            historical_open(&dir.0).unwrap().snapshot().unwrap(),
            snapshot
        );
    }

    #[test]
    fn recording_migration_preserves_removed_disabled_partial_and_noncanonical_choices() {
        for case in [
            "removed", "disabled", "partial", "missing", "tampered", "changed",
        ] {
            let dir = Directory::new();
            let mut store = legacy_recording_store(&dir);
            match case {
                "removed" => {
                    store.uninstall("mewu.gif", 1).unwrap();
                    store.set_enabled("mewu.recording-audio", 1, false).unwrap();
                }
                "disabled" => {
                    store.set_enabled("mewu.recording-audio", 1, false).unwrap();
                }
                "partial" => {
                    store
                        .connection
                        .execute("DELETE FROM registry WHERE id='mewu.video-trim'", [])
                        .unwrap();
                }
                "missing" => {
                    fs::remove_file(path(&dir.0, "mewu.gif", "1.0.0")).unwrap();
                }
                "tampered" => {
                    fs::write(path(&dir.0, "mewu.gif", "1.0.0"), b"{}").unwrap();
                }
                "changed" => {
                    let mut record = store.read("mewu.gif").unwrap().unwrap();
                    record.package.manifest.description = "custom official variant".into();
                    store
                        .connection
                        .execute(
                            "UPDATE registry SET payload=?1 WHERE id='mewu.gif'",
                            [json(&record).unwrap()],
                        )
                        .unwrap();
                    fs::write(
                        path(&dir.0, "mewu.gif", "1.0.0"),
                        json(&record.package.manifest).unwrap(),
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            let archived = store.legacy_recording_rows().unwrap();
            drop(store);
            let store = historical_open(&dir.0).unwrap();
            let expected = if case == "removed" {
                PluginState::Removed
            } else {
                PluginState::Disabled
            };
            assert_eq!(
                store.read("mewu.recording").unwrap().unwrap().state,
                expected,
                "{case}"
            );
            assert_eq!(migration_marker(&store).unwrap().records, archived);
            assert!(store.recording("mewu.recording", 1, "record").is_err());
            let snapshot = store.snapshot().unwrap();
            drop(store);
            let mut store = historical_open(&dir.0).unwrap();
            assert_eq!(store.snapshot().unwrap(), snapshot);
            // An explicit post-migration choice must not be recomputed from the archive.
            if expected == PluginState::Removed {
                install_legacy_official(&mut store, "mewu.recording", Some(1));
            } else {
                store.set_enabled("mewu.recording", 1, true).unwrap();
            }
            let selected = store.snapshot().unwrap();
            drop(store);
            assert_eq!(
                historical_open(&dir.0).unwrap().snapshot().unwrap(),
                selected
            );
        }
    }

    #[test]
    fn recording_migration_database_failure_rolls_back_all_rows_marker_files_and_revision() {
        let dir = Directory::new();
        let mut store = legacy_recording_store(&dir);
        let before = store.snapshot().unwrap();
        let rows = store.legacy_recording_rows().unwrap();
        store.connection.execute_batch("CREATE TRIGGER fail_recording BEFORE INSERT ON registry WHEN NEW.id='mewu.recording' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(store.migrate_recording_bundle().is_err());
        assert_eq!(store.snapshot().unwrap(), before);
        assert_eq!(store.legacy_recording_rows().unwrap(), rows);
        assert!(migration_marker(&store).is_none());
        assert!(!path(&dir.0, "mewu.recording", "1.0.0").exists());
        for row in rows {
            assert!(path(&dir.0, &row.id, "1.0.0").is_file());
        }
        store
            .connection
            .execute_batch("DROP TRIGGER fail_recording;")
            .unwrap();
        store.migrate_recording_bundle().unwrap();
        assert_eq!(store.global_revision().unwrap(), before.revision + 1);
    }

    #[test]
    fn interrupted_recording_migration_recovers_then_commits_once() {
        let dir = Directory::new();
        let store = legacy_recording_store(&dir);
        let before = store.global_revision().unwrap();
        let migration = RecordingMigration {
            version: 1,
            records: store.legacy_recording_rows().unwrap(),
        };
        let after = StoredRecord {
            package: Package {
                manifest: parse_manifest(OFFICIAL_RECORDING).unwrap(),
                source: PluginSource::Official,
            },
            revision: 1,
            state: PluginState::Enabled,
            previous: None,
        };
        let intent = FileIntent {
            before: None,
            after: after.clone(),
            recording_migration: Some(migration.clone()),
        };
        store
            .connection
            .execute(
                "INSERT INTO file_intent VALUES(1,?1)",
                [json(&intent).unwrap()],
            )
            .unwrap();
        store.apply_files(None, Some(&after)).unwrap();
        assert!(store.snapshot().is_err());
        drop(store);
        let store = historical_open(&dir.0).unwrap();
        assert_eq!(store.global_revision().unwrap(), before + 1);
        assert_eq!(migration_marker(&store).unwrap(), migration);
        assert!(store.recording_candidate().unwrap().is_some());
        let snapshot = store.snapshot().unwrap();
        drop(store);
        assert_eq!(
            historical_open(&dir.0).unwrap().snapshot().unwrap(),
            snapshot
        );
    }

    #[test]
    fn corrupt_registry_and_missing_migrated_row_fail_without_reseeding() {
        for invalid in ["{}", "{broken", "foreign"] {
            let dir = Directory::new();
            let store = legacy_recording_store(&dir);
            let payload = if invalid == "foreign" {
                let mut record = store.read("mewu.gif").unwrap().unwrap();
                record.package.source = source();
                json(&record).unwrap()
            } else {
                invalid.to_owned()
            };
            let old_file = fs::read(path(&dir.0, "mewu.gif", "1.0.0")).unwrap();
            store
                .connection
                .execute(
                    "UPDATE registry SET payload=?1 WHERE id='mewu.gif'",
                    [&payload],
                )
                .unwrap();
            let revision = store.global_revision().unwrap();
            drop(store);
            assert!(historical_open(&dir.0).is_err());
            let db = Connection::open(dir.0.join("plugins/plugins.db")).unwrap();
            let actual: String = db
                .query_row(
                    "SELECT payload FROM registry WHERE id='mewu.gif'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(actual, payload);
            assert_eq!(
                fs::read(path(&dir.0, "mewu.gif", "1.0.0")).unwrap(),
                old_file
            );
            assert_eq!(
                db.query_row("SELECT revision FROM metadata", [], |r| r.get::<_, i64>(0))
                    .unwrap() as u64,
                revision
            );
            assert!(!path(&dir.0, "mewu.recording", "1.0.0").exists());
        }
        let dir = Directory::new();
        let store = historical_open(&dir.0).unwrap();
        store
            .connection
            .execute("DELETE FROM registry WHERE id='mewu.recording'", [])
            .unwrap();
        drop(store);
        assert!(historical_open(&dir.0).is_err());
    }

    #[test]
    fn completed_recording_migration_does_not_override_a_later_version_or_rollback() {
        let dir = Directory::new();
        let mut store = historical_open(&dir.0).unwrap();
        let mut manifest = parse_manifest(OFFICIAL_RECORDING).unwrap();
        manifest.version = "1.1.0".into();
        manifest.description = "new official version".into();
        store
            .install_package(manifest, PluginSource::Official, Some(1), true)
            .unwrap();
        let snapshot = store.snapshot().unwrap();
        drop(store);
        let mut store = historical_open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), snapshot);
        store.rollback("mewu.recording", 2).unwrap();
        let rolled_back = store.snapshot().unwrap();
        drop(store);
        assert_eq!(
            historical_open(&dir.0).unwrap().snapshot().unwrap(),
            rolled_back
        );
    }

    #[test]
    fn gif_declaration_is_strict_and_carries_no_export_settings() {
        let manifest = parse_manifest(OFFICIAL_GIF).unwrap();
        assert_eq!(manifest.id, "mewu.gif");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(
            manifest.contributions,
            vec![PluginContribution::ArtifactVideoGif {
                id: "gif".into(),
                title: "GIF 导出".into(),
                engine: VideoGifEngine::WindowsMediaEditingGif,
            }]
        );
        for engine in [
            serde_json::json!("ffmpeg"),
            serde_json::json!("windows.media-editing"),
            serde_json::json!({"windows.media-editing-gif": null}),
            serde_json::json!(["windows.media-editing-gif"]),
            serde_json::json!(true),
            serde_json::Value::Null,
        ] {
            let mut value = serde_json::to_value(&manifest).unwrap();
            value["contributions"][0]["engine"] = engine;
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        for field in [
            "script",
            "command",
            "path",
            "endpoint",
            "codec",
            "fps",
            "range",
            "loop",
            "autoExport",
            "permissions",
        ] {
            let mut value = serde_json::to_value(&manifest).unwrap();
            value["contributions"][0][field] = serde_json::json!(true);
            assert!(
                parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{field}"
            );
        }
        assert!(serde_json::from_slice::<PluginContribution>(br#"{"kind":"artifact.video-gif","id":"gif","title":"GIF","engine":"windows.media-editing-gif","engine":"windows.media-editing-gif"}"#).is_err());
        let grant = GifGrant {
            plugin_id: "mewu.gif".into(),
            revision: 1,
            contribution_id: "gif".into(),
        };
        let mut value = serde_json::to_value(&grant).unwrap();
        value["autoExport"] = serde_json::json!(true);
        assert!(serde_json::from_value::<GifGrant>(value).is_err());
    }

    #[test]
    fn gif_candidate_and_exact_grant_reject_bad_kind_revision_or_package() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        install_legacy_official(&mut store, "mewu.recording", None);
        let grant = GifGrant {
            plugin_id: "mewu.recording".into(),
            revision: 1,
            contribution_id: "gif".into(),
        };
        assert_eq!(
            grant,
            GifGrant {
                plugin_id: "mewu.recording".into(),
                revision: 1,
                contribution_id: "gif".into()
            }
        );
        assert_eq!(
            store
                .video_gif(&grant.plugin_id, grant.revision, &grant.contribution_id)
                .unwrap(),
            VideoGifEngine::WindowsMediaEditingGif
        );
        assert!(gif_grant_is_current(&grant, &store.snapshot().unwrap()));
        assert!(store.video_gif("mewu.recording", 0, "gif").is_err());
        assert!(store.video_gif("mewu.recording", 1, "trim").is_err());
        assert!(store.video_gif("mewu.recording", 1, "trim").is_err());
        assert!(store.video_trim("mewu.recording", 1, "gif").is_err());
        assert!(store.workflow("mewu.recording", 1, "gif").is_err());
        let original = fs::read(path(&dir.0, "mewu.recording", "1.0.0")).unwrap();
        let mut manifest = parse_manifest(&original).unwrap();
        manifest.description = "changed".into();
        fs::write(
            path(&dir.0, "mewu.recording", "1.0.0"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert_eq!(
            store.gif_candidate().unwrap().unwrap().plugin_id,
            CORE_RECORDING
        );
        assert!(store.video_gif("mewu.recording", 1, "gif").is_err());
        assert!(!gif_grant_is_current(&grant, &store.snapshot().unwrap()));
        fs::remove_file(path(&dir.0, "mewu.recording", "1.0.0")).unwrap();
        assert_eq!(
            store.gif_candidate().unwrap().unwrap().plugin_id,
            CORE_RECORDING
        );
        assert!(store.video_gif("mewu.recording", 1, "gif").is_err());
        fs::write(path(&dir.0, "mewu.recording", "1.0.0"), original).unwrap();
        assert!(gif_grant_is_current(&grant, &store.snapshot().unwrap()));
    }

    #[test]
    fn gif_disable_uninstall_and_reinstall_do_not_revive_old_grants() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        install_legacy_official(&mut store, "mewu.recording", None);
        let original = GifGrant {
            plugin_id: "mewu.recording".into(),
            revision: 1,
            contribution_id: "gif".into(),
        };
        store.set_enabled("mewu.recording", 1, false).unwrap();
        let disabled = store.snapshot().unwrap();
        assert!(!gif_grant_is_current(&original, &disabled));
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), disabled);
        assert_eq!(
            store.gif_candidate().unwrap().unwrap().plugin_id,
            CORE_RECORDING
        );
        store.set_enabled("mewu.recording", 2, true).unwrap();
        let enabled = GifGrant {
            plugin_id: "mewu.recording".into(),
            revision: 3,
            contribution_id: "gif".into(),
        };
        assert_eq!(enabled.revision, 3);
        assert!(!gif_grant_is_current(&original, &store.snapshot().unwrap()));
        store.uninstall("mewu.recording", 3).unwrap();
        let removed = store.snapshot().unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), removed);
        assert!(!path(&dir.0, "mewu.recording", "1.0.0").exists());
        assert_eq!(
            store.gif_candidate().unwrap().unwrap().plugin_id,
            CORE_RECORDING
        );
        install_legacy_official(&mut store, "mewu.recording", Some(4));
        assert!(store.video_gif("mewu.recording", 3, "gif").is_err());
        assert!(!gif_grant_is_current(&enabled, &store.snapshot().unwrap()));
        assert_eq!(store.gif_candidate().unwrap().unwrap().revision, 1);
        // All capabilities use the same package revision.
        assert!(store.video_trim("mewu.recording", 1, "trim").is_err());
        assert!(store.video_trim("mewu.recording", 5, "trim").is_ok());
    }

    #[test]
    fn gif_core_candidate_stays_available_when_legacy_or_community_packages_change() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        install_legacy_official(&mut store, "mewu.recording", None);
        for id in ["z.gif", "a.gif"] {
            let mut manifest = parse_manifest(OFFICIAL_GIF).unwrap();
            manifest.id = id.into();
            let input = dir.0.join(format!("{id}.json"));
            fs::write(&input, serde_json::to_vec(&manifest).unwrap()).unwrap();
            store.install_local(&input, None).unwrap();
        }
        assert_eq!(
            store.gif_candidate().unwrap().unwrap().plugin_id,
            CORE_RECORDING
        );
        store.set_enabled("mewu.recording", 1, false).unwrap();
        assert_eq!(
            store.gif_candidate().unwrap().unwrap().plugin_id,
            CORE_RECORDING
        );
        fs::write(path(&dir.0, "a.gif", "1.0.0"), b"{}").unwrap();
        assert_eq!(
            store.gif_candidate().unwrap().unwrap().plugin_id,
            CORE_RECORDING
        );
        let current = store.gif_candidate().unwrap().unwrap();
        let snapshot = store.snapshot().unwrap();
        for invalid in [
            GifGrant {
                revision: 0,
                ..current.clone()
            },
            GifGrant {
                revision: current.revision + 1,
                ..current.clone()
            },
            GifGrant {
                contribution_id: "other".into(),
                ..current.clone()
            },
            GifGrant {
                plugin_id: "missing".into(),
                ..current.clone()
            },
            GifGrant {
                plugin_id: "mewu.recording".into(),
                contribution_id: "trim".into(),
                ..current.clone()
            },
        ] {
            assert!(!gif_grant_is_current(&invalid, &snapshot));
        }
    }

    #[test]
    fn recording_audio_declaration_never_carries_device_or_autostart_authority() {
        let parsed = parse_manifest(OFFICIAL_RECORDING_AUDIO).unwrap();
        assert_eq!(parsed.id, "mewu.recording-audio");
        assert_eq!(
            parsed.contributions,
            vec![PluginContribution::RecordingAudio {
                id: "audio".into(),
                title: "录屏声音".into(),
                engine: RecordingAudioEngine::WindowsWasapi
            }]
        );
        for engine in [
            serde_json::json!("wasapi"),
            serde_json::json!("ffmpeg"),
            serde_json::json!({"windows.wasapi":null}),
            serde_json::json!(["windows.wasapi"]),
            serde_json::Value::Null,
        ] {
            let mut value = serde_json::to_value(&parsed).unwrap();
            value["contributions"][0]["engine"] = engine;
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        for field in [
            "autoStart",
            "microphone",
            "deviceId",
            "script",
            "permissions",
            "endpoint",
        ] {
            let mut value = serde_json::to_value(&parsed).unwrap();
            value["contributions"][0][field] = serde_json::json!(true);
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        assert!(serde_json::from_slice::<PluginContribution>(br#"{"kind":"recording.audio","id":"audio","title":"Audio","engine":"windows.wasapi","engine":"windows.wasapi"}"#).is_err());
    }

    #[test]
    fn recording_audio_grant_never_survives_disable_reinstall_or_package_tampering() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        install_legacy_official(&mut store, "mewu.recording", None);
        assert_eq!(
            store.recording_audio("mewu.recording", 1, "audio").unwrap(),
            RecordingAudioEngine::WindowsWasapi
        );
        assert!(store
            .recording_audio("mewu.recording", 1, "dictation")
            .is_err());
        store.set_enabled("mewu.recording", 1, false).unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert!(store.recording_audio("mewu.recording", 1, "audio").is_err());
        store.set_enabled("mewu.recording", 2, true).unwrap();
        assert!(store.recording_audio("mewu.recording", 1, "audio").is_err());
        assert!(store.recording_audio("mewu.recording", 3, "audio").is_ok());
        store.uninstall("mewu.recording", 3).unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert!(!path(&dir.0, "mewu.recording", "1.0.0").exists());
        install_legacy_official(&mut store, "mewu.recording", Some(4));
        assert!(store.recording_audio("mewu.recording", 3, "audio").is_err());
        assert!(store.recording_audio("mewu.recording", 5, "audio").is_ok());
        fs::write(path(&dir.0, "mewu.recording", "1.0.0"), b"{}").unwrap();
        assert!(store.recording_audio("mewu.recording", 5, "audio").is_err());
        let snapshot = store.snapshot().unwrap();
        assert!(snapshot
            .plugins
            .iter()
            .find(|p| p.manifest.id == "mewu.recording")
            .unwrap()
            .error
            .is_some());
    }

    #[test]
    fn video_trim_declaration_is_a_strict_fixed_host_capability() {
        let parsed = parse_manifest(OFFICIAL_VIDEO_TRIM).unwrap();
        assert_eq!(parsed.id, "mewu.video-trim");
        assert_eq!(parsed.version, "1.0.0");
        assert_eq!(
            parsed.contributions,
            vec![PluginContribution::ArtifactVideoTrim {
                id: "trim".into(),
                title: "视频裁剪".into(),
                engine: VideoTrimEngine::WindowsMediaEditing,
            }]
        );
        for engine in [
            serde_json::json!("ffmpeg"),
            serde_json::json!("WindowsMediaEditing"),
            serde_json::json!({"windows.media-editing": null}),
            serde_json::json!(["windows.media-editing"]),
            serde_json::json!(null),
            serde_json::json!(true),
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(OFFICIAL_VIDEO_TRIM).unwrap();
            value["contributions"][0]["engine"] = engine;
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        for field in [
            "script",
            "command",
            "path",
            "endpoint",
            "codec",
            "range",
            "autoExport",
            "permissions",
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(OFFICIAL_VIDEO_TRIM).unwrap();
            value["contributions"][0][field] = serde_json::json!("forbidden");
            assert!(
                parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{field}"
            );
        }
        let duplicate=br#"{"kind":"artifact.video-trim","id":"trim","title":"Video","engine":"windows.media-editing","engine":"windows.media-editing"}"#;
        assert!(serde_json::from_slice::<PluginContribution>(duplicate).is_err());
    }

    #[test]
    fn video_trim_grant_requires_matching_kind_revision_and_untampered_package() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        install_legacy_official(&mut store, "mewu.recording", None);
        assert!(matches!(
            store.video_trim("mewu.recording", 1, "trim").unwrap(),
            PluginContribution::ArtifactVideoTrim {
                engine: VideoTrimEngine::WindowsMediaEditing,
                ..
            }
        ));
        assert!(store.video_trim("mewu.recording", 0, "trim").is_err());
        assert!(store.video_trim("mewu.recording", 1, "other").is_err());
        assert!(store.video_trim("mewu.codes", 1, "recognize").is_err());
        assert!(store.workflow("mewu.recording", 1, "trim").is_err());
        let file = path(&dir.0, "mewu.recording", "1.0.0");
        let original = fs::read(&file).unwrap();
        let mut manifest = parse_manifest(&original).unwrap();
        manifest.description = "changed".into();
        fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(store.video_trim("mewu.recording", 1, "trim").is_err());
        fs::remove_file(&file).unwrap();
        assert!(store.video_trim("mewu.recording", 1, "trim").is_err());
        fs::write(&file, original).unwrap();
        assert!(store.video_trim("mewu.recording", 1, "trim").is_ok());
    }

    #[test]
    fn video_trim_disabled_removed_and_explicit_reinstall_choices_are_durable() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        install_legacy_official(&mut store, "mewu.recording", None);
        store.set_enabled("mewu.recording", 1, false).unwrap();
        let disabled = store.snapshot().unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), disabled);
        assert!(store.video_trim("mewu.recording", 2, "trim").is_err());
        store.uninstall("mewu.recording", 2).unwrap();
        let removed = store.snapshot().unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), removed);
        assert!(!path(&dir.0, "mewu.recording", "1.0.0").exists());
        install_legacy_official(&mut store, "mewu.recording", Some(3));
        assert!(store.video_trim("mewu.recording", 3, "trim").is_err());
        assert!(store.video_trim("mewu.recording", 4, "trim").is_ok());
    }

    #[test]
    fn code_declaration_is_fixed_local_engine_without_extra_authority() {
        let parsed = parse_manifest(OFFICIAL_CODES).unwrap();
        assert_eq!(parsed.id, "mewu.codes");
        assert_eq!(parsed.version, "1.0.0");
        assert_eq!(
            parsed.contributions,
            vec![PluginContribution::Codes {
                id: "recognize".into(),
                title: "二维码与条码".into(),
                engine: CodeEngine::Rxing
            }]
        );
        for engine in [
            serde_json::json!("remote"),
            serde_json::json!("Rxing"),
            serde_json::json!({"rxing":null}),
            serde_json::json!(["rxing"]),
            serde_json::json!(null),
            serde_json::json!(true),
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(OFFICIAL_CODES).unwrap();
            value["contributions"][0]["engine"] = engine;
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        for field in [
            "url",
            "endpoint",
            "command",
            "script",
            "prompt",
            "tools",
            "formats",
            "autoOpen",
            "permissions",
            "model",
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(OFFICIAL_CODES).unwrap();
            value["contributions"][0][field] = serde_json::json!("unauthorized");
            assert!(
                parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{field}"
            );
        }
        let raw=br#"{"kind":"selection.codes","id":"recognize","title":"Codes","engine":"rxing","engine":"rxing"}"#;
        assert!(serde_json::from_slice::<PluginContribution>(raw).is_err());
    }

    #[test]
    fn code_authority_checks_exact_package_revision_and_kind() {
        let dir = Directory::new();
        let store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(
            store.codes("mewu.codes", 1, "recognize").unwrap(),
            CodeEngine::Rxing
        );
        assert!(store.codes("mewu.codes", 0, "recognize").is_err());
        assert!(store.codes("mewu.codes", 1, "dictation").is_err());
        assert!(store.codes("mewu.voice", 1, "dictation").is_err());
        assert!(store.workflow("mewu.codes", 1, "recognize").is_err());
        assert!(store.ocr("mewu.codes", 1, "recognize").is_err());
        let file = path(&dir.0, "mewu.codes", "1.0.0");
        let original = fs::read(&file).unwrap();
        let mut manifest = parse_manifest(&original).unwrap();
        manifest.description = "modified".into();
        fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(store.codes("mewu.codes", 1, "recognize").is_err());
        assert!(store
            .snapshot()
            .unwrap()
            .plugins
            .iter()
            .find(|p| p.manifest.id == "mewu.codes")
            .unwrap()
            .error
            .is_some());
        fs::remove_file(&file).unwrap();
        assert!(store.codes("mewu.codes", 1, "recognize").is_err());
        fs::write(&file, original).unwrap();
        assert!(store.codes("mewu.codes", 1, "recognize").is_ok());
    }

    #[test]
    fn code_disabled_and_removed_choices_survive_reopen_until_explicit_reinstall() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        store.set_enabled("mewu.codes", 1, false).unwrap();
        let disabled = store.snapshot().unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), disabled);
        assert!(store.codes("mewu.codes", 2, "recognize").is_err());
        store.set_enabled("mewu.codes", 2, true).unwrap();
        store.uninstall("mewu.codes", 3).unwrap();
        let removed = store.snapshot().unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), removed);
        assert!(!path(&dir.0, "mewu.codes", "1.0.0").exists());
        store.install_official("mewu.codes", Some(4)).unwrap();
        assert!(store.codes("mewu.codes", 4, "recognize").is_err());
        assert!(store.codes("mewu.codes", 5, "recognize").is_ok());
    }

    #[test]
    fn code_seed_preserves_existing_packages_and_only_adds_one_revision() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        store.set_enabled("mewu.ocr", 1, false).unwrap();
        store.uninstall("mewu.table", 1).unwrap();
        store
            .connection
            .execute("DELETE FROM registry WHERE id='mewu.codes'", [])
            .unwrap();
        fs::remove_file(path(&dir.0, "mewu.codes", "1.0.0")).unwrap();
        store
            .set_catalog_source(Some("https://example.test/catalog.json"))
            .unwrap();
        let old = store.snapshot().unwrap();
        assert_eq!(old.plugins.len(), BUNDLED.len() - 1);
        let files: Vec<_> = old
            .plugins
            .iter()
            .filter(|p| p.state != PluginState::Removed)
            .map(|p| {
                let file = path(&dir.0, &p.manifest.id, &p.manifest.version);
                (file.clone(), fs::read(&file).unwrap())
            })
            .collect();
        drop(store);
        let store = PluginStore::open(&dir.0).unwrap();
        let seeded = store.snapshot().unwrap();
        assert_eq!(seeded.revision, old.revision + 1);
        let mut previous = seeded.plugins.clone();
        previous.retain(|p| p.manifest.id != "mewu.codes");
        assert_eq!(previous, old.plugins);
        assert_eq!(
            store.get_catalog_source().unwrap().as_deref(),
            Some("https://example.test/catalog.json")
        );
        for (file, bytes) in files {
            assert_eq!(fs::read(file).unwrap(), bytes);
        }
        drop(store);
        let store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), seeded);
    }

    #[test]
    fn speech_declaration_accepts_only_fixed_engine_and_no_extra_authority() {
        let parsed = parse_manifest(OFFICIAL_VOICE).unwrap();
        assert_eq!(parsed.id, "mewu.voice");
        assert_eq!(parsed.version, "1.0.0");
        assert_eq!(
            parsed.contributions,
            vec![PluginContribution::InputSpeechToText {
                id: "dictation".into(),
                title: "语音输入".into(),
                engine: SpeechEngine::WindowsSapi,
            }]
        );
        assert_eq!(
            serde_json::to_value(&parsed.contributions[0]).unwrap()["engine"],
            "windows.sapi"
        );
        for field in [
            "url",
            "endpoint",
            "key",
            "deviceId",
            "audioPath",
            "command",
            "prompt",
            "script",
            "tools",
            "autoStart",
            "language",
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(OFFICIAL_VOICE).unwrap();
            value["contributions"][0][field] = serde_json::json!("forbidden");
            assert!(
                parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{field}"
            );
        }
        for engine in [
            "windows",
            "WindowsSapi",
            "windows.winrt",
            "azure",
            "whisper",
            "",
            "windows.sapi\u{0}",
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(OFFICIAL_VOICE).unwrap();
            value["contributions"][0]["engine"] = serde_json::json!(engine);
            assert!(
                parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{engine}"
            );
        }
        let mut missing: serde_json::Value = serde_json::from_slice(OFFICIAL_VOICE).unwrap();
        missing["contributions"][0]
            .as_object_mut()
            .unwrap()
            .remove("engine");
        assert!(parse_manifest(&serde_json::to_vec(&missing).unwrap()).is_err());
        for engine in [
            serde_json::json!({"windows.sapi": null}),
            serde_json::json!(["windows.sapi"]),
            serde_json::json!(null),
            serde_json::json!(true),
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(OFFICIAL_VOICE).unwrap();
            value["contributions"][0]["engine"] = engine;
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        // Deliberately raw duplicate keys; do not parse as Value first.
        let duplicate = br#"{"kind":"input.speech-to-text","id":"dictation","title":"Voice","engine":"windows.sapi","engine":"windows.sapi"}"#;
        assert!(serde_json::from_slice::<PluginContribution>(duplicate).is_err());
    }

    #[test]
    fn speech_authority_checks_revision_kind_and_exact_installed_package() {
        let dir = Directory::new();
        let store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(
            store.speech_to_text("mewu.voice", 1, "dictation").unwrap(),
            SpeechEngine::WindowsSapi
        );
        assert!(store.speech_to_text("mewu.voice", 0, "dictation").is_err());
        assert!(store.speech_to_text("mewu.voice", 1, "other").is_err());
        assert!(store.speech_to_text("mewu.ocr", 1, "recognize").is_err());
        assert!(store.workflow("mewu.voice", 1, "dictation").is_err());
        assert!(store.ocr("mewu.voice", 1, "dictation").is_err());
        assert!(store.memory_provider("mewu.voice", 1, "dictation").is_err());
        let package = path(&dir.0, "mewu.voice", "1.0.0");
        let original = fs::read(&package).unwrap();
        let mut changed = parse_manifest(&original).unwrap();
        changed.description = "Changed on disk".into();
        fs::write(&package, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(store.speech_to_text("mewu.voice", 1, "dictation").is_err());
        assert!(store
            .snapshot()
            .unwrap()
            .plugins
            .iter()
            .find(|p| p.manifest.id == "mewu.voice")
            .unwrap()
            .error
            .is_some());
        fs::remove_file(&package).unwrap();
        assert!(store.speech_to_text("mewu.voice", 1, "dictation").is_err());
        fs::write(&package, original).unwrap();
        assert!(store.speech_to_text("mewu.voice", 1, "dictation").is_ok());
    }

    #[test]
    fn speech_disable_removal_and_explicit_reinstall_remain_revision_bound() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        store.set_enabled("mewu.voice", 1, false).unwrap();
        let disabled = store.snapshot().unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), disabled);
        assert!(store.speech_to_text("mewu.voice", 1, "dictation").is_err());
        assert!(store.speech_to_text("mewu.voice", 2, "dictation").is_err());
        store.set_enabled("mewu.voice", 2, true).unwrap();
        assert!(store.speech_to_text("mewu.voice", 2, "dictation").is_err());
        assert!(store.speech_to_text("mewu.voice", 3, "dictation").is_ok());
        store.uninstall("mewu.voice", 3).unwrap();
        let removed = store.snapshot().unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), removed);
        assert!(!path(&dir.0, "mewu.voice", "1.0.0").exists());
        assert!(store.speech_to_text("mewu.voice", 4, "dictation").is_err());
        store.install_official("mewu.voice", Some(4)).unwrap();
        assert!(store.speech_to_text("mewu.voice", 4, "dictation").is_err());
        assert!(store.speech_to_text("mewu.voice", 5, "dictation").is_ok());
    }

    #[test]
    fn core_annotations_do_not_depend_on_plugin_state_and_reject_forged_grants() {
        let dir = Directory::new();
        let store = PluginStore::open(&dir.0).unwrap();
        let grant = mewu_core::VisualAnnotationGrant {
            plugin_id: "mewu.core.annotations".into(),
            plugin_revision: 1,
            contribution_id: "annotate".into(),
        };
        assert_eq!(
            store.visual_annotations(&grant).unwrap(),
            VisualAnnotationEngine::HostVectorV1
        );
        assert!(visual_annotation_grant_is_current(
            &grant,
            &PluginSnapshot {
                revision: 0,
                plugins: vec![]
            }
        ));
        assert!(store
            .snapshot()
            .unwrap()
            .plugins
            .iter()
            .all(|p| p.manifest.id != "mewu.annotations"));
        for forged in [
            mewu_core::VisualAnnotationGrant {
                plugin_revision: 2,
                ..grant.clone()
            },
            mewu_core::VisualAnnotationGrant {
                contribution_id: "other".into(),
                ..grant.clone()
            },
        ] {
            assert!(store.visual_annotations(&forged).is_err());
            assert!(!visual_annotation_grant_is_current(
                &forged,
                &store.snapshot().unwrap()
            ));
        }
        for field in ["command", "url", "permissions", "engine"] {
            let mut manifest: serde_json::Value =
                serde_json::from_slice(OFFICIAL_ANNOTATIONS).unwrap();
            manifest["contributions"][0][field] = serde_json::json!("untrusted");
            assert!(parse_manifest(&serde_json::to_vec(&manifest).unwrap()).is_err());
        }
    }

    #[test]
    fn speech_seed_only_adds_missing_package_preserving_other_packages() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        store.set_enabled("mewu.ocr", 1, false).unwrap();
        store.uninstall("mewu.table", 1).unwrap();
        // Recreate the previous release registry using only this test directory.
        store
            .connection
            .execute("DELETE FROM registry WHERE id='mewu.voice'", [])
            .unwrap();
        fs::remove_file(path(&dir.0, "mewu.voice", "1.0.0")).unwrap();
        let previous = store.snapshot().unwrap();
        assert_eq!(previous.plugins.len(), BUNDLED.len() - 1);
        let files: Vec<_> = previous
            .plugins
            .iter()
            .filter(|p| p.state != PluginState::Removed)
            .map(|p| {
                let file = path(&dir.0, &p.manifest.id, &p.manifest.version);
                (file.clone(), fs::read(file).unwrap())
            })
            .collect();
        drop(store);
        let store = PluginStore::open(&dir.0).unwrap();
        let mut seeded = store.snapshot().unwrap();
        assert_eq!(seeded.revision, previous.revision + 1);
        seeded.plugins.retain(|p| p.manifest.id != "mewu.voice");
        assert_eq!(seeded.plugins, previous.plugins);
        for (file, bytes) in files {
            assert_eq!(fs::read(file).unwrap(), bytes);
        }
        assert_eq!(
            store.speech_to_text("mewu.voice", 1, "dictation").unwrap(),
            SpeechEngine::WindowsSapi
        );
        let seeded = store.snapshot().unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), seeded);
        let mut chosen = parse_manifest(OFFICIAL_VOICE).unwrap();
        chosen.version = "0.9.0".into();
        store
            .install_package(chosen, PluginSource::Official, Some(1), true)
            .unwrap();
        let chosen = store.snapshot().unwrap();
        drop(store);
        assert_eq!(
            PluginStore::open(&dir.0).unwrap().snapshot().unwrap(),
            chosen
        );
    }

    #[test]
    fn memory_provider_declaration_cannot_inject_code_routes_or_credentials() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(
            store
                .memory_provider("mewu.memory-hindsight", 1, "hindsight")
                .unwrap(),
            MemoryAdapter::Hindsight
        );
        assert!(store
            .memory_provider("mewu.memory-hindsight", 0, "hindsight")
            .is_err());
        assert!(store.memory_provider("mewu.pin", 1, "pin").is_err());
        assert!(store
            .workflow("mewu.memory-hindsight", 1, "hindsight")
            .is_err());
        for field in [
            "url", "endpoint", "key", "bankId", "command", "prompt", "script", "tools",
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(OFFICIAL_MEMORY).unwrap();
            value["contributions"][0][field] = serde_json::json!("forbidden");
            assert!(
                parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{field}"
            );
        }
        let mut unsupported: serde_json::Value = serde_json::from_slice(OFFICIAL_MEMORY).unwrap();
        unsupported["contributions"][0]["adapter"] = serde_json::json!("arbitrary-code");
        assert!(parse_manifest(&serde_json::to_vec(&unsupported).unwrap()).is_err());
        store
            .set_enabled("mewu.memory-hindsight", 1, false)
            .unwrap();
        assert!(store
            .memory_provider("mewu.memory-hindsight", 2, "hindsight")
            .is_err());
        store.set_enabled("mewu.memory-hindsight", 2, true).unwrap();
        let package = path(&dir.0, "mewu.memory-hindsight", "1.0.0");
        let bytes = fs::read(&package).unwrap();
        fs::write(&package, b"{}").unwrap();
        assert!(store
            .memory_provider("mewu.memory-hindsight", 3, "hindsight")
            .is_err());
        fs::write(&package, bytes).unwrap();
        store.uninstall("mewu.memory-hindsight", 3).unwrap();
        let removed = store.snapshot().unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), removed);
        assert!(store
            .memory_provider("mewu.memory-hindsight", 4, "hindsight")
            .is_err());
        store
            .install_official("mewu.memory-hindsight", Some(4))
            .unwrap();
        assert!(store
            .memory_provider("mewu.memory-hindsight", 5, "hindsight")
            .is_ok());
    }

    #[test]
    fn memory_seed_only_adds_missing_package_without_restoring_disabled_or_removed_plugins() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        store.set_enabled("mewu.ocr", 1, false).unwrap();
        store.uninstall("mewu.table", 1).unwrap();
        store
            .connection
            .execute("DELETE FROM registry WHERE id='mewu.memory-hindsight'", [])
            .unwrap();
        fs::remove_file(path(&dir.0, "mewu.memory-hindsight", "1.0.0")).unwrap();
        let before = store.snapshot().unwrap();
        drop(store);
        let store = PluginStore::open(&dir.0).unwrap();
        let mut after = store.snapshot().unwrap();
        assert_eq!(after.revision, before.revision + 1);
        assert_eq!(
            store
                .memory_provider("mewu.memory-hindsight", 1, "hindsight")
                .unwrap(),
            MemoryAdapter::Hindsight
        );
        after
            .plugins
            .retain(|plugin| plugin.manifest.id != "mewu.memory-hindsight");
        assert_eq!(after.plugins, before.plugins);
    }

    #[test]
    fn pin_declaration_is_strict_and_authorization_checks_kind_revision_and_package() {
        let dir = Directory::new();
        let store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(
            store.pin("mewu.pin", 1, "pin").unwrap(),
            PluginContribution::Pin {
                id: "pin".into(),
                title: "贴图".into(),
            }
        );
        assert!(store.pin("mewu.pin", 0, "pin").is_err());
        assert!(store.pin("mewu.pin", 1, "capture").is_err());
        assert!(store.pin("mewu.scroll", 1, "capture").is_err());
        assert!(store.workflow("mewu.pin", 1, "pin").is_err());
        assert!(store.scroll("mewu.pin", 1, "pin").is_err());
        assert!(store.translation("mewu.pin", 1, "pin").is_err());
        for field in [
            "script",
            "url",
            "windowUrl",
            "html",
            "command",
            "prompt",
            "tools",
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(OFFICIAL_PIN).unwrap();
            value["contributions"][0][field] = serde_json::json!("unauthorized");
            assert!(
                parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{field}"
            );
        }
        let mut changed = parse_manifest(OFFICIAL_PIN).unwrap();
        changed.description = "different package bytes".into();
        fs::write(
            path(&dir.0, "mewu.pin", "1.0.0"),
            serde_json::to_vec_pretty(&changed).unwrap(),
        )
        .unwrap();
        assert!(store.pin("mewu.pin", 1, "pin").is_err());
    }

    #[test]
    fn pin_disable_and_removal_survive_restart_until_explicit_reinstall() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        store.set_enabled("mewu.pin", 1, false).unwrap();
        let disabled = store.snapshot().unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), disabled);
        assert!(store.pin("mewu.pin", 1, "pin").is_err());
        assert!(store.pin("mewu.pin", 2, "pin").is_err());
        store.set_enabled("mewu.pin", 2, true).unwrap();
        assert!(store.pin("mewu.pin", 2, "pin").is_err());
        assert!(store.pin("mewu.pin", 3, "pin").is_ok());
        store.uninstall("mewu.pin", 3).unwrap();
        let removed = store.snapshot().unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), removed);
        assert!(!path(&dir.0, "mewu.pin", "1.0.0").exists());
        assert!(store.pin("mewu.pin", 4, "pin").is_err());
        store.install_official("mewu.pin", Some(4)).unwrap();
        assert!(store.pin("mewu.pin", 5, "pin").is_ok());
    }

    #[test]
    fn pin_seed_adds_only_missing_package_and_preserves_existing_choices() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        store.set_enabled("mewu.ocr", 1, false).unwrap();
        store.uninstall("mewu.table", 1).unwrap();
        // Model a pre-pin registry using only this isolated test directory.
        store
            .connection
            .execute("DELETE FROM registry WHERE id='mewu.pin'", [])
            .unwrap();
        fs::remove_file(path(&dir.0, "mewu.pin", "1.0.0")).unwrap();
        let previous = store.snapshot().unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        let mut seeded = store.snapshot().unwrap();
        assert_eq!(seeded.revision, previous.revision + 1);
        assert!(store.pin("mewu.pin", 1, "pin").is_ok());
        seeded.plugins.retain(|p| p.manifest.id != "mewu.pin");
        assert_eq!(seeded.plugins, previous.plugins);
        let mut chosen = parse_manifest(OFFICIAL_PIN).unwrap();
        chosen.version = "0.9.0".into();
        store
            .install_package(chosen, PluginSource::Official, Some(1), true)
            .unwrap();
        let chosen = store.snapshot().unwrap();
        drop(store);
        let store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(store.snapshot().unwrap(), chosen);
        assert!(store.pin("mewu.pin", 1, "pin").is_err());
        assert!(store.pin("mewu.pin", 2, "pin").is_ok());
    }

    #[test]
    fn official_seed_removal_and_disable_survive_restart_without_reinstallation() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        let initial = store.snapshot().unwrap();
        assert_eq!(initial.plugins.len(), bundled_manifests().len());
        assert_eq!(initial.revision, bundled_manifests().len() as u64);
        let removed = store.uninstall("mewu.table", 1).unwrap();
        let disabled = store.set_enabled("mewu.ocr", 1, false).unwrap();
        assert_eq!(removed.state, PluginState::Removed);
        assert_eq!(disabled.state, PluginState::Disabled);
        assert!(!path(&dir.0, "mewu.table", "1.0.0").exists());
        let expected = store.snapshot().unwrap();
        drop(store);
        let mut reopened = PluginStore::open(&dir.0).unwrap();
        assert_eq!(reopened.snapshot().unwrap(), expected);
        assert!(!path(&dir.0, "mewu.table", "1.0.0").exists());
        let reinstalled = reopened
            .install_official("mewu.table", Some(removed.revision))
            .unwrap();
        assert_eq!(reinstalled.state, PluginState::Enabled);
        assert!(path(&dir.0, "mewu.table", "1.0.0").is_file());
    }

    #[test]
    fn bundled_ocr_checks_kind_revision_package_and_persistent_removal() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert_eq!(
            store.ocr("mewu.ocr", 1, "recognize").unwrap(),
            OcrEngine::Windows
        );
        assert!(store.ocr("mewu.ocr", 1, "drawing-tools").is_err());
        assert!(store.workflow("mewu.ocr", 1, "recognize").is_err());
        store.set_enabled("mewu.ocr", 1, false).unwrap();
        assert!(store.ocr("mewu.ocr", 1, "recognize").is_err());
        assert!(store.ocr("mewu.ocr", 2, "recognize").is_err());
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert!(store.ocr("mewu.ocr", 2, "recognize").is_err());
        store.set_enabled("mewu.ocr", 2, true).unwrap();
        let manifest_path = path(&dir.0, "mewu.ocr", "1.0.0");
        let bytes = fs::read(&manifest_path).unwrap();
        fs::write(&manifest_path, b"{}").unwrap();
        assert!(store.ocr("mewu.ocr", 3, "recognize").is_err());
        fs::write(&manifest_path, bytes).unwrap();
        assert!(store.ocr("mewu.ocr", 3, "recognize").is_ok());
        store.uninstall("mewu.ocr", 3).unwrap();
        drop(store);
        let mut store = PluginStore::open(&dir.0).unwrap();
        assert!(store.ocr("mewu.ocr", 4, "recognize").is_err());
        assert!(!manifest_path.exists());
        store.install_official("mewu.ocr", Some(4)).unwrap();
        assert!(store.ocr("mewu.ocr", 5, "recognize").is_ok());
    }

    #[test]
    fn cas_idempotence_and_workflow_fence_are_separate_from_ui_state() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        let record = install(&mut store, "1.0.0", None);
        let original = store.snapshot().unwrap();
        assert!(store
            .workflow("example.table", 1, "recognize-table")
            .is_ok());
        assert!(store.set_enabled("example.table", 0, false).is_err());
        assert_eq!(store.snapshot().unwrap(), original);
        assert_eq!(store.set_enabled("example.table", 1, true).unwrap(), record);
        assert_eq!(store.snapshot().unwrap(), original);
        let disabled = store.set_enabled("example.table", 1, false).unwrap();
        assert!(store
            .workflow("example.table", 1, "recognize-table")
            .is_err());
        assert!(store
            .workflow("example.table", disabled.revision, "recognize-table")
            .is_err());
        assert!(store.workflow("mewu.drawing", 1, "drawing-tools").is_err());
    }

    #[test]
    fn local_version_replacement_and_rollback_restore_exact_package_and_source() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        let first = install(&mut store, "1.0.0", None);
        let second = install(&mut store, "1.1.0", Some(first.revision));
        assert!(second.has_rollback);
        assert!(path(&dir.0, "example.table", "1.1.0").is_file());
        let back = store.rollback("example.table", second.revision).unwrap();
        assert_eq!(back.manifest, first.manifest);
        assert_eq!(back.source, first.source);
        assert_eq!(back.revision, 3);
        let snapshot = store.snapshot().unwrap();
        let mut changed = package("1.0.0");
        changed.name = "替换同版本".into();
        assert!(store
            .install_bytes(&serde_json::to_vec(&changed).unwrap(), source(), Some(3))
            .is_err());
        assert_eq!(store.snapshot().unwrap(), snapshot);
        store.uninstall("example.table", 3).unwrap();
        assert!(!path(&dir.0, "example.table", "1.0.0").exists());
        assert!(!path(&dir.0, "example.table", "1.1.0").exists());
    }

    #[test]
    fn database_failure_restores_files_and_preserves_all_revisions() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        install(&mut store, "1.0.0", None);
        let before = store.snapshot().unwrap();
        store.connection.execute_batch("CREATE TRIGGER fail_plugin_update BEFORE UPDATE ON registry BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(store
            .install_bytes(
                &serde_json::to_vec(&package("2.0.0")).unwrap(),
                source(),
                Some(1)
            )
            .is_err());
        assert_eq!(store.snapshot().unwrap(), before);
        assert!(!path(&dir.0, "example.table", "2.0.0").exists());
        assert!(store
            .workflow("example.table", 1, "recognize-table")
            .is_ok());
        assert!(store.uninstall("example.table", 1).is_err());
        assert_eq!(store.snapshot().unwrap(), before);
        assert!(path(&dir.0, "example.table", "1.0.0").is_file());
        assert!(store.set_enabled("example.table", 1, false).is_err());
        assert_eq!(store.snapshot().unwrap(), before);
    }

    #[test]
    fn interrupted_file_change_recovers_old_committed_state_on_open() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        install(&mut store, "1.0.0", None);
        let snapshot = store.snapshot().unwrap();
        let before = store.read("example.table").unwrap().unwrap();
        let mut after = before.clone();
        after.package.manifest.version = "2.0.0".into();
        after.previous = Some(before.package.clone());
        after.revision = 2;
        let intent = FileIntent {
            before: Some(before.clone()),
            after: after.clone(),
            recording_migration: None,
        };
        store
            .connection
            .execute(
                "INSERT INTO file_intent VALUES(1,?1)",
                [json(&intent).unwrap()],
            )
            .unwrap();
        store.apply_files(Some(&before), Some(&after)).unwrap();
        assert!(store.snapshot().is_err());
        drop(store);
        let reopened = PluginStore::open(&dir.0).unwrap();
        assert_eq!(reopened.snapshot().unwrap(), snapshot);
        assert!(!path(&dir.0, "example.table", "2.0.0").exists());
        assert!(reopened
            .workflow("example.table", 1, "recognize-table")
            .is_ok());
    }

    #[test]
    fn missing_or_tampered_package_is_not_available_or_silently_reseeded() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        fs::remove_file(path(&dir.0, "mewu.table", "1.0.0")).unwrap();
        let record = store
            .snapshot()
            .unwrap()
            .plugins
            .into_iter()
            .find(|r| r.manifest.id == "mewu.table")
            .unwrap();
        assert!(record.error.is_some());
        assert!(store.workflow("mewu.table", 1, "recognize-table").is_err());
        assert!(store.set_enabled("mewu.table", 1, true).is_err());
        drop(store);
        let mut reopened = PluginStore::open(&dir.0).unwrap();
        assert!(!path(&dir.0, "mewu.table", "1.0.0").exists());
        reopened.install_official("mewu.table", Some(1)).unwrap();
        assert!(reopened
            .workflow("mewu.table", 2, "recognize-table")
            .is_ok());
        fs::write(path(&dir.0, "mewu.table", "1.0.0"), b"{\"tampered\":true}").unwrap();
        assert!(reopened
            .workflow("mewu.table", 2, "recognize-table")
            .is_err());
        assert!(reopened
            .snapshot()
            .unwrap()
            .plugins
            .iter()
            .find(|r| r.manifest.id == "mewu.table")
            .unwrap()
            .error
            .is_some());
    }

    #[test]
    fn manifest_is_strict_and_official_identity_cannot_be_claimed() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        let original = store.snapshot().unwrap();
        assert!(store
            .install_bytes(OFFICIAL_TABLE, source(), Some(1))
            .is_err());
        assert!(store
            .install_bytes(OFFICIAL_TABLE, PluginSource::Official, Some(1))
            .is_err());
        for field in ["entrypoint", "permissions", "tools", "mcpServers"] {
            let mut value = serde_json::to_value(package("1.0.0")).unwrap();
            value[field] = serde_json::json!([]);
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        for id in ["../escape", "con.plugin", "Example.name", "a..b", "a/b"] {
            let mut value = package("1.0.0");
            value.id = id.into();
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        let mut value = package("1.0.0");
        value.license = "AGPL-3.0".into();
        assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(parse_manifest(&vec![b' '; MAX_MANIFEST_BYTES + 1]).is_err());
        assert_eq!(store.snapshot().unwrap(), original);
    }

    #[test]
    fn unknown_database_schema_is_not_overwritten() {
        let dir = Directory::new();
        let plugin_dir = dir.0.join("plugins");
        fs::create_dir(&plugin_dir).unwrap();
        let db = Connection::open(plugin_dir.join("plugins.db")).unwrap();
        db.execute_batch("CREATE TABLE precious(value TEXT); INSERT INTO precious VALUES('keep'); PRAGMA user_version=77;").unwrap();
        drop(db);
        assert!(PluginStore::open(&dir.0).is_err());
        let db = Connection::open(plugin_dir.join("plugins.db")).unwrap();
        let value: String = db
            .query_row("SELECT value FROM precious", [], |r| r.get(0))
            .unwrap();
        assert_eq!(value, "keep");
    }

    #[test]
    fn local_files_are_canonical_and_other_sources_cannot_replace_an_installation() {
        let dir = Directory::new();
        let first = dir.0.join("first.json");
        let second = dir.0.join("second.json");
        fs::write(&first, serde_json::to_vec(&package("1.0.0")).unwrap()).unwrap();
        fs::write(&second, serde_json::to_vec(&package("2.0.0")).unwrap()).unwrap();
        let mut store = PluginStore::open(&dir.0).unwrap();
        let installed = store.install_local(&first, None).unwrap();
        assert_eq!(
            installed.source,
            PluginSource::Local {
                path: fs::canonicalize(&first)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            }
        );
        let snapshot = store.snapshot().unwrap();
        assert!(store.install_local(&second, Some(1)).is_err());
        assert!(store
            .install_bytes(
                &serde_json::to_vec(&package("2.0.0")).unwrap(),
                source(),
                Some(1)
            )
            .is_err());
        assert_eq!(store.snapshot().unwrap(), snapshot);
        store.uninstall("example.table", 1).unwrap();
        assert!(store.install_local(&second, Some(2)).is_ok());
        assert!(read_validated_local(Path::new("relative.json")).is_err());
    }

    #[test]
    fn github_repository_format_and_origin_are_checked_without_network_access() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        install(&mut store, "1.0.0", None);
        for repository in [
            "https://github.com/example/table",
            "example/table.git",
            "../repo",
            "a/b/c",
            "a/",
        ] {
            let source = PluginSource::Github {
                repository: repository.into(),
                commit: "b".repeat(40),
                path: "plugin.json".into(),
            };
            assert!(store
                .install_bytes(
                    &serde_json::to_vec(&package("2.0.0")).unwrap(),
                    source,
                    Some(1)
                )
                .is_err());
        }
        let other = PluginSource::Github {
            repository: "other/table".into(),
            commit: "b".repeat(40),
            path: "plugin.json".into(),
        };
        assert!(store
            .install_bytes(
                &serde_json::to_vec(&package("2.0.0")).unwrap(),
                other,
                Some(1)
            )
            .is_err());
        let update = PluginSource::Github {
            repository: "Example/Table".into(),
            commit: "b".repeat(40),
            path: "plugin.json".into(),
        };
        assert!(store
            .install_bytes(
                &serde_json::to_vec(&package("2.0.0")).unwrap(),
                update,
                Some(1)
            )
            .is_ok());
    }

    #[test]
    fn conflicting_target_is_not_deleted_by_failed_install_and_catalog_preference_is_independent() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        let snapshot = store.snapshot().unwrap();
        let target = path(&dir.0, "example.table", "1.0.0");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"not our file").unwrap();
        assert!(store
            .install_bytes(
                &serde_json::to_vec(&package("1.0.0")).unwrap(),
                source(),
                None
            )
            .is_err());
        assert_eq!(fs::read(&target).unwrap(), b"not our file");
        assert_eq!(store.snapshot().unwrap(), snapshot);
        store
            .set_catalog_source(Some("https://example.com/catalog.json"))
            .unwrap();
        assert_eq!(store.snapshot().unwrap(), snapshot);
        drop(store);
        let mut reopened = PluginStore::open(&dir.0).unwrap();
        assert_eq!(
            reopened.get_catalog_source().unwrap().as_deref(),
            Some("https://example.com/catalog.json")
        );
        reopened.set_catalog_source(None).unwrap();
        assert_eq!(reopened.get_catalog_source().unwrap(), None);
    }
    #[test]
    fn video_drawing_is_an_explicit_closed_capability_and_snapshot_revoke_is_exact() {
        let dir = Directory::new();
        let mut store = PluginStore::open(&dir.0).unwrap();
        install_legacy_official(&mut store, "mewu.drawing", None);
        let grant = VideoDrawingGrant {
            plugin_id: "mewu.drawing".into(),
            revision: 1,
            contribution_id: "video-drawing-tools".into(),
        };
        assert_eq!(
            store
                .video_drawing_tools(&grant.plugin_id, 1, &grant.contribution_id)
                .unwrap()
                .len(),
            7
        );
        assert_eq!(
            store
                .drawing_tools("mewu.drawing", 1, "drawing-tools")
                .unwrap()
                .len(),
            9
        );
        assert!(store
            .video_drawing_tools("mewu.drawing", 1, "drawing-tools")
            .is_err());
        assert!(store
            .drawing_tools("mewu.drawing", 1, "video-drawing-tools")
            .is_err());
        assert!(video_drawing_grant_is_current(
            &grant,
            VideoDrawingTool::Pen,
            &store.snapshot().unwrap()
        ));
        let mut manifest = parse_manifest(OFFICIAL_DRAWING).unwrap();
        manifest.id = "example.video-drawing".into();
        // Screenshot support by itself does not opt a third party into video.
        manifest
            .contributions
            .retain(|v| matches!(v, PluginContribution::DrawingTools { .. }));
        store
            .install_bytes(&serde_json::to_vec(&manifest).unwrap(), source(), None)
            .unwrap();
        assert!(store
            .video_drawing_tools(&manifest.id, 1, "drawing-tools")
            .is_err());
        for bad in ["mosaic", "highlighter", "rich", "heal", "svg"] {
            let mut wire = serde_json::to_value(parse_manifest(OFFICIAL_DRAWING).unwrap()).unwrap();
            wire["contributions"][1]["tools"] = serde_json::json!([bad]);
            assert!(parse_manifest(&serde_json::to_vec(&wire).unwrap()).is_err());
        }
        for tools in [
            serde_json::json!([]),
            serde_json::json!(["pen", "pen"]),
            serde_json::json!([{"pen":null}]),
        ] {
            let mut wire = serde_json::to_value(parse_manifest(OFFICIAL_DRAWING).unwrap()).unwrap();
            wire["contributions"][1]["tools"] = tools;
            assert!(parse_manifest(&serde_json::to_vec(&wire).unwrap()).is_err());
        }
        let snapshot = store.snapshot().unwrap();
        let mut stale = grant.clone();
        stale.revision = 2;
        assert!(!video_drawing_grant_is_current(
            &stale,
            VideoDrawingTool::Pen,
            &snapshot
        ));
        let file = path(&dir.0, "mewu.drawing", "1.2.0");
        let bytes = fs::read(&file).unwrap();
        fs::write(&file, b"{}").unwrap();
        assert!(store
            .video_drawing_tools("mewu.drawing", 1, "video-drawing-tools")
            .is_err());
        assert!(!video_drawing_grant_is_current(
            &grant,
            VideoDrawingTool::Pen,
            &store.snapshot().unwrap()
        ));
        fs::write(&file, bytes).unwrap();
        store.set_enabled("mewu.drawing", 1, false).unwrap();
        assert!(!video_drawing_grant_is_current(
            &grant,
            VideoDrawingTool::Pen,
            &store.snapshot().unwrap()
        ));
        store.uninstall("mewu.drawing", 2).unwrap();
        let snap = store.snapshot().unwrap();
        drop(store);
        let reopened = PluginStore::open(&dir.0).unwrap();
        assert_eq!(reopened.snapshot().unwrap(), snap);
        assert!(!file.exists());
    }

    #[test]
    fn canonical_v11_revision2_exact_v10_previous_upgrades_once_using_single_previous_contract() {
        let dir = Directory::new();
        let store = legacy_drawing_v11_store(&dir.0, true);
        let before = store.read("mewu.drawing").unwrap().unwrap();
        let revision = store.global_revision().unwrap();
        let v11 = fs::read(path(&dir.0, "mewu.drawing", "1.1.0")).unwrap();
        assert!(store
            .check_package(before.previous.as_ref().unwrap())
            .is_ok());
        drop(store);
        let store = historical_open(&dir.0).unwrap();
        let after = store.read("mewu.drawing").unwrap().unwrap();
        assert_eq!(after.revision, 3);
        assert_eq!(after.package.manifest.version, "1.2.0");
        assert_eq!(after.previous, Some(before.package));
        assert_eq!(store.global_revision().unwrap(), revision + 1);
        assert_eq!(
            fs::read(path(&dir.0, "mewu.drawing", "1.1.0")).unwrap(),
            v11
        );
        assert!(!path(&dir.0, "mewu.drawing", "1.0.0").exists());
        let snapshot = store.snapshot().unwrap();
        drop(store);
        assert_eq!(
            historical_open(&dir.0).unwrap().snapshot().unwrap(),
            snapshot
        );
    }

    #[test]
    fn v11_upgrade_preserves_identifiable_user_choices_and_incomplete_states() {
        for case in [
            "disabled",
            "removed",
            "reenabled",
            "reinstalled",
            "revision2-no-previous",
            "revision1-previous",
            "noncanonical",
            "missing-current",
            "tampered-current",
            "missing-previous",
            "tampered-previous",
            "noncanonical-previous",
        ] {
            let dir = Directory::new();
            let mut store = legacy_drawing_v11_store(&dir.0, case.contains("previous"));
            let rev = store.read("mewu.drawing").unwrap().unwrap().revision;
            match case {
                "disabled" => {
                    store.set_enabled("mewu.drawing", rev, false).unwrap();
                }
                "removed" => {
                    store.uninstall("mewu.drawing", rev).unwrap();
                }
                "reenabled" => {
                    store.set_enabled("mewu.drawing", rev, false).unwrap();
                    store.set_enabled("mewu.drawing", rev + 1, true).unwrap();
                }
                "reinstalled" => {
                    store.uninstall("mewu.drawing", rev).unwrap();
                    store
                        .install_package(
                            parse_manifest(OFFICIAL_DRAWING_V1_1_0).unwrap(),
                            PluginSource::Official,
                            Some(rev + 1),
                            true,
                        )
                        .unwrap();
                }
                "revision2-no-previous"
                | "revision1-previous"
                | "noncanonical"
                | "noncanonical-previous" => {
                    let mut record = store.read("mewu.drawing").unwrap().unwrap();
                    match case {
                        "revision2-no-previous" => {
                            record.revision = 2;
                            record.previous = None;
                        }
                        "revision1-previous" => record.revision = 1,
                        "noncanonical" => {
                            record.package.manifest.description = "用户不同的完整清单".into()
                        }
                        _ => {
                            record.previous.as_mut().unwrap().manifest.description =
                                "不同旧清单".into()
                        }
                    }
                    // Write the chosen complete package as fixture setup; startup must not reinterpret it.
                    fs::write(
                        path(&dir.0, "mewu.drawing", "1.1.0"),
                        serde_json::to_vec_pretty(&record.package.manifest).unwrap(),
                    )
                    .unwrap();
                    if let Some(p) = &record.previous {
                        fs::write(
                            path(&dir.0, "mewu.drawing", &p.manifest.version),
                            serde_json::to_vec_pretty(&p.manifest).unwrap(),
                        )
                        .unwrap();
                    }
                    store
                        .connection
                        .execute(
                            "UPDATE registry SET payload=?1 WHERE id='mewu.drawing'",
                            [json(&record).unwrap()],
                        )
                        .unwrap();
                }
                "missing-current" => {
                    fs::remove_file(path(&dir.0, "mewu.drawing", "1.1.0")).unwrap()
                }
                "tampered-current" => {
                    fs::write(path(&dir.0, "mewu.drawing", "1.1.0"), b"{}").unwrap()
                }
                "missing-previous" => {
                    fs::remove_file(path(&dir.0, "mewu.drawing", "1.0.0")).unwrap()
                }
                "tampered-previous" => {
                    fs::write(path(&dir.0, "mewu.drawing", "1.0.0"), b"{}").unwrap()
                }
                _ => unreachable!(),
            }
            let before = store.read("mewu.drawing").unwrap();
            let revision = store.global_revision().unwrap();
            let bytes =
                ["1.0.0", "1.1.0", "1.2.0"].map(|v| fs::read(path(&dir.0, "mewu.drawing", v)).ok());
            drop(store);
            let reopened = PluginStore::open(&dir.0).unwrap();
            assert_eq!(reopened.read("mewu.drawing").unwrap(), before, "{case}");
            assert_eq!(reopened.global_revision().unwrap(), revision, "{case}");
            assert_eq!(
                ["1.0.0", "1.1.0", "1.2.0"].map(|v| fs::read(path(&dir.0, "mewu.drawing", v)).ok()),
                bytes,
                "{case}"
            );
        }
        // The old 1.0 state is explicitly outside this migration.
        let dir = Directory::new();
        let store = legacy_drawing_store(&dir.0);
        let before = store.snapshot().unwrap();
        drop(store);
        assert_eq!(
            PluginStore::open(&dir.0).unwrap().snapshot().unwrap(),
            before
        );
    }

    #[test]
    fn interrupted_v11_file_intent_recovers_then_commits_one_upgrade() {
        let dir = Directory::new();
        let store = legacy_drawing_v11_store(&dir.0, true);
        let before = store.read("mewu.drawing").unwrap().unwrap();
        let revision = store.global_revision().unwrap();
        let after = StoredRecord {
            package: Package {
                manifest: parse_manifest(OFFICIAL_DRAWING).unwrap(),
                source: PluginSource::Official,
            },
            revision: before.revision + 1,
            state: PluginState::Enabled,
            previous: Some(before.package.clone()),
        };
        let intent = FileIntent {
            before: Some(before.clone()),
            after: after.clone(),
            recording_migration: None,
        };
        store
            .connection
            .execute(
                "INSERT INTO file_intent(slot,payload) VALUES(1,?1)",
                [json(&intent).unwrap()],
            )
            .unwrap();
        store.apply_files(Some(&before), Some(&after)).unwrap();
        drop(store);
        let reopened = historical_open(&dir.0).unwrap();
        assert_eq!(reopened.read("mewu.drawing").unwrap(), Some(after));
        assert_eq!(reopened.global_revision().unwrap(), revision + 1);
        assert_eq!(
            reopened
                .connection
                .query_row("SELECT count(*) FROM file_intent", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            parse_manifest(&fs::read(path(&dir.0, "mewu.drawing", "1.1.0")).unwrap()).unwrap(),
            parse_manifest(OFFICIAL_DRAWING_V1_1_0).unwrap()
        );
    }
    // Append inside the existing plugins.rs #[cfg(test)] mod tests. Uses only its
    // owned synthetic Directory / legacy_drawing_v11_store. Fixed expected hashes
    // came from the pre-launch report; never derive them from an upgraded user DB.
    #[test]
    fn canonical_drawing_v12_upgrade_bytes_match_frozen_cold_update_oracle() {
        use sha2::{Digest, Sha256};
        let sha = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
        for (with_previous, registry_sha) in [
            (
                false,
                "2626c4b321ca38d1022c3af39af7b88f4b43572c75083b6d20ad7e9e0446d0eb",
            ),
            (
                true,
                "7c4d704e99f3706e8d836e4d0c7357a0f14f0d5923d39ee7a8cbe4e592ad33fc",
            ),
        ] {
            let dir = Directory::new();
            let old = legacy_drawing_v11_store(&dir.0, with_previous);
            let global_revision = old.global_revision().unwrap();
            let v11 = fs::read(path(&dir.0, "mewu.drawing", "1.1.0")).unwrap();
            drop(old);
            let upgraded = historical_open(&dir.0).unwrap();
            let raw: String = upgraded
                .connection
                .query_row(
                    "SELECT payload FROM registry WHERE id='mewu.drawing'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(sha(raw.as_bytes()), registry_sha);
            assert_eq!(upgraded.global_revision().unwrap(), global_revision + 1);
            assert_eq!(
                raw,
                json(&upgraded.read("mewu.drawing").unwrap().unwrap()).unwrap()
            );
            let installed = fs::read(path(&dir.0, "mewu.drawing", "1.2.0")).unwrap();
            assert_eq!(installed.len(), 800);
            assert_eq!(
                sha(&installed),
                "8dc0244aaab6f0b20bdd9e2f205b50adb6e6d0a47c6fa082df5281665993b277"
            );
            assert_eq!(
                installed,
                serde_json::to_vec_pretty(&parse_manifest(OFFICIAL_DRAWING).unwrap()).unwrap()
            );
            assert_eq!(
                fs::read(path(&dir.0, "mewu.drawing", "1.1.0")).unwrap(),
                v11
            );
        }
    }
}
