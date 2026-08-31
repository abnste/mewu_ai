using System.Security.Cryptography;
using System.Diagnostics;
using System.Windows.Controls;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;
using Xunit;
namespace MewuAI.Tests;
public sealed class RecordingIntegrationTests
{
    [Fact] public void RecordingNeedsFullFiveHundredMebibytesBeforeItStarts()
    {
        Assert.Throws<IOException>(()=>RecordingRuntimePolicy.EnsureEnoughSpaceToStart(RecordingRuntimePolicy.StartupMinimumFreeSpaceBytes-1));
        RecordingRuntimePolicy.EnsureEnoughSpaceToStart(RecordingRuntimePolicy.StartupMinimumFreeSpaceBytes);
    }

    [Fact] public void FailedRecordingCanNeverTransitionToCompleted()
    {
        var terminal=new RecordingTerminalState();
        Assert.True(terminal.TryFail());
        Assert.False(terminal.TryFail());
        Assert.False(terminal.TryComplete());
        Assert.Equal(RecordingTerminalResult.Failed,terminal.Current);
    }

    [Fact] public async Task RuntimeGuardCountsActiveRecordingTimeInsteadOfPausedWallTime()
    {
        var maximum=TimeSpan.FromMinutes(5);
        long activeTicks=TimeSpan.FromMinutes(4).Ticks;
        var firstDiskCheck=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var failed=new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
        var failureCount=0;
        var options=FastRuntimeGuardOptions(maximum);
        await using var guard=new RecordingRuntimeGuard(
            ()=>TimeSpan.FromTicks(Interlocked.Read(ref activeTicks)),
            ()=>{firstDiskCheck.TrySetResult();return options.MinimumFreeSpaceBytes+1;},
            options,
            message=>{Interlocked.Increment(ref failureCount);failed.TrySetResult(message);});

        guard.Start();
        await firstDiskCheck.Task.WaitAsync(TimeSpan.FromSeconds(2),TestContext.Current.CancellationToken);
        Assert.False(failed.Task.IsCompleted);

        Interlocked.Exchange(ref activeTicks,maximum.Ticks);
        var message=await failed.Task.WaitAsync(TimeSpan.FromSeconds(2),TestContext.Current.CancellationToken);
        await guard.Completion.WaitAsync(TimeSpan.FromSeconds(2),TestContext.Current.CancellationToken);

        Assert.Contains("时长上限",message,StringComparison.Ordinal);
        Assert.Equal(1,Volatile.Read(ref failureCount));
    }

    [Fact] public async Task RuntimeGuardStillChecksDiskWhileActiveTimeIsPaused()
    {
        var freeSpace=RecordingRuntimePolicy.RuntimeMinimumFreeSpaceBytes+1;
        var firstDiskCheck=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var failed=new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
        var failureCount=0;
        var options=FastRuntimeGuardOptions(TimeSpan.FromMinutes(5));
        await using var guard=new RecordingRuntimeGuard(
            ()=>TimeSpan.Zero,
            ()=>{firstDiskCheck.TrySetResult();return Interlocked.Read(ref freeSpace);},
            options,
            message=>{Interlocked.Increment(ref failureCount);failed.TrySetResult(message);});

        guard.Start();
        await firstDiskCheck.Task.WaitAsync(TimeSpan.FromSeconds(2),TestContext.Current.CancellationToken);
        Assert.False(failed.Task.IsCompleted);

        Interlocked.Exchange(ref freeSpace,options.MinimumFreeSpaceBytes);
        var message=await failed.Task.WaitAsync(TimeSpan.FromSeconds(2),TestContext.Current.CancellationToken);
        await guard.Completion.WaitAsync(TimeSpan.FromSeconds(2),TestContext.Current.CancellationToken);

        Assert.Contains("磁盘剩余空间",message,StringComparison.Ordinal);
        Assert.Equal(1,Volatile.Read(ref failureCount));
    }

    [Fact] public async Task DisposingRuntimeGuardCancelsAndJoinsItsPendingDelay()
    {
        var firstDiskCheck=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var options=FastRuntimeGuardOptions(TimeSpan.FromMinutes(5));
        var guard=new RecordingRuntimeGuard(
            ()=>TimeSpan.Zero,
            ()=>{firstDiskCheck.TrySetResult();return options.MinimumFreeSpaceBytes+1;},
            options,
            _=>throw new InvalidOperationException("取消监控时不应触发失败"));

        guard.Start();
        await firstDiskCheck.Task.WaitAsync(TimeSpan.FromSeconds(2),TestContext.Current.CancellationToken);
        var disposeTime=Stopwatch.StartNew();
        await guard.DisposeAsync();
        disposeTime.Stop();

        Assert.True(guard.Completion.IsCompleted);
        Assert.True(disposeTime.Elapsed<TimeSpan.FromSeconds(1),$"录屏监控退出耗时 {disposeTime.Elapsed.TotalMilliseconds:0} ms");
    }

