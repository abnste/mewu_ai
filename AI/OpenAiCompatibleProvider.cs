using System.Net.Http;
using System.Net.Http.Headers;
using System.Security.Cryptography;
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.AI;

public class OpenAiCompatibleProvider : IAiProvider
{
    // Keep the connection check provider-agnostic while still proving that the
    // model actually generated a response.  A non-empty response alone can be
    // returned by an error proxy, a fallback model, or a safety message, so the
    // check uses an exact challenge marker instead.
    internal const string ConnectionProbeMarker="MEWU_OK";
    internal const string ConnectionProbePrompt="Reply with exactly MEWU_OK and nothing else.";
    internal const int AttachmentCountLimit=16;
    internal const long RequestBodySizeLimit=64L*1024*1024;
    internal const long ResponseBodySizeLimit=8L*1024*1024;
    private const long JsonStructureBudget=4096;
    private static readonly HttpClient Client=new(new HttpClientHandler{AllowAutoRedirect=false,UseCookies=false}){Timeout=Timeout.InfiniteTimeSpan};
    private readonly AiProviderSettings _settings;
    private readonly string _apiKey;
    private readonly Uri _baseUri;
    private readonly Func<HttpRequestMessage,HttpCompletionOption,CancellationToken,Task<HttpResponseMessage>> _sendAsync;
    private readonly Func<AiRequest,TimeSpan> _requestTimeout;

    public string Id=>_settings.Id;
    public virtual AiProviderCapabilities Capabilities { get; }
    protected virtual bool StreamingContentIsCumulative=>false;
    protected virtual int MaxAttachmentCount=>AttachmentCountLimit;
    protected virtual long MaxRequestBodySize=>RequestBodySizeLimit;

    public OpenAiCompatibleProvider(AiProviderSettings settings,string apiKey)
        :this(settings,apiKey,Client.SendAsync,ProviderRequestTimeoutPolicy.For)
    {
    }

