using System.Buffers;
using System.Net.Http;
using System.Net.Http.Json;
using System.Security.Cryptography;
using System.Text.Json;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

public sealed class HermesRuntimeService : IDisposable, IAsyncDisposable
{
    internal static readonly string[] ReasoningEfforts=["none","minimal","low","medium","high","xhigh","max","ultra"];
    private const int MaxTtsBytes=32*1024*1024;
    private const int MaxTtsResponseBytes=44*1024*1024;
    private const int MaxTtsTextChars=100_000;
    private readonly HermesBackendService _backend;
    private readonly HermesJsonRpcClient _rpc;
    private readonly HttpClient _http;
    private HermesAiProvider? _provider;
    private readonly object _providerGate=new();
    private int _disposed;

    public HermesRuntimeService(HermesBackendService? backend=null)
    {
        _backend=backend??new HermesBackendService();
        _rpc=new HermesJsonRpcClient(_backend);
        _http=new HttpClient(new SocketsHttpHandler
        {
            UseProxy=false,
            UseCookies=false,
            AllowAutoRedirect=false,
            AutomaticDecompression=System.Net.DecompressionMethods.None,
            ConnectTimeout=TimeSpan.FromSeconds(15)
        }){Timeout=TimeSpan.FromSeconds(75)};
    }

    public HermesInstallation? Discover()=>_backend.Discover();
    public bool IsRunning=>_backend.IsRunning;
    internal event EventHandler<HermesRpcEvent>? EventReceived
    {
        add=>_rpc.EventReceived+=value;
        remove=>_rpc.EventReceived-=value;
    }
    internal long ConnectionGeneration=>_rpc.ConnectionGeneration;

    internal Task<HermesConnectionInfo> EnsureConnectedAsync(CancellationToken cancellationToken)=>_rpc.EnsureConnectedAsync(cancellationToken);

    public IAiProvider GetConversationProvider(HermesConversationKind kind,Func<AppSettings> settingsAccessor)
    {
        ArgumentNullException.ThrowIfNull(settingsAccessor);
        lock(_providerGate)
        {
            // Text and screen entry points deliberately share one Hermes
            // session. The request itself carries the expected response shape,
            // so switching surfaces or models never discards conversation state.
            return _provider??=new HermesAiProvider(this,settingsAccessor);
        }
    }

    public async Task<IReadOnlyList<HermesModelOption>> GetModelOptionsAsync(bool refresh,CancellationToken cancellationToken)
    {
        var result=await InvokeAsync("model.options",new{refresh},cancellationToken).ConfigureAwait(false);
        return ParseModelOptions(result);
    }

    public async Task<bool> TestConnectionAsync(CancellationToken cancellationToken)
    {
        _=await GetModelOptionsAsync(false,cancellationToken).ConfigureAwait(false);
        return true;
    }

    internal Task<JsonElement> InvokeAsync(string method,object? parameters,CancellationToken cancellationToken)=>_rpc.InvokeAsync(method,parameters,cancellationToken);

