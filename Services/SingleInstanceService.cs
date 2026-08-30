using System.IO.Pipes;
namespace mewu_ai_Assistant.Services;
public sealed class SingleInstanceService : IDisposable
{
    private const string DefaultName="MewuAI-7F2171C4"; private readonly string _name; private readonly Mutex _mutex; private readonly CancellationTokenSource _stop=new(); private readonly Task _listener; private readonly object _activationGate=new(); private Action? _activationRequested; private int _pendingActivations; private int _disposed;
    public bool IsPrimary { get; }
    public event Action? ActivationRequested
    {
        add
        {
            if(value is null)return;
            int pending;
            lock(_activationGate)
            {
                _activationRequested+=value;
                pending=_pendingActivations;
                _pendingActivations=0;
            }
            // Replay signals received during the tiny construction/subscription
            // window so a secondary launch can never be lost.
            for(var index=0;index<pending;index++)
                try{value();}catch{}
        }
        remove { if(value is null)return; lock(_activationGate)_activationRequested-=value; }
    }
    public SingleInstanceService():this(DefaultName){}
    internal SingleInstanceService(string name)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(name);_name=name;
        // The lifetime of the named kernel object is the ownership signal. No
        // thread-affine mutex acquisition is needed, so shutdown can safely run
        // on a different managed thread.
        _mutex=new Mutex(false,_name,out var created);IsPrimary=created;_listener=created?Task.Run(ListenAsync):Task.CompletedTask;
    }
    public void SignalPrimary()
    {
        for(var attempt=0;attempt<4;attempt++)
        {
            try{using var client=new NamedPipeClientStream(".",_name,PipeDirection.Out,PipeOptions.CurrentUserOnly);client.Connect(300);client.WriteByte(1);client.Flush();return;}
            catch when(attempt<3){Thread.Sleep(75);}
            catch{return;}
        }
    }
    private async Task ListenAsync()
    {
        var stopToken=_stop.Token;
        while(!stopToken.IsCancellationRequested)
        {
            try
            {
                await using var server=new NamedPipeServerStream(_name,PipeDirection.In,1,PipeTransmissionMode.Byte,PipeOptions.Asynchronous|PipeOptions.CurrentUserOnly);
                await server.WaitForConnectionAsync(stopToken).ConfigureAwait(false);
                using var readTimeout=CancellationTokenSource.CreateLinkedTokenSource(stopToken);
                readTimeout.CancelAfter(TimeSpan.FromSeconds(2));
                var signal=new byte[1];
                if(await server.ReadAsync(signal,readTimeout.Token).ConfigureAwait(false)>0&&signal[0]==1)DispatchActivation();
            }
            catch(OperationCanceledException) when(stopToken.IsCancellationRequested){break;}
            catch
            {
                try{await Task.Delay(250,stopToken).ConfigureAwait(false);}
                catch(OperationCanceledException){break;}
            }
        }
    }
    private void DispatchActivation()
    {
        Action? handler;
        lock(_activationGate)
        {
            handler=_activationRequested;
            if(handler is null)
            {
                if(_pendingActivations<int.MaxValue)Interlocked.Increment(ref _pendingActivations);
                return;
            }
        }
        try{handler();}catch{}
    }
    public void Dispose()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        _stop.Cancel();_mutex.Dispose();
        _=_listener.ContinueWith(task=>{_ = task.Exception;_stop.Dispose();},CancellationToken.None,TaskContinuationOptions.ExecuteSynchronously,TaskScheduler.Default);
    }
}
