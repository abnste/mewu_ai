using System.Windows.Input;
namespace mewu_ai_Assistant.Models;
public sealed class AppSettings
{
    public HotkeySetting CaptureHotkey { get; set; } = new();
    public bool LaunchAtStartup { get; set; }
    public double OverlayOpacity { get; set; } = .6; public int CaptureDelaySeconds { get; set; }
    public string DefaultImageFormat { get; set; } = "png"; public bool IncludeCaptureCursor { get; set; }
    public int RecordingFps { get; set; } = 30; public int RecordingQuality { get; set; } = 75; public int GifFps { get; set; } = 15; public bool IncludeRecordingCursor { get; set; } = true; public int TempCleanupDays { get; set; } = 3;
    public bool SaveConversationHistory { get; set; } public bool EnableVoiceInput { get; set; } public bool AutomaticallyStartListening { get; set; }
    public string VoiceLanguage { get; set; } = "system"; public string? DefaultProviderId { get; set; }
    public List<AiProviderSettings> Providers { get; set; } = [];
}
public sealed class HotkeySetting
{
    public Key Key { get; set; } = Key.A; public ModifierKeys Modifiers { get; set; } = ModifierKeys.Control | ModifierKeys.Shift;
    public string DisplayName => $"{Modifiers.ToString().Replace(", ", " + ")} + {Key}";
}
public sealed class AiProviderSettings
{
    public string Id { get; set; } = Guid.NewGuid().ToString("N"); public string Name { get; set; } = "OpenAI Compatible"; public string Type { get; set; } = "OpenAICompatible";
    public string BaseUrl { get; set; } = "https://api.openai.com/v1"; public string Model { get; set; } = "gpt-4.1-mini"; public string CredentialId { get; set; } = string.Empty;
    public Dictionary<string,string> CustomHeaders { get; set; } = [];
}
