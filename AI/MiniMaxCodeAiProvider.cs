// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net.Http;
using System.Net.Http.Headers;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.AI;

/// <summary>MiniMax Code desktop provider. It reuses the official desktop OAuth session in memory.</summary>
internal sealed class MiniMaxCodeAiProvider(string model) : IAiProvider
{
    private static readonly HttpClient Client=new(){Timeout=Timeout.InfiniteTimeSpan};
    private const long ImageLimit=20L*1024*1024,VideoLimit=50L*1024*1024,RequestLimit=64L*1024*1024;
    public string Id=>"minimax-code";
    private bool Vision=>MiniMaxCodeRuntime.KnownModels.FirstOrDefault(item=>item.Model.Equals(model,StringComparison.OrdinalIgnoreCase))?.SupportsVision==true;
    public AiProviderCapabilities Capabilities=>new(Vision,Vision,true,ImageLimit,VideoLimit,TimeSpan.FromHours(4),new HashSet<string>(StringComparer.OrdinalIgnoreCase){"image/png","image/jpeg","image/webp","image/gif","video/mp4"});

    public async Task<bool> TestConnectionAsync(CancellationToken cancellationToken)
    {
        var result=await SendAsync(new AiRequest{Prompt="Reply exactly MEWU_OK.",DisableReasoning=true,MaxOutputTokens=32},cancellationToken).ConfigureAwait(false);
        return result.Answer.Trim()=="MEWU_OK";
    }