    internal OpenAiCompatibleProvider(
        AiProviderSettings settings,
        string apiKey,
        Func<HttpRequestMessage,HttpCompletionOption,CancellationToken,Task<HttpResponseMessage>> sendAsync,
        Func<AiRequest,TimeSpan> requestTimeout)
    {
        ArgumentNullException.ThrowIfNull(settings);
        ArgumentNullException.ThrowIfNull(sendAsync);
        ArgumentNullException.ThrowIfNull(requestTimeout);
        ProviderHeaderPolicy.EnsureValid(settings.CustomHeaders);
        _settings=settings;
        _apiKey=apiKey??throw new ArgumentNullException(nameof(apiKey));
        _baseUri=ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl);
        _sendAsync=sendAsync;
        _requestTimeout=requestTimeout;
        Capabilities=VolcengineModelPolicy.IsEndpoint(_baseUri)
            ?VolcengineModelPolicy.GetCapabilities(settings.Model)
            :new(true,false,true,20L*1024*1024,0,TimeSpan.Zero,new HashSet<string>(["image/png","image/jpeg","image/webp"],StringComparer.OrdinalIgnoreCase));
    }

    public async Task<bool> TestConnectionAsync(CancellationToken token)
    {
        // Do not use a provider-specific endpoint or request shape here: the
        // regular chat-completions path is the compatibility contract shared
        // by MiniMax and other OpenAI-compatible providers.  The exact marker
        // makes a successful HTTP response insufficient on its own.
        var result=await SendAsync(new AiRequest{Prompt=ConnectionProbePrompt},token).ConfigureAwait(false);
        return MatchesConnectionProbe(result.Answer);
    }

    internal static bool MatchesConnectionProbe(string? answer)=>
        string.Equals(answer?.Trim(),ConnectionProbeMarker,StringComparison.OrdinalIgnoreCase);

    public virtual async Task<AiResult> SendAsync(AiRequest request,CancellationToken token)
    {
        ArgumentNullException.ThrowIfNull(request);
        try
        {
            var timeout=_requestTimeout(request);
            if(timeout<=TimeSpan.Zero||timeout==Timeout.InfiniteTimeSpan)throw new InvalidOperationException("Provider 请求超时必须是有限的正数");
            using var timeoutSource=CancellationTokenSource.CreateLinkedTokenSource(token);
            timeoutSource.CancelAfter(timeout);
            try{return await SendCoreAsync(request,timeoutSource.Token).ConfigureAwait(false);}
            catch(OperationCanceledException exception) when(token.IsCancellationRequested)
            {
                throw new OperationCanceledException("AI 请求已取消",exception,token);
            }
            catch(OperationCanceledException exception) when(timeoutSource.IsCancellationRequested)
            {
                throw new TimeoutException($"AI 请求超过 {FormatTimeout(timeout)} 未完成，请检查网络后重试",exception);
            }
        }
        finally
        {
            ClearOwnedAttachmentData(request.Attachments);
        }
    }

    private async Task<AiResult> SendCoreAsync(AiRequest request,CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        ValidateRequest(request);
        var streaming=request.StreamingProgress is not null&&Capabilities.SupportsStreaming;
        var attachments=await LoadAttachmentsAsync(request.Attachments,token).ConfigureAwait(false);
        SerializedRequest serialized;
        try
        {
            serialized=await Task.Run(()=>
            {
                token.ThrowIfCancellationRequested();
                var result=SerializeRequest(request,attachments,streaming,token);
                try{ValidateSerializedRequestBody(result.Body.LongLength);return result;}
                catch{CryptographicOperations.ZeroMemory(result.Body);throw;}
            },token).ConfigureAwait(false);
        }
        finally
        {
            foreach(var attachment in attachments)
                if(attachment.OwnsBytes)CryptographicOperations.ZeroMemory(attachment.Bytes);
        }

        HttpResponseMessage response;
        try
        {
            using var httpRequest=Create(HttpMethod.Post,"chat/completions");
            httpRequest.Content=new ByteArrayContent(serialized.Body);
            httpRequest.Content.Headers.ContentType=new MediaTypeHeaderValue("application/json"){CharSet="utf-8"};
            response=await _sendAsync(httpRequest,HttpCompletionOption.ResponseHeadersRead,token).ConfigureAwait(false);
        }
        finally{CryptographicOperations.ZeroMemory(serialized.Body);}
        using(response)
        {
        if(!response.IsSuccessStatusCode)throw new InvalidOperationException($"AI 请求失败（HTTP {(int)response.StatusCode}）");
        EnsureDeclaredResponseBodySize(response.Content);

        if(streaming)
        {
            var responseStream=await response.Content.ReadAsStreamAsync(token).ConfigureAwait(false);
            using var limitedStream=new ResponseSizeLimitedStream(responseStream,ResponseBodySizeLimit);
            using var reader=new StreamReader(limitedStream,Encoding.UTF8,true,4096,false);
            var accumulator=new StreamingResponseAccumulator(StreamingContentIsCumulative,request.ExpectStructuredResponse);
            var completed=false;
            while(await reader.ReadLineAsync(token).ConfigureAwait(false) is { } line)
            {
                if(!StreamingResponseParser.TryParse(line,out var delta,out var done))continue;
                if(accumulator.Accept(delta,done,request.StreamingProgress,request.StreamingCompletionPredicate)){completed=true;break;}
            }
            if(!completed)throw new InvalidDataException("AI 流式响应意外中断，请重试");
            return accumulator.BuildResult();
        }

        var json=await ReadResponseBodyAsStringAsync(response.Content,token).ConfigureAwait(false);
        using var document=JsonDocument.Parse(json);
        var message=document.RootElement.GetProperty("choices")[0].GetProperty("message");
        var answerText=ReadString(message,"content");
        var reasoningText=ReadString(message,"reasoning_content");
        if(reasoningText.Length==0)reasoningText=ReadString(message,"thinking_content");
        if(reasoningText.Length==0)reasoningText=StreamingResponseParser.ReadReasoningDetails(message);
        return StructuredResponseParser.Parse(answerText,reasoningText,request.ExpectStructuredResponse);
        }
    }

    protected virtual void ValidateRequest(AiRequest request)
    {
        if(request.MaxOutputTokens is <=0)throw new ArgumentOutOfRangeException(nameof(request),"最大输出 Token 必须大于 0");
        if(request.History is null)throw new InvalidOperationException("对话历史不能为空");
        if(request.Attachments is null)throw new InvalidOperationException("附件列表不能为空");
        if(request.Attachments.Count>MaxAttachmentCount)throw new InvalidOperationException($"单次请求最多支持 {MaxAttachmentCount} 个附件，请减少选区或分批发送");
        ConversationContextPolicy.EnsureValidForProvider(request.History);
        if(request.Prompt is null)throw new InvalidOperationException("当前问题不能为空");
        if(string.IsNullOrWhiteSpace(request.Prompt)&&request.Attachments.Count==0)throw new InvalidOperationException("问题和附件不能同时为空");
        var estimatedBodyBytes=EstimateJsonEnvelopeBytes(request);
        if(estimatedBodyBytes>MaxRequestBodySize)throw CreateRequestBodyTooLargeException(estimatedBodyBytes);
        var rawAttachmentBytes=0L;
        foreach(var attachment in request.Attachments)
        {
            if(attachment is null)throw new InvalidOperationException("附件列表包含空项");
            if(attachment.Data is not null&&!string.IsNullOrWhiteSpace(attachment.FilePath))throw new InvalidOperationException("附件不能同时包含内存数据和文件路径");
            if(!Enum.IsDefined(attachment.Type))throw new InvalidOperationException("附件类型无效");
            if(attachment.Type==AiAttachmentType.Image&&!Capabilities.SupportsImage)throw new NotSupportedException("当前模型不支持图片");
            if(attachment.Type==AiAttachmentType.Video&&!Capabilities.SupportsVideo)throw new NotSupportedException("当前模型不支持视频");
            if(string.IsNullOrWhiteSpace(attachment.MimeType))throw new InvalidOperationException("附件 MIME 类型不能为空");
            if(attachment.Type!=AiAttachmentType.Text&&Capabilities.AcceptedMimeTypes.Count>0&&!Capabilities.AcceptedMimeTypes.Contains(attachment.MimeType))throw new NotSupportedException($"当前模型不接受 {attachment.MimeType} 附件");
            var size=GetAttachmentSize(attachment);
            if(size<=0)throw new InvalidOperationException("附件内容为空");
            ValidateAttachmentSize(attachment,size);
            rawAttachmentBytes=AddSaturating(rawAttachmentBytes,size);
            estimatedBodyBytes=AddSaturating(estimatedBodyBytes,160);
            estimatedBodyBytes=AddSaturating(estimatedBodyBytes,EstimateBase64DataUrlBytes(size,attachment.MimeType));
            if(rawAttachmentBytes>MaxRequestBodySize||estimatedBodyBytes>MaxRequestBodySize)throw CreateRequestBodyTooLargeException(estimatedBodyBytes);
            if(attachment.Duration is { } duration&&duration<TimeSpan.Zero)throw new InvalidOperationException("视频时长不能为负数");
            if(attachment.Duration is { } limitedDuration&&Capabilities.MaxVideoDuration>TimeSpan.Zero&&limitedDuration>Capabilities.MaxVideoDuration)throw new InvalidOperationException("视频时长超过当前模型限制");
        }
    }

    protected virtual void ValidateAttachmentSize(AiAttachment attachment,long size)
    {
        var limit=Capabilities.MaxSizeFor(attachment.Type);
        if(limit>0&&size>limit)
        {
            var kind=attachment.Type==AiAttachmentType.Image?"图片":attachment.Type==AiAttachmentType.Video?"视频":"文本文件";
            throw new InvalidOperationException($"{kind}超过当前模型的 {FormatMegabytes(limit)} MB 单文件限制");
        }
    }

    protected virtual void ValidateSerializedRequestBody(long utf8Length)
    {
        if(utf8Length>MaxRequestBodySize)throw CreateRequestBodyTooLargeException(utf8Length);
    }

    protected virtual InvalidOperationException CreateRequestBodyTooLargeException(long bytes)=>
        new($"附件经 Base64 展开后请求体预计为 {FormatMegabytes(bytes)} MB，超过 64 MB 聚合限制；请减少附件数量，或将单个视频压缩至约 47 MB 以下");

    protected static long GetAttachmentSize(AiAttachment attachment)
    {
        if(attachment.Data is not null)return attachment.Data.LongLength;
        if(string.IsNullOrWhiteSpace(attachment.FilePath))throw new InvalidOperationException("附件缺少数据或文件路径");
        var file=new FileInfo(attachment.FilePath);
        if(!file.Exists)throw new FileNotFoundException("附件文件不存在",attachment.FilePath);
        return file.Length;
    }

    protected HttpRequestMessage Create(HttpMethod method,string relative)
    {
        var request=new HttpRequestMessage(method,new Uri(_baseUri,relative.TrimStart('/')));
        var hasCustomAuthorization=_settings.CustomHeaders.Keys.Any(name=>name.Equals("Authorization",StringComparison.OrdinalIgnoreCase));
        if(!hasCustomAuthorization&&!string.IsNullOrWhiteSpace(_apiKey))request.Headers.Authorization=new AuthenticationHeaderValue("Bearer",_apiKey);
        foreach(var header in _settings.CustomHeaders)
            if(!request.Headers.TryAddWithoutValidation(header.Key,header.Value))throw new InvalidOperationException($"无法添加 Provider 请求头：{header.Key}");
        return request;
    }

    private async Task<List<LoadedAttachment>> LoadAttachmentsAsync(IReadOnlyList<AiAttachment> attachments,CancellationToken token)
    {
        var loaded=new List<LoadedAttachment>(attachments.Count);
        try
        {
            foreach(var attachment in attachments)
            {
                token.ThrowIfCancellationRequested();
                if(attachment.Data is { } data)
                {
                    ValidateAttachmentSize(attachment,data.LongLength);
                    loaded.Add(new(attachment,data,attachment.ProviderOwnsData));
                    continue;
                }
                var bytes=await File.ReadAllBytesAsync(attachment.FilePath!,token).ConfigureAwait(false);
                ValidateAttachmentSize(attachment,bytes.LongLength);
                loaded.Add(new(attachment,bytes,true));
            }
            return loaded;
        }
        catch
        {
            foreach(var attachment in loaded)
                if(attachment.OwnsBytes)CryptographicOperations.ZeroMemory(attachment.Bytes);
            throw;
        }
    }

    internal long EstimateRequestBodyBytes(AiRequest request)
    {
        ArgumentNullException.ThrowIfNull(request);
        if(request.History is null||request.Attachments is null)return long.MaxValue;
        var total=EstimateJsonEnvelopeBytes(request);
        foreach(var attachment in request.Attachments)
        {
            if(attachment is null)return long.MaxValue;
            total=AddSaturating(total,160);
            total=AddSaturating(total,EstimateBase64DataUrlBytes(GetAttachmentSize(attachment),attachment.MimeType));
        }
        return total;
    }

    internal static long EstimateBase64DataUrlBytes(long rawBytes,string? mimeType)
    {
        if(rawBytes<0)return long.MaxValue;
        var groups=rawBytes>(long.MaxValue-2)?long.MaxValue:(rawBytes+2)/3;
        var base64=groups>long.MaxValue/4?long.MaxValue:groups*4;
        return AddSaturating(base64,AddSaturating(13,EstimateJsonStringBytesUpperBound(mimeType)));
    }

    private long EstimateJsonEnvelopeBytes(AiRequest request)
    {
        var total=JsonStructureBudget;
        total=AddSaturating(total,EstimateJsonStringBytesUpperBound(request.Prompt));
        total=AddSaturating(total,EstimateJsonStringBytesUpperBound(_settings.Model));
        foreach(var message in request.History)
        {
            total=AddSaturating(total,64);
            total=AddSaturating(total,EstimateJsonStringBytesUpperBound(message?.Role));
            total=AddSaturating(total,EstimateJsonStringBytesUpperBound(message?.Text));
        }
        return total;
    }

    private SerializedRequest SerializeRequest(AiRequest request,IReadOnlyList<LoadedAttachment> attachments,bool streaming,CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        var content=new List<object>();
        if(!string.IsNullOrWhiteSpace(request.Prompt))content.Add(new{type="text",text=request.Prompt});
        foreach(var loaded in attachments)
        {
            token.ThrowIfCancellationRequested();
            var attachment=loaded.Attachment;
            var dataUrl=$"data:{attachment.MimeType};base64,{Convert.ToBase64String(loaded.Bytes)}";
            token.ThrowIfCancellationRequested();
            if(attachment.Type==AiAttachmentType.Image)content.Add(new{type="image_url",image_url=new{url=dataUrl}});
            else if(attachment.Type==AiAttachmentType.Video)content.Add(new{type="video_url",video_url=new{url=dataUrl,fps=2}});
            else content.Add(new{type="text",text=System.Text.Encoding.UTF8.GetString(loaded.Bytes)});
        }

        var messages=new List<object>(request.History.Count+1);
        foreach(var message in request.History)
        {
            token.ThrowIfCancellationRequested();
            messages.Add(new{role=message.Role,content=message.Text});
        }
        messages.Add(new{role="user",content});
        var bodyValues=new Dictionary<string,object?>{{"model",_settings.Model},{"messages",messages},{"temperature",.2},{"stream",streaming}};
        var miniMaxM3=_settings.Type.Equals("MiniMax",StringComparison.OrdinalIgnoreCase)&&_settings.Model.Equals("MiniMax-M3",StringComparison.OrdinalIgnoreCase);
        if(request.MaxOutputTokens is { } maxTokens)bodyValues[miniMaxM3?"max_completion_tokens":"max_tokens"]=maxTokens;
        if(miniMaxM3)
        {
            bodyValues["reasoning_split"]=true;
            bodyValues["thinking"]=new{type=request.DisableReasoning?"disabled":"adaptive"};
        }
        else if(request.DisableReasoning&&HostMatches(_baseUri.Host,"volces.com"))
        {
            bodyValues["thinking"]=new{type="disabled"};
            bodyValues["reasoning_effort"]="minimal";
        }
        token.ThrowIfCancellationRequested();
        var body=JsonSerializer.SerializeToUtf8Bytes(bodyValues);
        try
        {
            token.ThrowIfCancellationRequested();
            return new(body);
        }
        catch
        {
            CryptographicOperations.ZeroMemory(body);
            throw;
        }
    }

    private static bool HostMatches(string host,string domain)=>host.Equals(domain,StringComparison.OrdinalIgnoreCase)||host.EndsWith("."+domain,StringComparison.OrdinalIgnoreCase);
    private static string FormatTimeout(TimeSpan timeout)=>timeout.TotalMinutes>=1?$"{timeout.TotalMinutes:0.#} 分钟":$"{timeout.TotalSeconds:0.#} 秒";
    private static string FormatMegabytes(long bytes)=>bytes==long.MaxValue?"超大":(bytes/(1024d*1024d)).ToString("0.##",System.Globalization.CultureInfo.InvariantCulture);
    private static void EnsureDeclaredResponseBodySize(HttpContent content)
    {
        if(content.Headers.ContentLength is >ResponseBodySizeLimit)throw CreateResponseBodyTooLargeException();
    }

    private static async Task<string> ReadResponseBodyAsStringAsync(HttpContent content,CancellationToken token)
    {
        var responseStream=await content.ReadAsStreamAsync(token).ConfigureAwait(false);
        using var limitedStream=new ResponseSizeLimitedStream(responseStream,ResponseBodySizeLimit);
        var capacity=content.Headers.ContentLength is >0 ? checked((int)content.Headers.ContentLength.Value) : 0;
        using var buffer=capacity>0?new MemoryStream(capacity):new MemoryStream();
        var chunk=new byte[81920];
        try
        {
            while(true)
            {
                var count=await limitedStream.ReadAsync(chunk.AsMemory(),token).ConfigureAwait(false);
                if(count==0)break;
                buffer.Write(chunk,0,count);
            }
            return Encoding.UTF8.GetString(buffer.GetBuffer(),0,checked((int)buffer.Length));
        }
        finally
        {
            CryptographicOperations.ZeroMemory(chunk);
            if(buffer.TryGetBuffer(out var segment))CryptographicOperations.ZeroMemory(segment.AsSpan(0,checked((int)buffer.Length)));
        }
    }

    private static InvalidDataException CreateResponseBodyTooLargeException()=>
        new($"AI 返回内容超过 {FormatMegabytes(ResponseBodySizeLimit)} MB 安全上限，请缩短问题、清理对话历史或降低最大输出 Token 后重试");

    internal static long EstimateJsonStringBytesUpperBound(string? value)
    {
        if(string.IsNullOrEmpty(value))return 2;
        var total=2L;
        foreach(var character in value)
        {
            var encodedBytes=character switch
            {
                _ when char.IsSurrogate(character)=>6,
                _ when JavaScriptEncoder.Default.WillEncode(character)=>6,
                <=(char)0x7f=>1,
                <=(char)0x7ff=>2,
                _=>3
            };
            total=AddSaturating(total,encodedBytes);
        }
        return total;
    }
    private static long AddSaturating(long left,long right)=>left<0||right<0||left>long.MaxValue-right?long.MaxValue:left+right;
    private static string ReadString(JsonElement value,string name)=>value.TryGetProperty(name,out var property)&&property.ValueKind==JsonValueKind.String?property.GetString()??string.Empty:string.Empty;

    private static void ClearOwnedAttachmentData(IReadOnlyList<AiAttachment>? attachments)
    {
        if(attachments is null)return;
        foreach(var attachment in attachments)
            if(attachment is {ProviderOwnsData:true,Data:{ } data})CryptographicOperations.ZeroMemory(data);
    }

    private sealed record LoadedAttachment(AiAttachment Attachment,byte[] Bytes,bool OwnsBytes);
    private sealed record SerializedRequest(byte[] Body);

    private sealed class ResponseSizeLimitedStream(Stream inner,long limit):Stream
    {
        private long _bytesRead;

        public override bool CanRead=>inner.CanRead;
        public override bool CanSeek=>false;
        public override bool CanWrite=>false;
        public override long Length=>throw new NotSupportedException();
        public override long Position { get=>throw new NotSupportedException();set=>throw new NotSupportedException(); }
        public override void Flush()=>throw new NotSupportedException();
        public override long Seek(long offset,SeekOrigin origin)=>throw new NotSupportedException();
        public override void SetLength(long value)=>throw new NotSupportedException();
        public override void Write(byte[] buffer,int offset,int count)=>throw new NotSupportedException();

        public override int Read(byte[] buffer,int offset,int count)=>Read(buffer.AsSpan(offset,count));

        public override int Read(Span<byte> buffer)
        {
            var count=inner.Read(buffer[..GetProbeLength(buffer.Length)]);
            return CommitRead(count);
        }

        public override Task<int> ReadAsync(byte[] buffer,int offset,int count,CancellationToken cancellationToken)=>
            ReadAsync(buffer.AsMemory(offset,count),cancellationToken).AsTask();

        public override async ValueTask<int> ReadAsync(Memory<byte> buffer,CancellationToken cancellationToken=default)
        {
            var count=await inner.ReadAsync(buffer[..GetProbeLength(buffer.Length)],cancellationToken).ConfigureAwait(false);
            return CommitRead(count);
        }

        protected override void Dispose(bool disposing)
        {
            if(disposing)inner.Dispose();
            base.Dispose(disposing);
        }

        private int GetProbeLength(int requested)
        {
            if(requested==0)return 0;
            var remaining=limit-_bytesRead;
            return (int)Math.Min(requested,remaining+1);
        }

        private int CommitRead(int count)
        {
            if(count>limit-_bytesRead)throw CreateResponseBodyTooLargeException();
            _bytesRead+=count;
            return count;
        }
    }
}

