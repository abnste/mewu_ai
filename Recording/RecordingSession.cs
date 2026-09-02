using System.Diagnostics;
using ScreenRecorderLib;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Forms=System.Windows.Forms;
using MewuScreenRect=mewu_ai_Assistant.Models.ScreenRect;
namespace mewu_ai_Assistant.Recording;
public sealed class RecordingSession : IDisposable,IAsyncDisposable
{
    private static readonly TimeSpan FileReleaseTimeout=TimeSpan.FromSeconds(10);
    private static readonly TimeSpan FileReleaseRetryDelay=TimeSpan.FromMilliseconds(100);
    private static readonly TimeSpan DeletedFileStabilityWindow=TimeSpan.FromSeconds(1);
    private static readonly TimeSpan CompletedFileWaitTimeout=TimeSpan.FromSeconds(5);
    private static readonly TimeSpan RecordingStartupStopTimeout=TimeSpan.FromSeconds(5);
    private readonly AppSettings _settings;
    private readonly MewuScreenRect _region;
    private readonly TempFileService _temp=new();
    private readonly Stopwatch _elapsed=new();
    private readonly object _stateGate=new();
    private readonly object _disposeGate=new();
    private readonly TaskCompletionSource _terminal=new(TaskCreationOptions.RunContinuationsAsynchronously);
    private readonly TaskCompletionSource _recordingReady=new(TaskCreationOptions.RunContinuationsAsynchronously);
    private readonly Action<string,Exception>? _logError;
    private readonly RecordingRuntimeGuardOptions _runtimeGuardOptions;
    private readonly RecordingTerminalState _terminalState=new();
    private Recorder? _recorder;
    private TempMediaLease? _videoLease;
    private RecordingRuntimeGuard? _runtimeGuard;
    private bool _paused;
    private int _recordingStarted;
    private int _stopRequested;
    private int _disposed;
    private Task? _cleanupTask;
    private Task? _stopTask;

