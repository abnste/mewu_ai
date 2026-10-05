// SPDX-License-Identifier: MPL-2.0
namespace mewu_ai_Assistant.Models;

/// <summary>A local autofill mapping. Persisted values use a DPAPI credential reference.</summary>
public sealed class MemoryEntry
{
    public string Id { get; set; } = Guid.NewGuid().ToString("N");
    public List<string> Keywords { get; set; } = [];
    public bool Sensitive { get; set; }
    public bool Enabled { get; set; } = true;
    public string FieldKind { get; set; } = "auto";
    public string CredentialId { get; set; } = string.Empty;
    public string Value { get; set; } = string.Empty;
    public override string ToString() => string.Join("、", Keywords);
}