internal sealed class StreamingResponseAccumulator
{
    private readonly StringBuilder _answer=new();
    private readonly bool _contentIsCumulative;
    private readonly bool _expectStructuredResponse;
    private string _cumulativeAnswer=string.Empty;
    private string _reasoning=string.Empty;

    internal StreamingResponseAccumulator(bool contentIsCumulative=false,bool expectStructuredResponse=false)
    {
        _contentIsCumulative=contentIsCumulative;
        _expectStructuredResponse=expectStructuredResponse;
    }

    public bool Accept(AiStreamDelta delta,bool done,IProgress<AiStreamDelta>? progress,Func<string,bool>? completionPredicate)
    {
        var contentDelta=delta.Content;
        var contentChanged=delta.Content.Length>0;
        if(delta.Content.Length>0)
        {
            if(_contentIsCumulative)
            {
                contentDelta=AppendCumulativeBlock(ref _cumulativeAnswer,delta.Content);
            }
            else _answer.Append(delta.Content);
        }
        contentChanged=contentDelta.Length>0;
        var reasoningDelta=delta.ReasoningContent;
        if(delta.ReasoningIsCumulative)
        {
            reasoningDelta=AppendCumulativeBlock(ref _reasoning,delta.ReasoningContent);
        }
        else if(delta.ReasoningContent.Length>0)_reasoning+=delta.ReasoningContent;
        if(contentDelta.Length>0||reasoningDelta.Length>0)progress?.Report(new AiStreamDelta(contentDelta,reasoningDelta));
        if(contentChanged&&completionPredicate?.Invoke(CurrentAnswer())==true)return true;
        return done;
    }