    public string VideoPath { get; private set; }=string.Empty;
    public TimeSpan Elapsed { get { lock(_stateGate)return _elapsed.Elapsed; } }
    public event Action<string>? Completed;
    public event Action<string>? Failed;
    public RecordingSession(AppSettings settings,MewuScreenRect region):this(settings,region,static (component,exception)=>new PrivacyLogger().Error(component,exception),null){}
    internal RecordingSession(AppSettings settings,MewuScreenRect region,Action<string,Exception>? logError):this(settings,region,logError,null){}
    internal RecordingSession(AppSettings settings,MewuScreenRect region,Action<string,Exception>? logError,RecordingRuntimeGuardOptions? runtimeGuardOptions)
    {
        _settings=settings;
        _logError=logError;
        _runtimeGuardOptions=runtimeGuardOptions??new RecordingRuntimeGuardOptions();
        _runtimeGuardOptions.Validate();
        if(region.Width<2||region.Height<2)throw new ArgumentOutOfRangeException(nameof(region),"录屏区域至少需要 2 × 2 像素");
        _region=new MewuScreenRect(region.X,region.Y,region.Width&~1,region.Height&~1);
    }
    public void Start()
    {
        ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed)!=0,this);
        if(_recorder is not null)throw new InvalidOperationException("录屏会话已经启动");
        RecordingRuntimePolicy.EnsureEnoughSpaceToStart(_runtimeGuardOptions.AvailableFreeSpaceProbe(_temp.DirectoryPath));
        VideoPath=_temp.NewFile(".mp4");
        _videoLease=TempMediaRegistry.Shared.Acquire(VideoPath);
        try{StartRecorder();}
        catch
        {
            _terminalState.TryFail();StopElapsed();
            _terminal.TrySetResult();
            _=EnsureCleanupStarted();
            throw;
        }
    }
    private void StartRecorder()
    {
        var screens=Forms.Screen.AllScreens;var displayRects=screens.Select(x=>new MewuScreenRect(x.Bounds.X,x.Bounds.Y,x.Bounds.Width,x.Bounds.Height)).ToArray();var slices=RecordingLayoutService.CreateSlices(_region,displayRects);var sources=new List<RecordingSourceBase>();foreach(var slice in slices){var screen=screens[Array.IndexOf(displayRects,slice.Display)];sources.Add(new DisplayRecordingSource(screen.DeviceName){SourceRect=new ScreenRecorderLib.ScreenRect(slice.Source.X-slice.Display.X,slice.Source.Y-slice.Display.Y,slice.Source.Width,slice.Source.Height),Position=new ScreenPoint(slice.Output.X,slice.Output.Y),OutputSize=new ScreenSize(slice.Output.Width,slice.Output.Height),IsCursorCaptureEnabled=_settings.IncludeRecordingCursor,IsBorderRequired=false});}if(sources.Count==0)throw new InvalidOperationException("选区不在可录制显示器范围内");
        var options=new RecorderOptions{SourceOptions=new SourceOptions{RecordingSources=sources},OutputOptions=new OutputOptions{RecorderMode=RecorderMode.Video,OutputFrameSize=new ScreenSize(_region.Width,_region.Height)},VideoEncoderOptions=new VideoEncoderOptions{Encoder=new H264VideoEncoder(),Framerate=Math.Clamp(_settings.RecordingFps,10,60),Quality=Math.Clamp(_settings.RecordingQuality,20,100),IsHardwareEncodingEnabled=true,IsFixedFramerate=true},AudioOptions=new AudioOptions{IsAudioEnabled=false},MouseOptions=new MouseOptions{IsMousePointerEnabled=_settings.IncludeRecordingCursor}};
        _recorder=Recorder.CreateRecorder(options);
        _recorder.OnRecordingComplete+=(_,e)=>
        {
            try
            {
                Interlocked.Exchange(ref _stopRequested,1);StopElapsed();CancelRuntimeGuard();
                if(Volatile.Read(ref _disposed)!=0)return;
                var outputPath=string.IsNullOrWhiteSpace(e.FilePath)?VideoPath:e.FilePath;
                if(string.IsNullOrWhiteSpace(outputPath)||!WaitForCompletedFile(outputPath)||!AdoptCompletedOutputPath(outputPath))
                {
                    TryReportFailure("录屏输出文件为空");
                    return;
                }
                if(_terminalState.TryComplete())
                {
                    try{Completed?.Invoke(VideoPath);}
                    catch(Exception ex){Log("RecordingCompletedHandler",ex);}
                }
            }
            finally{_terminal.TrySetResult();}
        };
        _recorder.OnRecordingFailed+=(_,e)=>
        {
            try
            {
                Interlocked.Exchange(ref _stopRequested,1);StopElapsed();CancelRuntimeGuard();
                if(Volatile.Read(ref _disposed)==0)TryReportFailure(e.Error);
            }
            finally{_terminal.TrySetResult();}
        };
        _recorder.OnStatusChanged+=(_,e)=>
        {
            if(e.Status==RecorderStatus.Recording)_recordingReady.TrySetResult();
        };
        lock(_stateGate){_paused=false;_elapsed.Restart();}
        // ScreenRecorderLib's path overload lets Media Foundation own the
        // file stream and finalize the MP4 directly. Its managed Stream
        // bridge can complete with a zero-byte FileStream on current .NET;
        // no custom stream sharing is needed because cleanup happens only
        // after the recorder has stopped and been disposed.
        _recorder.Record(VideoPath);
        Volatile.Write(ref _recordingStarted,1);
        StartRuntimeGuard();
    }
    private void StartRuntimeGuard()
    {
        var runtimeGuard=new RecordingRuntimeGuard(
            ()=>Elapsed,
            ()=>_runtimeGuardOptions.AvailableFreeSpaceProbe(_temp.DirectoryPath),
            _runtimeGuardOptions,
            StopForRuntimeFailure,
            exception=>Log("RecordingRuntimeGuard",exception));
        var installed=false;
        lock(_disposeGate)
        {
            if(_cleanupTask is null&&Volatile.Read(ref _disposed)==0&&Volatile.Read(ref _stopRequested)==0&&_terminalState.Current==RecordingTerminalResult.Pending)
            {
                _runtimeGuard=runtimeGuard;
                runtimeGuard.Start();
                installed=true;
            }
        }
        if(!installed)_=runtimeGuard.DisposeAsync();
    }
    private bool IsPaused { get { lock(_stateGate)return _paused; } }
    private void StopElapsed(){lock(_stateGate)_elapsed.Stop();}
    public void Stop(){if(Volatile.Read(ref _disposed)!=0||Interlocked.CompareExchange(ref _stopRequested,1,0)!=0)return;StopElapsed();CancelRuntimeGuard();_=EnsureRecorderStopStarted(_recorder);}
    // 暂停期间 Stopwatch 停止，因此不计入最大录制时长；运行保护任务本身不停，仍会检查磁盘。
    public void Pause(){if(Volatile.Read(ref _disposed)!=0||Volatile.Read(ref _stopRequested)!=0||IsPaused)return;_recorder?.Pause();lock(_stateGate){_paused=true;_elapsed.Stop();}}
    public void Resume(){if(Volatile.Read(ref _disposed)!=0||Volatile.Read(ref _stopRequested)!=0||!IsPaused)return;_recorder?.Resume();lock(_stateGate){_paused=false;_elapsed.Start();}}
    private void StopForRuntimeFailure(string error)
    {
        if(Volatile.Read(ref _disposed)!=0||Interlocked.CompareExchange(ref _stopRequested,1,0)!=0)return;
        StopElapsed();
        if(!TryReportFailure(error))return;
        _=EnsureRecorderStopStarted(_recorder);
    }
    private bool TryReportFailure(string error)
    {
        if(!_terminalState.TryFail())return false;
        try{Failed?.Invoke(error);}
        catch(Exception ex){Log("RecordingFailedHandler",ex);}
        return true;
    }
    private void CancelRuntimeGuard()
    {
        try{Volatile.Read(ref _runtimeGuard)?.Cancel();}
        catch(ObjectDisposedException){}
    }
    public TempMediaLease RetainCompletedVideo()
    {
        if(_terminalState.Current!=RecordingTerminalResult.Completed||string.IsNullOrWhiteSpace(VideoPath))throw new InvalidOperationException("录屏尚未成功完成，无法保留视频");
        return TempMediaRegistry.Shared.AcquireExistingFile(VideoPath);
    }

    /// <summary>
    /// ScreenRecorderLib normally writes to the requested path, but some
    /// Media Foundation sinks report the finalized path in the completion
    /// callback.  Make that callback path the single source of truth and move
    /// the registry lease before exposing the completed event.
    /// </summary>
    private bool AdoptCompletedOutputPath(string outputPath)
    {
        try
        {
            var normalized=Path.GetFullPath(outputPath);
            if(!File.Exists(normalized))return false;
            if(string.Equals(Path.GetFullPath(VideoPath),normalized,StringComparison.OrdinalIgnoreCase))
            {
                VideoPath=normalized;
                return true;
            }

            var replacement=TempMediaRegistry.Shared.AcquireExistingFile(normalized);
            var previous=Interlocked.Exchange(ref _videoLease,replacement);
            VideoPath=normalized;
            previous?.Dispose();
            return true;
        }
        catch(Exception ex)
        {
            Log("RecordingOutputPath",ex);
            return false;
        }
    }
    public void Dispose()
    {
        _=EnsureCleanupStarted();
    }
    public async ValueTask DisposeAsync()=>await EnsureCleanupStarted().ConfigureAwait(false);
    private Task EnsureCleanupStarted()
    {
        lock(_disposeGate)
        {
            if(_cleanupTask is not null)return _cleanupTask;
            Interlocked.Exchange(ref _disposed,1);
            Interlocked.Exchange(ref _stopRequested,1);
            _terminalState.TryDispose();
            StopElapsed();
            CancelRuntimeGuard();
            var recorder=_recorder;
            var runtimeGuard=_runtimeGuard;
            _cleanupTask=Task.Run(()=>CleanupAsync(recorder,runtimeGuard));
            return _cleanupTask;
        }
    }
    private async Task CleanupAsync(Recorder? recorder,RecordingRuntimeGuard? runtimeGuard)
    {
        await EnsureRecorderStopStarted(recorder).ConfigureAwait(false);
        if(runtimeGuard is not null)
        {
            try{await runtimeGuard.DisposeAsync().ConfigureAwait(false);}
            catch(Exception ex){Log("RecordingRuntimeGuardDispose",ex);}
        }
        if(Volatile.Read(ref _recordingStarted)!=0&&!_terminal.Task.IsCompleted)
        {
            try{await _terminal.Task.WaitAsync(TimeSpan.FromSeconds(15)).ConfigureAwait(false);}
            catch(Exception ex)when(ex is TimeoutException or OperationCanceledException){Log("RecordingStopWait",ex);}
        }
        try{recorder?.Dispose();}
        catch(Exception ex){Log("RecordingDispose",ex);}
        finally
        {
            try{await DeleteIncompleteOutputAsync().ConfigureAwait(false);}
            finally{Interlocked.Exchange(ref _videoLease,null)?.Dispose();}
        }
    }
    private Task EnsureRecorderStopStarted(Recorder? recorder)
    {
        lock(_disposeGate)return _stopTask??=StopRecorderWhenReadyAsync(recorder);
    }
    private async Task StopRecorderWhenReadyAsync(Recorder? recorder)
    {
        if(recorder is null)return;
        try
        {
            // ScreenRecorderLib 7 can leave its path output permanently locked
            // when Stop is called while its asynchronous Media Foundation sink
            // is still starting. Wait for the first Recording status (or a
            // terminal callback) before stopping, with a finite upper bound.
            if(Volatile.Read(ref _recordingStarted)!=0&&!_recordingReady.Task.IsCompleted&&!_terminal.Task.IsCompleted)
                await Task.WhenAny(_recordingReady.Task,_terminal.Task,Task.Delay(RecordingStartupStopTimeout)).ConfigureAwait(false);
            if(!_terminal.Task.IsCompleted)recorder.Stop();
        }
        catch(Exception ex)
        {
            Log("RecordingStop",ex);
            _terminal.TrySetResult();
        }
    }
    private async Task DeleteIncompleteOutputAsync()
    {
        if(_terminalState.Current==RecordingTerminalResult.Completed||string.IsNullOrWhiteSpace(VideoPath))return;
        var path=VideoPath;
        var started=Stopwatch.GetTimestamp();
        long absentSince=0;
        do
        {
            if(_terminalState.Current==RecordingTerminalResult.Completed)return;
            if(File.Exists(path))
            {
                absentSince=0;
                TryDelete(path);
            }
            else
            {
                if(absentSince==0)absentSince=Stopwatch.GetTimestamp();
                else if(Stopwatch.GetElapsedTime(absentSince)>=DeletedFileStabilityWindow)return;
            }
            await Task.Delay(FileReleaseRetryDelay).ConfigureAwait(false);
        }while(Stopwatch.GetElapsedTime(started)<FileReleaseTimeout);
        if(File.Exists(path))Log("RecordingCleanup",new IOException("无法清理未完成的临时录屏文件"));
    }
    private static bool TryDelete(string path)
    {
        try{if(File.Exists(path))File.Delete(path);return true;}
        catch(IOException){return false;}
        catch(UnauthorizedAccessException){return false;}
    }
    private static bool WaitForCompletedFile(string path)
    {
        var started=Stopwatch.GetTimestamp();
        long previousLength=-1;
        var stableReads=0;
        do
        {
            try
            {
                if(File.Exists(path))
                {
                    // ScreenRecorderLib raises completion while its native
                    // sink may still own the file handle.  Inspecting the
                    // metadata length does not require opening that handle;
                    // requiring FileShare.Read here would turn a valid
                    // recording into a false failure.  The overlay awaits
                    // DisposeAsync (which releases Media Foundation) before
                    // handing the file to the decoder.
                    var length=new FileInfo(path).Length;
                    if(length>0&&length==previousLength)stableReads++;
                    else stableReads=0;
                    previousLength=length;
                    if(length>0&&stableReads>=2)return true;
                }
            }
            catch(IOException){}
            catch(UnauthorizedAccessException){}
            Thread.Sleep(FileReleaseRetryDelay);
        }while(Stopwatch.GetElapsedTime(started)<CompletedFileWaitTimeout);
        return false;
    }
    private void Log(string component,Exception exception){try{_logError?.Invoke(component,exception);}catch{}}
}

