using System.Windows.Threading;
using System.Security.Cryptography;
using Windows.Media.Core;
using WinMediaPlayer = Windows.Media.Playback.MediaPlayer;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Synthesizes speech through the isolated local Hermes runtime and plays one
/// response at a time. Windows Media Foundation's WinRT MediaPlayer is used
/// instead of WPF's legacy MediaPlayer because Hermes can legitimately return
/// Ogg/Opus (for example, the local minimax-mmx provider). The staged file
/// remains leased until playback has fully closed.
/// </summary>
public sealed class HermesReadAloudService : IDisposable
{
    private readonly Dispatcher _dispatcher;
    private readonly HermesSpeechFileStore _fileStore;
    private readonly object _gate=new();
    private readonly HashSet<TaskCompletionSource> _operations=[];
    private CancellationTokenSource? _activeRequest;
    private Playback? _playback;
    private long _generation;
    private int _disposed;

    public HermesReadAloudService(Dispatcher dispatcher)
        :this(dispatcher,new HermesSpeechFileStore(new TempFileService(),TempMediaRegistry.Shared)){}

    internal HermesReadAloudService(Dispatcher dispatcher,HermesSpeechFileStore fileStore)
    {
        _dispatcher=dispatcher??throw new ArgumentNullException(nameof(dispatcher));
        _fileStore=fileStore??throw new ArgumentNullException(nameof(fileStore));
    }

