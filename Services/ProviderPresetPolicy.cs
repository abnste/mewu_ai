using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal sealed record ProviderPreset(string Id, string Name, string Type, string BaseUrl, string DefaultModel);

internal static class ProviderPresetPolicy
{
    internal static readonly ProviderPreset[] All =
    [
        new("MiniMax", "MiniMax", "MiniMax", "https://api.minimaxi.com/v1", "MiniMax-M3"),
        new("MiniMaxGlobal", "MiniMax (International)", "MiniMax", "https://api.minimax.io/v1", "MiniMax-M3"),
        new("Volcengine", "火山引擎", "OpenAICompatible", VolcengineModelPolicy.StandardBaseUrl, ""),
        new("OpenAI", "OpenAI", "OpenAICompatible", "https://api.openai.com/v1", ""),
        new("Custom", "自定义 / OpenAI compatible", "OpenAICompatible", "", "")
    ];

    internal static ProviderPreset Detect(AiProviderSettings settings) => All.FirstOrDefault(p =>
        p.Id != "Custom" && p.Type.Equals(settings.Type, StringComparison.OrdinalIgnoreCase) &&
        p.BaseUrl.Equals(settings.BaseUrl.TrimEnd('/'), StringComparison.OrdinalIgnoreCase)) ?? All[^1];

    internal static AiProviderSettings Create(ProviderPreset preset) => new()
    {
        Name = preset.Name, Type = preset.Type, BaseUrl = preset.BaseUrl, Model = preset.DefaultModel
    };

    internal static bool IsUntouchedDraft(AiProviderSettings settings, bool hasPendingKey)
    {
        var preset = Detect(settings);
        return !hasPendingKey && string.IsNullOrEmpty(settings.CredentialId) &&
            settings.CustomHeaders.Count == 0 && settings.SensitiveHeaderCredentialIds.Count == 0 &&
            settings.BaseUrl == preset.BaseUrl && settings.Model == preset.DefaultModel && settings.Name == preset.Name;
    }

    internal static string DisplayName(AiProviderSettings settings) =>
        settings.Type.Equals("MiniMax", StringComparison.OrdinalIgnoreCase) && settings.Name == "MiniMax M3"
            ? "MiniMax" : settings.Name;
}
