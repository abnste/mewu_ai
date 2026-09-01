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
    public void CrashDiagnosticsReportsAnUncleanPreviousSessionAndKeepsStructuredMarker()
    {
        var root=TestDirectory();
        try
        {
            var marker=Path.Combine(root,"Diagnostics","active-session.json");
            var logs=Path.Combine(root,"Logs");
            var first=new CrashDiagnosticsService(marker,new PrivacyLogger(logs),_=>false,()=>DateTimeOffset.Parse("2026-09-01T10:00:00Z"));
            first.StartSession(101);first.Mark("屏幕助手：停止并封装区域录屏");
            var second=new CrashDiagnosticsService(marker,new PrivacyLogger(logs),_=>false,()=>DateTimeOffset.Parse("2026-09-01T10:01:00Z"));
            second.StartSession(202);
            var record=System.Text.Json.JsonSerializer.Deserialize<CrashDiagnosticsService.CrashSessionMarker>(File.ReadAllText(marker));
            Assert.NotNull(record);Assert.Equal(202,record.ProcessId);Assert.False(record.CleanExit);
            Assert.Contains("PreviousSessionCrash",string.Join('\n',Directory.EnumerateFiles(logs).Select(File.ReadAllText)));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void CrashDiagnosticsDoesNotReportACleanPreviousSession()
    {
        var root=TestDirectory();
        try
        {
            var marker=Path.Combine(root,"Diagnostics","active-session.json");var logs=Path.Combine(root,"Logs");var logger=new PrivacyLogger(logs);
            var first=new CrashDiagnosticsService(marker,logger,_=>false);first.StartSession(301);first.CleanExit();
            var before=string.Join('\n',Directory.EnumerateFiles(logs).Select(File.ReadAllText));
            new CrashDiagnosticsService(marker,logger,_=>false).StartSession(302);
            var after=string.Join('\n',Directory.EnumerateFiles(logs).Select(File.ReadAllText));
            Assert.DoesNotContain("PreviousSessionCrash",after[before.Length..]);
        }
        finally{Directory.Delete(root,true);}
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
