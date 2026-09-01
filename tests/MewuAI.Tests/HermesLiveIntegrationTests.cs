using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Models;
using Xunit;

namespace MewuAI.Tests;

/// <summary>
/// Opt-in checks against the user's installed Hermes. They only start the
/// loopback child owned by this test and never edit Hermes files or attach to
/// an already-running Hermes process.
/// </summary>
public sealed class HermesLiveIntegrationTests
{
    [Fact]
    public async Task InstalledHermesStartsOwnedBackendAndReturnsModels()
    {
        if(!string.Equals(Environment.GetEnvironmentVariable("MEWU_HERMES_LIVE"),"1",StringComparison.Ordinal))return;
        var installation=new HermesDiscoveryService().Discover();
        Assert.NotNull(installation);
        await using var runtime=new HermesRuntimeService();
        using var timeout=new CancellationTokenSource(TimeSpan.FromSeconds(90));

        var options=await runtime.GetModelOptionsAsync(false,timeout.Token);

        Assert.NotEmpty(options);
        Assert.Contains(options,option=>option.IsCurrent);
        Assert.All(options,option=>
        {
            Assert.False(string.IsNullOrWhiteSpace(option.Provider));
            Assert.False(string.IsNullOrWhiteSpace(option.Model));
        });
    }

    [Fact]
    public async Task InstalledHermesKeepsTwoPromptsInOneConversation()
    {
        if(!string.Equals(Environment.GetEnvironmentVariable("MEWU_HERMES_LIVE_PROMPT"),"1",StringComparison.Ordinal))return;
        await using var runtime=new HermesRuntimeService();
        using var timeout=new CancellationTokenSource(TimeSpan.FromMinutes(5));
        var options=await runtime.GetModelOptionsAsync(false,timeout.Token);
        var current=options.First(option=>option.IsCurrent);
        var settings=new AppSettings
        {
            HermesEnabled=true,
            HermesProvider=current.Provider,
            HermesModel=current.Model,
            HermesReasoningEffort=current.ReasoningEfforts.Contains("low",StringComparer.Ordinal)?"low":current.ReasoningEfforts[0]
        };
        var provider=runtime.GetConversationProvider(HermesConversationKind.Text,()=>settings);
        var marker=$"MEWU_{Guid.NewGuid():N}";

        var first=await provider.SendAsync(new AiRequest{Prompt=$"记住校验词 {marker}，现在只回复‘已记住’。"},timeout.Token);
        var second=await provider.SendAsync(new AiRequest{Prompt="只回复上一条消息中的校验词，不要解释。"},timeout.Token);

        Assert.False(string.IsNullOrWhiteSpace(first.Answer));
        Assert.Contains(marker,second.Answer,StringComparison.Ordinal);
        Assert.Same(provider,runtime.GetConversationProvider(HermesConversationKind.Screen,()=>settings));
    }
}
