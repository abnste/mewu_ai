// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net.Http;
using System.Security.Cryptography;
using System.Text.Json;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.AI;

/// <summary>Turns bounded error responses into fixed messages without echoing provider input or secrets.</summary>
internal static class ProviderHttpError
{
    private const int MaxErrorBytes=16*1024;
    private static readonly HashSet<string> KnownFields=new(StringComparer.Ordinal)
    {
        "model","messages","content","type","video_url","image_url","url","fps","detail",
        "source","media_type","data","max_tokens","max_completion_tokens","temperature","top_p",
        "thinking","reasoning_split","stream","service_tier"
    };

    internal static async Task<InvalidOperationException> ReadAsync(HttpResponseMessage response,bool hasVideo,CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        var traceId=ReadTraceId(response);
        var bytes=new byte[MaxErrorBytes+1];
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(token);
        timeout.CancelAfter(TimeSpan.FromSeconds(5));
        try
        {
            if(response.Content.Headers.ContentLength is >MaxErrorBytes)return Create((int)response.StatusCode,hasVideo,ReadOnlyMemory<byte>.Empty,traceId);
            await using var stream=await response.Content.ReadAsStreamAsync(timeout.Token).ConfigureAwait(false);
            var count=0;
            while(count<bytes.Length)
            {
                var read=await stream.ReadAsync(bytes.AsMemory(count),timeout.Token).ConfigureAwait(false);
                if(read==0)break;
                count+=read;
            }
            token.ThrowIfCancellationRequested();
            return Create((int)response.StatusCode,hasVideo,count<=MaxErrorBytes?bytes.AsMemory(0,count):ReadOnlyMemory<byte>.Empty,traceId);
        }
        catch(OperationCanceledException)when(!token.IsCancellationRequested){return Create((int)response.StatusCode,hasVideo,ReadOnlyMemory<byte>.Empty,traceId);}
        catch(Exception ex)when(ex is IOException or HttpRequestException){token.ThrowIfCancellationRequested();return Create((int)response.StatusCode,hasVideo,ReadOnlyMemory<byte>.Empty,traceId);}
        finally{CryptographicOperations.ZeroMemory(bytes);}
    }

    internal static InvalidOperationException Create(int status,bool hasVideo,ReadOnlyMemory<byte> body,string traceId="")
    {
        var reason=string.Empty;
        var field=string.Empty;
        var numericCode=string.Empty;
        try
        {
            if(body.Length>0&&body.Length<=MaxErrorBytes)
            {
                using var document=JsonDocument.Parse(body,new JsonDocumentOptions{MaxDepth=16});
                var root=document.RootElement;
                var error=Property(root,"error");
                reason=Classify(Text(error,"message"));
                field=SafeField(Property(error,"param"));
                if(reason.Length==0)reason=Classify(Text(root,"message"));
                var baseResponse=Property(root,"base_resp");
                if(reason.Length==0)reason=Classify(Text(baseResponse,"status_msg"));
                numericCode=ReadCode(Property(baseResponse,"status_code"));
                if(numericCode.Length==0)numericCode=ReadCode(Property(error,"code"));
                if(numericCode.Length==0)numericCode=ReadCode(Property(root,"code"));
                var details=Property(root,"detail");
                if(reason.Length==0&&details.ValueKind==JsonValueKind.String)reason=Classify(details.GetString()??string.Empty);
                if(details.ValueKind==JsonValueKind.Array)
                {
                    foreach(var detail in details.EnumerateArray().Take(8))
                    {
                        if(field.Length==0)field=SafeField(Property(detail,"loc"));
                        if(reason.Length==0)reason=Classify(Text(detail,"msg"));
                    }
                }
            }
        }
        catch(JsonException){/* HTML, truncated JSON and unknown errors retain the HTTP status. */}

        if(reason.Length==0)reason=ClassifyCode(numericCode);
        if(reason.Length==0)reason=status switch
        {
            401 or 403=>LocalizationService.T("服务拒绝认证，请检查当前渠道的登录状态或 API Key。","Authentication was rejected. Check this connection's login or API key."),
            413=>LocalizationService.T("请求内容过大，请压缩视频或减少附件后重试。","The request is too large. Compress the video or reduce attachments."),
            429=>LocalizationService.T("请求过于频繁或额度不足，请检查服务额度并稍后重试。","The service is rate-limited or out of quota. Check usage and retry later."),
            422 when hasVideo=>LocalizationService.T("服务端拒绝了视频请求，但没有提供可识别的具体原因。请用一段较小的 MP4 视频重试，确认是否与该文件有关。","The service rejected the video request without a recognized reason. Try a smaller MP4 to check whether the file is the cause."),
            400 or 422=>LocalizationService.T("服务端未接受请求参数，请检查模型和高级请求设置。","The service rejected the request parameters. Check the model and advanced request settings."),
            _=>LocalizationService.T("服务暂时无法完成请求，请稍后重试。","The service could not complete the request. Please retry later.")
        };
        var label=hasVideo?LocalizationService.T("视频请求失败","Video request failed"):LocalizationService.T("AI 请求失败","AI request failed");
        var detailText=field.Length==0?string.Empty:LocalizationService.T($" 参数：{field}。",$" Parameter: {field}.");
        var codeText=numericCode.Length==0?string.Empty:LocalizationService.T($" 服务代码：{numericCode}。",$" Service code: {numericCode}.");
        var traceText=traceId.Length==0?string.Empty:LocalizationService.T($" 追踪编号：{traceId}。",$" Trace ID: {traceId}.");
        return new InvalidOperationException($"{label}（HTTP {status}）。{reason}{detailText}{codeText}{traceText}");
    }

