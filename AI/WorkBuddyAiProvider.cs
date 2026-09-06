// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.AI;

internal sealed class WorkBuddyAiProvider(string model,string effort,bool supportsImage) : IAiProvider
{
    internal const long ImageLimit=20L*1024*1024,VideoLimit=512L*1024*1024;
    public string Id=>"workbuddy";
    public AiProviderCapabilities Capabilities {get;}=new(supportsImage,supportsImage,true,ImageLimit,VideoLimit,TimeSpan.FromHours(4),new HashSet<string>(StringComparer.OrdinalIgnoreCase){"image/png","image/jpeg","image/webp","image/gif","video/mp4"});

    public async Task<bool> TestConnectionAsync(CancellationToken cancellationToken)
    {
        var response=await SendAsync(new AiRequest{Prompt="Reply exactly MEWU_OK. Do not use tools."},cancellationToken).ConfigureAwait(false);
        return response.Answer.Trim()=="MEWU_OK";
    }

    public async Task<AiResult> SendAsync(AiRequest request,CancellationToken cancellationToken)
    {
        var leases=new List<TempMediaLease>();var paths=new List<string>();
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(TimeSpan.FromMinutes(10));var token=timeout.Token;
        WorkBuddyAcpServer? server=null;
        try
        {
            Validate(request,supportsImage);
            token.ThrowIfCancellationRequested();
            var hasVideo=request.Attachments.Any(item=>item.Type==AiAttachmentType.Video);
            server=await WorkBuddyAcpServer.StartAsync(token,hasVideo).ConfigureAwait(false);
            var catalog=await server.NewSessionAsync(token).ConfigureAwait(false);
            var selected=catalog.Models.SingleOrDefault(item=>item.Model==model)
                ??throw new InvalidOperationException("所选 WorkBuddy 模型当前不可用，请在设置中重新检测并选择。");
            await server.ConfigureAsync(catalog,model,request.DisableReasoning?"disabled":effort,token).ConfigureAwait(false);
            if(request.Attachments.Any(item=>item.Type is AiAttachmentType.Image or AiAttachmentType.Video)&&!selected.SupportsImage)
                throw new InvalidOperationException("当前 WorkBuddy 模型不支持视觉输入。");
            var input=new List<object>();var text=new StringBuilder();
            foreach(var message in ConversationContextPolicy.CreateBoundedHistory(request.History))text.AppendLine($"[{message.Role}]\n{message.Text}");
            text.AppendLine("[current user request]").AppendLine(request.Prompt);
            foreach(var (attachment,index) in request.Attachments.Select((item,index)=>(item,index)))
            {
                token.ThrowIfCancellationRequested();
                var extension=attachment.Type switch{AiAttachmentType.Video=>".mp4",AiAttachmentType.Text=>".txt",_=>attachment.MimeType switch{"image/jpeg"=>".jpg","image/webp"=>".webp","image/gif"=>".gif",_=>".png"}};
                var path=Path.Combine(server.WorkingDirectory,$"attachment-{index}{extension}");
                leases.Add(TempMediaRegistry.Shared.Acquire(path));paths.Add(path);
                if(attachment.Data is { } data)await File.WriteAllBytesAsync(path,data,token).ConfigureAwait(false);
                else
                {
                    leases.Add(TempMediaRegistry.Shared.AcquireExistingFile(attachment.FilePath!));
                    await using var source=new FileStream(attachment.FilePath!,FileMode.Open,FileAccess.Read,FileShare.Read,81920,true);
                    var limit=Limit(attachment.Type);
                    if(source.Length>limit)throw new InvalidOperationException("WorkBuddy 附件超过大小限制。");
                    await using var destination=new FileStream(path,FileMode.CreateNew,FileAccess.Write,FileShare.None,81920,true);
                    await CopyBoundedAsync(source,destination,limit,token).ConfigureAwait(false);
                }
                if(attachment.Type==AiAttachmentType.Image)
                {
                    input.Add(new{type="text",text=$"Attachment regionIndex={index}"});
                    var bytes=await File.ReadAllBytesAsync(path,token).ConfigureAwait(false);
                    try{input.Add(new{type="image",data=Convert.ToBase64String(bytes),mimeType=attachment.MimeType});}
                    finally{CryptographicOperations.ZeroMemory(bytes);}
                }
                else if(attachment.Type==AiAttachmentType.Video)
                    text.AppendLine($"Video attachment regionIndex={index}: {JsonSerializer.Serialize(path)}. Use local tools to inspect this video and its timeline; do not modify the input. Place any decoded frames only in your working directory. If you cannot actually inspect it, report that limitation; never invent video contents.");
                else
                {
                    var bytes=await File.ReadAllBytesAsync(path,token).ConfigureAwait(false);
                    try{text.AppendLine($"Text attachment regionIndex={index}:\n{Encoding.UTF8.GetString(bytes)}");}
                    finally{CryptographicOperations.ZeroMemory(bytes);}
                }
            }
            input.Insert(0,new{type="text",text=text.ToString()});
            var turn=new WorkBuddyTurnCollector(catalog.SessionId,request,token);
            server.Notification+=turn.Receive;
            var finished=false;
            try
            {
                var response=await server.InvokeAsync("session/prompt",new{sessionId=catalog.SessionId,prompt=input},token,TimeSpan.FromMinutes(10)).ConfigureAwait(false);
                var result=turn.Finish(response);finished=true;
                token.ThrowIfCancellationRequested();return result;
            }
            finally
            {
                turn.Stop();server.Notification-=turn.Receive;
                if(!finished)
                {
                    using var interruptTimeout=new CancellationTokenSource(TimeSpan.FromSeconds(2));
                    try{await server.WriteAsync(new{jsonrpc="2.0",method="session/cancel",@params=new{sessionId=catalog.SessionId}},interruptTimeout.Token).ConfigureAwait(false);}catch(Exception ex)when(ex is IOException or InvalidOperationException or OperationCanceledException){}
                }
            }
        }
        catch(OperationCanceledException)when(!cancellationToken.IsCancellationRequested){throw new TimeoutException("WorkBuddy 本轮处理超过 10 分钟，已停止。请缩短视频或拆分问题后重试。");}
        finally
        {
            try{if(server is not null)await server.DisposeAsync().ConfigureAwait(false);}
            finally
            {
                foreach(var lease in leases)lease.Dispose();
                foreach(var path in paths)
                    try{TempMediaRegistry.Shared.TryExecuteIfUnleased(path,false,()=>File.Delete(path));}catch(Exception ex)when(ex is IOException or UnauthorizedAccessException){}
                if(server is not null)server.CleanWorkspace();
                foreach(var attachment in request.Attachments)
                    if(attachment.ProviderOwnsData&&attachment.Data is { } bytes)CryptographicOperations.ZeroMemory(bytes);
            }
        }
    }