    public async Task<HermesSpeechAudio> SynthesizeSpeechAsync(string text,CancellationToken cancellationToken)
    {
        if(string.IsNullOrWhiteSpace(text))throw new ArgumentException("朗读内容不能为空。",nameof(text));
        if(text.Length>MaxTtsTextChars)throw new InvalidOperationException("朗读内容过长，请缩短后重试。");
        var connection=await _rpc.EnsureConnectedAsync(cancellationToken).ConfigureAwait(false);
        if(!connection.HttpBaseUri.IsLoopback||!string.Equals(connection.HttpBaseUri.Scheme,Uri.UriSchemeHttp,StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("拒绝向非本机 Hermes 地址发送朗读内容。");
        using var request=new HttpRequestMessage(HttpMethod.Post,new Uri(connection.HttpBaseUri,"api/audio/speak"));
        request.Headers.TryAddWithoutValidation("X-Hermes-Session-Token",_backend.GetSessionToken());
        request.Content=JsonContent.Create(new{text});
        using var response=await _http.SendAsync(request,HttpCompletionOption.ResponseHeadersRead,cancellationToken).ConfigureAwait(false);
        if(!response.IsSuccessStatusCode)throw new InvalidOperationException($"Hermes 自动朗读不可用（HTTP {(int)response.StatusCode}）。请检查 Hermes 的语音配置。");
        if(response.Content.Headers.ContentLength is >MaxTtsResponseBytes)throw new InvalidDataException("Hermes 返回的朗读数据超过安全上限。");
        await using var stream=await response.Content.ReadAsStreamAsync(cancellationToken).ConfigureAwait(false);
        return await DecodeSpeechResponseAsync(stream,cancellationToken).ConfigureAwait(false);
    }

    internal static IReadOnlyList<HermesModelOption> ParseModelOptions(JsonElement result)
    {
        var options=new List<HermesModelOption>();
        if(result.ValueKind!=JsonValueKind.Object||!result.TryGetProperty("providers",out var providers)||providers.ValueKind!=JsonValueKind.Array)return options;
        var currentProvider=ReadString(result,"provider");
        var currentModel=ReadString(result,"model");
        foreach(var providerRow in providers.EnumerateArray())
        {
            if(providerRow.ValueKind!=JsonValueKind.Object)continue;
            var provider=ReadString(providerRow,"slug");
            if(string.IsNullOrWhiteSpace(provider))continue;
            var providerName=ReadString(providerRow,"name");
            if(string.IsNullOrWhiteSpace(providerName))providerName=provider;
            if(!providerRow.TryGetProperty("models",out var models)||models.ValueKind!=JsonValueKind.Array)continue;
            var capabilities=providerRow.TryGetProperty("capabilities",out var caps)&&caps.ValueKind==JsonValueKind.Object?caps:default;
            foreach(var modelElement in models.EnumerateArray())
            {
                var model=modelElement.ValueKind==JsonValueKind.String?modelElement.GetString():null;
                if(string.IsNullOrWhiteSpace(model))continue;
                var supportsReasoning=true;
                if(capabilities.ValueKind==JsonValueKind.Object&&capabilities.TryGetProperty(model,out var modelCaps)&&modelCaps.ValueKind==JsonValueKind.Object&&modelCaps.TryGetProperty("reasoning",out var reasoning)&&reasoning.ValueKind is JsonValueKind.True or JsonValueKind.False)
                    supportsReasoning=reasoning.GetBoolean();
                var isCurrent=string.Equals(provider,currentProvider,StringComparison.Ordinal)&&string.Equals(model,currentModel,StringComparison.Ordinal);
                options.Add(new HermesModelOption(provider,model,$"{providerName} · {model}",supportsReasoning?ReasoningEfforts:["none"],isCurrent));
            }
        }
        return options.OrderByDescending(option=>option.IsCurrent).ToList();
    }

    internal static HermesSpeechAudio DecodeSpeechDataUrl(string dataUrl)
    {
        var comma=dataUrl.IndexOf(',');
        if(comma<=5||!dataUrl.StartsWith("data:",StringComparison.OrdinalIgnoreCase)||dataUrl[..comma].IndexOf(";base64",StringComparison.OrdinalIgnoreCase)<0)
            throw new InvalidDataException("Hermes 返回了无效的音频数据。");
        var metadata=dataUrl[5..comma];
        var semicolon=metadata.IndexOf(';');
        var mime=(semicolon>=0?metadata[..semicolon]:metadata).Trim().ToLowerInvariant();
        var extension=mime switch{"audio/mpeg" or "audio/mp3"=>".mp3","audio/wav" or "audio/x-wav"=>".wav","audio/ogg"=>".ogg","audio/flac" or "audio/x-flac"=>".flac","audio/mp4" or "audio/m4a"=>".m4a",_=>throw new NotSupportedException($"Hermes 返回了不支持的音频格式：{mime}")};
        var encodedLength=dataUrl.Length-comma-1;
        var maxEncodedLength=((MaxTtsBytes+2L)/3L)*4L;
        if(encodedLength<=0||encodedLength>maxEncodedLength)throw new InvalidDataException("Hermes 返回的音频为空或超过安全上限。");
        byte[] bytes;
        try{bytes=Convert.FromBase64String(dataUrl[(comma+1)..]);}
        catch(FormatException ex){throw new InvalidDataException("Hermes 返回的音频 Base64 无效。",ex);}
        if(bytes.Length==0||bytes.Length>MaxTtsBytes){Array.Clear(bytes);throw new InvalidDataException("Hermes 返回的音频为空或超过安全上限。");}
        return new HermesSpeechAudio(mime,extension,bytes);
    }

    private static async Task<HermesSpeechAudio> DecodeSpeechResponseAsync(Stream stream,CancellationToken cancellationToken)
    {
        var buffer=ArrayPool<byte>.Shared.Rent(81_920);
        using var payload=new MemoryStream();
        try
        {
            while(true)
            {
                var read=await stream.ReadAsync(buffer.AsMemory(),cancellationToken).ConfigureAwait(false);
                if(read==0)break;
                if(payload.Length+read>MaxTtsResponseBytes)throw new InvalidDataException("Hermes 返回的朗读数据超过安全上限。");
                payload.Write(buffer,0,read);
            }
            using var document=JsonDocument.Parse(payload.GetBuffer().AsMemory(0,checked((int)payload.Length)),new JsonDocumentOptions{MaxDepth=16});
            if(!document.RootElement.TryGetProperty("data_url",out var dataElement)||dataElement.ValueKind!=JsonValueKind.String)
                throw new InvalidDataException("Hermes 自动朗读没有返回音频。");
            var dataUrl=dataElement.GetString();
            if(string.IsNullOrWhiteSpace(dataUrl))throw new InvalidDataException("Hermes 自动朗读没有返回音频。");
            return DecodeSpeechDataUrl(dataUrl);
        }
        catch(JsonException ex){throw new InvalidDataException("Hermes 自动朗读返回了无效数据。",ex);}
        finally
        {
            CryptographicOperations.ZeroMemory(buffer);
            ArrayPool<byte>.Shared.Return(buffer);
            if(payload.TryGetBuffer(out var bytes))CryptographicOperations.ZeroMemory(bytes.AsSpan(0,checked((int)payload.Length)));
        }
    }

    private static string ReadString(JsonElement element,string name)=>element.TryGetProperty(name,out var value)&&value.ValueKind==JsonValueKind.String?value.GetString()??string.Empty:string.Empty;

    public void Dispose()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        lock(_providerGate){_provider?.Dispose();_provider=null;}
        _rpc.Dispose();_backend.Dispose();_http.Dispose();
    }

    public async ValueTask DisposeAsync()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        lock(_providerGate){_provider?.Dispose();_provider=null;}
        await _rpc.DisposeAsync().ConfigureAwait(false);
        await _backend.DisposeAsync().ConfigureAwait(false);
        _http.Dispose();
    }
}

public sealed class HermesSpeechAudio(string mimeType,string extension,byte[] data):IDisposable
{
    private int _disposed;
    public string MimeType { get; }=mimeType;
    public string Extension { get; }=extension;
    public byte[] Data { get; }=data;
    public void Dispose(){if(Interlocked.Exchange(ref _disposed,1)==0)CryptographicOperations.ZeroMemory(Data);}
}
