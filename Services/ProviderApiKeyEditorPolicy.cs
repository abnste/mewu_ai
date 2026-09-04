using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class ProviderApiKeyEditorPolicy
{
    internal static string ReadForDisplay(AiProviderSettings provider, IReadOnlyDictionary<string,string> pending,
        ISet<string> deleting, bool captureProtected, Func<string,string?> readCredential)
    {
        if (!captureProtected || deleting.Contains(provider.Id)) return string.Empty;
        return pending.TryGetValue(provider.Id, out var draft) ? draft : readCredential(provider.CredentialId) ?? string.Empty;
    }

    internal static void RecordEdit(string providerId, string value, IDictionary<string,string> pending, ISet<string> deleting)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            pending.Remove(providerId);
            deleting.Add(providerId);
        }
        else
        {
            pending[providerId] = value;
            deleting.Remove(providerId);
        }
    }
}