    internal static void Validate(AiRequest request,bool image)
    {
        ArgumentNullException.ThrowIfNull(request);
        ConversationContextPolicy.EnsureValidForProvider(request.History);
        if(string.IsNullOrWhiteSpace(request.Prompt)||request.Prompt.Length>100_000)throw new InvalidOperationException("请输入问题，并将问题控制在 100,000 字符内。");
        if(request.Attachments.Count>16)throw new InvalidOperationException("单次最多发送 16 个附件。");
        long total=0,textBytes=0,inlineBytes=0;
        foreach(var attachment in request.Attachments)
        {
            if(!Enum.IsDefined(attachment.Type))throw new InvalidOperationException("不支持的 WorkBuddy 附件类型。");
            if(attachment.Type!=AiAttachmentType.Text&&!image)throw new InvalidOperationException("当前 WorkBuddy 模型不支持视觉输入。");
            var size=attachment.Data?.LongLength??new FileInfo(attachment.FilePath??throw new InvalidOperationException("附件已不可用。")).Length;
            if(size<=0||size>Limit(attachment.Type))throw new InvalidOperationException("WorkBuddy 附件为空或过大（图片 20 MB，视频 512 MB，文本 8 MB）。");
            total+=size;if(attachment.Type==AiAttachmentType.Text)textBytes+=size;
            if(attachment.Type==AiAttachmentType.Image)inlineBytes+=4*((size+2)/3)+512;
            if(attachment.Type==AiAttachmentType.Text)inlineBytes+=size*6;
            if(inlineBytes+request.Prompt.Length*6L+150_000>64L*1024*1024)throw new InvalidOperationException("WorkBuddy 内联附件超过 64 MiB，请减少图片或文本附件。");
            if(total>768L*1024*1024||textBytes>8L*1024*1024)throw new InvalidOperationException("本轮附件总大小过大，请减少附件。");
            if(attachment.Type==AiAttachmentType.Image&&!new[]{"image/png","image/jpeg","image/webp","image/gif"}.Contains(attachment.MimeType,StringComparer.OrdinalIgnoreCase))throw new InvalidOperationException("WorkBuddy 图片仅支持 PNG、JPEG、WebP 和 GIF。");
            if(attachment.Type==AiAttachmentType.Video&&attachment.MimeType!="video/mp4")throw new InvalidOperationException("WorkBuddy 本机视频交接目前支持 MP4。");
        }
    }

