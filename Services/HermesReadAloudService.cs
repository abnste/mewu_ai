using System.Windows.Media;
using System.Windows.Threading;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Synthesizes speech through the isolated local Hermes runtime and plays one
/// response at a time. MediaPlayer needs a local file and is dispatcher-bound,
/// so the staged file remains leased until playback has fully closed.
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
            using(var audio=await runtime.SynthesizeSpeechAsync(text,request.Token).ConfigureAwait(false))
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
        var player=new MediaPlayer();
        var playback=new Playback(generation,player,speechFile);
        player.MediaEnded+=PlaybackEnded;
        player.MediaFailed+=PlaybackFailed;
        try
        {
            _playback=playback;
            player.Volume=1.0;
            player.Open(new Uri(speechFile.Path,UriKind.Absolute));
            player.Play();
            return playback;
        }
        catch
        {
            CompletePlaybackCore(playback,null,canceled:true);
            throw;
        }
    }

    private void PlaybackEnded(object? sender,EventArgs eventArgs)
    {
        _dispatcher.VerifyAccess();
        if(_playback is { } playback&&ReferenceEquals(playback.Player,sender))
            CompletePlaybackCore(playback,null,canceled:false);
    }

    private void PlaybackFailed(object? sender,ExceptionEventArgs eventArgs)
    {
        _dispatcher.VerifyAccess();
        if(_playback is not { } playback||!ReferenceEquals(playback.Player,sender))return;
        var error=new InvalidOperationException("Hermes 朗读音频播放失败。",eventArgs.ErrorException);
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
        playback.Player.MediaEnded-=PlaybackEnded;
        playback.Player.MediaFailed-=PlaybackFailed;
        try{playback.Player.Close();}
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

    private sealed class Playback(long generation,MediaPlayer player,HermesSpeechFile file)
    {
        internal long Generation { get; }=generation;
        internal MediaPlayer Player { get; }=player;
        internal HermesSpeechFile File { get; }=file;
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
        var path=_tempFiles.NewFile(audio.Extension);
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
                await stream.WriteAsync(audio.Data,cancellationToken).ConfigureAwait(false);
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
