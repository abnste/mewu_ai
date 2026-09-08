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
    public double OverlayOpacity { get; set; } = .6; public int CaptureDelaySeconds { get; set; }
    public string DefaultImageFormat { get; set; } = "png"; public bool IncludeCaptureCursor { get; set; }
    public int RecordingFps { get; set; } = 30; public int RecordingQuality { get; set; } = 75; public int GifFps { get; set; } = 15; public bool IncludeRecordingCursor { get; set; } = true; public int TempCleanupDays { get; set; } = 3;
    public bool RecordSystemAudio { get; set; } = true;
    public bool RecordMicrophone { get; set; }
    public bool SaveConversationHistory { get; set; } public bool EnableVoiceInput { get; set; } public bool AutomaticallyStartListening { get; set; }
    public string VoiceLanguage { get; set; } = "system"; public string? DefaultProviderId { get; set; }
    /// <summary>Last conversation channel selected in the screen assistant.</summary>
    public string ConversationChannelId { get; set; } = string.Empty;
    public bool HermesEnabled { get; set; }
    public bool CodexEnabled { get; set; }
    public string CodexModel { get; set; } = string.Empty;
    public string CodexReasoningEffort { get; set; } = "medium";
    public bool CodexSupportsImage { get; set; }
    public bool WorkBuddyEnabled { get; set; }
    public string WorkBuddyModel { get; set; } = string.Empty;
    public string WorkBuddyReasoningEffort { get; set; } = "enabled";
    public bool WorkBuddySupportsImage { get; set; }
    public bool MiniMaxCodeEnabled { get; set; }
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
    public Dictionary<string,string> CustomHeaders { get; set; } = [];
    public Dictionary<string,System.Text.Json.JsonElement> RequestParameters { get; set; } = [];
    public Dictionary<string,string> SensitiveHeaderCredentialIds { get; set; } = [];
    public override string ToString()=>Name;
}
