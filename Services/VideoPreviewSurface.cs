using System.Diagnostics;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using Microsoft.Graphics.Canvas;
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
    private WinMediaPlayer? _player;
    private CanvasBitmap? _surface;
    private CanvasDevice? _device;
    private WriteableBitmap? _displayFrame;
    private byte[]? _latestPixels;
    private long _latestFrameGeneration;
    private long _lastAcceptedFrameTimestamp;
    private long _presentedFrameCount;
    private int _width;
    private int _height;
    private int _frameBusy;
    private int _frameDispatchPending;
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
    internal TimeSpan Position
    {
        get
        {
            VerifyDispatcher();
            return _player?.PlaybackSession.Position??TimeSpan.Zero;
        }
    }
    internal long PresentedFrameCount => Interlocked.Read(ref _presentedFrameCount);

    internal event Action? Opened;
    internal event Action<Exception>? Failed;
    internal event Action? Ended;
    internal event Action<TimeSpan>? FramePresented;

    internal void Load(string path, bool autoplay)
    {
        CrashDiagnosticsService.MarkOperation("视频预览：加载媒体");
        VerifyDispatcher();
        ObjectDisposedException.ThrowIf(_disposed, this);
        var normalized = Path.GetFullPath(path);
        if (!File.Exists(normalized))
            throw new FileNotFoundException("录屏文件已不可用", normalized);

        CloseSourceCore();
        _playing = autoplay;
        Interlocked.Exchange(ref _failureGeneration,-1);
        var player = new WinMediaPlayer
        {
            IsMuted = true,
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
            player.Source = MediaSource.CreateFromUri(new Uri(normalized, UriKind.Absolute));
            if (autoplay) player.Play();
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

    internal async Task SeekAsync(TimeSpan position,bool pauseAfterSeek,CancellationToken cancellationToken=default)
    {
        CrashDiagnosticsService.MarkOperation("视频预览：跳转时间轴");
        VerifyDispatcher();
        ObjectDisposedException.ThrowIf(_disposed,this);
        var player=_player??throw new InvalidOperationException("视频尚未加载");
        var session=player.PlaybackSession;
        if(!session.CanSeek)throw new InvalidOperationException("当前视频不支持跳转");
        if(position<TimeSpan.Zero)position=TimeSpan.Zero;
        var duration=session.NaturalDuration;
        if(duration>TimeSpan.Zero&&position>duration)position=duration;
        if(pauseAfterSeek){player.Pause();_playing=false;}
        var previousFrameCount=PresentedFrameCount;
        var completion=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var frameReady=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        Windows.Foundation.TypedEventHandler<MediaPlaybackSession,object>? handler=null;
        Action<TimeSpan>? frameHandler=null;
        Action<TimeSpan>? steppedFrameHandler=null;
        handler=(_,_)=>completion.TrySetResult();
        frameHandler=_=>frameReady.TrySetResult();
        session.SeekCompleted+=handler;
        FramePresented+=frameHandler;
        var seekTimer=Stopwatch.StartNew();
        TimeSpan RemainingTimeout()
        {
            var remaining=SeekPresentationTimeout-seekTimer.Elapsed;
            if(remaining<=TimeSpan.Zero)throw new TimeoutException("视频跳转后未能在限定时间内呈现目标帧");
            return remaining;
        }
        try
        {
            session.Position=position;
            await completion.Task.WaitAsync(RemainingTimeout(),cancellationToken);
            // Microsoft documents that a paused MediaPlayer can report an
            // imprecise frame position after seeking. Advancing one decoded
            // frame after Pause makes the frame-server surface settle on the
            // requested visual frame instead of retaining the previous GOP
            // frame while the session position has already changed.
            if(pauseAfterSeek&&(duration<=TimeSpan.Zero||position<duration))
            {
                var steppedFrameReady=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
                steppedFrameHandler=_=>steppedFrameReady.TrySetResult();
                FramePresented+=steppedFrameHandler;
                player.StepForwardOneFrame();
                await steppedFrameReady.Task.WaitAsync(RemainingTimeout(),cancellationToken);
            }
            else if(PresentedFrameCount<=previousFrameCount)
                await frameReady.Task.WaitAsync(RemainingTimeout(),cancellationToken);
        }
        finally
        {
            session.SeekCompleted-=handler;
            FramePresented-=frameHandler;
            if(steppedFrameHandler is not null)FramePresented-=steppedFrameHandler;
        }
    }

    internal void Stop()
    {
        VerifyDispatcher();
        if (_disposed) return;
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

            lock (_frameGate)
            {
                if (_disposed || !ReferenceEquals(_player, sender)) return;
                _width = width;
                _height = height;
                _device ??= CanvasDevice.GetSharedDevice();
                _surface?.Dispose();
                // CanvasBitmap implements IDirect3DSurface, the destination
                // required by CopyFrameToVideoSurface.
                using var bitmap = new SoftwareBitmap(BitmapPixelFormat.Bgra8, width, height, BitmapAlphaMode.Ignore);
                _surface = CanvasBitmap.CreateFromSoftwareBitmap(_device, bitmap);
            }
            var generation=Volatile.Read(ref _generation);
            RaiseOnUi(() =>
            {
                if (IsCurrentPlayer(sender,generation)) Opened?.Invoke();
            });
        }
        catch (Exception ex)
        {
            RaiseFailed(sender,ex);
        }
    }

    private void OnVideoFrameAvailable(WinMediaPlayer sender, object args)
    {
        // The frame server can run much faster than WPF's render dispatcher.
        // If a frame is already waiting for the UI, drop this one before the
        // GPU readback and managed allocation.  A 4K frame is ~32 MiB, so
        // decoding every callback while the UI is behind quickly floods the
        // large-object heap without making playback look smoother.
        if (Volatile.Read(ref _disposed) || Volatile.Read(ref _frameDispatchPending) != 0 || Interlocked.Exchange(ref _frameBusy, 1) != 0) return;
        try
        {
            var now=Stopwatch.GetTimestamp();
            var previous=Volatile.Read(ref _lastAcceptedFrameTimestamp);
            if(previous!=0&&now-previous<MinimumFrameIntervalTicks)return;
            Volatile.Write(ref _lastAcceptedFrameTimestamp,now);

            byte[]? pixels = null;
            long generation=0;
            lock (_frameGate)
            {
                if (_disposed || !ReferenceEquals(_player, sender) || _surface is null || _width <= 0 || _height <= 0)
                    return;

                sender.CopyFrameToVideoSurface(_surface);
                pixels = _surface.GetPixelBytes();
                var required = checked(_width * _height * 4);
                if (pixels.Length < required) return;
                generation=Volatile.Read(ref _generation);
                _latestPixels = pixels;
                _latestFrameGeneration = generation;
            }
            ScheduleFrameDispatch(generation);
        }
        catch (Exception ex)
        {
            RaiseFailed(sender,ex);
        }
        finally
        {
            Volatile.Write(ref _frameBusy, 0);
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
            }
            else
            {
                pixels=null;
                width=height=0;
            }
        }
        try
        {
            if (!_disposed && pixels is not null && width>0 && height>0)
            {
                if(_displayFrame is null||_displayFrame.PixelWidth!=width||_displayFrame.PixelHeight!=height)
                    _displayFrame=new WriteableBitmap(width,height,96,96,PixelFormats.Bgra32,null);
                _displayFrame.WritePixels(new Int32Rect(0,0,width,height),pixels,width*4,0);
                if(!ReferenceEquals(_view.Source,_displayFrame))_view.Source=_displayFrame;
                Interlocked.Increment(ref _presentedFrameCount);
                var player=_player;
                if(player is not null)FramePresented?.Invoke(player.PlaybackSession.Position);
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

    private void CloseSourceCore()
    {
        Interlocked.Increment(ref _generation);
        WinMediaPlayer? player;
        lock (_frameGate)
        {
            player = _player;
            _player = null;
            _surface?.Dispose();
            _surface = null;
            _width = _height = 0;
            _latestPixels = null;
            _latestFrameGeneration = 0;
            _lastAcceptedFrameTimestamp = 0;
            Interlocked.Exchange(ref _presentedFrameCount,0);
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
        _displayFrame = null;
        _view.Source = null;
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