internal enum RecordingTerminalResult
{
    Pending,
    Completed,
    Failed,
    Disposed
}

internal sealed class RecordingTerminalState
{
    private int _value;
    internal RecordingTerminalResult Current=>(RecordingTerminalResult)Volatile.Read(ref _value);
    internal bool TryComplete()=>TrySet(RecordingTerminalResult.Completed);
    internal bool TryFail()=>TrySet(RecordingTerminalResult.Failed);
    internal bool TryDispose()=>TrySet(RecordingTerminalResult.Disposed);
    private bool TrySet(RecordingTerminalResult result)=>Interlocked.CompareExchange(ref _value,(int)result,(int)RecordingTerminalResult.Pending)==(int)RecordingTerminalResult.Pending;
}

internal static class RecordingRuntimePolicy
{
    internal const long StartupMinimumFreeSpaceBytes=500L*1024*1024;
    internal const long RuntimeMinimumFreeSpaceBytes=256L*1024*1024;
    internal static readonly TimeSpan MaximumActiveDuration=TimeSpan.FromMinutes(30);
    internal static readonly TimeSpan CheckInterval=TimeSpan.FromSeconds(5);
    internal static readonly TimeSpan ShutdownTimeout=TimeSpan.FromSeconds(2);

    internal static void EnsureEnoughSpaceToStart(long availableFreeSpaceBytes)
    {
        if(availableFreeSpaceBytes<StartupMinimumFreeSpaceBytes)throw new IOException("磁盘剩余空间不足 500 MB，无法安全开始录屏");
    }

