using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ProviderApiKeyEditorTests
{
    [Fact]
    public void ExistingKeyIsLoadedAtActualLengthWithoutStagingReplacement()
    {
        var provider = new AiProviderSettings { CredentialId = "saved" };
        var pending = new Dictionary<string,string>();
        var deleting = new HashSet<string>();
        var display = ProviderApiKeyEditorPolicy.ReadForDisplay(provider, pending, deleting, true, _ => "example-key-123");
        Assert.Equal("example-key-123", display);
        Assert.Equal(15, display.Length);
        Assert.Empty(pending);
        Assert.Empty(deleting);
    }

    [Fact]
    public void ManualClearMeansDeleteAndEditingRestoresDraft()
    {
        var provider = new AiProviderSettings();
        var pending = new Dictionary<string,string>();
        var deleting = new HashSet<string>();
        ProviderApiKeyEditorPolicy.RecordEdit(provider.Id, "", pending, deleting);
        Assert.Contains(provider.Id, deleting);
        Assert.Empty(ProviderApiKeyEditorPolicy.ReadForDisplay(provider, pending, deleting, true, _ => "old"));
        ProviderApiKeyEditorPolicy.RecordEdit(provider.Id, "replacement", pending, deleting);
        Assert.DoesNotContain(provider.Id, deleting);
        Assert.Equal("replacement", ProviderApiKeyEditorPolicy.ReadForDisplay(provider, pending, deleting, true, _ => "old"));
    }

    [Fact]
    public void SwitchingProvidersDoesNotLeakDraftAndProtectionFailureNeverReadsKey()
    {
        var a = new AiProviderSettings();
        var b = new AiProviderSettings();
        var pending = new Dictionary<string,string> { [a.Id] = "a-draft" };
        var deleting = new HashSet<string>();
        Assert.Equal("b-key", ProviderApiKeyEditorPolicy.ReadForDisplay(b, pending, deleting, true, _ => "b-key"));
        Assert.Empty(ProviderApiKeyEditorPolicy.ReadForDisplay(a, pending, deleting, false, _ => throw new InvalidOperationException()));
        Assert.Equal("a-draft", ProviderApiKeyEditorPolicy.ReadForDisplay(a, pending, deleting, true, _ => throw new InvalidOperationException()));
    }
}
