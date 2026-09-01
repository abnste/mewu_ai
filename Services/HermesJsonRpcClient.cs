using System.Buffers;
using System.Collections.Concurrent;
using System.Security.Cryptography;
using System.Net.WebSockets;
using System.Text.Json;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

public sealed class HermesJsonRpcClient : IDisposable, IAsyncDisposable
{
    private const int MaxInboundFrameBytes=16*1024*1024;
    private const int MaxOutboundFrameBytes=64*1024*1024;
    private static readonly TimeSpan ConnectTimeout=TimeSpan.FromSeconds(20);
    private readonly HermesBackendService _backend;
    private readonly SemaphoreSlim _connectGate=new(1,1),_sendGate=new(1,1);
    private readonly ConcurrentDictionary<long,TaskCompletionSource<JsonElement>> _pending=[];
    private ClientWebSocket? _socket;
    private CancellationTokenSource? _connectionLifetime;
    private Task? _receiveLoop;
    private TaskCompletionSource<bool>? _gatewayReady;
    private long _nextId;
    private long _connectionGeneration;
    private int _disposed;

    public HermesJsonRpcClient(HermesBackendService backend)=>_backend=backend??throw new ArgumentNullException(nameof(backend));
    public event EventHandler<HermesRpcEvent>? EventReceived;
    public long ConnectionGeneration=>Volatile.Read(ref _connectionGeneration);