    public async Task SpeakAsync(
        HermesRuntimeService runtime,
        string text,
        string? profile,
        CancellationToken cancellationToken=default)
    {
        ArgumentNullException.ThrowIfNull(runtime);
        if(string.IsNullOrWhiteSpace(text))return;
        ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed)!=0,this);

        var request=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        var operation=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        long generation;
        try{generation=BeginRequest(request,operation);}
        catch{request.Dispose();throw;}
        HermesSpeechFile? stagedFile=null;
        try
        {
            StopPlaybackOnDispatcher();
            using(var audio=await runtime.SynthesizeSpeechAsync(text,profile,request.Token).ConfigureAwait(false))
                stagedFile=await _fileStore.StageAsync(audio,request.Token).ConfigureAwait(false);
            request.Token.ThrowIfCancellationRequested();

            var readyFile=stagedFile??throw new InvalidDataException("Hermes 朗读音频暂存失败。");
            Playback? playback=null;
            await _dispatcher.InvokeAsync(() =>
            {
                request.Token.ThrowIfCancellationRequested();
                if(!IsCurrent(generation,request))return;
                playback=StartPlaybackCore(generation,readyFile);
                stagedFile=null;
            },DispatcherPriority.Normal,request.Token);

            if(playback is null)throw new OperationCanceledException("Hermes 自动朗读已被新的播放请求替换。",request.Token);
            await playback.Completion.Task.WaitAsync(request.Token).ConfigureAwait(false);
        }
        catch(OperationCanceledException) when(request.IsCancellationRequested)
        {
            StopGeneration(generation,request);
            throw;
        }
        finally
        {
            stagedFile?.Dispose();
            EndRequest(request,operation);
        }
    }

    public Task SpeakAsync(HermesRuntimeService runtime,string text,CancellationToken cancellationToken=default)
        =>SpeakAsync(runtime,text,"default",cancellationToken);

    public void Stop()
    {
        CancellationTokenSource? request;
        lock(_gate)
        {
            _generation++;
            request=_activeRequest;
            _activeRequest=null;
        }
        try{request?.Cancel();}catch(ObjectDisposedException){}
        StopPlaybackOnDispatcher();
    }

    private long BeginRequest(CancellationTokenSource request,TaskCompletionSource operation)
    {
        CancellationTokenSource? previous;
        long generation;
        lock(_gate)
        {
            ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed)!=0,this);
            generation=++_generation;
            previous=_activeRequest;
            _activeRequest=request;
            _operations.Add(operation);
        }
        try{previous?.Cancel();}catch(ObjectDisposedException){}
        return generation;
    }

    private bool IsCurrent(long generation,CancellationTokenSource request)
    {
        lock(_gate)
            return Volatile.Read(ref _disposed)==0&&generation==_generation&&ReferenceEquals(_activeRequest,request);
    }

    private void EndRequest(CancellationTokenSource request,TaskCompletionSource operation)
    {
        lock(_gate)
        {
            if(ReferenceEquals(_activeRequest,request))_activeRequest=null;
            _operations.Remove(operation);
        }
        request.Dispose();
        operation.TrySetResult();
    }

    private void StopGeneration(long generation,CancellationTokenSource request)
    {
        lock(_gate)
        {
            if(generation!=_generation||!ReferenceEquals(_activeRequest,request))return;
            _generation++;
            _activeRequest=null;
        }
        StopPlaybackOnDispatcher(generation);
    }

    private Playback StartPlaybackCore(long generation,HermesSpeechFile speechFile)
    {
        _dispatcher.VerifyAccess();
        StopPlaybackCore();
        var player=new WinMediaPlayer
        {
            AutoPlay=false,
            IsMuted=false,
            IsLoopingEnabled=false
        };
        var playback=new Playback(generation,player,speechFile);
        player.MediaOpened+=PlaybackOpened;
        player.MediaEnded+=PlaybackEnded;
        player.MediaFailed+=PlaybackFailed;
        try
        {
            _playback=playback;
            player.Volume=1.0;
            player.Source=MediaSource.CreateFromUri(new Uri(speechFile.Path,UriKind.Absolute));

            // A missing MediaOpened event must not leave the read-aloud
            // request waiting forever. This is also useful on machines where
            // the Windows audio codec is unavailable.
            var timeout=new DispatcherTimer(DispatcherPriority.Background,_dispatcher)
            {
                Interval=TimeSpan.FromSeconds(15)
            };
            timeout.Tick+=(_,_) =>
            {
                timeout.Stop();
                if(_playback is { } current&&ReferenceEquals(current,playback)&&!playback.Opened)
                    CompletePlaybackCore(playback,new TimeoutException("Hermes 朗读音频打开超时，请检查系统音频编码支持。"),canceled:false);
            };
            playback.OpenTimeout=timeout;
            timeout.Start();
            return playback;
        }
        catch
        {
            CompletePlaybackCore(playback,null,canceled:true);
            throw;
        }
    }

    private void PlaybackOpened(WinMediaPlayer sender,object args)
    {
        if(!_dispatcher.CheckAccess())
        {
            if(!_dispatcher.HasShutdownStarted&&!_dispatcher.HasShutdownFinished)
                _= _dispatcher.BeginInvoke(new Action(()=>PlaybackOpened(sender,args)));
            return;
        }
        if(_playback is not { } playback||!ReferenceEquals(playback.Player,sender))return;
        playback.Opened=true;
        playback.OpenTimeout?.Stop();
        try{sender.Play();}
        catch(Exception ex)
        {
            try{new PrivacyLogger().Error("HermesReadAloudPlayback",ex);}catch{}
            CompletePlaybackCore(playback,ex,canceled:false);
        }
    }

    private void PlaybackEnded(WinMediaPlayer sender,object eventArgs)
    {
        if(!_dispatcher.CheckAccess())
        {
            if(!_dispatcher.HasShutdownStarted&&!_dispatcher.HasShutdownFinished)
                _= _dispatcher.BeginInvoke(new Action(()=>PlaybackEnded(sender,eventArgs)));
            return;
        }
        if(_playback is { } playback&&ReferenceEquals(playback.Player,sender))
            CompletePlaybackCore(playback,null,canceled:false);
    }

    private void PlaybackFailed(WinMediaPlayer sender,Windows.Media.Playback.MediaPlayerFailedEventArgs eventArgs)
    {
        if(!_dispatcher.CheckAccess())
        {
            if(!_dispatcher.HasShutdownStarted&&!_dispatcher.HasShutdownFinished)
                _= _dispatcher.BeginInvoke(new Action(()=>PlaybackFailed(sender,eventArgs)));
            return;
        }
        if(_playback is not { } playback||!ReferenceEquals(playback.Player,sender))return;
        var error=new InvalidOperationException($"Hermes 朗读音频播放失败（{eventArgs.Error}）。");
        try{new PrivacyLogger().Error("HermesReadAloudPlayback",error);}catch{}
        CompletePlaybackCore(playback,error,canceled:false);
    }

    private void StopPlaybackOnDispatcher(long? generation=null)
    {
        void StopAction()
        {
            if(generation is not null&&_playback?.Generation!=generation.Value)return;
            StopPlaybackCore();
        }

        try
        {
            if(_dispatcher.CheckAccess())StopAction();
            else if(!_dispatcher.HasShutdownStarted&&!_dispatcher.HasShutdownFinished)_dispatcher.Invoke(StopAction);
        }
        catch(Exception ex)when(ex is TaskCanceledException or InvalidOperationException)
        {
            try{new PrivacyLogger().Error("HermesReadAloudStop",ex);}catch{}
        }
    }

    private void StopPlaybackCore()
    {
        _dispatcher.VerifyAccess();
        if(_playback is { } playback)CompletePlaybackCore(playback,null,canceled:true);
    }

    private void CompletePlaybackCore(Playback playback,Exception? error,bool canceled)
    {
        _dispatcher.VerifyAccess();
        if(ReferenceEquals(_playback,playback))_playback=null;
        playback.OpenTimeout?.Stop();
        playback.OpenTimeout=null;
        playback.Player.MediaOpened-=PlaybackOpened;
        playback.Player.MediaEnded-=PlaybackEnded;
        playback.Player.MediaFailed-=PlaybackFailed;
        try{playback.Player.Pause();}
        catch(Exception pauseError){try{new PrivacyLogger().Error("HermesReadAloudPause",pauseError);}catch{}}
        try{playback.Player.Dispose();}
        catch(Exception closeError){try{new PrivacyLogger().Error("HermesReadAloudClose",closeError);}catch{}}
        playback.File.Dispose();

        if(error is not null)playback.Completion.TrySetException(error);
        else if(canceled)playback.Completion.TrySetCanceled();
        else playback.Completion.TrySetResult();
    }

    public void Dispose()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        Task[] operations;
        lock(_gate)operations=_operations.Select(operation=>operation.Task).ToArray();
        Stop();
        if(operations.Length==0)return;
        try
        {
            if(!Task.WhenAll(operations).Wait(TimeSpan.FromSeconds(5)))
                new PrivacyLogger().Error("HermesReadAloudDispose",new TimeoutException("等待 Hermes 自动朗读停止超时。"));
        }
        catch(AggregateException ex)
        {
            try{new PrivacyLogger().Error("HermesReadAloudDispose",ex.Flatten());}catch{}
        }
    }

    private sealed class Playback(long generation,WinMediaPlayer player,HermesSpeechFile file)
    {
        internal long Generation { get; }=generation;
        internal WinMediaPlayer Player { get; }=player;
        internal HermesSpeechFile File { get; }=file;
        internal bool Opened { get; set; }
        internal DispatcherTimer? OpenTimeout { get; set; }
        internal TaskCompletionSource Completion { get; }=new(TaskCreationOptions.RunContinuationsAsynchronously);
    }

}

