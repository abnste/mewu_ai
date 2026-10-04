// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Diagnostics;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using Microsoft.Graphics.Canvas;
using mewu_ai_Assistant.Models;
using Windows.Graphics.Imaging;
using Windows.Media.Core;
using Windows.Media.Playback;
using WinMediaPlayer = Windows.Media.Playback.MediaPlayer;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Renders a local video into a WPF Image using the WinRT MediaPlayer frame
/// server. WPF's MediaElement depends on the legacy Windows Media Player
/// runtime and fails on current machines where that optional component is not
/// installed. The frame server gives us decoded BGRA pixels while keeping the
/// player and the image in the same overlay/window.
/// </summary>
internal sealed class VideoPreviewSurface : IDisposable
{
    private const int MaxVideoDimension = 8192;
    private const int MaxPreviewLongEdge = 1280;
    private static readonly TimeSpan SeekPresentationTimeout=TimeSpan.FromSeconds(20);
    private static readonly long MinimumFrameIntervalTicks = Math.Max(1, Stopwatch.Frequency / 15);
    private readonly Image _view;
    private readonly Dispatcher _dispatcher;
    private readonly object _frameGate = new();
    private readonly SemaphoreSlim _seekGate=new(1,1);
    private CancellationTokenSource? _sourceCancellation;
    private CancellationTokenSource? _presentationCancellation=new();
    private VideoClipRange? _playbackRange;
    private long _nativeFrameIntervalTicks=TimeSpan.TicksPerMillisecond;
    private WinMediaPlayer? _player;
    private CanvasBitmap? _surface;
    private CanvasDevice? _device;
    private WriteableBitmap? _displayFrame;
    private byte[]? _latestPixels;
    private long _latestFrameGeneration;
    private long _latestFramePositionTicks;
    private long _lastPresentedPositionTicks;
    private long _lastAcceptedFrameTimestamp;
    private long _presentedFrameCount;
    private long _decodedFrameCount;
    private long _presentationVersion;
    private bool _holdPresentation;
    private DecodedFrame? _stagedFrame;
    private string? _sourcePath;
    private TimeSpan? _initialPosition;
    private SettledSeek? _settledSeek;
    private event Action? FrameDecoded;
    private sealed record DecodedFrame(byte[] Pixels, int Width, int Height, long PositionTicks);
    private readonly record struct SettledSeek(long Generation,long TargetTicks,long NativeTicks,long PresentedTicks,long FrameCount);
    private int _width;
    private int _height;
    private int _frameBusy;
    private int _frameDispatchPending;
    private int _forceNextFrame;
    private long _generation;
    private long _failureGeneration=-1;
    private bool _disposed;
    private bool _playing;

    internal VideoPreviewSurface(Image view, Dispatcher dispatcher)
    {
        _view = view ?? throw new ArgumentNullException(nameof(view));
        _dispatcher = dispatcher ?? throw new ArgumentNullException(nameof(dispatcher));
        _view.Stretch = Stretch.Fill;
        _view.IsHitTestVisible = false;
    }

    internal bool IsPlaying => _playing;
    internal TimeSpan Duration=>_player?.PlaybackSession.NaturalDuration??TimeSpan.Zero;
    internal VideoClipRange? PlaybackRange=>_playbackRange;
    internal TimeSpan Position
    {
        get
        {
            VerifyDispatcher();
            return _player is { } player?ToSourcePosition(player.PlaybackSession.Position,_playbackRange):TimeSpan.Zero;
        }
    }
    internal long PresentedFrameCount => Interlocked.Read(ref _presentedFrameCount);
    internal TimeSpan LastPresentedPosition=>TimeSpan.FromTicks(Math.Max(0,Interlocked.Read(ref _lastPresentedPositionTicks)));

    internal event Action? Opened;
    internal event Action<Exception>? Failed;
    internal event Action? Ended;
    internal event Action<TimeSpan>? FramePresented;

