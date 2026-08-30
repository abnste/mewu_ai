using System.Runtime.InteropServices;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ClipboardServiceTests
{
    [Fact]
    public async Task PersistentVideoCopyCanStageAsynchronouslyBeforeClipboardWrite()
    {
        var root=TestDirectory();var staging=Path.Combine(root,"Clipboard");var source=Path.Combine(root,"source.mp4");
        try
        {
            var bytes=Enumerable.Range(0,8192).Select(index=>(byte)(index%251)).ToArray();
            await File.WriteAllBytesAsync(source,bytes,TestContext.Current.CancellationToken);
            string? clipboardPath=null;
            var result=await ClipboardService.TrySetPersistentFileDropListAsync(
                source,
                staging,
                path=>clipboardPath=path,
                TestContext.Current.CancellationToken,
                delay:_=>{});

            Assert.True(result.Success);Assert.Null(result.Error);Assert.Equal(result.StagedPath,clipboardPath);
            Assert.NotNull(result.StagedPath);Assert.Equal(bytes,await File.ReadAllBytesAsync(result.StagedPath!,TestContext.Current.CancellationToken));
        }
        finally{if(Directory.Exists(root))Directory.Delete(root,true);}
    }

    [Fact]
    public async Task CancelledPersistentVideoCopyLeavesNoStagedFile()
    {
        var root=TestDirectory();var staging=Path.Combine(root,"Clipboard");var source=Path.Combine(root,"source.mp4");
        try
        {
            await File.WriteAllBytesAsync(source,[1,2,3],TestContext.Current.CancellationToken);
            using var cancellation=new CancellationTokenSource();cancellation.Cancel();
            await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>ClipboardService.TrySetPersistentFileDropListAsync(source,staging,_=>{},cancellation.Token));
            Assert.False(Directory.Exists(staging)&&Directory.EnumerateFiles(staging).Any());
        }
        finally{if(Directory.Exists(root))Directory.Delete(root,true);}
    }

    [Fact]
    public void BusyClipboard_IsRetriedAndCanRecover()
    {
        var attempts = 0;
        var delays = 0;

        var success = ClipboardService.TryExecute(
            () =>
            {
                attempts++;
                if (attempts < 3) throw new ExternalException("busy");
            },
            out var error,
            delay: _ => delays++);

        Assert.True(success);
        Assert.Null(error);
        Assert.Equal(3, attempts);
        Assert.Equal(2, delays);
    }

    [Fact]
    public void PermanentlyBusyClipboard_FailsAfterTheFiniteRetryBudget()
    {
        var attempts = 0;

        var success = ClipboardService.TryExecute(
            () =>
            {
                attempts++;
                throw new ExternalException("busy");
            },
            out var error,
            ClipboardService.RetryCount,
            _ => { },
            null);

        Assert.False(success);
        Assert.Equal(ClipboardService.RetryCount, attempts);
        Assert.Equal("系统剪贴板暂时不可用，请稍后重试", error);
    }

    [Fact]
    public void NonBusyFailures_AreNotRetried()
    {
        var attempts = 0;
        var logged = new List<(string Component, Exception Exception)>();

        var success = ClipboardService.TryExecute(
            () =>
            {
                attempts++;
                throw new InvalidOperationException("invalid");
            },
            out var error,
            ClipboardService.RetryCount,
            _ => throw new Xunit.Sdk.XunitException("delay should not run"),
            (component, exception) => logged.Add((component, exception)));

        Assert.False(success);
        Assert.Equal(1, attempts);
        Assert.NotNull(error);
        var entry=Assert.Single(logged);
        Assert.Equal("Clipboard",entry.Component);
        Assert.IsType<InvalidOperationException>(entry.Exception);
    }

    private static string TestDirectory()
    {
        var path=Path.Combine(Path.GetTempPath(),"MewuAI.Tests",Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(path);return path;
    }
}
