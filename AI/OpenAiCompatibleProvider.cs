// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
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
    internal static readonly TimeSpan ReasoningOnlyRetryDelay=TimeSpan.FromMilliseconds(250);
    internal static readonly TimeSpan InterruptedStreamRetryDelay=TimeSpan.FromMilliseconds(250);
    internal static readonly TimeSpan PrematureTerminalGracePeriod=TimeSpan.FromMilliseconds(1500);
    private const long JsonStructureBudget=4096;
    private readonly AiProviderSettings _settings;
    private readonly string _apiKey;
    private readonly Uri _baseUri;
    private readonly Func<HttpRequestMessage,HttpCompletionOption,CancellationToken,Task<HttpResponseMessage>> _sendAsync;
    private readonly HttpClient _httpClient;
    private readonly Func<AiRequest,TimeSpan> _requestTimeout;

    public string Id=>_settings.Id;
    public virtual AiProviderCapabilities Capabilities { get; }
    protected virtual bool StreamingContentIsCumulative=>ProviderModelPolicy.UsesCumulativeContent(_settings);
    protected virtual int MaxAttachmentCount=>AttachmentCountLimit;
    protected virtual long MaxRequestBodySize=>ProviderModelPolicy.MaximumRequestBytes(_settings);
    protected virtual int VideoSamplingFramesPerSecond=>2;
    private string Protocol=>ProviderProtocolPolicy.ApiFormat(_settings);

    public OpenAiCompatibleProvider(AiProviderSettings settings,string apiKey)
        :this(settings,apiKey,NetworkHttpClientFactory.Create(),null,ProviderRequestTimeoutPolicy.For)
    {
    }

    internal OpenAiCompatibleProvider(
        AiProviderSettings settings,
        string apiKey,
        Func<HttpRequestMessage,HttpCompletionOption,CancellationToken,Task<HttpResponseMessage>>? sendAsync,
        Func<AiRequest,TimeSpan> requestTimeout)
        :this(settings,apiKey,NetworkHttpClientFactory.Create(),sendAsync,requestTimeout)
    {
    }

    private OpenAiCompatibleProvider(
        AiProviderSettings settings,
        string apiKey,
        HttpClient httpClient,
        Func<HttpRequestMessage,HttpCompletionOption,CancellationToken,Task<HttpResponseMessage>>? sendAsync,
        Func<AiRequest,TimeSpan> requestTimeout)
    {
        ArgumentNullException.ThrowIfNull(settings);
        ArgumentNullException.ThrowIfNull(requestTimeout);
        ProviderHeaderPolicy.EnsureValid(settings.CustomHeaders);
        ProviderProtocolPolicy.Validate(settings);
        ProviderModelPolicy.ValidateRequestParameters(settings);
        _settings=settings;
        _apiKey=apiKey??throw new ArgumentNullException(nameof(apiKey));
        _httpClient=httpClient??throw new ArgumentNullException(nameof(httpClient));
        _baseUri=ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl);
        _sendAsync=sendAsync??httpClient.SendAsync;
        _requestTimeout=requestTimeout;
        Capabilities=ProviderModelPolicy.GetCapabilities(settings);
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
            try
            {
                AiResult result;
                try
                {
                    result=await SendCoreAsync(request,timeoutSource.Token).ConfigureAwait(false);
                }
                catch(InvalidDataException exception) when(IsUnexpectedStreamInterruption(exception)&&request.StreamingCompletionPredicate is null)
                {
                    // A proxy can close an SSE connection without sending the
                    // final [DONE]/finish_reason event.  Retry once as a
                    // regular (non-streaming) completion so a complete answer
                    // can still be recovered without duplicating live UI text.
                    new PrivacyLogger().Info("OpenAiStreamRecovery",
                        $"流式连接在终止事件前结束；provider={_settings.Type};model={_settings.Model};准备非流式兜底重试");
                    await Task.Delay(InterruptedStreamRetryDelay,timeoutSource.Token).ConfigureAwait(false);
                    try
                    {
                        var recovered=await SendCoreAsync(CreateInterruptedStreamRecoveryRequest(request),timeoutSource.Token).ConfigureAwait(false);
                        if(!string.IsNullOrWhiteSpace(recovered.Answer))
                        {
                            new PrivacyLogger().Info("OpenAiStreamRecovery",
                                $"非流式兜底重试成功；answerChars={recovered.Answer.Length}");
                            return recovered;
                        }
                    }
                    catch(Exception retryException) when(retryException is not OperationCanceledException)
                    {
                        new PrivacyLogger().Info("OpenAiStreamRecovery",
                            $"非流式兜底重试失败；error={retryException.GetType().Name}");
                    }
                    throw;
                }
                if(!ShouldRetryReasoningOnly(result))return result;

                new PrivacyLogger().Info("ReasoningOnlyRecovery",
                    $"初次响应只有思考内容；provider={_settings.Type};model={_settings.Model};reasoningChars={result.Reasoning.Length};structured={request.ExpectStructuredResponse};准备一次正文兜底重试");
                await Task.Delay(ReasoningOnlyRetryDelay,timeoutSource.Token).ConfigureAwait(false);
                var retryRequest=CreateReasoningRecoveryRequest(request);
                var retried=await SendCoreAsync(retryRequest,timeoutSource.Token).ConfigureAwait(false);
                if(!string.IsNullOrWhiteSpace(retried.Answer))
                {
                    // Preserve the first pass in the UI's reasoning card while
                    // letting the recovered answer be rendered normally.
                    retried=retried with { Reasoning=MergeReasoning(result.Reasoning,retried.Reasoning) };
                    new PrivacyLogger().Info("ReasoningOnlyRecovery",
                        $"正文兜底重试成功；answerChars={retried.Answer.Length};reasoningChars={retried.Reasoning.Length}");
                    return retried;
                }

                // Keep the original reasoning-only result so the UI does not
                // replace a useful first-pass trace with an empty retry trace.
                new PrivacyLogger().Info("ReasoningOnlyRecovery",
                    $"正文兜底重试仍无正文；retryReasoningChars={retried.Reasoning.Length}");
                return result;
            }
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
            // Attachment buffers belong to the caller only after the whole
            // request (including reasoning-only / interrupted-stream recovery)
            // has completed. Clearing them here made the retry serialize a
            // zeroed image and caused otherwise valid DeepSeek vision retries
            // to fail with HTTP 400.
            ClearOwnedAttachmentData(request.Attachments);
        }
    }

    private static bool ShouldRetryReasoningOnly(AiResult result)
        =>string.IsNullOrWhiteSpace(result.Answer)
            &&!string.IsNullOrWhiteSpace(result.Reasoning);

    private static bool IsUnexpectedStreamInterruption(InvalidDataException exception)=>
        exception.Message.Contains("AI 流式响应意外中断",StringComparison.Ordinal);

    private static bool IsErrorStreamEvent(string line)
    {
        var trimmed=line.Trim();
        if(trimmed.StartsWith("data",StringComparison.OrdinalIgnoreCase))
        {
            var colon=trimmed.IndexOf(':');
            if(colon<0)return false;
            trimmed=trimmed[(colon+1)..].Trim();
        }
        if(trimmed.Length==0||!trimmed.StartsWith('{'))return false;
        try
        {
            using var document=JsonDocument.Parse(trimmed);
            var type=document.RootElement.TryGetProperty("type",out var property)&&property.ValueKind==JsonValueKind.String
                ?property.GetString()??string.Empty:string.Empty;
            return type.Equals("error",StringComparison.OrdinalIgnoreCase)||type.Equals("response.error",StringComparison.OrdinalIgnoreCase)||type.Equals("response.failed",StringComparison.OrdinalIgnoreCase);
        }
        catch(JsonException){return false;}
    }

    private static AiRequest CreateInterruptedStreamRecoveryRequest(AiRequest request)
        =>new()
        {
            Prompt=request.Prompt+"\n\nRecovery instruction: the previous streaming connection ended before its terminal event. Return the complete final answer now.",
            History=request.History,
            Attachments=request.Attachments,
            // Switch to a normal response body. This avoids repeating partial
            // content in the live stream and bypasses a broken SSE proxy.
            StreamingProgress=null,
            AgentProgress=request.AgentProgress,
            InteractionHandler=request.InteractionHandler,
            StreamingCompletionPredicate=null,
            ExpectStructuredResponse=request.ExpectStructuredResponse,
            DisableReasoning=request.DisableReasoning,
            MaxOutputTokens=request.MaxOutputTokens,
            UseModelMaximumOutputTokens=request.UseModelMaximumOutputTokens
        };

    private static AiRequest CreateReasoningRecoveryRequest(AiRequest request)
        =>new()
        {
            Prompt=request.Prompt+"\n\nRecovery instruction: the previous response contained reasoning only and no final answer. Do not output reasoning. Return the complete final answer now. If a structured response was requested, return one complete valid JSON object with the required answer field.",
            History=request.History,
            Attachments=request.Attachments,
            // Keep the transport mode unchanged for providers that only expose
            // streaming, but suppress retry reasoning from the UI. The first
            // pass already rendered that trace and the final result will merge
            // it back into the completed response if an answer arrives.
            StreamingProgress=request.StreamingProgress is null?null:new AnswerOnlyProgress(request.StreamingProgress),
            AgentProgress=request.AgentProgress,
            InteractionHandler=request.InteractionHandler,
            StreamingCompletionPredicate=null,
            ExpectStructuredResponse=request.ExpectStructuredResponse,
            DisableReasoning=true,
            MaxOutputTokens=request.MaxOutputTokens,
            UseModelMaximumOutputTokens=request.UseModelMaximumOutputTokens
        };

    private static string MergeReasoning(string first,string second)
    {
        first=first.Trim();second=second.Trim();
        if(first.Length==0)return second;
        if(second.Length==0||second.StartsWith(first,StringComparison.Ordinal))return first.Length>=second.Length?first:second;
        return $"{first}\n\n{second}";
    }

    private sealed class AnswerOnlyProgress(IProgress<AiStreamDelta> inner):IProgress<AiStreamDelta>
    {
        public void Report(AiStreamDelta value)
        {
            if(value.Content.Length>0)inner.Report(new AiStreamDelta(value.Content,string.Empty));
        }
    }

    private async Task<AiResult> SendCoreAsync(AiRequest request,CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        ValidateRequest(request);
        var streaming=request.StreamingProgress is not null&&Capabilities.SupportsStreaming;
        var startedAt=System.Diagnostics.Stopwatch.GetTimestamp();
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
            // Do not clear caller-owned in-memory bytes here: the outer
            // SendAsync finally performs that exactly once after all retries.
            // File-loaded temporary buffers remain provider-owned and can be
            // wiped as soon as serialization has copied them into JSON.
            foreach(var attachment in attachments)
                if(attachment.OwnsBytes && attachment.Attachment.Data is null)
                    CryptographicOperations.ZeroMemory(attachment.Bytes);
        }

        HttpResponseMessage response;
        try
        {
            using var httpRequest=Create(HttpMethod.Post,Protocol switch{"anthropic"=>"messages","responses"=>"responses",_=>"chat/completions"});
            httpRequest.Content=new ByteArrayContent(serialized.Body);
            httpRequest.Content.Headers.ContentType=new MediaTypeHeaderValue("application/json"){CharSet="utf-8"};
            new PrivacyLogger().Info("OpenAiRequestStarted",
                $"provider={_settings.Type};model={_settings.Model};host={httpRequest.RequestUri?.Host};path={httpRequest.RequestUri?.AbsolutePath};protocol={Protocol};stream={streaming};requestBytes={serialized.Body.Length}");
            response=await _sendAsync(httpRequest,HttpCompletionOption.ResponseHeadersRead,token).ConfigureAwait(false);
            new PrivacyLogger().Info("OpenAiRequestHeaders",
                $"provider={_settings.Type};host={httpRequest.RequestUri?.Host};path={httpRequest.RequestUri?.AbsolutePath};protocol={Protocol};stream={streaming};requestBytes={serialized.Body.Length};elapsedMs={ElapsedMilliseconds(startedAt)};status={(int)response.StatusCode}");
        }
        catch(OperationCanceledException) when(token.IsCancellationRequested)
        {
            new PrivacyLogger().Info("OpenAiRequestCanceled",
                $"provider={_settings.Type};host={_baseUri.Host};protocol={Protocol};stream={streaming};requestBytes={serialized.Body.Length};elapsedMs={ElapsedMilliseconds(startedAt)}");
            throw;
        }
        finally{CryptographicOperations.ZeroMemory(serialized.Body);}
        using(response)
        {
        if(!response.IsSuccessStatusCode)throw await ProviderHttpError.ReadAsync(response,request.Attachments.Any(item=>item.Type==AiAttachmentType.Video),token).ConfigureAwait(false);
        EnsureDeclaredResponseBodySize(response.Content);

        if(streaming)
        {
            var responseStream=await response.Content.ReadAsStreamAsync(token).ConfigureAwait(false);
            using var limitedStream=new ResponseSizeLimitedStream(responseStream,ResponseBodySizeLimit);
            using var reader=new StreamReader(limitedStream,Encoding.UTF8,true,4096,false);
            var accumulator=new StreamingResponseAccumulator(StreamingContentIsCumulative,request.ExpectStructuredResponse);
            var completed=false;
            var terminalSeen=false;
            var linesRead=0;
            var parsedEvents=0;
            var ignoredLines=0;
            CancellationTokenSource? terminalGrace=null;
            try
            {
                while(true)
                {
                    string? line;
                    try
                    {
                        line=await reader.ReadLineAsync(terminalGrace?.Token??token).ConfigureAwait(false);
                    }
                    catch(OperationCanceledException) when(terminalGrace is {IsCancellationRequested:true}&&!token.IsCancellationRequested)
                    {
                        break;
                    }
                    if(line is null)break;
                    linesRead++;
                    if(IsErrorStreamEvent(line))throw new InvalidDataException("AI 流式响应返回了错误事件，请检查模型、套餐和 API 配置后重试");
                    if(!StreamingResponseParser.TryParse(line,out var delta,out var done,out var truncated))
                    {
                        ignoredLines++;
                        continue;
                    }
                    parsedEvents++;
                    // Only MiniMax's documented reasoning_details is cumulative.
                    // OpenRouter and other compatible streams send actual deltas,
                    // including repeated words that must not be deduplicated.
                    if(delta.ReasoningIsCumulative&&!StreamingContentIsCumulative)delta=delta with{ReasoningIsCumulative=false};
                    var accepted=accumulator.Accept(delta,done&&!truncated,request.StreamingProgress,request.StreamingCompletionPredicate);
                    if(truncated&&!accepted)throw new InvalidDataException("AI 回复达到输出长度限制，未收到完整内容，请缩小范围后重试");

                    // Some gateways mark a reasoning event as terminal before
                    // sending the final content event. Keep the connection open
                    // for a short grace window so a late answer is not discarded.
                    if(done&&accumulator.RawAnswer.Length==0)
                    {
                        terminalSeen=true;
                        terminalGrace??=CancellationTokenSource.CreateLinkedTokenSource(token);
                        terminalGrace.CancelAfter(PrematureTerminalGracePeriod);
                        continue;
                    }
                    if(accumulator.RawAnswer.Length>0&&terminalGrace is not null)
                    {
                        terminalGrace.Dispose();terminalGrace=null;terminalSeen=false;
                    }
                    if(accepted){completed=true;break;}
                }
            }
            finally
            {
                terminalGrace?.Dispose();
            }
            if(!completed&&!terminalSeen)
            {
                new PrivacyLogger().Info("OpenAiStreamInterrupted",
                    $"连接在终止事件前结束；provider={_settings.Type};model={_settings.Model};lines={linesRead};parsed={parsedEvents};ignored={ignoredLines};answerChars={accumulator.RawAnswer.Length};reasoningChars={accumulator.RawReasoning.Length}");
                throw new InvalidDataException("AI 流式响应意外中断，请重试");
            }
            if(terminalSeen)
                new PrivacyLogger().Info("OpenAiStreamTerminal",
                    $"终止事件后正文仍为空；provider={_settings.Type};model={_settings.Model};reasoningChars={accumulator.RawReasoning.Length};graceMs={(int)PrematureTerminalGracePeriod.TotalMilliseconds}");
            token.ThrowIfCancellationRequested();
            return BuildProviderResult(accumulator.RawAnswer,accumulator.RawReasoning,request.ExpectStructuredResponse);
        }

        var json=await ReadResponseBodyAsStringAsync(response.Content,token).ConfigureAwait(false);
        JsonDocument document;
        try{document=JsonDocument.Parse(json);}
        catch(JsonException) when(TryParseBufferedStream(json,request.ExpectStructuredResponse,out var recovered))
        {
            return recovered;
        }
        using(document)
        {
        // Each wire protocol has a different terminal/length shape. Never
        // probe `choices` before selecting the protocol: Anthropic and
        // Responses bodies are valid JSON but do not contain that property.
        if(Protocol=="anthropic")
        {
            var answer=new StringBuilder();var reasoning=new StringBuilder();
            if(document.RootElement.TryGetProperty("content",out var blocks)&&blocks.ValueKind==JsonValueKind.Array)
                foreach(var block in blocks.EnumerateArray())
                {
                    var type=ReadString(block,"type");var text=ReadString(block,"text");
                    if(type.Contains("thinking",StringComparison.OrdinalIgnoreCase)||type.Contains("reasoning",StringComparison.OrdinalIgnoreCase))
                    {
                        if(text.Length==0)text=ReadString(block,"thinking");
                        reasoning.Append(text);
                    }
                    else if(type is "text" or "output_text")answer.Append(text);
                }
            if(answer.Length==0)answer.Append(ReadString(document.RootElement,"output_text"));
            if(string.Equals(ReadString(document.RootElement,"stop_reason"),"max_tokens",StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("AI 回复达到输出长度限制，未收到完整内容，请缩小范围后重试");
            token.ThrowIfCancellationRequested();
            return BuildProviderResult(answer.ToString(),reasoning.ToString(),request.ExpectStructuredResponse);
        }
        if(Protocol=="responses"&&!document.RootElement.TryGetProperty("choices",out _))
        {
            var answer=ReadString(document.RootElement,"output_text");
            var reasoning=new StringBuilder();
            if(answer.Length==0&&document.RootElement.TryGetProperty("output",out var output)&&output.ValueKind==JsonValueKind.Array)
                foreach(var item in output.EnumerateArray())
                    if(item.TryGetProperty("content",out var content)&&content.ValueKind==JsonValueKind.Array)
                        foreach(var part in content.EnumerateArray())
                        {
                            var partType=ReadString(part,"type");var partText=ReadString(part,"text");
                            if(partType.Contains("reasoning",StringComparison.OrdinalIgnoreCase)||partType.Contains("summary",StringComparison.OrdinalIgnoreCase))reasoning.Append(partText);
                            else if(partType is "output_text" or "text"||partType.Length==0)answer+=partText;
                        }
            var status=ReadString(document.RootElement,"status");
            if(status.Equals("incomplete",StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("AI 回复达到输出长度限制，未收到完整内容，请缩小范围后重试");
            token.ThrowIfCancellationRequested();
            return BuildProviderResult(answer,reasoning.ToString(),request.ExpectStructuredResponse);
        }
        if(!document.RootElement.TryGetProperty("choices",out var choices)||choices.ValueKind!=JsonValueKind.Array||choices.GetArrayLength()==0)
            throw new InvalidDataException("AI 返回了无法识别的响应格式");
        if(choices[0].TryGetProperty("finish_reason",out var finishReason)&&finishReason.ValueKind==JsonValueKind.String&&finishReason.GetString()=="length")
            throw new InvalidDataException("AI 回复达到输出长度限制，未收到完整内容，请缩小范围后重试");
        var message=document.RootElement.GetProperty("choices")[0].GetProperty("message");
        var (answerText,typedReasoning)=StreamingResponseParser.ReadContentParts(message);
        var reasoningText=ReadString(message,"reasoning_content");
        if(reasoningText.Length==0)reasoningText=ReadString(message,"thinking_content");
        if(reasoningText.Length==0)reasoningText=ReadString(message,"reasoning");
        if(reasoningText.Length==0)reasoningText=typedReasoning;
        if(reasoningText.Length==0)reasoningText=StreamingResponseParser.ReadReasoningDetails(message);
        token.ThrowIfCancellationRequested();
        return BuildProviderResult(answerText,reasoningText,request.ExpectStructuredResponse);
        }
    }

    }

    private static bool TryParseBufferedStream(string body,bool expectStructuredResponse,out AiResult result)
    {
        result=new(string.Empty,[]);
        var accumulator=new StreamingResponseAccumulator(false,expectStructuredResponse);
        var parsed=false;
        var terminal=false;
        foreach(var line in body.Split(new[]{"\r\n","\n","\r"},StringSplitOptions.None))
        {
            if(!StreamingResponseParser.TryParse(line,out var delta,out var done,out var truncated))continue;
            parsed=true;
            terminal|=done;
            if(truncated)throw new InvalidDataException("AI 回复达到输出长度限制，未收到完整内容，请缩小范围后重试");
            accumulator.Accept(delta,done,null,null);
        }
        if(!parsed||!terminal)return false;
        result=accumulator.BuildResult();
        return !string.IsNullOrWhiteSpace(result.Answer)||!string.IsNullOrWhiteSpace(result.Reasoning);
    }

    private AiResult BuildProviderResult(string rawContent,string rawReasoning,bool expectStructuredResponse)
    {
        var result=StructuredResponseParser.Parse(rawContent,rawReasoning,expectStructuredResponse);
        if(!ProviderModelPolicy.RequiresAssistantContinuation(_settings)||string.IsNullOrWhiteSpace(result.Answer))return result;
        return result with{ContinuationMessage=new AiMessage("assistant",result.Answer)
        {
            ProviderContent=rawContent,
            ReasoningContent=rawReasoning.Length==0?null:rawReasoning
        }};
    }

    private IEnumerable<AiMessage> RequestHistory(IReadOnlyList<AiMessage> history)
    {
        if(!ProviderModelPolicy.RequiresAssistantContinuation(_settings))return history;
        var result=new List<AiMessage>();
        var index=0;
        if(history.Count>0&&history[0] is {Role:{ } firstRole}&&firstRole.Equals("system",StringComparison.OrdinalIgnoreCase))
        {result.Add(history[0]);index=1;}
        for(;index+1<history.Count;index+=2)
        {
            // Old persisted replies contain presentation text only. Replaying
            // them would invent an incomplete thinking-model conversation.
            if(history[index+1]?.ProviderContent is null)continue;
            result.Add(history[index]);result.Add(history[index+1]);
        }
        return result;
    }

    protected virtual void ValidateRequest(AiRequest request)
    {
        if(request.MaxOutputTokens is <=0)throw new ArgumentOutOfRangeException(nameof(request),"最大输出 Token 必须大于 0");
        if(request.History is null)throw new InvalidOperationException("对话历史不能为空");
        if(request.Attachments is null)throw new InvalidOperationException("附件列表不能为空");
        if(request.Attachments.Count>MaxAttachmentCount)throw new InvalidOperationException($"单次请求最多支持 {MaxAttachmentCount} 个附件，请减少选区或分批发送");
        var imageLimit=ProviderModelPolicy.MaximumImageCount(_settings);
        if(request.Attachments.Count(item=>item?.Type==AiAttachmentType.Image)>imageLimit)
            throw new InvalidOperationException(LocalizationService.T($"当前模型单次最多支持 {imageLimit} 张图片，请减少引用区域或分批发送。",$"This model supports at most {imageLimit} images per request. Reduce the referenced regions or send them in batches."));
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
            if(string.IsNullOrWhiteSpace(attachment.MimeType))throw new InvalidOperationException("附件 MIME 类型不能为空");
            var size=GetAttachmentSize(attachment);
            if(size<=0)throw new InvalidOperationException("附件内容为空");
            ValidateAttachmentSize(attachment,size);
            rawAttachmentBytes=AddSaturating(rawAttachmentBytes,size);
            estimatedBodyBytes=AddSaturating(estimatedBodyBytes,160);
            estimatedBodyBytes=AddSaturating(estimatedBodyBytes,EstimateBase64DataUrlBytes(size,attachment.MimeType));
            if(rawAttachmentBytes>MaxRequestBodySize||estimatedBodyBytes>MaxRequestBodySize)throw CreateRequestBodyTooLargeException(estimatedBodyBytes);
            if(attachment.Duration is { } duration&&duration<TimeSpan.Zero)throw new InvalidOperationException("视频时长不能为负数");
            if(attachment.Type==AiAttachmentType.Video&&attachment.Duration is { } videoDuration&&videoDuration<ProviderModelPolicy.MinimumVideoDuration(_settings))
                throw new InvalidOperationException(LocalizationService.T("当前通义千问模型要求视频至少 2 秒，请录制更长的视频或改为截图。","This Qwen model requires a video at least 2 seconds long. Record a longer video or use a screenshot."));
            if(attachment.Duration is { } limitedDuration&&Capabilities.MaxVideoDuration>TimeSpan.Zero&&limitedDuration>Capabilities.MaxVideoDuration)throw new InvalidOperationException("视频时长超过当前模型限制");
        }
        foreach(var attachment in request.Attachments)
        {
            if(attachment.Type==AiAttachmentType.Image&&!Capabilities.SupportsImage)throw new NotSupportedException("当前模型不支持图片理解，请在设置中选择多模态模型");
            if(attachment.Type==AiAttachmentType.Video&&!Capabilities.SupportsVideo)throw new NotSupportedException("当前模型不支持视频理解，请在设置中选择视频多模态模型");
            if(attachment.Type!=AiAttachmentType.Text&&Capabilities.AcceptedMimeTypes.Count>0&&!Capabilities.AcceptedMimeTypes.Contains(attachment.MimeType))throw new NotSupportedException($"当前模型不接受 {attachment.MimeType} 附件");
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
        new($"请求体（含附件的 Base64 展开）预计为 {FormatMegabytes(bytes)} MB，超过当前服务的 {FormatMegabytes(MaxRequestBodySize)} MB 聚合限制；请减少附件数量或压缩附件");

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
        var request=new HttpRequestMessage(method,ProviderProtocolPolicy.BuildRequestUri(_settings,"/"+relative));
        var hasCustomAuthorization=_settings.CustomHeaders.Keys.Any(name=>name.Equals("Authorization",StringComparison.OrdinalIgnoreCase));
        if(!hasCustomAuthorization&&!string.IsNullOrWhiteSpace(_apiKey)&&ProviderProtocolPolicy.AuthMode(_settings)!="none")
        {
            if(ProviderProtocolPolicy.AuthMode(_settings)=="api_key")request.Headers.TryAddWithoutValidation("x-api-key",_apiKey);
            else request.Headers.Authorization=new AuthenticationHeaderValue("Bearer",_apiKey);
        }
        foreach(var header in _settings.CustomHeaders)
            if(!request.Headers.TryAddWithoutValidation(header.Key,header.Value))throw new InvalidOperationException($"无法添加 Provider 请求头：{header.Key}");
        if((Protocol=="anthropic"||ProviderModelPolicy.NeedsAnthropicVersion(_settings))&&!request.Headers.Contains("anthropic-version"))
            request.Headers.TryAddWithoutValidation("anthropic-version","2023-06-01");
        if(Protocol=="anthropic"&&!request.Headers.Contains("anthropic-beta"))request.Headers.TryAddWithoutValidation("anthropic-beta","prompt-caching-2024-07-31");
        if(!string.IsNullOrWhiteSpace(_settings.AccountIdHeader)&&!request.Headers.Contains("ChatGPT-Account-Id"))request.Headers.TryAddWithoutValidation("ChatGPT-Account-Id",_settings.AccountIdHeader);
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
        var continuation=ProviderModelPolicy.RequiresAssistantContinuation(_settings);
        foreach(var message in RequestHistory(request.History))
        {
            var providerMessage=continuation&&message is {Role:{ } role}&&role.Equals("assistant",StringComparison.OrdinalIgnoreCase);
            total=AddSaturating(total,64);
            total=AddSaturating(total,EstimateJsonStringBytesUpperBound(message?.Role));
            total=AddSaturating(total,EstimateJsonStringBytesUpperBound(providerMessage?message?.ProviderContent??message?.Text:message?.Text));
            if(providerMessage&&message?.ReasoningContent is { } reasoning)
                total=AddSaturating(total,EstimateJsonStringBytesUpperBound(reasoning));
        }
        return total;
    }

    private SerializedRequest SerializeRequest(AiRequest request,IReadOnlyList<LoadedAttachment> attachments,bool streaming,CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        if(Protocol=="anthropic")return SerializeAnthropicRequest(request,attachments,streaming,token);
        if(Protocol=="responses")return SerializeResponsesRequest(request,attachments,streaming,token);
        var content=new List<object>();
        if(!string.IsNullOrWhiteSpace(request.Prompt))content.Add(new{type="text",text=request.Prompt});
        foreach(var loaded in attachments)
        {
            token.ThrowIfCancellationRequested();
            var attachment=loaded.Attachment;
            var dataUrl=$"data:{attachment.MimeType};base64,{Convert.ToBase64String(loaded.Bytes)}";
            token.ThrowIfCancellationRequested();
            if(attachment.Type==AiAttachmentType.Image)content.Add(new{type="image_url",image_url=new{url=dataUrl}});
            else if(attachment.Type==AiAttachmentType.Video)
            {
                var videoUrl=new Dictionary<string,object?>{{"url",dataUrl}};
                if(ProviderModelPolicy.UsesVideoSamplingField(_settings))videoUrl["fps"]=VideoSamplingFramesPerSecond;
                content.Add(new{type="video_url",video_url=videoUrl});
            }
            else content.Add(new{type="text",text=System.Text.Encoding.UTF8.GetString(loaded.Bytes)});
        }

        var messages=new List<object>(request.History.Count+1);
        var continuation=ProviderModelPolicy.RequiresAssistantContinuation(_settings);
        foreach(var message in RequestHistory(request.History))
        {
            token.ThrowIfCancellationRequested();
            var providerMessage=continuation&&message.Role.Equals("assistant",StringComparison.OrdinalIgnoreCase);
            var value=new Dictionary<string,object?>{{"role",message.Role},{"content",providerMessage?message.ProviderContent??message.Text:message.Text}};
            if(providerMessage&&message.ReasoningContent is { } reasoning)
                value["reasoning_content"]=reasoning;
            messages.Add(value);
        }
        messages.Add(new{role="user",content});
        var bodyValues=new Dictionary<string,object?>{{"model",_settings.Model},{"messages",messages},{"stream",streaming}};
        ProviderModelPolicy.ApplyRequestParameters(bodyValues,_settings,request);
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

    private SerializedRequest SerializeAnthropicRequest(AiRequest request,IReadOnlyList<LoadedAttachment> attachments,bool streaming,CancellationToken token)
    {
        var messages=new List<object>();
        var history=RequestHistory(request.History);
        var system=string.Join("\n\n",history.Where(item=>item.Role.Equals("system",StringComparison.OrdinalIgnoreCase)).Select(item=>item.Text).Where(text=>!string.IsNullOrWhiteSpace(text)));
        foreach(var message in history.Where(item=>item.Role is "user" or "assistant"))
        {
            var messageContent=message.Role.Equals("assistant",StringComparison.OrdinalIgnoreCase)
                ? message.ProviderContent??message.Text : message.Text;
            messages.Add(new{role=message.Role,content=messageContent});
        }
        var content=new List<object>();if(!string.IsNullOrWhiteSpace(request.Prompt))content.Add(new{type="text",text=request.Prompt});
        foreach(var loaded in attachments)
        {
            token.ThrowIfCancellationRequested();var item=loaded.Attachment;var data=Convert.ToBase64String(loaded.Bytes);
            if(item.Type==AiAttachmentType.Image)content.Add(new{type="image",source=new{type="base64",media_type=item.MimeType,data}});
            else if(item.Type==AiAttachmentType.Text)content.Add(new{type="text",text=Encoding.UTF8.GetString(loaded.Bytes)});
            else throw new NotSupportedException("Anthropic Messages API 不支持视频附件");
        }
        messages.Add(new{role="user",content});
        var body=new Dictionary<string,object?>{{"model",_settings.Model},{"messages",messages},{"max_tokens",request.MaxOutputTokens??8192},{"stream",streaming}};
        if(system.Length>0)body["system"]=system;
        if(request.DisableReasoning)body["thinking"]=new{type="disabled"};
        ProviderModelPolicy.ApplyRequestParameters(body,_settings,request);
        return new(JsonSerializer.SerializeToUtf8Bytes(body));
    }

    private SerializedRequest SerializeResponsesRequest(AiRequest request,IReadOnlyList<LoadedAttachment> attachments,bool streaming,CancellationToken token)
    {
        var input=new List<object>();
        var history=RequestHistory(request.History);
        var instructions=string.Join("\n\n",history.Where(item=>item.Role.Equals("system",StringComparison.OrdinalIgnoreCase)).Select(item=>item.Text).Where(text=>!string.IsNullOrWhiteSpace(text)));
        foreach(var message in history.Where(item=>item.Role is "user" or "assistant"))
        {
            var text=message.Role.Equals("assistant",StringComparison.OrdinalIgnoreCase)?message.ProviderContent??message.Text:message.Text;
            input.Add(new{role=message.Role,content=new[]{new{type=message.Role=="assistant"?"output_text":"input_text",text}}});
        }
        var content=new List<object>();if(!string.IsNullOrWhiteSpace(request.Prompt))content.Add(new{type="input_text",text=request.Prompt});
        foreach(var loaded in attachments){token.ThrowIfCancellationRequested();if(loaded.Attachment.Type==AiAttachmentType.Image)content.Add(new{type="input_image",image_url=$"data:{loaded.Attachment.MimeType};base64,{Convert.ToBase64String(loaded.Bytes)}"});else if(loaded.Attachment.Type==AiAttachmentType.Text)content.Add(new{type="input_text",text=Encoding.UTF8.GetString(loaded.Bytes)});else throw new NotSupportedException("OpenAI Responses API 不支持视频附件");}
        input.Add(new{role="user",content});
        var body=new Dictionary<string,object?>{{"model",_settings.Model},{"input",input},{"stream",streaming}};
        if(instructions.Length>0)body["instructions"]=instructions;
        if(request.MaxOutputTokens is { } max)body["max_output_tokens"]=max;
        if(request.DisableReasoning)body["reasoning"]=new{effort="minimal"};
        foreach(var parameter in _settings.RequestParameters)body[parameter.Key]=parameter.Value;
        return new(JsonSerializer.SerializeToUtf8Bytes(body));
    }

    private static string FormatTimeout(TimeSpan timeout)=>timeout.TotalMinutes>=1?$"{timeout.TotalMinutes:0.#} 分钟":$"{timeout.TotalSeconds:0.#} 秒";
    private static long ElapsedMilliseconds(long startedAt)=>checked((long)(System.Diagnostics.Stopwatch.GetElapsedTime(startedAt).TotalMilliseconds));
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
    internal string RawAnswer=>CurrentAnswer();
    internal string RawReasoning=>_reasoning;
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
