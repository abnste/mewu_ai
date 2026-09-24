using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ObsidianVaultDiscoveryTests
{
    [Fact]
    public void DiscoveryHonorsCancellationBeforeFilesystemAccess()
    {
        using var cancellation=new CancellationTokenSource();
        cancellation.Cancel();
        Assert.Throws<OperationCanceledException>(()=>ObsidianVaultService.FindVaults(cancellation.Token));
    }
}
