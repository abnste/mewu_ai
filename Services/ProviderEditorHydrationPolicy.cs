using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Builds the editable Provider copy used by the settings page. A damaged
/// credential mapping is a configuration problem, not a reason to make the
/// settings page itself impossible to open. The original stored object is
/// never mutated here; callers can show the warning and require the user to
/// repair or remove the affected Provider before saving.
/// </summary>
internal static class ProviderEditorHydrationPolicy
{
    internal static ProviderEditorHydrationResult TryHydrate(
        AiProviderSettings source,
        ProviderHeaderCredentialService credentials,
        ISet<string>? unavailableHeaders = null)
    {
        ArgumentNullException.ThrowIfNull(source);
        ArgumentNullException.ThrowIfNull(credentials);

        try
        {
            return new(credentials.CreateHydratedCopy(source, unavailableHeaders), null);
        }
        catch (InvalidOperationException)
        {
            // Keep the raw mapping in the private editable copy so a failed
            // save cannot silently discard the old credential references.
            // SettingsWindow blocks saving this Provider until it is repaired
            // or removed, while still remaining usable for the other tabs.
            return new(CloneForEditing(source),
                "敏感 Header 凭据映射无法安全读取。为避免覆盖旧凭据，本轮不能直接保存此 Provider；请删除后重新添加。原设置和凭据未被改动。");
        }
        catch (ArgumentException)
        {
            return new(CloneForEditing(source),
                "敏感 Header 凭据映射格式无效。为避免覆盖旧凭据，本轮不能直接保存此 Provider；请删除后重新添加。原设置和凭据未被改动。");
        }
    }

    internal static AiProviderSettings CloneForEditing(AiProviderSettings source)
    {
        ArgumentNullException.ThrowIfNull(source);
        return new AiProviderSettings
        {
            Id = source.Id ?? string.Empty,
            Name = source.Name ?? "Provider",
            Type = source.Type ?? string.Empty,
            BaseUrl = source.BaseUrl ?? string.Empty,
            Model = source.Model ?? string.Empty,
            CredentialId = source.CredentialId ?? string.Empty,
            RequestParameters = source.RequestParameters is null ? null! : new(source.RequestParameters),
            CustomHeaders = source.CustomHeaders is null
                ? new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
                : new Dictionary<string, string>(source.CustomHeaders, StringComparer.OrdinalIgnoreCase),
            SensitiveHeaderCredentialIds = source.SensitiveHeaderCredentialIds is null
                ? new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
                : new Dictionary<string, string>(source.SensitiveHeaderCredentialIds, StringComparer.OrdinalIgnoreCase)
        };
    }
}

internal sealed record ProviderEditorHydrationResult(
    AiProviderSettings Provider,
    string? Warning);
