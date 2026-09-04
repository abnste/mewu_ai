using System.Diagnostics;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class HermesStartupLifecycleTests
{
    private static TaskCompletionSource NewSignal()=>new(TaskCreationOptions.RunContinuationsAsynchronously);

    private static async Task<Process> ExitedProcessAsync()
    {
        var info=new ProcessStartInfo(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System),"cmd.exe"))
        {UseShellExecute=false,CreateNoWindow=true};
        info.ArgumentList.Add("/d");info.ArgumentList.Add("/c");info.ArgumentList.Add("exit 1");
        var process=Process.Start(info)!;
        using var timeout=new CancellationTokenSource(TimeSpan.FromSeconds(15));
        await process.WaitForExitAsync(timeout.Token);
        return process;
    }

    [Fact]
    public async Task ExitWaitsForTheFinalDiagnosticBeforeReportingFailure()
    {
        using var process=await ExitedProcessAsync();
        var ready=new TaskCompletionSource<int>(TaskCreationOptions.RunContinuationsAsynchronously);
        var stderrClosed=NewSignal();
        var diagnostics=new HermesStartupDiagnostics();
        var wait=HermesBackendService.WaitForReadyAsync(process,ready.Task,Task.CompletedTask,
            Task.CompletedTask,stderrClosed.Task,diagnostics,TestContext.Current.CancellationToken);
        Assert.False(wait.IsCompleted);
        diagnostics.Observe("ModuleNotFoundError: private-sensitive-value");
        stderrClosed.SetResult();
        var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>wait);
        Assert.Contains("Python",error.Message,StringComparison.Ordinal);
        Assert.DoesNotContain("private",error.Message,StringComparison.Ordinal);
    }

    [Fact]
    public async Task ReadyFollowedByExitIsNotAcceptedAsALiveBackend()
    {
        using var process=await ExitedProcessAsync();
        await Assert.ThrowsAsync<InvalidOperationException>(()=>HermesBackendService.WaitForReadyAsync(
            process,Task.FromResult(12345),Task.CompletedTask,Task.CompletedTask,Task.CompletedTask,
            new HermesStartupDiagnostics(),TestContext.Current.CancellationToken));
    }

    [Fact]
    public async Task CancellationInterruptsOutputDrain()
    {
        using var process=await ExitedProcessAsync();
        using var cancellation=new CancellationTokenSource();
        var openPipe=NewSignal();
        var wait=HermesBackendService.WaitForReadyAsync(process,Task.FromResult(12345),Task.CompletedTask,
            openPipe.Task,openPipe.Task,new HermesStartupDiagnostics(),cancellation.Token);
        Assert.False(wait.IsCompleted);
        cancellation.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>wait);
    }
}