    [Fact] public async Task RuntimeDiskFailureStopsRealSessionOnceAndSuppressesCompleted()
    {
        var probeCalls=0;
        var failedCount=0;
        var completedCount=0;
        var failed=new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
        var options=new RecordingRuntimeGuardOptions
        {
            CheckInterval=TimeSpan.FromMilliseconds(10),
            ShutdownTimeout=TimeSpan.FromMilliseconds(500),
            MaximumActiveDuration=TimeSpan.FromMinutes(5),
            MinimumFreeSpaceBytes=RecordingRuntimePolicy.RuntimeMinimumFreeSpaceBytes,
            AvailableFreeSpaceProbe=_=>Interlocked.Increment(ref probeCalls)==1
                ? RecordingRuntimePolicy.StartupMinimumFreeSpaceBytes
                : RecordingRuntimePolicy.RuntimeMinimumFreeSpaceBytes
        };
        var session=new RecordingSession(new AppSettings{RecordingFps=10,IncludeRecordingCursor=false},new ScreenRect(0,0,128,128),null,options);
        try
        {
            session.Failed+=message=>{Interlocked.Increment(ref failedCount);failed.TrySetResult(message);};
            session.Completed+=_=>Interlocked.Increment(ref completedCount);
            session.Start();

            var message=await failed.Task.WaitAsync(TimeSpan.FromSeconds(5),TestContext.Current.CancellationToken);
            await Task.Delay(300,TestContext.Current.CancellationToken);
            await session.DisposeAsync();

            Assert.Contains("磁盘剩余空间",message,StringComparison.Ordinal);
            Assert.Equal(1,Volatile.Read(ref failedCount));
            Assert.Equal(0,Volatile.Read(ref completedCount));
        }
        finally
        {
            session.Dispose();
            try{if(session.VideoPath.Length>0&&File.Exists(session.VideoPath))File.Delete(session.VideoPath);}catch(IOException){}
        }
    }