internal sealed class HermesSpeechFileStore
{
    private readonly TempFileService _tempFiles;
    private readonly TempMediaRegistry _registry;

    internal HermesSpeechFileStore(TempFileService tempFiles,TempMediaRegistry registry)
    {
        _tempFiles=tempFiles??throw new ArgumentNullException(nameof(tempFiles));
        _registry=registry??throw new ArgumentNullException(nameof(registry));
    }

    internal async Task<HermesSpeechFile> StageAsync(HermesSpeechAudio audio,CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(audio);
        if(audio.Data.Length==0)throw new InvalidDataException("Hermes 返回的朗读音频为空。");
        byte[]? converted=null;
        var extension=audio.Extension;
        ReadOnlyMemory<byte> payload=audio.Data;
        if(HermesAudioTranscoder.IsOggOpus(audio.MimeType,audio.Extension,audio.Data))
        {
            converted=HermesAudioTranscoder.DecodeToWave(audio.Data);
            extension=".wav";
            payload=converted;
        }
        try
        {
            var path=_tempFiles.NewFile(extension);
            var lease=_registry.Acquire(path);
            try
            {
                await using(var stream=new FileStream(
                                path,
                                FileMode.CreateNew,
                                FileAccess.Write,
                                FileShare.None,
                                81_920,
                                FileOptions.Asynchronous|FileOptions.WriteThrough))
                {
                    await stream.WriteAsync(payload,cancellationToken).ConfigureAwait(false);
                    await stream.FlushAsync(cancellationToken).ConfigureAwait(false);
                }
                return new HermesSpeechFile(path,lease,_registry);
            }
            catch
            {
                lease.Dispose();
                TryDelete(path,_registry);
                throw;
            }
        }
        finally
        {
            if(converted is not null)CryptographicOperations.ZeroMemory(converted);
        }
    }

    internal static void TryDelete(string path,TempMediaRegistry registry)
    {
        try
        {
            _=registry.TryExecuteIfUnleased(path,includeDescendants:false,()=>
            {
                if(File.Exists(path))File.Delete(path);
            });
        }
        catch(Exception ex)when(ex is IOException or UnauthorizedAccessException)
        {
            try{new PrivacyLogger().Error("HermesReadAloudTempDelete",ex);}catch{}
        }
    }
}

internal sealed class HermesSpeechFile : IDisposable
{
    private readonly TempMediaRegistry _registry;
    private TempMediaLease? _lease;
    private int _disposed;

    internal HermesSpeechFile(string path,TempMediaLease lease,TempMediaRegistry registry)
    {
        Path=path;
        _lease=lease;
        _registry=registry;
    }

    internal string Path { get; }

    public void Dispose()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        Interlocked.Exchange(ref _lease,null)?.Dispose();
        HermesSpeechFileStore.TryDelete(Path,_registry);
    }
}