    private static long Limit(AiAttachmentType type)=>type switch{AiAttachmentType.Image=>ImageLimit,AiAttachmentType.Video=>VideoLimit,_=>8L*1024*1024};
    private static async Task CopyBoundedAsync(Stream source,Stream destination,long limit,CancellationToken token)
    {
        var buffer=new byte[81920];long total=0;
        try{int read;while((read=await source.ReadAsync(buffer,token).ConfigureAwait(false))>0){total+=read;if(total>limit)throw new InvalidOperationException("附件大小在读取中超出限制。");await destination.WriteAsync(buffer.AsMemory(0,read),token).ConfigureAwait(false);}}
        finally{CryptographicOperations.ZeroMemory(buffer);}
    }
}

internal sealed class WorkBuddyTurnCollector(string session,AiRequest request,CancellationToken token)
{
    private readonly StringBuilder _answer=new(),_reasoning=new();
    private readonly object _gate=new();
    private bool _stopped;
    internal void Stop(){lock(_gate)_stopped=true;}
    internal void Receive(string method,JsonElement data)
    {
        lock(_gate)
        {
            if(_stopped||token.IsCancellationRequested||method!="session/update"||WorkBuddyAcpServer.Text(data,"sessionId")!=session)return;
            if(!data.TryGetProperty("update",out var update))return;
            if(update.TryGetProperty("_meta",out var meta)&&meta.ValueKind==JsonValueKind.Object&&
               (meta.TryGetProperty("codebuddy.ai/memberEvent",out _)||meta.TryGetProperty("codebuddy.ai/isCompactInternal",out var compact)&&compact.ValueKind==JsonValueKind.True))return;
            var type=WorkBuddyAcpServer.Text(update,"sessionUpdate");
            if(type is "agent_message_chunk" or "agent_thought_chunk")
            {
                if(!update.TryGetProperty("content",out var content)||WorkBuddyAcpServer.Text(content,"type")!="text")return;
                var value=WorkBuddyAcpServer.Text(content,"text");var thought=type=="agent_thought_chunk";
                var target=thought?_reasoning:_answer;
                if(target.Length+value.Length>(thought?200_000:2_000_000))throw new InvalidDataException("WorkBuddy 回答超过安全限制。");
                target.Append(value);request.StreamingProgress?.Report(thought?new("",value):new(value,""));
            }
            else if(type=="tool_call")
            {
                // Text followed by a tool call is interim commentary. Only the
                // answer after the last tool belongs in the final result/history.
                _answer.Clear();
                request.AgentProgress?.Report(new(AiAgentEventKind.ToolStarted,"WorkBuddy 正在分析本机附件"));
            }
        }
    }
    internal AiResult Finish(JsonElement response)
    {
        lock(_gate)
        {
            token.ThrowIfCancellationRequested();
            if(_stopped)throw new OperationCanceledException("WorkBuddy 请求已结束。");
            _stopped=true;
            if(WorkBuddyAcpServer.Text(response,"stopReason")!="end_turn")throw new InvalidDataException("WorkBuddy 本轮未完整结束，请检查登录、额度或重试。");
            if(response.TryGetProperty("_meta",out var meta))
            {
                var outcome=WorkBuddyAcpServer.Text(meta,"codebuddy.ai/outcome");
                // ACP marks a completed answer PARTIAL_SUCCESS after any failed tool
                // attempt, even when a later tool succeeds. This is telemetry, not EOF.
                if(WorkBuddyAcpServer.Text(meta,"codebuddy.ai/errorMessage").Length>0||outcome.Length>0&&outcome is not ("SUCCESS" or "PARTIAL_SUCCESS"))
                    throw new InvalidDataException("WorkBuddy 本轮未成功完成，请在官方客户端检查模型、额度及工具状态后重试。");
            }
            var text=_answer.ToString();
            if(string.IsNullOrWhiteSpace(text))throw new InvalidDataException("WorkBuddy 没有返回完整正文。");
            return request.ExpectStructuredResponse?StructuredResponseParser.Parse(text,_reasoning.ToString(),true):new(text,[],_reasoning.ToString());
        }
    }
}