    [Fact] public async Task RecordsSmallRegionToRealMp4()
    {
        var session=new RecordingSession(new AppSettings{RecordingFps=10,GifFps=2,IncludeRecordingCursor=false},new ScreenRect(0,0,128,128),null);var done=new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);session.Completed+=p=>done.TrySetResult(p);session.Failed+=e=>done.TrySetException(new InvalidOperationException(e));
        var gifPath=Path.Combine(Path.GetTempPath(),$"mewu-recording-{Guid.NewGuid():N}.gif");
        try
        {
            var token=TestContext.Current.CancellationToken;session.Start();Assert.True(TempMediaRegistry.Shared.IsLeased(session.VideoPath));await Task.Delay(1200,token);session.Stop();var path=await done.Task.WaitAsync(TimeSpan.FromSeconds(20),token);Assert.Equal(Path.GetFullPath(path),Path.GetFullPath(session.VideoPath),StringComparer.OrdinalIgnoreCase);Assert.True(File.Exists(path));Assert.True(new FileInfo(path).Length>1000);Assert.True(TempMediaRegistry.Shared.IsLeased(path));using var retained=session.RetainCompletedVideo();Assert.Equal(Path.GetFullPath(path),retained.Path,StringComparer.OrdinalIgnoreCase);await session.DisposeAsync();Assert.True(TempMediaRegistry.Shared.IsLeased(path));var sourceHash=SHA256.HashData(await File.ReadAllBytesAsync(path,token));await VerifyPreviewAutoplaysAsync(path,token);
            var export=await GifExportService.ExportFromVideoAsync(path,gifPath,5,token);Assert.True(File.Exists(gifPath));Assert.True(new FileInfo(gifPath).Length>100);Assert.InRange(export.FrameCount,1,10);Assert.Equal(sourceHash,SHA256.HashData(await File.ReadAllBytesAsync(path,token)));
            using var stream=File.OpenRead(gifPath);var decoder=new GifBitmapDecoder(stream,BitmapCreateOptions.PreservePixelFormat,BitmapCacheOption.OnLoad);Assert.Equal(export.FrameCount,decoder.Frames.Count);var durationCentiseconds=decoder.Frames.Sum(frame=>Convert.ToInt32(Assert.IsType<BitmapMetadata>(frame.Metadata).GetQuery("/grctlext/Delay")));Assert.InRange(Math.Abs(durationCentiseconds*10-export.Duration.TotalMilliseconds),0,10);
        }
        finally
        {
            // Always release the native recorder before touching either output
            // file, including failure paths that never reach Stop().
            try{await session.DisposeAsync();}
            finally
            {
                try{if(File.Exists(session.VideoPath))File.Delete(session.VideoPath);}catch(IOException){}
                try{if(File.Exists(gifPath))File.Delete(gifPath);}catch(IOException){}
            }
        }
    }

    [Fact] public async Task ElapsedTimeExcludesPausedInterval()
    {
        var session=new RecordingSession(new AppSettings{RecordingFps=10,IncludeRecordingCursor=false},new ScreenRect(0,0,128,128),null);var done=new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);session.Completed+=path=>done.TrySetResult(path);session.Failed+=error=>done.TrySetException(new InvalidOperationException(error));
        try
        {
            var token=TestContext.Current.CancellationToken;session.Start();await Task.Delay(400,token);session.Pause();var beforePause=session.Elapsed;await Task.Delay(500,token);var duringPause=session.Elapsed;session.Resume();await Task.Delay(400,token);var afterResume=session.Elapsed;session.Stop();await done.Task.WaitAsync(TimeSpan.FromSeconds(20),token);
            Assert.True(beforePause>=TimeSpan.FromMilliseconds(200));Assert.InRange((duringPause-beforePause).TotalMilliseconds,0,100);Assert.True(afterResume-duringPause>=TimeSpan.FromMilliseconds(200));
        }
        finally
        {
            // Dispose the recorder before deleting its output.  The implicit
            // await-using cleanup used to run after this block and could race
            // with the file deletion while ScreenRecorderLib still held it.
            try{await session.DisposeAsync();}
            finally{try{if(File.Exists(session.VideoPath))File.Delete(session.VideoPath);}catch(IOException){}}
        }
    }

    [Fact] public async Task DisposingActiveSessionDeletesAbandonedTemporaryVideo()
    {
        var session=new RecordingSession(new AppSettings{RecordingFps=10,IncludeRecordingCursor=false},new ScreenRect(0,0,128,128),null);
        var path=string.Empty;
        try
        {
            session.Start();path=session.VideoPath;await Task.Delay(350,TestContext.Current.CancellationToken);
            var disposeCall=Stopwatch.StartNew();session.Dispose();disposeCall.Stop();
            Assert.True(disposeCall.Elapsed<TimeSpan.FromMilliseconds(500),$"Dispose 阻塞了 {disposeCall.Elapsed.TotalMilliseconds:0} ms");
            await session.DisposeAsync();
            Assert.False(File.Exists(path));
        }
        finally
        {
            session.Dispose();
            try{if(path.Length>0&&File.Exists(path))File.Delete(path);}catch(IOException){}
        }
    }

    private static RecordingRuntimeGuardOptions FastRuntimeGuardOptions(TimeSpan maximumActiveDuration)=>new()
    {
        CheckInterval=TimeSpan.FromMilliseconds(10),
        ShutdownTimeout=TimeSpan.FromMilliseconds(500),
        MaximumActiveDuration=maximumActiveDuration,
        MinimumFreeSpaceBytes=RecordingRuntimePolicy.RuntimeMinimumFreeSpaceBytes
    };

    private static async Task VerifyPreviewAutoplaysAsync(string path,CancellationToken cancellationToken)
    {
        var completion=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var thread=new Thread(() =>
        {
            VideoPreviewSurface? preview=null;
            DispatcherTimer? poll=null;
            Exception? terminalError=null;
            var completed=0;
            try
            {
                var dispatcher=Dispatcher.CurrentDispatcher;
                var view=new Image();
                var opened=false;
                var elapsed=Stopwatch.StartNew();
                preview=new VideoPreviewSurface(view,dispatcher);
                void Finish(Exception? error)
                {
                    if(Interlocked.Exchange(ref completed,1)!=0)return;
                    terminalError=error;
                    poll?.Stop();
                    dispatcher.BeginInvokeShutdown(DispatcherPriority.Send);
                }
                preview.Opened+=()=>opened=true;
                preview.Failed+=error=>Finish(error);
                poll=new DispatcherTimer(TimeSpan.FromMilliseconds(50),DispatcherPriority.Background,(_,_)=>
                {
                    if(opened&&view.Source is not null&&preview.PresentedFrameCount>=2&&preview.IsPlaying)Finish(null);
                    else if(elapsed.Elapsed>TimeSpan.FromSeconds(6))Finish(new TimeoutException("录屏视频未能在原位预览表面自动播放"));
                },dispatcher);
                preview.Load(path,autoplay:true);
                poll.Start();
                Dispatcher.Run();
            }
            catch(Exception ex){terminalError=ex;}
            finally
            {
                try{poll?.Stop();preview?.Dispose();}
                catch(Exception ex){terminalError??=ex;}
                if(terminalError is null)completion.TrySetResult();else completion.TrySetException(terminalError);
            }
        }){IsBackground=true,Name="MewuAI.VideoPreviewTest"};
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        await completion.Task.WaitAsync(TimeSpan.FromSeconds(10),cancellationToken);
    }
}