    internal static string DurationLimitMessage(TimeSpan maximumActiveDuration)
    {
        var duration=maximumActiveDuration.TotalMinutes>=1
            ? $"{maximumActiveDuration.TotalMinutes:0.#} 分钟"
            : $"{Math.Max(1,maximumActiveDuration.TotalSeconds):0.#} 秒";
        return $"录屏已达到 {duration} 的安全时长上限，已自动停止；请分段录制";
    }

    internal static string FreeSpaceLimitMessage(long minimumFreeSpaceBytes)
    {
        var mebibytes=Math.Max(1,minimumFreeSpaceBytes/(1024*1024));
        return $"磁盘剩余空间已降到 {mebibytes} MB 的安全阈值，录屏已自动停止；请释放空间后重试";
    }
}

internal sealed class RecordingRuntimeGuardOptions
{
    internal Func<string,long> AvailableFreeSpaceProbe { get; init; }=GetAvailableFreeSpace;
    internal long MinimumFreeSpaceBytes { get; init; }=RecordingRuntimePolicy.RuntimeMinimumFreeSpaceBytes;
    internal TimeSpan MaximumActiveDuration { get; init; }=RecordingRuntimePolicy.MaximumActiveDuration;
    internal TimeSpan CheckInterval { get; init; }=RecordingRuntimePolicy.CheckInterval;
    internal TimeSpan ShutdownTimeout { get; init; }=RecordingRuntimePolicy.ShutdownTimeout;