    internal void Load(string path, bool autoplay,VideoClipRange? playbackRange=null,TimeSpan? initialPosition=null)
    {
        CrashDiagnosticsService.MarkOperation("视频预览：加载媒体");
        VerifyDispatcher();
        ObjectDisposedException.ThrowIf(_disposed, this);
        var normalized = Path.GetFullPath(path);
        if (!File.Exists(normalized))
            throw new FileNotFoundException("录屏文件已不可用", normalized);
        if(playbackRange is { } range&&(range.Start<TimeSpan.Zero||range.End<=range.Start))
            throw new ArgumentOutOfRangeException(nameof(playbackRange));

        // Range editing reloads the same media. Keep the last displayed frame
        // until the new decoder has settled at the requested source position.
        var retainFrame=initialPosition.HasValue&&StringComparer.OrdinalIgnoreCase.Equals(normalized,_sourcePath);
        CloseSourceCore(retainFrame);
        _sourcePath=normalized;
        _initialPosition=initialPosition;
        _holdPresentation=initialPosition.HasValue;
        _sourceCancellation=new CancellationTokenSource();
        _playbackRange=playbackRange;
        if(!retainFrame)Interlocked.Exchange(ref _lastPresentedPositionTicks,playbackRange?.Start.Ticks??0);
        _playing = autoplay;
        Interlocked.Exchange(ref _failureGeneration,-1);
        var player = new WinMediaPlayer
        {
            IsMuted = false,
            AutoPlay = false,
            IsLoopingEnabled = true
        };
        _player = player;
        player.MediaOpened += OnMediaOpened;
        player.MediaFailed += OnMediaFailed;
        player.MediaEnded += OnMediaEnded;
        player.VideoFrameAvailable += OnVideoFrameAvailable;
        try
        {
            // Frame-server mode must be enabled before playback starts. In
            // this mode MediaPlayer intentionally does not use a native visual;
            // every available frame is copied to the WPF Image below.
            player.IsVideoFrameServerEnabled = true;
            var source=MediaSource.CreateFromUri(new Uri(normalized, UriKind.Absolute));
            player.Source=playbackRange is { } bounded
                ?new MediaPlaybackItem(source,bounded.Start,bounded.Duration):new MediaPlaybackItem(source);
            if (autoplay&&!initialPosition.HasValue) player.Play();
        }
        catch (Exception ex)
        {
            CloseSourceCore();
            // Load is dispatcher-affine, so report the synchronous failure
            // before returning.  Raising through a later dispatcher callback
            // after CloseSourceCore could let a subsequent Load receive a
            // stale error from the previous source.
            if (!_disposed) Failed?.Invoke(ex);
            throw;
        }
    }

    internal void Play()
    {
        CrashDiagnosticsService.MarkOperation("视频预览：正在播放");
        VerifyDispatcher();
        ObjectDisposedException.ThrowIf(_disposed, this);
        var player = _player ?? throw new InvalidOperationException("视频尚未加载");
        CancelPresentationWaits();
        _initialPosition=null;
        _holdPresentation=false;
        _stagedFrame=null;
        player.Play();
        _playing = true;
    }

    internal void Pause()
    {
        CrashDiagnosticsService.MarkOperation("视频预览：暂停");
        VerifyDispatcher();
        if (_disposed) return;
        _player?.Pause();
        _playing = false;
    }