    public async Task<AiResult> SendAsync(AiRequest request,CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(request);Validate(request,model,Vision);
        var session=MiniMaxCodeRuntime.TryGetDesktopSession()??throw new InvalidOperationException("未发现 MiniMax Code 桌面版登录会话，请点击“打开 MiniMax Code”完成登录后刷新状态。");
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);timeout.CancelAfter(TimeSpan.FromMinutes(10));
        try
        {
            using var message=new HttpRequestMessage(HttpMethod.Post,$"{session.BaseUrl.TrimEnd('/')}/messages");
            message.Headers.Authorization=new AuthenticationHeaderValue("Bearer",session.AccessToken);
            message.Headers.TryAddWithoutValidation("anthropic-version","2023-06-01");
            message.Content=new StringContent(BuildPayload(request,model),Encoding.UTF8,"application/json");
            using var response=await Client.SendAsync(message,HttpCompletionOption.ResponseHeadersRead,timeout.Token).ConfigureAwait(false);
            if(!response.IsSuccessStatusCode)
            {
                if(response.StatusCode is System.Net.HttpStatusCode.Unauthorized or System.Net.HttpStatusCode.Forbidden)
                    throw new InvalidOperationException("MiniMax Code 桌面登录已失效，请点击“打开 MiniMax Code”重新登录。");
                throw await ProviderHttpError.ReadAsync(response,request.Attachments.Any(item=>item.Type==AiAttachmentType.Video),timeout.Token).ConfigureAwait(false);
            }
            var answer=new StringBuilder();var reasoning=new StringBuilder();var sawStop=false;
            await using var stream=await response.Content.ReadAsStreamAsync(timeout.Token).ConfigureAwait(false);
            using var reader=new StreamReader(stream,Encoding.UTF8);
            while(await reader.ReadLineAsync(timeout.Token).ConfigureAwait(false) is { } line)
            {
                if(!line.StartsWith("data:",StringComparison.OrdinalIgnoreCase))continue;
                var json=line[5..].Trim();if(json.Length==0||json=="[DONE]")continue;
                using var document=JsonDocument.Parse(json);var root=document.RootElement;var type=Text(root,"type");
                if(type=="message_stop"){sawStop=true;continue;}
                if(type!="content_block_delta"||!root.TryGetProperty("delta",out var delta))continue;
                var deltaType=Text(delta,"type");var value=Text(delta,"text");if(value.Length==0)continue;
                if(deltaType=="thinking_delta")reasoning.Append(value);else if(deltaType=="text_delta")answer.Append(value);else continue;
                request.StreamingProgress?.Report(deltaType=="thinking_delta"?new AiStreamDelta("",value):new AiStreamDelta(value,""));
                if(answer.Length>2_000_000||reasoning.Length>500_000)throw new InvalidDataException("MiniMax Code 回复超过安全限制，已停止。");
            }
            timeout.Token.ThrowIfCancellationRequested();
            if(!sawStop)throw new InvalidDataException("MiniMax Code 连接提前结束，未收到完整回答。");
            if(answer.Length==0||string.IsNullOrWhiteSpace(answer.ToString()))throw new InvalidDataException("MiniMax Code 未返回有效正文。");
            return StructuredResponseParser.Parse(answer.ToString(),reasoning.ToString(),request.ExpectStructuredResponse);
        }
        catch(OperationCanceledException)when(!cancellationToken.IsCancellationRequested){throw new TimeoutException("MiniMax Code 请求超过 10 分钟，已停止。请重试或缩短附件。");}
        finally
        {
            foreach(var attachment in request.Attachments)if(attachment.ProviderOwnsData&&attachment.Data is { } bytes)CryptographicOperations.ZeroMemory(bytes);
        }
    }

    internal static string BuildPayload(AiRequest request,string model)
    {
        var messages=new List<object>();
        foreach(var item in ConversationContextPolicy.CreateBoundedHistory(request.History).Where(item=>item.Role is "user" or "assistant"))messages.Add(new{role=item.Role,content=item.Text});
        var content=new List<object>{new{type="text",text=request.Prompt}};
        foreach(var attachment in request.Attachments)
        {
            if(attachment.Type==AiAttachmentType.Text){var bytes=ReadBytes(attachment);try{content.Add(new{type="text",text=Encoding.UTF8.GetString(bytes)});}finally{CryptographicOperations.ZeroMemory(bytes);}}
            else{var bytes=ReadBytes(attachment);try{var source=new{type="base64",media_type=attachment.MimeType,data=Convert.ToBase64String(bytes)};content.Add(attachment.Type==AiAttachmentType.Image?new{type="image",source}:(object)new{type="video",source});}finally{CryptographicOperations.ZeroMemory(bytes);}}
        }
        messages.Add(new{role="user",content});
        var max=request.MaxOutputTokens.GetValueOrDefault(4096);if(max is <1 or >32_000)max=4096;
        return JsonSerializer.Serialize(new{model,max_tokens=max,stream=true,system="You are MiniMax Code running inside MewuAI. Answer the user's request directly. Do not call tools or return JSON wrappers unless requested.",messages});
    }

    internal static void Validate(AiRequest request,string model,bool vision)
    {
        MiniMaxCodeRuntime.ValidateModel(model);ConversationContextPolicy.EnsureValidForProvider(request.History);
        if(string.IsNullOrWhiteSpace(request.Prompt)||request.Prompt.Length>100_000)throw new InvalidOperationException("请输入问题，并控制在 100,000 字符内。");
        if(request.Attachments.Count>16)throw new InvalidOperationException("MiniMax Code 单次最多接收 16 个附件。");
        long total=0,estimated=Encoding.UTF8.GetByteCount(request.Prompt)*2L+100_000;
        foreach(var item in request.Attachments)
        {
            if(!Enum.IsDefined(item.Type))throw new InvalidOperationException("不支持的附件类型。");
            if(item.Type!=AiAttachmentType.Text&&!vision)throw new InvalidOperationException("当前 MiniMax Code 模型不支持视觉输入，请选择 MiniMax-M3。");
            var size=item.Data?.LongLength??new FileInfo(item.FilePath??throw new InvalidOperationException("附件已不可用。")).Length;var limit=item.Type switch{AiAttachmentType.Image=>ImageLimit,AiAttachmentType.Video=>VideoLimit,_=>8L*1024*1024};
            if(size<=0||size>limit)throw new InvalidOperationException("MiniMax Code 附件超限：图片 20 MB、视频 50 MB、文本 8 MB。");
            total+=size;estimated+=item.Type==AiAttachmentType.Text?size*2:4*((size+2)/3)+512;
            if(total>RequestLimit||estimated>RequestLimit)throw new InvalidOperationException("MiniMax Code 内联请求超过 64 MiB，请减少附件。");
            if(item.Type==AiAttachmentType.Image&&!new[]{"image/png","image/jpeg","image/webp","image/gif"}.Contains(item.MimeType,StringComparer.OrdinalIgnoreCase)||item.Type==AiAttachmentType.Video&&!item.MimeType.Equals("video/mp4",StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException("MiniMax Code 不支持此附件格式。");
        }
    }
    private static byte[] ReadBytes(AiAttachment attachment)=>attachment.Data is { } bytes?(byte[])bytes.Clone():File.ReadAllBytes(attachment.FilePath??throw new InvalidOperationException("附件已不可用。"));
    private static string Text(JsonElement element,string key)=>element.ValueKind==JsonValueKind.Object&&element.TryGetProperty(key,out var value)&&value.ValueKind==JsonValueKind.String?value.GetString()??string.Empty:string.Empty;
}