    internal void Validate()
    {
        ArgumentNullException.ThrowIfNull(AvailableFreeSpaceProbe);
        if(MinimumFreeSpaceBytes<=0)throw new ArgumentOutOfRangeException(nameof(MinimumFreeSpaceBytes));
        if(MaximumActiveDuration<=TimeSpan.Zero)throw new ArgumentOutOfRangeException(nameof(MaximumActiveDuration));
        if(CheckInterval<=TimeSpan.Zero)throw new ArgumentOutOfRangeException(nameof(CheckInterval));
        if(ShutdownTimeout<=TimeSpan.Zero)throw new ArgumentOutOfRangeException(nameof(ShutdownTimeout));
    }

    private static long GetAvailableFreeSpace(string directoryPath)
    {
        var root=Path.GetPathRoot(Path.GetFullPath(directoryPath));
        if(string.IsNullOrWhiteSpace(root))throw new IOException("无法确定录屏临时目录所在磁盘");
        return new DriveInfo(root).AvailableFreeSpace;
    }
}

internal sealed class RecordingRuntimeGuard : IAsyncDisposable
{
    private readonly Func<TimeSpan> _activeElapsedProbe;
    private readonly Func<long> _availableFreeSpaceProbe;
    private readonly RecordingRuntimeGuardOptions _options;
    private readonly Action<string> _limitReached;
    private readonly Action<Exception>? _logError;
    private readonly CancellationTokenSource _stop=new();
    private readonly object _disposeGate=new();
    private Task? _monitorTask;
    private Task? _disposeTask;
    private int _started;
    private int _limitSignaled;

