using System.Windows.Input;
using System.Text.Json.Serialization;
namespace mewu_ai_Assistant.Models;
public sealed class AppSettings
{
    public HotkeySetting CaptureHotkey { get; set; } = new();
    public bool LaunchAtStartup { get; set; }
    public string UiLanguage { get; set; } = "system";
    public double OverlayOpacity { get; set; } = .6; public int CaptureDelaySeconds { get; set; }
    public string DefaultImageFormat { get; set; } = "png"; public bool IncludeCaptureCursor { get; set; }
    public int RecordingFps { get; set; } = 30; public int RecordingQuality { get; set; } = 75; public int GifFps { get; set; } = 15; public bool IncludeRecordingCursor { get; set; } = true; public int TempCleanupDays { get; set; } = 3;
    public bool SaveConversationHistory { get; set; } public bool EnableVoiceInput { get; set; } public bool AutomaticallyStartListening { get; set; }
    public string VoiceLanguage { get; set; } = "system"; public string? DefaultProviderId { get; set; }
    public bool HermesEnabled { get; set; }
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
    [JsonRequired] public string Id { get; set; } = Guid.NewGuid().ToString("N"); public string Name { get; set; } = "MiniMax M3"; [JsonRequired] public string Type { get; set; } = "MiniMax";
    [JsonRequired] public string BaseUrl { get; set; } = "https://api.minimaxi.com/v1"; [JsonRequired] public string Model { get; set; } = "MiniMax-M3"; public string CredentialId { get; set; } = string.Empty;
    public Dictionary<string,string> CustomHeaders { get; set; } = [];
    public Dictionary<string,string> SensitiveHeaderCredentialIds { get; set; } = [];
    public override string ToString()=>Name;
}
