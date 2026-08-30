using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class LifecycleSecurityTests
{
    [Fact]
    public void StartupCommandUsesAQuotedAbsoluteExecutablePath()
    {
        var relative=Path.Combine("folder with spaces","MewuAI.exe");
        Assert.Equal($"\"{Path.GetFullPath(relative)}\"",StartupService.BuildCommand(relative));
        Assert.Throws<ArgumentException>(()=>StartupService.BuildCommand("   "));
    }

    [Fact]
    public async Task SingleInstanceSignalsPrimaryAndCanDisposeOnAnotherThread()
    {
        var name=$"MewuAI-Tests-{Guid.NewGuid():N}";
        var primary=await Task.Run(()=>new SingleInstanceService(name),TestContext.Current.CancellationToken);
        try
        {
            Assert.True(primary.IsPrimary);
            var activated=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            primary.ActivationRequested+=()=>activated.TrySetResult();
            using(var secondary=new SingleInstanceService(name))
            {
                Assert.False(secondary.IsPrimary);
                secondary.SignalPrimary();
            }
            await activated.Task.WaitAsync(TimeSpan.FromSeconds(3),TestContext.Current.CancellationToken);
        }
        finally
        {
            await Task.Run(primary.Dispose,TestContext.Current.CancellationToken);
        }
    }

    [Fact]
    public async Task ActivationSignalReceivedBeforeSubscriptionIsReplayed()
    {
        var name=$"MewuAI-Tests-{Guid.NewGuid():N}";
        using var primary=new SingleInstanceService(name);
        Assert.True(primary.IsPrimary);
        using(var secondary=new SingleInstanceService(name))
        {
            Assert.False(secondary.IsPrimary);
            secondary.SignalPrimary();
        }
        await Task.Delay(250,TestContext.Current.CancellationToken);
        var activated=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        primary.ActivationRequested+=()=>activated.TrySetResult();
        await activated.Task.WaitAsync(TimeSpan.FromSeconds(3),TestContext.Current.CancellationToken);
    }

    [Fact]
    public void StartupActivationGateDefersAndCoalescesSignalsUntilStartupCompletes()
    {
        var gate=new StartupActivationGate();var activations=0;
        gate.Signal(()=>activations++);gate.Signal(()=>activations++);
        Assert.Equal(0,activations);

        gate.MarkStarted(()=>activations++);
        Assert.Equal(1,activations);

        gate.Signal(()=>activations++);
        Assert.Equal(2,activations);
    }

    [Fact]
    public void TempCleanupIsBestEffortWhenItsDirectoryDisappears()
    {
        var root=TestDirectory();var service=new TempFileService(root);Directory.Delete(root,true);
        service.Cleanup(TimeSpan.Zero);
        Assert.Throws<AggregateException>(()=>service.Cleanup(TimeSpan.Zero,true));
    }

    [Theory]
    [InlineData(".")]
    [InlineData(".tar.gz")]
    [InlineData(".this-extension-is-too-long")]
    public void TempFilesRejectAmbiguousExtensions(string extension)
    {
        var root=TestDirectory();
        try{Assert.Throws<ArgumentException>(()=>new TempFileService(root).NewFile(extension));}
        finally{Directory.Delete(root,true);}
    }

    private static string TestDirectory()
    {
        var path=Path.Combine(Path.GetTempPath(),"MewuAI.Tests",Guid.NewGuid().ToString("N"));Directory.CreateDirectory(path);return path;
    }
}