    internal async Task<TimeSpan> SeekAsync(TimeSpan position,bool pauseAfterSeek,CancellationToken cancellationToken=default)
    {
        CrashDiagnosticsService.MarkOperation("视频预览：跳转时间轴");
        VerifyDispatcher();
        ObjectDisposedException.ThrowIf(_disposed,this);
        var player=_player??throw new InvalidOperationException("视频尚未加载");
        var generation=Volatile.Read(ref _generation);
        using var linked=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken,_sourceCancellation!.Token,_presentationCancellation!.Token);
        await _seekGate.WaitAsync(linked.Token);
        try
        {
            EnsureCurrentPlayer(player,generation,linked.Token);
            return await SeekCoreAsync(player,generation,position,pauseAfterSeek,linked.Token);
        }
        finally{_seekGate.Release();}
    }

    private async Task<TimeSpan> SeekCoreAsync(WinMediaPlayer player,long generation,TimeSpan position,bool pauseAfterSeek,CancellationToken cancellationToken)
    {
        var session=player.PlaybackSession;
        if(!session.CanSeek)throw new InvalidOperationException("当前视频不支持跳转");
        var offset=_playbackRange?.Start??TimeSpan.Zero;
        position=position<=offset?TimeSpan.Zero:position-offset;
        var duration=session.NaturalDuration;
        if(duration>TimeSpan.Zero&&position>duration)position=duration;
        // A video's final frame may end before its nominal duration. A native
        // end seek at the physical EOF can return the old surface with an EOF
        // timestamp; stepping beyond that point may produce no further frame.
        // Decode from just before the final frame and step into it instead.
        // Interior clip ends retain the native bounded-item end seek.
        // Keep source-start seeks at zero, including clips shorter than a frame.
        var frameIntervalTicks=Interlocked.Read(ref _nativeFrameIntervalTicks);
        var tailSeek=pauseAfterSeek&&duration>TimeSpan.Zero&&position>TimeSpan.Zero&&
            duration.Ticks-position.Ticks<=frameIntervalTicks;
        if(tailSeek)
        {
            if(IsPhysicalSourceTail(player,frameIntervalTicks))
            {
                var leadTicks=Math.Min(frameIntervalTicks,duration.Ticks);
                var halfLeadTicks=leadTicks/2+leadTicks%2;
                position=TimeSpan.FromTicks(Math.Max(0,duration.Ticks-leadTicks-halfLeadTicks));
            }
            else position=duration;
        }
        if(pauseAfterSeek){player.Pause();_playing=false;}
        if(pauseAfterSeek&&_settledSeek is { } prior&&prior.Generation==generation&&
           prior.TargetTicks==position.Ticks&&prior.FrameCount>0&&prior.FrameCount==PresentedFrameCount&&
           prior.NativeTicks==session.Position.Ticks&&prior.PresentedTicks==LastPresentedPosition.Ticks)
        {
            // Repeating an already decoded target can complete the native seek
            // without producing another step frame. Reuse only that successful
            // seek's unchanged, actually presented frame, never a timestamp
            // merely close to the new request or to the end of the video.
            return TimeSpan.FromTicks(prior.PresentedTicks);
        }
        _settledSeek=null;
        // Paused seeking can first decode a GOP reference frame. Stage these
        // intermediate frames rather than briefly flashing them on screen.
        var presentationVersion=++_presentationVersion;
        void EnsurePresentationCurrent()
        {
            EnsureCurrentPlayer(player,generation,cancellationToken);
            if(presentationVersion!=_presentationVersion)throw new OperationCanceledException(cancellationToken);
        }
        _holdPresentation=true;
        _stagedFrame=null;
        var previousFrameCount=Interlocked.Read(ref _decodedFrameCount);
        var completion=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var frameReady=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        Windows.Foundation.TypedEventHandler<MediaPlaybackSession,object>? handler=null;
        Action? frameHandler=null;
        Action? steppedFrameHandler=null;
        handler=(_,_)=>{if(IsCurrentPlayer(player,generation))completion.TrySetResult();};
        frameHandler=()=>{if(IsCurrentPlayer(player,generation))frameReady.TrySetResult();};
        session.SeekCompleted+=handler;
        FrameDecoded+=frameHandler;
        var seekTimer=Stopwatch.StartNew();
        TimeSpan RemainingTimeout()
        {
            var remaining=SeekPresentationTimeout-seekTimer.Elapsed;
            if(remaining<=TimeSpan.Zero)throw new TimeoutException("视频跳转后未能在限定时间内呈现目标帧");
            return remaining;
        }
        var settled=false;
        try
        {
            if(session.Position!=position)
            {
                // This seek may provide the only final frame. Do not let the
                // ordinary playback throttle discard it after a paused load.
                if(tailSeek)ResetPendingFrameForStep();
                session.Position=position;
                await completion.Task.WaitAsync(RemainingTimeout(),cancellationToken);
            }
            EnsurePresentationCurrent();
            // Microsoft documents that a paused MediaPlayer can report an
            // imprecise frame position after seeking. Advancing one decoded
            // frame after Pause makes the frame-server surface settle on the
            // requested visual frame instead of retaining the previous GOP
            // frame while the session position has already changed.
            if(pauseAfterSeek&&(duration<=TimeSpan.Zero||position<duration))
            {
                var steppedFrameReady=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
                steppedFrameHandler=()=>{if(IsCurrentPlayer(player,generation))steppedFrameReady.TrySetResult();};
                FrameDecoded+=steppedFrameHandler;
                ResetPendingFrameForStep();
                player.StepForwardOneFrame();
                await steppedFrameReady.Task.WaitAsync(RemainingTimeout(),cancellationToken);
            }
            else if(Interlocked.Read(ref _decodedFrameCount)<=previousFrameCount)
                await frameReady.Task.WaitAsync(RemainingTimeout(),cancellationToken);
            EnsurePresentationCurrent();
            if(_stagedFrame is { } frame)PresentFrame(frame);
            settled=true;
            var presented=LastPresentedPosition;
            if(pauseAfterSeek&&IsCurrentPlayer(player,generation)&&presentationVersion==_presentationVersion&&
               !cancellationToken.IsCancellationRequested&&PresentedFrameCount>0)
                _settledSeek=new(generation,position.Ticks,session.Position.Ticks,presented.Ticks,PresentedFrameCount);
            return presented;
        }
        finally
        {
            // A source replacement cancels this seek and disposes its native
            // player before the awaited continuation gets back to the UI.
            try{session.SeekCompleted-=handler;}catch(InvalidOperationException){}catch(System.Runtime.InteropServices.COMException){}
            FrameDecoded-=frameHandler;
            if(steppedFrameHandler is not null)FrameDecoded-=steppedFrameHandler;
            // An old cancelled seek must not release a newer load's hold.
            if(IsCurrentPlayer(player,generation)&&presentationVersion==_presentationVersion)
            {
                if(!settled)_settledSeek=null;
                _holdPresentation=!settled;
                _stagedFrame=null;
            }
        }
    }

    private bool IsPhysicalSourceTail(WinMediaPlayer player,long frameIntervalTicks)
    {
        if(_playbackRange is not { } range)return true;
        // The already opened MediaSource retains the original file duration;
        // no extra file or metadata reader is needed for a start-only trim.
        return player.Source is MediaPlaybackItem item&&item.Source.Duration is { } sourceDuration&&
            sourceDuration>TimeSpan.Zero&&range.End.Ticks>=sourceDuration.Ticks-Math.Min(sourceDuration.Ticks,frameIntervalTicks);
    }

    internal void Stop()
    {
        VerifyDispatcher();
        if (_disposed) return;
        CancelPresentationWaits();
        _initialPosition=null;
        _holdPresentation=false;
        _stagedFrame=null;
        var player = _player;
        if (player is not null)
        {
            try { player.Pause(); } catch { }
            try { player.PlaybackSession.Position = TimeSpan.Zero; } catch { }
        }
        _playing = false;
    }

    internal void CloseSource()
    {
        VerifyDispatcher();
        if (_disposed) return;
        CloseSourceCore();
    }

    private void OnMediaOpened(WinMediaPlayer sender, object args)
    {
        try
        {
            var naturalWidth = checked((int)sender.PlaybackSession.NaturalVideoWidth);
            var naturalHeight = checked((int)sender.PlaybackSession.NaturalVideoHeight);
            if (naturalWidth <= 0 || naturalHeight <= 0 || naturalWidth > MaxVideoDimension || naturalHeight > MaxVideoDimension)
                throw new InvalidDataException("视频没有可显示的画面尺寸");
            var (width,height)=CalculatePreviewSize(naturalWidth,naturalHeight);
            var frameIntervalTicks=ReadNativeFrameIntervalTicks(sender);

            lock (_frameGate)
            {
                if (_disposed || !ReferenceEquals(_player, sender)) return;
                _width = width;
                _height = height;
                Interlocked.Exchange(ref _nativeFrameIntervalTicks,frameIntervalTicks);
                _device ??= CanvasDevice.GetSharedDevice();
                _surface?.Dispose();
                // CanvasBitmap implements IDirect3DSurface, the destination
                // required by CopyFrameToVideoSurface.
                using var bitmap = new SoftwareBitmap(BitmapPixelFormat.Bgra8, width, height, BitmapAlphaMode.Ignore);
                _surface = CanvasBitmap.CreateFromSoftwareBitmap(_device, bitmap);
            }
            var generation=Volatile.Read(ref _generation);
            RaiseOnUi(()=>_ = CompleteOpenedAsync(sender,generation));
        }
        catch (Exception ex)
        {
            RaiseFailed(sender,ex);
        }
    }

    private static long ReadNativeFrameIntervalTicks(WinMediaPlayer player)
    {
        // The decoder has already opened these tracks. Reading their encoding
        // properties once avoids opening the source file again on every seek.
        try
        {
            if(player.Source is MediaPlaybackItem item)
            {
                var tracks=item.VideoTracks;
                var index=tracks.SelectedIndex;
                if(index<0&&tracks.Count==1)index=0;
                if(index>=0&&index<tracks.Count)
                {
                    var rate=tracks[index].GetEncodingProperties().FrameRate;
                    if(rate.Numerator>0&&rate.Denominator>0)
                        return Math.Max(1,(long)Math.Ceiling(TimeSpan.TicksPerSecond*(double)rate.Denominator/rate.Numerator));
                }
            }
        }
        catch(InvalidOperationException){ }
        catch(System.Runtime.InteropServices.COMException){ }
        return TimeSpan.TicksPerMillisecond;
    }

    private async Task CompleteOpenedAsync(WinMediaPlayer player,long generation)
    {
        if(!IsCurrentPlayer(player,generation))return;
        var sourceToken=_sourceCancellation!.Token;
        using var presentation=CancellationTokenSource.CreateLinkedTokenSource(sourceToken,_presentationCancellation!.Token);
        var token=presentation.Token;
        try
        {
            // Some Windows builds ignore Play before MediaOpened. A bounded,
            // paused load instead decodes one frame before announcing ready.
            if(_initialPosition is { } initialPosition)
            {
                var autoplay=_playing;
                await _seekGate.WaitAsync(token);
                try
                {
                    EnsureCurrentPlayer(player,generation,token);
                    await SeekCoreAsync(player,generation,initialPosition,true,token);
                    EnsureCurrentPlayer(player,generation,token);
                    _initialPosition=null;
                    if(autoplay){player.Play();_playing=true;}
                }
                finally{_seekGate.Release();}
            }
            else if(_playing)player.Play();
            else if(_playbackRange is not null)
            {
                await _seekGate.WaitAsync(token);
                try
                {
                    EnsureCurrentPlayer(player,generation,token);
                    if(!_playing)
                    {
                        var ready=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
                        void Presented(TimeSpan _){if(IsCurrentPlayer(player,generation))ready.TrySetResult();}
                        FramePresented+=Presented;
                        try
                        {
                            player.Pause();ResetPendingFrameForStep();player.StepForwardOneFrame();
                            await ready.Task.WaitAsync(SeekPresentationTimeout,token);
                        }
                        finally{FramePresented-=Presented;}
                    }
                }
                finally{_seekGate.Release();}
            }
            EnsureCurrentPlayer(player,generation,token);
            Opened?.Invoke();
        }
        catch(OperationCanceledException)when(token.IsCancellationRequested)
        {
            // Play/Stop supersedes initial positioning without unloading the
            // already opened source. Do not leave its host waiting for ready.
            if(!sourceToken.IsCancellationRequested&&IsCurrentPlayer(player,generation))Opened?.Invoke();
        }
        catch(Exception ex){RaiseFailed(player,ex);}
    }

    private void ResetPendingFrameForStep()
    {
        lock(_frameGate)
        {
            _latestPixels=null;
            Volatile.Write(ref _lastAcceptedFrameTimestamp,0);
            Volatile.Write(ref _forceNextFrame,1);
        }
    }

    private static TimeSpan ToSourcePosition(TimeSpan relative,VideoClipRange? range)
    {
        if(range is not { } bounded)return relative<TimeSpan.Zero?TimeSpan.Zero:relative;
        return TimeSpan.FromTicks(bounded.Start.Ticks+Math.Clamp(relative.Ticks,0,bounded.Duration.Ticks));
    }

    private void EnsureCurrentPlayer(WinMediaPlayer player,long generation,CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        if(!IsCurrentPlayer(player,generation))throw new OperationCanceledException(token);
    }

    private void OnVideoFrameAvailable(WinMediaPlayer sender, object args)
    {
        // The frame server can run much faster than WPF's render dispatcher.
        // If a frame is already waiting for the UI, drop this one before the
        // GPU readback and managed allocation.  A 4K frame is ~32 MiB, so
        // decoding every callback while the UI is behind quickly floods the
        // large-object heap without making playback look smoother.
        if(Volatile.Read(ref _disposed))return;
        var forced=Volatile.Read(ref _forceNextFrame)!=0;
        if(Volatile.Read(ref _frameDispatchPending)!=0&&!forced)return;
        var ownsBusy=Interlocked.CompareExchange(ref _frameBusy,1,0)==0;
        if(!ownsBusy&&!forced)return;
        try
        {
            var now=Stopwatch.GetTimestamp();
            var previous=Volatile.Read(ref _lastAcceptedFrameTimestamp);
            if(!forced&&previous!=0&&now-previous<MinimumFrameIntervalTicks)return;
            Volatile.Write(ref _lastAcceptedFrameTimestamp,now);

            byte[]? pixels = null;
            long generation=0;
            lock (_frameGate)
            {
                if (_disposed || !ReferenceEquals(_player, sender) || _surface is null || _width <= 0 || _height <= 0)
                    return;

                // MediaPlayer is outside Win2D and does not acquire its
                // device lock. Our per-preview gate cannot protect the shared
                // device from other previews or native resource cleanup.
                // Hold Win2D's lock across the external copy and readback;
                // otherwise annotated pin + GC can crash the GPU driver.
                using var deviceLock=_device!.Lock();
                sender.CopyFrameToVideoSurface(_surface);
                var capturedPositionTicks=ToSourcePosition(sender.PlaybackSession.Position,_playbackRange).Ticks;
                pixels = _surface.GetPixelBytes();
                var required = checked(_width * _height * 4);
                if (pixels.Length < required) return;
                Volatile.Write(ref _forceNextFrame,0);
                generation=Volatile.Read(ref _generation);
                _latestPixels = pixels;
                _latestFrameGeneration = generation;
                // Bind the session position to the pixels at capture time.
                // Reading Position later on the WPF dispatcher associates a
                // delayed UI frame with a newer timestamp and visibly shifts
                // every time-based annotation ahead of the video.
                _latestFramePositionTicks=capturedPositionTicks;
            }
            ScheduleFrameDispatch(generation);
        }
        catch (Exception ex)
        {
            RaiseFailed(sender,ex);
        }
        finally
        {
            if(ownsBusy)Volatile.Write(ref _frameBusy, 0);
        }
    }

    private void ScheduleFrameDispatch(long generation)
    {
        if (Interlocked.Exchange(ref _frameDispatchPending, 1) != 0) return;
        if (_dispatcher.HasShutdownStarted)
        {
            Interlocked.Exchange(ref _frameDispatchPending, 0);
            return;
        }
        _ = _dispatcher.BeginInvoke(DispatcherPriority.Render, new Action(() => DeliverLatestFrame(generation)));
    }

    private void DeliverLatestFrame(long generation)
    {
        byte[]? pixels;
        int width;
        int height;
        long positionTicks;
        lock (_frameGate)
        {
            // A render callback can outlive a replaced player.  Never let an
            // old frame blank or overwrite the newly loaded preview.
            if (generation==Volatile.Read(ref _generation)&&_latestFrameGeneration==generation)
            {
                pixels=_latestPixels;
                _latestPixels=null;
                width=_width;
                height=_height;
                positionTicks=_latestFramePositionTicks;
            }
            else
            {
                pixels=null;
                width=height=0;
                positionTicks=0;
            }
        }
        try
        {
            if (!_disposed && pixels is not null && width>0 && height>0)
            {
                var frame=new DecodedFrame(pixels,width,height,positionTicks);
                if(_holdPresentation)_stagedFrame=frame;
                else PresentFrame(frame);
                if(!_disposed&&generation==Volatile.Read(ref _generation))
                {
                    Interlocked.Increment(ref _decodedFrameCount);
                    FrameDecoded?.Invoke();
                }
            }
        }
        catch(Exception ex)
        {
            var player=_player;
            if(player is not null)RaiseFailed(player,ex);
        }
        finally
        {
            Interlocked.Exchange(ref _frameDispatchPending, 0);
        }
        lock (_frameGate)
        {
            if (_latestPixels is not null && !_disposed)ScheduleFrameDispatch(_latestFrameGeneration);
        }
    }

    private void PresentFrame(DecodedFrame frame)
    {
        if(_displayFrame is null||_displayFrame.PixelWidth!=frame.Width||_displayFrame.PixelHeight!=frame.Height)
            _displayFrame=new WriteableBitmap(frame.Width,frame.Height,96,96,PixelFormats.Bgra32,null);
        _displayFrame.WritePixels(new Int32Rect(0,0,frame.Width,frame.Height),frame.Pixels,frame.Width*4,0);
        if(!ReferenceEquals(_view.Source,_displayFrame))_view.Source=_displayFrame;
        Interlocked.Increment(ref _presentedFrameCount);
        Interlocked.Exchange(ref _lastPresentedPositionTicks,frame.PositionTicks);
        FramePresented?.Invoke(TimeSpan.FromTicks(frame.PositionTicks));
    }

    internal static (int Width,int Height) CalculatePreviewSize(int width,int height)
    {
        if(width<=0||height<=0)throw new ArgumentOutOfRangeException(nameof(width),"视频尺寸必须大于零");
        var longEdge=Math.Max(width,height);
        if(longEdge<=MaxPreviewLongEdge)return(width,height);
        var scale=MaxPreviewLongEdge/(double)longEdge;
        return(Math.Max(1,(int)Math.Round(width*scale)),Math.Max(1,(int)Math.Round(height*scale)));
    }

    private void OnMediaFailed(WinMediaPlayer sender, Windows.Media.Playback.MediaPlayerFailedEventArgs args)
    {
        var error = new InvalidOperationException($"视频解码失败：{args.Error}");
        RaiseFailed(sender,error);
    }

    private void OnMediaEnded(WinMediaPlayer sender, object args)
    {
        // IsLoopingEnabled normally handles this without a seek round-trip;
        // still expose the event for hosts that want to update their status.
        var generation=Volatile.Read(ref _generation);
        RaiseOnUi(() =>
        {
            if (IsCurrentPlayer(sender,generation)) Ended?.Invoke();
        });
    }

    private void CancelPresentationWaits()
    {
        _presentationVersion++;
        _settledSeek=null;
        var cancellation=_presentationCancellation;
        _presentationCancellation=_disposed?null:new CancellationTokenSource();
        try{cancellation?.Cancel();}finally{cancellation?.Dispose();}
    }

    private void CloseSourceCore(bool retainDisplayedFrame=false)
    {
        Interlocked.Increment(ref _generation);
        CancelPresentationWaits();
        var cancellation=_sourceCancellation;_sourceCancellation=null;
        try{cancellation?.Cancel();}finally{cancellation?.Dispose();}
        WinMediaPlayer? player;
        lock (_frameGate)
        {
            player = _player;
            _player = null;
            _playbackRange=null;
            Interlocked.Exchange(ref _nativeFrameIntervalTicks,TimeSpan.TicksPerMillisecond);
            _surface?.Dispose();
            _surface = null;
            _width = _height = 0;
            _latestPixels = null;
            _latestFrameGeneration = 0;
            _latestFramePositionTicks = 0;
            Volatile.Write(ref _forceNextFrame,0);
            if(!retainDisplayedFrame)Interlocked.Exchange(ref _lastPresentedPositionTicks,0);
            _lastAcceptedFrameTimestamp = 0;
            Interlocked.Exchange(ref _presentedFrameCount,0);
            Interlocked.Exchange(ref _decodedFrameCount,0);
        }
        if (player is not null)
        {
            try { player.VideoFrameAvailable -= OnVideoFrameAvailable; } catch { }
            try { player.MediaOpened -= OnMediaOpened; } catch { }
            try { player.MediaFailed -= OnMediaFailed; } catch { }
            try { player.MediaEnded -= OnMediaEnded; } catch { }
            try { player.Dispose(); } catch { }
        }
        _playing = false;
        _holdPresentation=false;
        _stagedFrame=null;
        _sourcePath=null;
        _initialPosition=null;
        if(!retainDisplayedFrame)
        {
            _displayFrame = null;
            _view.Source = null;
        }
    }

    private bool IsCurrentPlayer(WinMediaPlayer sender)=>IsCurrentPlayer(sender,Volatile.Read(ref _generation));

    private bool IsCurrentPlayer(WinMediaPlayer sender,long generation)
    {
        lock (_frameGate) return !_disposed && generation==_generation && ReferenceEquals(_player,sender);
    }

    private void RaiseFailed(WinMediaPlayer sender,Exception exception)
    {
        var generation=Volatile.Read(ref _generation);
        if (!IsCurrentPlayer(sender,generation)) return;
        if (Interlocked.CompareExchange(ref _failureGeneration,generation,-1)==generation)return;
        RaiseOnUi(() =>
        {
            if (!IsCurrentPlayer(sender,generation)) return;
            _settledSeek=null;
            _playing = false;
            try { sender.Pause(); } catch { }
            Failed?.Invoke(exception);
        });
    }

    private void RaiseOnUi(Action? callback)
    {
        if (callback is null || _dispatcher.HasShutdownStarted) return;
        if (_dispatcher.CheckAccess()) callback();
        else _ = _dispatcher.BeginInvoke(callback);
    }

    private void VerifyDispatcher()
    {
        if (!_dispatcher.CheckAccess()) throw new InvalidOperationException("视频预览必须在 UI 线程操作");
    }

    public void Dispose()
    {
        if (_disposed) return;
        VerifyDispatcher();
        _disposed = true;
        CloseSourceCore();
        // _device comes from Win2D's process-wide shared device. It must not be
        // disposed here because another preview can be using the same device.
        _device = null;
    }
}