    internal RecordingRuntimeGuard(Func<TimeSpan> activeElapsedProbe,Func<long> availableFreeSpaceProbe,RecordingRuntimeGuardOptions options,Action<string> limitReached,Action<Exception>? logError=null)
    {
        _activeElapsedProbe=activeElapsedProbe??throw new ArgumentNullException(nameof(activeElapsedProbe));
        _availableFreeSpaceProbe=availableFreeSpaceProbe??throw new ArgumentNullException(nameof(availableFreeSpaceProbe));
        _options=options??throw new ArgumentNullException(nameof(options));
        _limitReached=limitReached??throw new ArgumentNullException(nameof(limitReached));
        _logError=logError;
        _options.Validate();
    }

    internal Task Completion=>Volatile.Read(ref _monitorTask)??Task.CompletedTask;

    internal void Start()
    {
        if(Interlocked.CompareExchange(ref _started,1,0)!=0)throw new InvalidOperationException("录屏运行保护已经启动");
        lock(_disposeGate)
        {
            if(_disposeTask is not null)throw new ObjectDisposedException(nameof(RecordingRuntimeGuard));
            _monitorTask=MonitorAsync(_stop.Token);
        }
    }

    internal void Cancel()
    {
        try{_stop.Cancel();}
        catch(ObjectDisposedException){}
    }

    public ValueTask DisposeAsync()=>new(EnsureDisposeStarted());

    private Task EnsureDisposeStarted()
    {
        lock(_disposeGate)return _disposeTask??=DisposeCoreAsync();
    }

    private async Task DisposeCoreAsync()
    {
        Cancel();
        var monitorTask=Volatile.Read(ref _monitorTask);
        if(monitorTask is not null)
        {
            try{await monitorTask.WaitAsync(_options.ShutdownTimeout).ConfigureAwait(false);}
            catch(TimeoutException ex){Log(ex);}
        }
        _stop.Dispose();
    }

    private async Task MonitorAsync(CancellationToken cancellationToken)
    {
        try
        {
            while(true)
            {
                await Task.Delay(_options.CheckInterval,cancellationToken).ConfigureAwait(false);
                var activeElapsed=_activeElapsedProbe();
                if(activeElapsed>=_options.MaximumActiveDuration)
                {
                    SignalLimit(RecordingRuntimePolicy.DurationLimitMessage(_options.MaximumActiveDuration));
                    return;
                }
                if(_availableFreeSpaceProbe()<=_options.MinimumFreeSpaceBytes)
                {
                    SignalLimit(RecordingRuntimePolicy.FreeSpaceLimitMessage(_options.MinimumFreeSpaceBytes));
                    return;
                }
            }
        }
        catch(OperationCanceledException)when(cancellationToken.IsCancellationRequested){}
        catch(Exception ex)
        {
            Log(ex);
            SignalLimit("录屏期间无法检查磁盘剩余空间，已为安全起见自动停止");
        }
    }

    private void SignalLimit(string message)
    {
        if(Interlocked.CompareExchange(ref _limitSignaled,1,0)!=0)return;
        try{_limitReached(message);}
        catch(Exception ex){Log(ex);}
    }

    private void Log(Exception exception){try{_logError?.Invoke(exception);}catch{}}
}
