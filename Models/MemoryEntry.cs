// SPDX-License-Identifier: MPL-2.0
namespace mewu_ai_Assistant.Models;

/// <summary>A local autofill mapping. Values are stored in the local settings file.</summary>
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
