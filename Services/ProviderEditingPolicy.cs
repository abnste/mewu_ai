using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class ProviderEditingPolicy
{
    // A malformed settings file can contain an arbitrary number of IDs that
    // collide with generated candidates. Keep repair strictly bounded so the
    // settings page can never spin forever while trying to recover it.
    private const int RandomIdentityAttempts = 8;
    private const int DeterministicIdentityAttempts = 32;

    internal static ProviderEditingIdentityResult RepairIdentities(
        IList<AiProviderSettings> editableProviders,
        string? storedDefaultProviderId)
        => RepairIdentities(editableProviders, storedDefaultProviderId, static () => Guid.NewGuid().ToString("N"));

    /// <summary>
    /// Repairs blank/duplicate Provider IDs. The generator overload is kept
    /// internal so tests can exercise collision and exhaustion paths without
    /// relying on the practically impossible event of a GUID collision.
    /// </summary>
    internal static ProviderEditingIdentityResult RepairIdentities(
        IList<AiProviderSettings> editableProviders,
        string? storedDefaultProviderId,
        Func<string> candidateFactory)
    {
        ArgumentNullException.ThrowIfNull(editableProviders);
        ArgumentNullException.ThrowIfNull(candidateFactory);
        var defaultMatches = string.IsNullOrWhiteSpace(storedDefaultProviderId)
            ? []
            : editableProviders
                .Select((provider, index) => (provider, index))
                .Where(entry => string.Equals(entry.provider.Id, storedDefaultProviderId, StringComparison.Ordinal))
                .Select(entry => entry.index)
                .ToList();
        var unambiguousDefaultIndex = defaultMatches.Count == 1 ? defaultMatches[0] : -1;
        var usedIds = new HashSet<string>(StringComparer.Ordinal);
        var repairedCount = 0;

        for (var index = 0; index < editableProviders.Count; index++)
        {
            var provider = editableProviders[index];
            if (string.IsNullOrWhiteSpace(provider.Id) || !usedIds.Add(provider.Id))
            {
                provider.Id = CreateReplacementId(usedIds, index, candidateFactory);
                repairedCount++;
            }
        }

        var mappedDefault = unambiguousDefaultIndex >= 0
            ? editableProviders[unambiguousDefaultIndex].Id
            : null;
        return new ProviderEditingIdentityResult(
            mappedDefault,
            editableProviders.Count > 0 && mappedDefault is null,
            repairedCount);
    }

    private static string CreateReplacementId(
        HashSet<string> usedIds,
        int providerIndex,
        Func<string> candidateFactory)
    {
        for (var attempt = 0; attempt < RandomIdentityAttempts; attempt++)
        {
            var candidate = candidateFactory();
            if (!string.IsNullOrWhiteSpace(candidate) && usedIds.Add(candidate))
                return candidate;
        }

        // Deterministic candidates make the repair reproducible when a custom
        // generator repeatedly collides, while still keeping the retry count
        // finite for hostile/corrupt input.
        for (var attempt = 0; attempt < DeterministicIdentityAttempts; attempt++)
        {
            var candidate = $"mewu-provider-{providerIndex}-{attempt}";
            if (usedIds.Add(candidate))
                return candidate;
        }

        throw new InvalidOperationException(
            $"无法为第 {providerIndex + 1} 个 Provider 生成唯一 ID；请删除重复配置后重试。");
    }
}

internal sealed record ProviderEditingIdentityResult(
    string? DefaultProviderId,
    bool RequiresDefaultSelection,
    int RepairedIdentityCount);
