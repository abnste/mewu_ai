// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class MemoryStore
{
    private const string Prefix = "memory-";

    internal static string EnsureCredentialId(MemoryEntry entry)
    {
        if (string.IsNullOrWhiteSpace(entry.CredentialId))
            entry.CredentialId = Prefix + entry.Id;
        return entry.CredentialId;
    }

    internal static void Save(MemoryEntry entry, string value, CredentialService? credentials = null)
    {
        if (string.IsNullOrWhiteSpace(value)) throw new InvalidOperationException("记忆值不能为空。");
        if (entry.Keywords is null || entry.Keywords.Count == 0) throw new InvalidOperationException("至少需要一个关键词。");
        var id = EnsureCredentialId(entry);
        if (!id.StartsWith(Prefix, StringComparison.Ordinal)) throw new InvalidOperationException("记忆凭据标识无效。");
        (credentials ?? new CredentialService()).Save(id, value);
        entry.Value = string.Empty;
    }

    internal static string? Read(MemoryEntry entry, CredentialService? credentials = null)
        => string.IsNullOrEmpty(entry.Value) ? (entry.CredentialId?.StartsWith(Prefix, StringComparison.Ordinal) != true ? null : (credentials ?? new CredentialService()).Read(entry.CredentialId)) : entry.Value;

    internal static void Delete(MemoryEntry entry)
    {
        entry.Value = string.Empty;
        if (entry.CredentialId?.StartsWith(Prefix, StringComparison.Ordinal) == true) new CredentialService().Delete(entry.CredentialId);
    }

    internal static string Normalize(string text)
        => new(text.Where(char.IsLetterOrDigit).Select(char.ToLowerInvariant).ToArray());

    internal static IReadOnlyList<(MemoryEntry Entry, string Keyword)> Match(IEnumerable<MemoryEntry> entries, string text)
    {
        var normalized = Normalize(text);
        if (normalized.Length == 0) return [];
        return entries.Where(entry => entry.Enabled && entry.Keywords is { Count: > 0 })
            .SelectMany(entry => entry.Keywords.Where(keyword => !string.IsNullOrWhiteSpace(keyword)).Select(keyword => (Entry: entry, Keyword: keyword.Trim())))
            .Where(item =>
            {
                var keyword = Normalize(item.Keyword);
                return keyword.Length >= 2 && normalized.Contains(keyword, StringComparison.Ordinal);
            })
            .GroupBy(item => item.Entry.Id, StringComparer.OrdinalIgnoreCase).Select(group => group.First()).ToArray();
    }
}