    private string CurrentAnswer()=>_contentIsCumulative?_cumulativeAnswer:_answer.ToString();
    public AiResult BuildResult()=>StructuredResponseParser.Parse(CurrentAnswer(),_reasoning,_expectStructuredResponse);

    private static string AppendCumulativeBlock(ref string accumulated,string incoming)
    {
        if(incoming.Length==0)return string.Empty;
        if(accumulated.Length==0){accumulated=incoming;return incoming;}
        if(incoming.StartsWith(accumulated,StringComparison.Ordinal))
        {
            var suffix=incoming[accumulated.Length..];accumulated=incoming;return suffix;
        }
        if(accumulated.StartsWith(incoming,StringComparison.Ordinal))return string.Empty;

        // MiniMax usually emits the whole cumulative value, but a long
        // multimodal response can restart that cumulative value at a sentence
        // boundary.  Such a reset is a continuation, not an instruction to
        // discard everything already received.  Preserve the prior segment and
        // remove only an exact suffix/prefix overlap so neither the final result
        // nor the live UI loses text or duplicates the boundary.
        var overlap=FindSuffixPrefixOverlap(accumulated,incoming);
        var addition=incoming[overlap..];
        accumulated=string.Concat(accumulated,addition);
        return addition;
    }

    private static int FindSuffixPrefixOverlap(string accumulated,string incoming)
    {
        if(accumulated.Length==0||incoming.Length==0)return 0;
        var prefix=new int[incoming.Length];
        for(var index=1;index<incoming.Length;index++)
        {
            var matched=prefix[index-1];
            while(matched>0&&incoming[index]!=incoming[matched])matched=prefix[matched-1];
            if(incoming[index]==incoming[matched])matched++;
            prefix[index]=matched;
        }

        var current=0;
        var start=Math.Max(0,accumulated.Length-incoming.Length);
        for(var index=start;index<accumulated.Length;index++)
        {
            while(current>0&&accumulated[index]!=incoming[current])current=prefix[current-1];
            if(accumulated[index]==incoming[current])current++;
            if(current==incoming.Length)
            {
                if(index==accumulated.Length-1)return current;
                current=prefix[current-1];
            }
        }
        return current;
    }
}
