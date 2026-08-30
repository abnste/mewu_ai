using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ProviderEditingPolicyTests
{
    [Fact]
    public void DuplicateAndMissingIdsAreRepairedOnlyInTheEditableCopies()
    {
        var stored = new[]
        {
            new AiProviderSettings { Id = "duplicate" },
            new AiProviderSettings { Id = "duplicate" },
            new AiProviderSettings { Id = string.Empty }
        };
        var editable = stored.Select(ProviderHeaderCredentialService.Clone).ToList();

        var result = ProviderEditingPolicy.RepairIdentities(editable, "duplicate");

        Assert.Equal(3, editable.Select(provider => provider.Id).Distinct(StringComparer.Ordinal).Count());
        Assert.All(editable, provider => Assert.False(string.IsNullOrWhiteSpace(provider.Id)));
        Assert.Equal(new[] { "duplicate", "duplicate", string.Empty }, stored.Select(provider => provider.Id));
        Assert.Null(result.DefaultProviderId);
        Assert.True(result.RequiresDefaultSelection);
        Assert.Equal(2, result.RepairedIdentityCount);
    }

    [Fact]
    public void UnambiguousDefaultFollowsItsEditableProviderAfterRepair()
    {
        var providers = new List<AiProviderSettings>
        {
            new() { Id = string.Empty },
            new() { Id = "default" }
        };

        var result = ProviderEditingPolicy.RepairIdentities(providers, "default");

        Assert.Equal("default", result.DefaultProviderId);
        Assert.False(result.RequiresDefaultSelection);
        Assert.Equal(1, result.RepairedIdentityCount);
    }

    [Fact]
    public void IdentityRepairFallsBackToBoundedDeterministicCandidateAfterCollisions()
    {
        var providers = new List<AiProviderSettings>
        {
            new() { Id = "kept" },
            new() { Id = string.Empty }
        };
        var attempts = 0;

        var result = ProviderEditingPolicy.RepairIdentities(
            providers,
            null,
            () =>
            {
                attempts++;
                return "kept";
            });

        Assert.Equal(8, attempts);
        Assert.Equal("mewu-provider-1-0", providers[1].Id);
        Assert.Equal(1, result.RepairedIdentityCount);
    }

    [Fact]
    public void IdentityRepairFailsClearlyWhenAllFiniteCandidatesAreOccupied()
    {
        const int targetIndex = 32;
        var providers = Enumerable.Range(0, targetIndex)
            .Select(index => new AiProviderSettings { Id = $"mewu-provider-{targetIndex}-{index}" })
            .Append(new AiProviderSettings { Id = string.Empty })
            .ToList();
        var attempts = 0;

        var error = Assert.Throws<InvalidOperationException>(() => ProviderEditingPolicy.RepairIdentities(
            providers,
            null,
            () =>
            {
                attempts++;
                return "";
            }));

        Assert.Equal(8, attempts);
        Assert.Contains("无法为第 33 个 Provider 生成唯一 ID", error.Message, StringComparison.Ordinal);
        Assert.Equal(string.Empty, providers[targetIndex].Id);
    }

    [Fact]
    public void UnavailableHeaderStateCanDistinguishProvidersAfterIdsAreRepaired()
    {
        var first = new AiProviderSettings { Id = string.Empty };
        var second = new AiProviderSettings { Id = string.Empty };
        var providers = new List<AiProviderSettings> { first, second };
        var unavailable = new Dictionary<AiProviderSettings, HashSet<string>>(ReferenceEqualityComparer.Instance)
        {
            [first] = new HashSet<string>(["Authorization"], StringComparer.OrdinalIgnoreCase),
            [second] = new HashSet<string>(["X-Api-Key"], StringComparer.OrdinalIgnoreCase)
        };

        ProviderEditingPolicy.RepairIdentities(providers, null);

        Assert.NotEqual(first.Id, second.Id);
        Assert.Equal("Authorization", Assert.Single(unavailable[first]));
        Assert.Equal("X-Api-Key", Assert.Single(unavailable[second]));
    }

    [Theory]
    [InlineData(true, true)]
    [InlineData(false, false)]
    public void TemporaryMediaCleanupIsBlockedOnlyWhileCaptureIsActive(bool active, bool blocked)
    {
        Assert.Equal(blocked, TempMediaCleanupPolicy.GetBlockReason(active) is not null);
    }
}