    public async Task<HermesConnectionInfo> EnsureConnectedAsync(CancellationToken cancellationToken)
    {
        ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed)!=0,this);
        var connection=await _backend.EnsureStartedAsync(cancellationToken).ConfigureAwait(false);
        if(_socket?.State==WebSocketState.Open&&_gatewayReady?.Task.IsCompletedSuccessfully==true)return connection;
        await _connectGate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            if(_socket?.State==WebSocketState.Open&&_gatewayReady?.Task.IsCompletedSuccessfully==true)return connection;
            await CloseConnectionAsync().ConfigureAwait(false);
            var socket=new ClientWebSocket();
            // This is a loopback-only child process. Never let a machine or
            // user proxy observe the bearer token or local RPC traffic.
            socket.Options.Proxy=null;
            socket.Options.SetRequestHeader("X-Hermes-Session-Token",_backend.GetSessionToken());
            var wsUri=new Uri($"ws://127.0.0.1:{connection.Port}/api/ws?token={Uri.EscapeDataString(_backend.GetSessionToken())}");
            using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            timeout.CancelAfter(ConnectTimeout);
            try{await socket.ConnectAsync(wsUri,timeout.Token).ConfigureAwait(false);}
            catch(OperationCanceledException) when(!cancellationToken.IsCancellationRequested)
            {
                socket.Dispose();
                throw new TimeoutException("连接 Hermes 本地后端超时。");
            }
            _socket=socket;
            _connectionLifetime=new CancellationTokenSource();
            _gatewayReady=new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
            _receiveLoop=ReceiveLoopAsync(socket,_gatewayReady,_connectionLifetime.Token);
            try{await _gatewayReady.Task.WaitAsync(timeout.Token).ConfigureAwait(false);}
            catch(OperationCanceledException) when(!cancellationToken.IsCancellationRequested)
            {throw new TimeoutException("Hermes 本地后端未完成协议握手。");}
            Interlocked.Increment(ref _connectionGeneration);
            return connection;
        }
        catch
        {
            await CloseConnectionAsync().ConfigureAwait(false);
            throw;
        }
        finally{_connectGate.Release();}
    }

    public async Task<JsonElement> InvokeAsync(string method,object? parameters,CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(method);
        await EnsureConnectedAsync(cancellationToken).ConfigureAwait(false);
        var socket=_socket??throw new InvalidOperationException("Hermes WebSocket 尚未连接。");
        var id=Interlocked.Increment(ref _nextId);
        var completion=new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        if(!_pending.TryAdd(id,completion))throw new InvalidOperationException("Hermes 请求编号冲突。");
        byte[]? payload=null;
        try
        {
            payload=JsonSerializer.SerializeToUtf8Bytes(new{jsonrpc="2.0",id,method,@params=parameters??new{}});
            if(payload.Length>MaxOutboundFrameBytes)throw new InvalidDataException("Hermes 请求超过 64 MiB 安全上限。");
            await _sendGate.WaitAsync(cancellationToken).ConfigureAwait(false);
            try
            {
                if(socket.State!=WebSocketState.Open)throw new IOException("Hermes WebSocket 已断开。");
                await socket.SendAsync(payload,WebSocketMessageType.Text,true,cancellationToken).ConfigureAwait(false);
            }
            finally{_sendGate.Release();}
            return await completion.Task.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        finally
        {
            if(payload is not null)CryptographicOperations.ZeroMemory(payload);
            _pending.TryRemove(id,out _);
        }
    }

    private async Task ReceiveLoopAsync(ClientWebSocket socket,TaskCompletionSource<bool> gatewayReady,CancellationToken cancellationToken)
    {
        var buffer=ArrayPool<byte>.Shared.Rent(64*1024);
        Exception? failure=null;
        try
        {
            while(socket.State==WebSocketState.Open&&!cancellationToken.IsCancellationRequested)
            {
                using var frame=new MemoryStream();
                WebSocketReceiveResult result;
                do
                {
                    result=await socket.ReceiveAsync(buffer,cancellationToken).ConfigureAwait(false);
                    if(result.MessageType==WebSocketMessageType.Close)return;
                    if(result.MessageType!=WebSocketMessageType.Text)throw new InvalidDataException("Hermes 返回了不支持的 WebSocket 帧。");
                    if(frame.Length+result.Count>MaxInboundFrameBytes)throw new InvalidDataException("Hermes 返回帧超过安全上限。");
                    frame.Write(buffer,0,result.Count);
                }while(!result.EndOfMessage);
                HandleFrame(frame.GetBuffer(),checked((int)frame.Length),gatewayReady);
            }
        }
        catch(OperationCanceledException) when(cancellationToken.IsCancellationRequested){}
        catch(Exception ex){failure=ex;}
        finally
        {
            ArrayPool<byte>.Shared.Return(buffer,clearArray:true);
            var terminal=failure??new IOException("Hermes WebSocket 已断开。");
            gatewayReady.TrySetException(terminal);
            foreach(var pending in _pending.Values)pending.TrySetException(terminal);
        }
    }

    private void HandleFrame(byte[] bytes,int count,TaskCompletionSource<bool> gatewayReady)
    {
        using var document=JsonDocument.Parse(bytes.AsMemory(0,count));
        var root=document.RootElement;
        if(root.ValueKind!=JsonValueKind.Object)return;
        if(root.TryGetProperty("id",out var idElement)&&idElement.TryGetInt64(out var id)&&_pending.TryGetValue(id,out var completion))
        {
            if(root.TryGetProperty("error",out var error)&&error.ValueKind==JsonValueKind.Object)
            {
                var code=error.TryGetProperty("code",out var codeElement)&&codeElement.TryGetInt32(out var parsedCode)?parsedCode:-1;
                var message=error.TryGetProperty("message",out var messageElement)?messageElement.GetString():null;
                completion.TrySetException(new HermesRpcException(code,message??"Hermes 请求失败。"));
            }
            else if(root.TryGetProperty("result",out var result))completion.TrySetResult(result.Clone());
            else completion.TrySetResult(default);
            return;
        }
        if(!root.TryGetProperty("method",out var method)||method.GetString()!="event"||!root.TryGetProperty("params",out var parameters)||parameters.ValueKind!=JsonValueKind.Object)return;
        var type=parameters.TryGetProperty("type",out var typeElement)?typeElement.GetString()??string.Empty:string.Empty;
        var sessionId=parameters.TryGetProperty("session_id",out var sessionElement)?sessionElement.GetString()??string.Empty:string.Empty;
        var payload=parameters.TryGetProperty("payload",out var payloadElement)?payloadElement.Clone():JsonSerializer.SerializeToElement(new{});
        if(type=="gateway.ready")gatewayReady.TrySetResult(true);
        try{EventReceived?.Invoke(this,new HermesRpcEvent(type,sessionId,payload));}catch(Exception ex){try{new PrivacyLogger().Error("HermesEventConsumer",ex);}catch{}}
    }

    private async Task CloseConnectionAsync()
    {
        var lifetime=Interlocked.Exchange(ref _connectionLifetime,null);
        lifetime?.Cancel();
        var socket=Interlocked.Exchange(ref _socket,null);
        if(socket is not null)
        {
            try
            {
                if(socket.State==WebSocketState.Open)
                {
                    using var timeout=new CancellationTokenSource(TimeSpan.FromSeconds(2));
                    await socket.CloseOutputAsync(WebSocketCloseStatus.NormalClosure,"MewuAI closing",timeout.Token).ConfigureAwait(false);
                }
            }
            catch(Exception ex)when(ex is OperationCanceledException or WebSocketException or IOException){}
            finally{socket.Abort();socket.Dispose();}
        }
        var receive=Interlocked.Exchange(ref _receiveLoop,null);
        if(receive is not null)try{await receive.WaitAsync(TimeSpan.FromSeconds(2)).ConfigureAwait(false);}catch{}
        lifetime?.Dispose();
        _gatewayReady=null;
    }

    public void Dispose()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        try{CloseConnectionAsync().GetAwaiter().GetResult();}catch{}
        _connectGate.Dispose();_sendGate.Dispose();
    }

    public async ValueTask DisposeAsync()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        try{await CloseConnectionAsync().ConfigureAwait(false);}catch{}
        _connectGate.Dispose();_sendGate.Dispose();
    }
}

public sealed class HermesRpcException(int code,string message):InvalidOperationException(message)
{
    public int Code { get; }=code;
}
