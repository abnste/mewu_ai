// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows.Input;
using System.Text.Json.Serialization;
namespace mewu_ai_Assistant.Models;
public sealed class AppSettings
{
    public HotkeySetting CaptureHotkey { get; set; } = new();
    public bool LaunchAtStartup { get; set; }
    public bool TeachingMode { get; set; } = true;
    public string UiLanguage { get; set; } = "system";
    public bool ThinkingGlowEnabled { get; set; } = true;
    public string ThinkingGlowColor { get; set; } = "#A7C7FF";
    public double OverlayOpacity { get; set; } = .6; public int CaptureDelaySeconds { get; set; }
    public string DefaultImageFormat { get; set; } = "png"; public bool IncludeCaptureCursor { get; set; }
    public int RecordingFps { get; set; } = 30; public int RecordingQuality { get; set; } = 75; public int GifFps { get; set; } = 15; public bool IncludeRecordingCursor { get; set; } = true; public int TempCleanupDays { get; set; } = 3;
    public bool RecordSystemAudio { get; set; } = true;
    public bool RecordMicrophone { get; set; }
    public bool SaveConversationHistory { get; set; } public bool EnableVoiceInput { get; set; } public bool AutomaticallyStartListening { get; set; }
    public string VoiceLanguage { get; set; } = "system"; public string? DefaultProviderId { get; set; }
    public string NetworkProxyMode { get; set; } = "system";
    public string NetworkProxyUrl { get; set; } = string.Empty;
    /// <summary>Last conversation channel selected in the screen assistant.</summary>
    public string ConversationChannelId { get; set; } = string.Empty;
    public bool HermesEnabled { get; set; }
    public bool CodexEnabled { get; set; }
    public string CodexExecutablePath { get; set; } = string.Empty;
    public string CodexModel { get; set; } = string.Empty;
    public string CodexReasoningEffort { get; set; } = "medium";
    public bool CodexSupportsImage { get; set; }
    public bool WorkBuddyEnabled { get; set; }
    public string WorkBuddyExecutablePath { get; set; } = string.Empty;
    public string WorkBuddyModel { get; set; } = string.Empty;
    public string WorkBuddyReasoningEffort { get; set; } = "enabled";
    public bool WorkBuddySupportsImage { get; set; }
    public bool MiniMaxCodeEnabled { get; set; }
    /// <summary>QQ 邮箱 MCP 上下文注入开关。令牌本身经 CredentialService（DPAPI）单独保存，不落入设置文件。</summary>
    public bool QqMailMcpEnabled { get; set; }
    /// <summary>网易邮箱（163/126 等）SMTP 代发开关。授权码经 CredentialService（DPAPI）单独保存。</summary>
    public bool NetEaseMailEnabled { get; set; }
    public string NetEaseMailAccount { get; set; } = string.Empty;
    public string NetEaseMailFromName { get; set; } = string.Empty;
    /// <summary>钉钉（企业内部应用）图片分享开关。AppSecret 经 CredentialService（DPAPI）单独保存。</summary>
    public bool DingTalkEnabled { get; set; }
    public string DingTalkAppKey { get; set; } = string.Empty;
    public string DingTalkAgentId { get; set; } = string.Empty;
    public string DingTalkTargetUsers { get; set; } = string.Empty;
    /// <summary>飞书（自建应用）图片分享开关。AppSecret 经 CredentialService（DPAPI）单独保存。</summary>
    public bool FeishuEnabled { get; set; }
    public string FeishuAppId { get; set; } = string.Empty;
    public string FeishuTargetId { get; set; } = string.Empty;
    public string FeishuTargetType { get; set; } = "chat_id";
    /// <summary>Obsidian 截图笔记开关与目标 vault（纯本地文件操作，无凭据）。</summary>
    public bool ObsidianEnabled { get; set; }
    public string ObsidianVaultPath { get; set; } = string.Empty;
    public string ObsidianAttachFolder { get; set; } = "attachments";
    public string ObsidianNoteFolder { get; set; } = "MewuAI";
    public bool ObsidianOpenAfterSave { get; set; } = true;
    /// <summary>腾讯 ima 知识库截图归档开关。API Key 经 CredentialService（DPAPI）单独保存，
    /// 凭证在 ima.qq.com/agent-interface 生成（ima 未向第三方开放扫码授权）。</summary>
    public bool ImaEnabled { get; set; }
    public string ImaClientId { get; set; } = string.Empty;
    public string ImaKnowledgeBaseId { get; set; } = string.Empty;
    public string ImaKnowledgeBaseName { get; set; } = string.Empty;
    /// <summary>Scrapling 爬虫（无头抓取 URL 内容）的安装目录。留空时自动探测
    /// 常见位置（D:\scrapling_app、D:\scrapling（爬虫）等）；目录内需含 venv\Scripts\python.exe。</summary>
    public string ScraplingPath { get; set; } = string.Empty;
    // Empty means the desktop channel has not been configured yet. The
    // settings page offers MiniMax-M3 as the first selectable model.
    public string MiniMaxCodeModel { get; set; } = string.Empty;
    public string HermesProfile { get; set; } = "default";
    public string HermesProvider { get; set; } = string.Empty;
    public string HermesModel { get; set; } = string.Empty;
    public string HermesReasoningEffort { get; set; } = "medium";
    public bool HermesAutoReadAloud { get; set; }
    public List<AiProviderSettings> Providers { get; set; } = [];
    [JsonIgnore] public List<string> ConfigurationErrors { get; } = [];
    [JsonIgnore] public bool HasSensitiveCredentialErrors { get; internal set; }
}
public sealed class HotkeySetting
{
    public Key Key { get; set; } = Key.S; public ModifierKeys Modifiers { get; set; } = ModifierKeys.Shift | ModifierKeys.Alt;
}
public sealed class AiProviderSettings
{
    [JsonRequired] public string Id { get; set; } = Guid.NewGuid().ToString("N"); public string Name { get; set; } = "MiniMax"; [JsonRequired] public string Type { get; set; } = "MiniMax";
    [JsonRequired] public string BaseUrl { get; set; } = "https://api.minimaxi.com/v1"; [JsonRequired] public string Model { get; set; } = "MiniMax-M3"; public string CredentialId { get; set; } = string.Empty;
    /// <summary>Wire protocol used by the upstream. Auto keeps legacy behavior and detects official endpoints.</summary>
    public string ApiFormat { get; set; } = "auto";
    /// <summary>Authentication policy: auto, bearer, api_key, anthropic_api_key, anthropic_auth_token, none.</summary>
    public string AuthMode { get; set; } = "auto";
    /// <summary>Optional complete request path (for regional gateways and plan endpoints).</summary>
    public string RequestPath { get; set; } = string.Empty;
    /// <summary>Optional provider region/plan labels retained for routing and diagnostics.</summary>
    public string Region { get; set; } = string.Empty;
    public string Plan { get; set; } = string.Empty;
    /// <summary>Optional account/organization header value (stored as a credential reference when sensitive).</summary>
    public string AccountIdHeader { get; set; } = string.Empty;
    public Dictionary<string,string> CustomHeaders { get; set; } = [];
    public Dictionary<string,System.Text.Json.JsonElement> RequestParameters { get; set; } = [];
    public Dictionary<string,string> SensitiveHeaderCredentialIds { get; set; } = [];
    public override string ToString()=>Name;
}
