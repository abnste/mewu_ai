using System.Diagnostics;
using System.Runtime.InteropServices;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class TempMediaLifecycleTests
{
    [Fact]
    public void RegistryNormalizesPathsAndReferenceCountsLeases()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"clip.mp4");File.WriteAllText(path,"video");
            var registry=new TempMediaRegistry();
            using var first=registry.AcquireExistingFile(path);
            using var second=registry.AcquireExistingFile(Path.Combine(root,".","clip.mp4"));

            Assert.Equal(2,registry.ActiveLeaseCount);
            Assert.Equal(1,registry.ActivePathCount);
            Assert.Equal(2,Assert.Single(registry.Snapshot()).Value);
            first.Dispose();
            Assert.True(registry.IsLeased(path));
            first.Dispose();
            second.Dispose();
            Assert.False(registry.IsLeased(path));
            Assert.Empty(registry.Snapshot());
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task RegistryReferenceCountingIsThreadSafe()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"clip.mp4");File.WriteAllText(path,"video");
            var registry=new TempMediaRegistry();
            var acquisitions=Enumerable.Range(0,64)
                .Select(_=>Task.Run(()=>registry.AcquireExistingFile(path),TestContext.Current.CancellationToken))
                .ToArray();
            var leases=await Task.WhenAll(acquisitions);
            Assert.Equal(64,registry.ActiveLeaseCount);
            Assert.Equal(1,registry.ActivePathCount);

            await Task.WhenAll(leases.Select(lease=>Task.Run(()=>
            {
                lease.Dispose();
                lease.Dispose();
            },TestContext.Current.CancellationToken)));
            Assert.Equal(0,registry.ActiveLeaseCount);
            Assert.Empty(registry.Snapshot());
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task CleanupAndExistingFileAcquisitionAreSerialized()
    {
        var root=TestDirectory();
        using var entered=new ManualResetEventSlim();using var releaseDelete=new ManualResetEventSlim();
        try
        {
            var path=Path.Combine(root,"clip.mp4");File.WriteAllText(path,"video");
            var registry=new TempMediaRegistry();
            var deletion=Task.Run(()=>registry.TryExecuteIfUnleased(path,false,()=>
            {
                entered.Set();
                releaseDelete.Wait(TestContext.Current.CancellationToken);
                File.Delete(path);
            }),TestContext.Current.CancellationToken);
            Assert.True(entered.Wait(TimeSpan.FromSeconds(10),TestContext.Current.CancellationToken));
            var acquisition=Task.Run(() => Record.Exception(()=>registry.AcquireExistingFile(path).Dispose()),TestContext.Current.CancellationToken);
            await Task.Delay(50,TestContext.Current.CancellationToken);
            Assert.False(acquisition.IsCompleted);

            releaseDelete.Set();
            Assert.True(await deletion);
            Assert.IsType<FileNotFoundException>(await acquisition);
            Assert.Empty(registry.Snapshot());
        }
        finally
        {
            releaseDelete.Set();
            if(Directory.Exists(root))Directory.Delete(root,true);
        }
    }

    [Fact]
    public void CleanupSkipsLeasedFilesAndDeletesThemAfterRelease()
    {
        var root=TestDirectory();
        try
        {
            var registry=new TempMediaRegistry();var service=new TempFileService(root,registry);
            var path=Path.Combine(root,"clip.mp4");File.WriteAllText(path,"video");
            using(var lease=registry.AcquireExistingFile(path))
            {
                var protectedResult=service.Cleanup(TimeSpan.Zero,true);
                Assert.True(File.Exists(path));
                Assert.Equal(0,protectedResult.DeletedCount);
                Assert.Equal(1,protectedResult.SkippedLeasedCount);
            }

            var releasedResult=service.Cleanup(TimeSpan.Zero,true);
            Assert.False(File.Exists(path));
            Assert.Equal(1,releasedResult.DeletedCount);
            Assert.Equal(0,releasedResult.SkippedLeasedCount);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void CleanupDoesNotRecursivelyDeleteDirectoryContainingALeasedFile()
    {
        var root=TestDirectory();
        try
        {
            var registry=new TempMediaRegistry();var service=new TempFileService(root,registry);
            var child=Path.Combine(root,"frames");Directory.CreateDirectory(child);
            var path=Path.Combine(child,"frame.bin");File.WriteAllText(path,"frame");
            using(var lease=registry.AcquireExistingFile(path))
            {
                var result=service.Cleanup(TimeSpan.Zero,true);
                Assert.True(Directory.Exists(child));
                Assert.Equal(1,result.SkippedLeasedCount);
            }

            service.Cleanup(TimeSpan.Zero,true);
            Assert.False(Directory.Exists(child));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task ShutdownWaitObservesReleaseWithoutPollingAndIsBoundedOnTimeout()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"clip.mp4");File.WriteAllText(path,"video");
            var registry=new TempMediaRegistry();var lease=registry.AcquireExistingFile(path);
            using var waitingStarted=new ManualResetEventSlim();
            var waiting=Task.Run(() =>
            {
                waitingStarted.Set();
                return registry.WaitForNoActiveLeases(TimeSpan.FromSeconds(10));
            },TestContext.Current.CancellationToken);
            Assert.True(waitingStarted.Wait(TimeSpan.FromSeconds(10),TestContext.Current.CancellationToken));
            Assert.False(waiting.IsCompleted);
            lease.Dispose();
            Assert.True(await waiting.WaitAsync(TimeSpan.FromSeconds(10),TestContext.Current.CancellationToken));

            using var retained=registry.AcquireExistingFile(path);
            var stopwatch=Stopwatch.StartNew();
            Assert.False(registry.WaitForNoActiveLeases(TimeSpan.FromMilliseconds(60)));
            stopwatch.Stop();
            Assert.InRange(stopwatch.ElapsedMilliseconds,40,5000);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void PersistentFileDropStagesAnIndependentCopyAndAgeCleanupRemovesItLater()
    {
        var root=TestDirectory();var staging=Path.Combine(root,"Clipboard");var source=Path.Combine(root,"source.mp4");
        try
        {
            var bytes=Enumerable.Range(0,2048).Select(index=>(byte)(index%251)).ToArray();File.WriteAllBytes(source,bytes);
            string? clipboardPath=null;
            var copied=ClipboardService.TrySetPersistentFileDropList(
                source,staging,path=>clipboardPath=path,out var stagedPath,out var error,delay:_=>{});

            Assert.True(copied,error);
            Assert.Equal(stagedPath,clipboardPath);
            Assert.NotEqual(Path.GetFullPath(source),stagedPath);
            Assert.Equal(bytes,File.ReadAllBytes(Assert.IsType<string>(stagedPath)));
            File.Delete(source);
            Assert.True(File.Exists(stagedPath));

            File.SetLastWriteTimeUtc(stagedPath,DateTime.UtcNow-TimeSpan.FromDays(8));
            var result=ClipboardService.CleanupStagedFiles(TimeSpan.FromDays(7),staging,true);
            Assert.Equal(1,result.DeletedCount);
            Assert.False(File.Exists(stagedPath));
        }
        finally{if(Directory.Exists(root))Directory.Delete(root,true);}
    }

    [Fact]
    public void FailedClipboardWriteRollsBackItsStagedCopy()
    {
        var root=TestDirectory();var staging=Path.Combine(root,"Clipboard");var source=Path.Combine(root,"source.mp4");
        try
        {
            File.WriteAllText(source,"video");var attempts=0;
            var copied=ClipboardService.TrySetPersistentFileDropList(
                source,
                staging,
                _=>{attempts++;throw new ExternalException("busy");},
                out var stagedPath,
                out var error,
                retryCount:3,
                delay:_=>{},
                logError:(_,_)=>{});

            Assert.False(copied);
            Assert.Null(stagedPath);
            Assert.NotNull(error);
            Assert.Equal(3,attempts);
            Assert.Empty(Directory.EnumerateFileSystemEntries(staging));
        }
        finally{if(Directory.Exists(root))Directory.Delete(root,true);}
    }

    [Fact]
    public void CleanupPolicyBlocksAnyActiveMediaLeaseEvenWithoutAnOverlay()
    {
        Assert.Null(TempMediaCleanupPolicy.GetBlockReason(false,0));
        Assert.Contains("正在录制、预览、保存或贴视频",TempMediaCleanupPolicy.GetBlockReason(false,1));
    }

    [Fact]
    public void AtomicSaveDoesNotTreatAMissingSamePathAsSuccess()
    {
        var missing=Path.Combine(TestDirectory(),"missing.mp4");
        try{Assert.Throws<FileNotFoundException>(()=>AtomicFileService.Copy(missing,Path.Combine(Path.GetDirectoryName(missing)!,".","missing.mp4")));}
        finally{Directory.Delete(Path.GetDirectoryName(missing)!,true);}
    }

    private static string TestDirectory()
    {
        var path=Path.Combine(Path.GetTempPath(),"MewuAI.Tests",Guid.NewGuid().ToString("N"));Directory.CreateDirectory(path);return path;
    }
}