    private static string Classify(string message)
    {
        if(message.Length>2048)return string.Empty;
        var text=message.ToLowerInvariant();
        if(text.Contains("context length")||text.Contains("context_length")||text.Contains("maximum context")||text.Contains("上下文长度")||text.Contains("输入过长"))
            return LocalizationService.T("视频和历史内容超过模型上下文限制，请缩短视频或开始新对话。","The video and history exceed the model's context limit. Shorten the video or start a new conversation.");
        if(text.Contains("file too large")||text.Contains("video size exceeds")||text.Contains("request entity too large")||text.Contains("payload too large")||text.Contains("文件过大")||text.Contains("视频过大")||text.Contains("请求体过大"))
            return LocalizationService.T("视频或请求体超过服务端大小限制，请压缩视频或减少附件。","The video or request exceeds the service's size limit. Compress the video or reduce attachments.");
        if(text.Contains("failed to decode video")||text.Contains("video decode failed")||text.Contains("invalid video")||text.Contains("unsupported video")||text.Contains("unsupported codec")||text.Contains("failed to parse video")||text.Contains("视频解码失败")||text.Contains("视频解析失败")||text.Contains("无法解析视频")||text.Contains("不支持的视频")||text.Contains("不支持的编码"))
            return LocalizationService.T("服务端无法读取视频，请重新录制，或将文件转换为 H.264 编码的 MP4 后重试。","The service cannot read this video. Record it again or convert it to H.264 MP4 and retry.");
        if(text.Contains("invalid base64")||text.Contains("base64 decoding")||text.Contains("base64 编码")||text.Contains("base64解码"))
            return LocalizationService.T("服务端未能解析附件编码，请重新添加附件后重试。","The service could not parse the attachment encoding. Add the attachment again and retry.");
        if((text.Contains("video")||text.Contains("视频")||text.Contains("media")||text.Contains("媒体"))&&
           (text.Contains("content")||text.Contains("内容")))
            return LocalizationService.T("服务端拒绝了这段视频内容，未提供更具体的原因。请换一段不含敏感信息的短视频，或切换到其他可用 AI 渠道重试。","The service rejected this video's content without a more specific reason. Try a short video without sensitive information, or switch to another available AI channel.");
        return string.Empty;
    }

    private static string ClassifyCode(string code)=>code switch
    {
        // MiniMax documents 1039 as a token-limit error and 2013 as an
        // invalid-parameter response. Keep the guidance fixed rather than
        // echoing the server body, which can contain media or prompt values.
        "1039"=>LocalizationService.T("视频和历史内容超过模型上下文限制，请缩短视频或开始新对话。","The video and history exceed the model's context limit. Shorten the video or start a new conversation."),
        "2013"=>LocalizationService.T("服务端未接受请求参数，请检查模型和高级请求设置后重试。","The service rejected the request parameters. Check the model and advanced request settings before retrying."),
        _=>string.Empty
    };

    private static string SafeField(JsonElement value)
    {
        if(value.ValueKind==JsonValueKind.String)return KnownFields.Contains(value.GetString()??string.Empty)?value.GetString()!:string.Empty;
        if(value.ValueKind!=JsonValueKind.Array)return string.Empty;
        // Report only a known schema field, never a provider-supplied path, URL, input value or arbitrary index.
        return value.EnumerateArray().Take(16).Where(item=>item.ValueKind==JsonValueKind.String)
            .Select(item=>item.GetString()??string.Empty).LastOrDefault(KnownFields.Contains)??string.Empty;
    }
    private static string ReadCode(JsonElement value)
    {
        if(value.ValueKind==JsonValueKind.Number&&value.TryGetInt32(out var numeric)&&numeric is >0 and <100000)return numeric.ToString(System.Globalization.CultureInfo.InvariantCulture);
        if(value.ValueKind==JsonValueKind.String&&int.TryParse(value.GetString(),System.Globalization.NumberStyles.None,System.Globalization.CultureInfo.InvariantCulture,out numeric)&&numeric is >0 and <100000)return numeric.ToString(System.Globalization.CultureInfo.InvariantCulture);
        return string.Empty;
    }
    private static string ReadTraceId(HttpResponseMessage response)
    {
        foreach(var name in new[]{"trace_id","trace-id","x-trace-id","x-request-id","request-id","x-mm-request-id"})
            if(response.Headers.TryGetValues(name,out var values))
                foreach(var value in values)
                    if(value.Length is >0 and <=128&&value.All(character=>char.IsAsciiLetterOrDigit(character)||character is '-' or '_' or '.'))return value;
        return string.Empty;
    }
    private static JsonElement Property(JsonElement value,string key)=>value.ValueKind==JsonValueKind.Object&&value.TryGetProperty(key,out var property)?property:default;
    private static string Text(JsonElement value,string key)=>Property(value,key) is {ValueKind:JsonValueKind.String} field?field.GetString()??string.Empty:string.Empty;
}
