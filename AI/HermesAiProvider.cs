using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.AI;

public sealed class HermesAiProvider : IAiProvider,IDisposable
{
    private const long MaxImageBytes=25L*1024*1024;
    private const long MaxVideoBytes=512L*1024*1024;
    private const int MaxAttachmentCount=16;
    private static readonly TimeSpan InterruptDrainTimeout=TimeSpan.FromSeconds(12);
    private static readonly IReadOnlySet<string> AcceptedMimeTypes=new HashSet<string>(StringComparer.OrdinalIgnoreCase){"image/png","image/jpeg","image/webp","image/gif","image/bmp","video/mp4"};
    private readonly HermesRuntimeService _runtime;
    private readonly string _profile;
    private readonly Func<AppSettings> _settingsAccessor;
    private readonly SemaphoreSlim _turnGate=new(1,1),_sessionGate=new(1,1);
    private readonly object _activeGate=new();
    private string? _sessionId;
    private string? _storedSessionId;
    private long _connectionGeneration;
    private string _appliedProvider=string.Empty,_appliedModel=string.Empty,_appliedReasoning=string.Empty;
    private ActiveTurn? _active;
    private long _generation;
    private int _disposed;

    public HermesAiProvider(HermesRuntimeService runtime,string profile,Func<AppSettings> settingsAccessor)
    {
        _runtime=runtime??throw new ArgumentNullException(nameof(runtime));
        _profile=HermesRuntimeService.NormalizeProfile(profile);
        _settingsAccessor=settingsAccessor??throw new ArgumentNullException(nameof(settingsAccessor));
        _runtime.EventReceived+=OnHermesEvent;
    }

    // Kept for internal callers compiled against the first bridge prototype;
    // both entry points now intentionally share the same runtime conversation.
    internal HermesAiProvider(HermesRuntimeService runtime,HermesConversationKind kind,Func<AppSettings> settingsAccessor):this(runtime,settingsAccessor().HermesProfile,settingsAccessor){}

    public string Id=>"hermes-local";
    public AiProviderCapabilities Capabilities { get; }=new(true,true,true,MaxImageBytes,MaxVideoBytes,TimeSpan.FromHours(4),AcceptedMimeTypes);

    public Task<bool> TestConnectionAsync(CancellationToken cancellationToken)=>_runtime.TestConnectionAsync(cancellationToken);

    public async Task<AiResult> SendAsync(AiRequest request,CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(request);
        ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed)!=0,this);
        var gateHeld=false;
        var attachedImagePaths=new List<string>();
        ActiveTurn? turn=null;
        var promptStarted=false;
        try
        {
            ValidateRequest(request);
            await _turnGate.WaitAsync(cancellationToken).ConfigureAwait(false);
            gateHeld=true;
            cancellationToken.ThrowIfCancellationRequested();
            var sessionId=await EnsureSessionAsync(cancellationToken).ConfigureAwait(false);
            await ApplySettingsAsync(sessionId,cancellationToken).ConfigureAwait(false);
            var prompt=BuildPrompt(request);
            foreach(var attachment in request.Attachments)
            {
                cancellationToken.ThrowIfCancellationRequested();
                if(attachment.Type==AiAttachmentType.Image)
                {
                    var bytes=await ReadAttachmentAsync(attachment,MaxImageBytes,cancellationToken).ConfigureAwait(false);
                    try
                    {
                        // JsonSerializer writes byte[] as Base64 directly into
                        // the RPC buffer, which is cleared after SendAsync. Do
                        // not create a second immutable Base64 string here.
                        var result=await _runtime.InvokeAsync("image.attach_bytes",new{session_id=sessionId,content_base64=bytes,filename=ImageFilename(attachment.MimeType)},cancellationToken).ConfigureAwait(false);
                        if(TryReadString(result,"path",out var imagePath))attachedImagePaths.Add(imagePath);
                    }
                    finally{if(!ReferenceEquals(bytes,attachment.Data))CryptographicOperations.ZeroMemory(bytes);}
                }
                else if(attachment.Type==AiAttachmentType.Video)
                {
                    var path=attachment.FilePath;
                    if(string.IsNullOrWhiteSpace(path)||!File.Exists(path))throw new FileNotFoundException("Hermes 要分析的视频已不可用。",path);
                    var info=new FileInfo(path);
                    if(info.Length>MaxVideoBytes)throw new InvalidOperationException("Hermes 本地会话单个视频暂限 512 MB，请先压缩或裁剪。");
                    var result=await _runtime.InvokeAsync("file.attach",new{session_id=sessionId,path=info.FullName,name=info.Name},cancellationToken).ConfigureAwait(false);
                    if(TryReadString(result,"ref_text",out var reference))prompt+=$"\n\n视频附件：{reference}";
                }
                else if(attachment.Type==AiAttachmentType.Text)
                {
                    var bytes=await ReadAttachmentAsync(attachment,8L*1024*1024,cancellationToken).ConfigureAwait(false);
                    try{prompt+=$"\n\n文本附件（{Path.GetFileName(attachment.FilePath??"文本") }）：\n{System.Text.Encoding.UTF8.GetString(bytes)}";}
                    finally{if(!ReferenceEquals(bytes,attachment.Data))CryptographicOperations.ZeroMemory(bytes);}
                }
            }

            turn=new ActiveTurn(Interlocked.Increment(ref _generation),sessionId,request,cancellationToken);
            lock(_activeGate)_active=turn;
            using var cancellationRegistration=cancellationToken.Register(static state=>
            {
                var tuple=((HermesAiProvider Owner,string SessionId))state!;
                _=tuple.Owner.InterruptSafelyAsync(tuple.SessionId);
            },(this,sessionId));
            promptStarted=true;
            _=await _runtime.InvokeAsync("prompt.submit",new{session_id=sessionId,text=prompt},cancellationToken).ConfigureAwait(false);
            var terminal=await turn.Completion.Task.WaitAsync(cancellationToken).ConfigureAwait(false);
            cancellationToken.ThrowIfCancellationRequested();
            if(terminal.Status.Equals("interrupted",StringComparison.OrdinalIgnoreCase))throw new OperationCanceledException("Hermes 会话已中断。",cancellationToken);
            if(terminal.Status.Equals("error",StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException(string.IsNullOrWhiteSpace(terminal.Error)?"Hermes 会话执行失败。":terminal.Error);
            if(!terminal.Status.Equals("complete",StringComparison.OrdinalIgnoreCase))throw new InvalidDataException("Hermes 返回了无法确认的会话终态。");
            if(string.IsNullOrWhiteSpace(terminal.Text))throw new InvalidOperationException("Hermes 已结束本轮，但没有生成可显示的正文。");
            return request.ExpectStructuredResponse
                ?StructuredResponseParser.Parse(terminal.Text,terminal.Reasoning)
                :new AiResult(terminal.Text,[],terminal.Reasoning);
        }
        catch(OperationCanceledException) when(cancellationToken.IsCancellationRequested)
        {
            if(turn is not null&&promptStarted)await InterruptAndDrainAsync(turn).ConfigureAwait(false);
            throw;
        }
        catch
        {
            if(turn is not null&&promptStarted)await InterruptAndDrainAsync(turn).ConfigureAwait(false);
            throw;
        }
        finally
        {
            if(turn is not null)lock(_activeGate){if(ReferenceEquals(_active,turn))_active=null;}
            if((!promptStarted||turn?.Completion.Task.IsCompleted!=true)&&_sessionId is { } session)
                foreach(var path in attachedImagePaths)try{await _runtime.InvokeAsync("image.detach",new{session_id=session,path},CancellationToken.None).ConfigureAwait(false);}catch{}
            if(gateHeld)_turnGate.Release();
            ClearOwnedAttachmentData(request.Attachments);
        }
    }

    private async Task<string> EnsureSessionAsync(CancellationToken cancellationToken)
    {
        _=await _runtime.EnsureConnectedAsync(cancellationToken).ConfigureAwait(false);
        var generation=_runtime.ConnectionGeneration;
        if(_sessionId is {Length:>0} existing&&_connectionGeneration==generation)return existing;
        await _sessionGate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            _=await _runtime.EnsureConnectedAsync(cancellationToken).ConfigureAwait(false);
            generation=_runtime.ConnectionGeneration;
            if(_sessionId is {Length:>0} current&&_connectionGeneration==generation)return current;
            var settings=_settingsAccessor();
            ValidateSettings(settings);
            if(!string.IsNullOrWhiteSpace(_storedSessionId))
            {
                JsonElement resumed;
                try
                {
                    resumed=await _runtime.InvokeAsync("session.resume",new
                    {
                        session_id=_storedSessionId,
                        profile=_profile,
                        source="desktop",
                        omit_messages=true,
                        close_on_disconnect=true
                    },cancellationToken).ConfigureAwait(false);
                }
                catch(HermesRpcException ex)when(ex.Code==4007)
                {
                    throw new InvalidOperationException("Hermes 原会话暂时无法恢复。为避免丢失上下文，喵呜AI 没有自动新建会话；请稍后重试。",ex);
                }
                if(!TryReadString(resumed,"session_id",out var resumedId))throw new InvalidDataException("Hermes 未返回恢复后的会话编号。");
                _sessionId=resumedId;
                if(TryReadString(resumed,"stored_session_id",out var resumedStored))_storedSessionId=resumedStored;
                _connectionGeneration=generation;
                _appliedProvider=string.Empty;
                _appliedModel=string.Empty;
                _appliedReasoning=string.Empty;
                return resumedId;
            }
            var createParams=new Dictionary<string,object?>
            {
                ["title"]="喵呜AI 本机会话",
                ["hidden"]=true,
                ["close_on_disconnect"]=true,
                ["source"]="desktop"
            };
            createParams["profile"]=_profile;
            if(!string.IsNullOrWhiteSpace(settings.HermesModel))createParams["model"]=settings.HermesModel;
            if(!string.IsNullOrWhiteSpace(settings.HermesProvider))createParams["provider"]=settings.HermesProvider;
            if(!string.IsNullOrWhiteSpace(settings.HermesReasoningEffort))createParams["reasoning_effort"]=NormalizeReasoning(settings.HermesReasoningEffort);
            var result=await _runtime.InvokeAsync("session.create",createParams,cancellationToken).ConfigureAwait(false);
            if(!TryReadString(result,"session_id",out var created))throw new InvalidDataException("Hermes 未返回会话编号。");
            _sessionId=created;
            _storedSessionId=ReadString(result,"stored_session_id",string.Empty);
            if(string.IsNullOrWhiteSpace(_storedSessionId))throw new InvalidDataException("Hermes 未返回可持续恢复的会话编号。");
            _connectionGeneration=generation;
            _appliedProvider=settings.HermesProvider.Trim();
            _appliedModel=settings.HermesModel.Trim();
            _appliedReasoning=NormalizeReasoning(settings.HermesReasoningEffort);
            return created;
        }
        finally{_sessionGate.Release();}
    }

    private async Task ApplySettingsAsync(string sessionId,CancellationToken cancellationToken)
    {
        var settings=_settingsAccessor();
        ValidateSettings(settings);
        var provider=settings.HermesProvider.Trim();
        var model=settings.HermesModel.Trim();
        var reasoning=NormalizeReasoning(settings.HermesReasoningEffort);
        if((!string.Equals(provider,_appliedProvider,StringComparison.Ordinal)||!string.Equals(model,_appliedModel,StringComparison.Ordinal))&&model.Length>0)
        {
            var modelValue=provider.Length>0?$"{model} --provider {provider} --session":$"{model} --session";
            var result=await _runtime.InvokeAsync("config.set",new{session_id=sessionId,profile=_profile,key="model",value=modelValue,confirm_expensive_model=true},cancellationToken).ConfigureAwait(false);
            if(result.TryGetProperty("confirm_required",out var confirm)&&confirm.ValueKind==JsonValueKind.True)
                throw new InvalidOperationException(ReadString(result,"confirm_message","Hermes 要求确认模型切换，请在设置页重新选择后测试连接。"));
            _appliedProvider=provider;_appliedModel=model;
        }
        if(!string.Equals(reasoning,_appliedReasoning,StringComparison.Ordinal))
        {
            _=await _runtime.InvokeAsync("config.set",new{session_id=sessionId,profile=_profile,key="reasoning",value=reasoning},cancellationToken).ConfigureAwait(false);
            _appliedReasoning=reasoning;
        }
    }

    private string BuildPrompt(AiRequest request)
    {
        var prompt=request.Prompt?.Trim()??string.Empty;
        if(!request.ExpectStructuredResponse)return prompt;
        var system=request.History?.FirstOrDefault(message=>string.Equals(message.Role,"system",StringComparison.OrdinalIgnoreCase))?.Text;
        return string.IsNullOrWhiteSpace(system)?prompt:$"{system.Trim()}\n\n用户问题：{prompt}";
    }

    private void OnHermesEvent(object? sender,HermesRpcEvent message)
    {
        if(message.Type=="session.info"&&string.Equals(message.SessionId,_sessionId,StringComparison.Ordinal)&&TryReadString(message.Payload,"stored_session_id",out var stored))
            _storedSessionId=stored;
        ActiveTurn? turn;
        lock(_activeGate)turn=_active;
        if(turn is null)return;
        var isCurrentSession=string.Equals(turn.SessionId,message.SessionId,StringComparison.Ordinal);
        if(!isCurrentSession&&!(message.Type=="error"&&string.IsNullOrEmpty(message.SessionId)))return;
        try
        {
            switch(message.Type)
            {
                case "message.delta":
                    if(TryReadString(message.Payload,"text",out var content)&&content.Length>0)turn.Request.StreamingProgress?.Report(new AiStreamDelta(content,string.Empty));
                    break;
                case "reasoning.delta":
                    if(TryReadString(message.Payload,"text",out var reasoning)&&reasoning.Length>0)turn.Request.StreamingProgress?.Report(new AiStreamDelta(string.Empty,reasoning));
                    break;
                case "reasoning.available":
                    if(TryReadString(message.Payload,"text",out var availableReasoning)&&availableReasoning.Length>0)turn.Request.StreamingProgress?.Report(new AiStreamDelta(string.Empty,availableReasoning,true));
                    break;
                case "thinking.delta":
                    if(TryReadString(message.Payload,"text",out var thinking)&&thinking.Length>0)ReportAgent(turn,AiAgentEventKind.Status,"Hermes 正在思考",thinking);
                    break;
                case "message.interim":
                    if(TryReadString(message.Payload,"text",out var interim)&&interim.Length>0)ReportAgent(turn,AiAgentEventKind.Status,"阶段性进展",interim);
                    break;
                case "message.complete":
                    var text=ReadString(message.Payload,"text",string.Empty);
                    var finalReasoning=ReadString(message.Payload,"reasoning",string.Empty);
                    var status=ReadString(message.Payload,"status",string.Empty);
                    if(status is not ("complete" or "interrupted" or "error"))
                    {
                        turn.Completion.TrySetException(new InvalidDataException("Hermes 结束事件缺少明确终态，已拒绝将不完整回复写入会话。"));
                        break;
                    }
                    var error=ReadString(message.Payload,"error",status=="error"?text:string.Empty);
                    turn.Completion.TrySetResult(new HermesTerminalMessage(text,finalReasoning,status,error));
                    break;
                case "error":
                    turn.Completion.TrySetResult(new HermesTerminalMessage(string.Empty,string.Empty,"error",ReadString(message.Payload,"message",ReadString(message.Payload,"error","Hermes 本地后端发生错误。"))));
                    break;
                case "status.update":
                    ReportAgent(turn,AiAgentEventKind.Status,ReadString(message.Payload,"text","Hermes 正在处理"),ReadString(message.Payload,"kind",string.Empty));
                    break;
                case "tool.start":
                    ReportAgent(turn,AiAgentEventKind.ToolStarted,ToolTitle(message.Payload),ToolDetail(message.Payload));
                    break;
                case "tool.progress":
                    ReportAgent(turn,AiAgentEventKind.ToolProgress,ToolTitle(message.Payload),ToolDetail(message.Payload));
                    break;
                case "tool.complete":
                    ReportAgent(turn,AiAgentEventKind.ToolCompleted,ToolTitle(message.Payload),ToolDetail(message.Payload),ReadString(message.Payload,"status",string.Empty)=="error");
                    break;
                case "subagent.start":
                    ReportAgent(turn,AiAgentEventKind.ToolStarted,"Hermes 子任务",ReadString(message.Payload,"label",ReadString(message.Payload,"task",string.Empty)));
                    break;
                case "subagent.progress":
                    ReportAgent(turn,AiAgentEventKind.ToolProgress,"Hermes 子任务",ReadString(message.Payload,"text",ReadString(message.Payload,"message",string.Empty)));
                    break;
                case "subagent.complete":
                    ReportAgent(turn,AiAgentEventKind.ToolCompleted,"Hermes 子任务",ReadString(message.Payload,"text",ReadString(message.Payload,"message",string.Empty)),ReadString(message.Payload,"status",string.Empty)=="error");
                    break;
                case "approval.request":_ = HandleApprovalAsync(turn,message.Payload);break;
                case "clarify.request" or "input.request":_ = HandleClarificationAsync(turn,message.Payload);break;
                case "sudo.request":_ = HandleSecretAsync(turn,message.Payload,true);break;
                case "secret.request":_ = HandleSecretAsync(turn,message.Payload,false);break;
                case "terminal.read.request":_ = RejectDesktopBridgeAsync(turn,message.Payload,"terminal.read.respond","text","终端读取");break;
                case "preview.read.request":_ = RejectDesktopBridgeAsync(turn,message.Payload,"preview.read.respond","text","Hermes Desktop 预览读取");break;
                case "preview.act.request":_ = RejectDesktopBridgeAsync(turn,message.Payload,"preview.act.respond","text","Hermes Desktop 预览操作");break;
                case "window.read.request":_ = RejectDesktopBridgeAsync(turn,message.Payload,"window.read.respond","text","窗口读取");break;
                case "tour.request":_ = RejectDesktopBridgeAsync(turn,message.Payload,"tour.respond","text","Hermes Desktop 导览");break;
                case "mcp.setup.request":_ = RejectMcpSetupAsync(turn,message.Payload);break;
            }
        }
        catch(Exception ex){turn.Completion.TrySetException(ex);}
    }

    private async Task HandleApprovalAsync(ActiveTurn turn,JsonElement payload)
    {
        var requestId=ReadString(payload,"request_id",string.Empty);
        if(requestId.Length==0)return;
        try{_=await _runtime.InvokeAsync("approval.received",new{session_id=turn.SessionId,request_id=requestId},turn.CancellationToken).ConfigureAwait(false);}catch{}
        var allowed=new HashSet<string>(["once","session","always","deny"],StringComparer.OrdinalIgnoreCase);
        if(payload.TryGetProperty("allow_permanent",out var allowPermanent)&&allowPermanent.ValueKind==JsonValueKind.False)allowed.Remove("always");
        var choices=ReadStringArray(payload,"choices").Where(allowed.Contains).Distinct(StringComparer.OrdinalIgnoreCase).ToList();
        if(choices.Count==0)choices=allowed.Where(choice=>choice!="always").ToList();
        if(!choices.Contains("deny",StringComparer.OrdinalIgnoreCase))choices.Add("deny");
        var command=ReadString(payload,"command",string.Empty);
        var reason=ReadString(payload,"description",ReadString(payload,"reason",ReadString(payload,"message",string.Empty)));
        var response=await AskAsync(turn,new AiInteractionRequest(AiInteractionKind.Approval,requestId,"Hermes 请求执行操作",string.Join("\n",new[]{command,reason}.Where(value=>!string.IsNullOrWhiteSpace(value))),choices),new AiInteractionResponse(string.Empty,"deny")).ConfigureAwait(false);
        var choice=choices.Contains(response.Choice,StringComparer.OrdinalIgnoreCase)?response.Choice:"deny";
        if(choice.Equals("always",StringComparison.OrdinalIgnoreCase))
        {
            var confirmation=await AskAsync(turn,new AiInteractionRequest(AiInteractionKind.Approval,$"{requestId}:always","再次确认永久授权","永久允许会写入 Hermes 的命令授权规则，并影响之后的会话。仅在你确认信任该命令时继续。",["always","deny"]),new AiInteractionResponse(string.Empty,"deny")).ConfigureAwait(false);
            choice=confirmation.Choice.Equals("always",StringComparison.OrdinalIgnoreCase)?"always":"deny";
        }
        _=await _runtime.InvokeAsync("approval.respond",new{session_id=turn.SessionId,request_id=requestId,choice,all=false},turn.CancellationToken).ConfigureAwait(false);
    }

    private async Task HandleClarificationAsync(ActiveTurn turn,JsonElement payload)
    {
        var requestId=ReadString(payload,"request_id",string.Empty);
        if(requestId.Length==0)return;
        if(payload.TryGetProperty("questions",out var questions)&&questions.ValueKind==JsonValueKind.Array)
        {
            var locked=payload.TryGetProperty("answers",out var answers)&&answers.ValueKind==JsonValueKind.Object?answers:default;
            foreach(var question in questions.EnumerateArray())
            {
                var qid=ReadString(question,"qid",string.Empty);
                if(qid.Length==0)continue;
                if(locked.ValueKind==JsonValueKind.Object&&locked.TryGetProperty(qid,out var lockedAnswer)&&lockedAnswer.ValueKind==JsonValueKind.String)continue;
                var choices=ReadStringArray(question,"choices");
                var multiSelect=ReadBoolean(question,"multi_select");
                var response=await AskAsync(turn,new AiInteractionRequest(AiInteractionKind.Clarification,requestId,"Hermes 需要补充信息",ReadString(question,"question","请补充信息"),choices,false,multiSelect,qid),new AiInteractionResponse(string.Empty)).ConfigureAwait(false);
                var formatted=FormatClarificationAnswer(response,multiSelect,choices);
                _=await _runtime.InvokeAsync("clarify.respond",new{session_id=turn.SessionId,request_id=requestId,question_id=qid,answer=formatted},turn.CancellationToken).ConfigureAwait(false);
            }
            return;
        }
        var singleChoices=ReadStringArray(payload,"choices");
        var singleMultiSelect=ReadBoolean(payload,"multi_select");
        var answer=await AskAsync(turn,new AiInteractionRequest(AiInteractionKind.Clarification,requestId,"Hermes 需要补充信息",ReadString(payload,"question","请补充信息"),singleChoices,false,singleMultiSelect),new AiInteractionResponse(string.Empty)).ConfigureAwait(false);
        _=await _runtime.InvokeAsync("clarify.respond",new{session_id=turn.SessionId,request_id=requestId,answer=FormatClarificationAnswer(answer,singleMultiSelect,singleChoices)},turn.CancellationToken).ConfigureAwait(false);
    }

    private async Task HandleSecretAsync(ActiveTurn turn,JsonElement payload,bool sudo)
    {
        var requestId=ReadString(payload,"request_id",string.Empty);
        if(requestId.Length==0)return;
        var kind=sudo?AiInteractionKind.SudoPassword:AiInteractionKind.Secret;
        var title=sudo?"Hermes 需要系统密码":"Hermes 需要敏感信息";
        var message=ReadString(payload,"prompt",ReadString(payload,"message",sudo?"请输入密码；内容不会保存到对话历史。":"请输入所需信息；内容不会保存到对话历史。"));
        var response=await AskAsync(turn,new AiInteractionRequest(kind,requestId,title,message,[],true),new AiInteractionResponse(string.Empty)).ConfigureAwait(false);
        var method=sudo?"sudo.respond":"secret.respond";
        object parameters=sudo?new{session_id=turn.SessionId,request_id=requestId,password=response.Value}:new{session_id=turn.SessionId,request_id=requestId,value=response.Value};
        _=await _runtime.InvokeAsync(method,parameters,turn.CancellationToken).ConfigureAwait(false);
    }

    private async Task RejectDesktopBridgeAsync(ActiveTurn turn,JsonElement payload,string method,string responseKey,string feature)
    {
        var requestId=ReadString(payload,"request_id",string.Empty);
        if(requestId.Length==0)return;
        ReportAgent(turn,AiAgentEventKind.Status,$"{feature}当前不可用","喵呜AI 已安全拒绝仅 Hermes Desktop 提供的客户端桥接请求。",true);
        var body=JsonSerializer.Serialize(new{success=false,error=$"{feature} requires Hermes Desktop and is not exposed by the MewuAI client."});
        var parameters=new Dictionary<string,object?>{{"session_id",turn.SessionId},{"request_id",requestId},{responseKey,body}};
        try{_=await _runtime.InvokeAsync(method,parameters,turn.CancellationToken).ConfigureAwait(false);}catch{}
    }

    private async Task RejectMcpSetupAsync(ActiveTurn turn,JsonElement payload)
    {
        var requestId=ReadString(payload,"request_id",string.Empty);
        if(requestId.Length==0)return;
        var server=ReadString(payload,"server",string.Empty);
        ReportAgent(turn,AiAgentEventKind.Status,"MCP 配置需要 Hermes Desktop",server.Length==0?"当前会话已安全拒绝交互式 MCP 配置。":$"未更改 MCP：{server}",true);
        var result=JsonSerializer.Serialize(new{status="declined",server,detail="Interactive MCP setup is not exposed by the MewuAI client."});
        try{_=await _runtime.InvokeAsync("mcp.setup.respond",new{session_id=turn.SessionId,request_id=requestId,result},turn.CancellationToken).ConfigureAwait(false);}catch{}
    }

    private static async Task<AiInteractionResponse> AskAsync(ActiveTurn turn,AiInteractionRequest request,AiInteractionResponse fallback)
    {
        if(turn.Request.InteractionHandler is null)return fallback;
        try{return await turn.Request.InteractionHandler(request,turn.CancellationToken).ConfigureAwait(false)??fallback;}
        catch(OperationCanceledException) when(turn.CancellationToken.IsCancellationRequested){return fallback;}
        catch{return fallback;}
    }

    private async Task InterruptSafelyAsync(string sessionId)
    {
        try{_=await _runtime.InvokeAsync("session.interrupt",new{session_id=sessionId},CancellationToken.None).ConfigureAwait(false);}catch{}
    }

    private async Task InterruptAndDrainAsync(ActiveTurn turn)
    {
        if(turn.Completion.Task.IsCompleted)return;
        await InterruptSafelyAsync(turn.SessionId).ConfigureAwait(false);
        try{_=await turn.Completion.Task.WaitAsync(InterruptDrainTimeout).ConfigureAwait(false);}
        catch(TimeoutException){await AbandonLiveSessionAsync(turn.SessionId).ConfigureAwait(false);}
        catch(OperationCanceledException){}
        catch{}
    }

    private async Task AbandonLiveSessionAsync(string sessionId)
    {
        try{_=await _runtime.InvokeAsync("session.close",new{session_id=sessionId},CancellationToken.None).ConfigureAwait(false);}catch{}
        if(string.Equals(_sessionId,sessionId,StringComparison.Ordinal))
        {
            _sessionId=null;
            _connectionGeneration=0;
            _appliedProvider=string.Empty;
            _appliedModel=string.Empty;
            _appliedReasoning=string.Empty;
        }
    }

    private static void ReportAgent(ActiveTurn turn,AiAgentEventKind kind,string title,string detail="",bool error=false)=>turn.Request.AgentProgress?.Report(new AiAgentEvent(kind,title,detail,error));
    private static string ToolTitle(JsonElement payload)=>ReadString(payload,"name",ReadString(payload,"tool",ReadString(payload,"title","Hermes 工具")));
    private static string ToolDetail(JsonElement payload)=>ReadString(payload,"message",ReadString(payload,"detail",ReadString(payload,"output",string.Empty)));

    private void ValidateSettings(AppSettings settings)
    {
        ArgumentNullException.ThrowIfNull(settings);
        if(!string.Equals(HermesRuntimeService.NormalizeProfile(settings.HermesProfile),_profile,StringComparison.Ordinal))
            throw new InvalidOperationException("Hermes Agent / 人格已切换，请重新发送本轮消息。");
        ValidateToken(settings.HermesProvider,"Hermes Provider");
        ValidateToken(settings.HermesModel,"Hermes 模型");
        _=NormalizeReasoning(settings.HermesReasoningEffort);
    }

    private static void ValidateToken(string? value,string name)
    {
        if(string.IsNullOrWhiteSpace(value))return;
        if(value.Any(character=>char.IsWhiteSpace(character)||char.IsControl(character)||character is '\'' or '"' or '\\')||value.Contains("--",StringComparison.Ordinal))
            throw new InvalidOperationException($"{name} 值无效，请从设置页的 Hermes 模型列表重新选择。");
    }

    private static string NormalizeReasoning(string? value)
    {
        var normalized=string.IsNullOrWhiteSpace(value)?"medium":value.Trim().ToLowerInvariant();
        if(!HermesRuntimeService.ReasoningEfforts.Contains(normalized,StringComparer.Ordinal))throw new InvalidOperationException("Hermes 思考程度无效，请在设置页重新选择。");
        return normalized;
    }

    private static void ValidateRequest(AiRequest request)
    {
        ArgumentNullException.ThrowIfNull(request.History);
        ArgumentNullException.ThrowIfNull(request.Attachments);
        if((request.Prompt?.Length??0)>1_000_000)throw new InvalidOperationException("发送给 Hermes 的问题过长，请缩短后重试。");
        if(request.Attachments.Count>MaxAttachmentCount)throw new InvalidOperationException($"Hermes 单次最多接收 {MaxAttachmentCount} 个附件。");
        long imageBytes=0,videoBytes=0;
        foreach(var attachment in request.Attachments)
        {
            if(attachment is null)throw new InvalidOperationException("Hermes 附件不能为空。");
            if(attachment.Type!=AiAttachmentType.Text&&!AcceptedMimeTypes.Contains(attachment.MimeType))throw new InvalidOperationException($"Hermes 暂不支持附件格式：{attachment.MimeType}");
            var size=GetAttachmentSize(attachment);
            if(size<=0)throw new InvalidOperationException("Hermes 附件内容为空。");
            if(attachment.Type==AiAttachmentType.Image)
            {
                if(size>MaxImageBytes)throw new InvalidOperationException("发送给 Hermes 的图片超过 25 MB。");
                imageBytes=AddSaturating(imageBytes,size);
            }
            else if(attachment.Type==AiAttachmentType.Video)
            {
                if(attachment.Data is not null)throw new InvalidOperationException("Hermes 视频附件必须使用本机文件，不能以内存缓冲发送。");
                if(size>MaxVideoBytes)throw new InvalidOperationException("Hermes 本地会话单个视频暂限 512 MB，请先压缩或裁剪。");
                if(attachment.Duration>TimeSpan.FromHours(4))throw new InvalidOperationException("Hermes 本地会话暂不接收超过 4 小时的视频。");
                videoBytes=AddSaturating(videoBytes,size);
            }
            else if(attachment.Type==AiAttachmentType.Text)
            {
                if(size>8L*1024*1024)throw new InvalidOperationException("发送给 Hermes 的文本文件超过 8 MB。");
            }
            else throw new InvalidOperationException("Hermes 收到了未知附件类型。");
        }
        // Images are Base64 encoded into individual RPC frames. Keep the
        // aggregate bounded as well so a cancelled batch cannot create a very
        // large succession of managed allocations.
        if(imageBytes>48L*1024*1024)throw new InvalidOperationException("发送给 Hermes 的图片总量超过 48 MB，请减少选区数量或尺寸。");
        if(videoBytes>2L*1024*1024*1024)throw new InvalidOperationException("发送给 Hermes 的视频总量超过 2 GB，请减少视频数量或裁剪后重试。");
    }

    private static long GetAttachmentSize(AiAttachment attachment)
    {
        if(attachment.Data is { } data)return data.LongLength;
        if(string.IsNullOrWhiteSpace(attachment.FilePath))throw new InvalidOperationException("Hermes 附件缺少数据或文件路径。");
        var file=new FileInfo(attachment.FilePath);
        if(!file.Exists)throw new FileNotFoundException("Hermes 附件文件不存在。",attachment.FilePath);
        return file.Length;
    }

    private static long AddSaturating(long left,long right)=>left<0||right<0||left>long.MaxValue-right?long.MaxValue:left+right;

    private static string FormatClarificationAnswer(AiInteractionResponse response,bool multiSelect,IReadOnlyList<string> choices)
    {
        if(!multiSelect)return string.IsNullOrWhiteSpace(response.Value)?response.Choice:response.Value;
        var values=response.Values?.Where(value=>!string.IsNullOrWhiteSpace(value)).ToList()??[];
        if(values.Count==0&&!string.IsNullOrWhiteSpace(response.Value))
        {
            try
            {
                using var document=JsonDocument.Parse(response.Value);
                if(document.RootElement.ValueKind==JsonValueKind.Array)
                    values=document.RootElement.EnumerateArray().Where(value=>value.ValueKind==JsonValueKind.String).Select(value=>value.GetString()!).Where(value=>!string.IsNullOrWhiteSpace(value)).ToList();
                else values.Add(response.Value);
            }
            catch(JsonException){values.Add(response.Value);}
        }
        if(choices.Count>0)
        {
            var canonical=new Dictionary<string,string>(StringComparer.OrdinalIgnoreCase);
            foreach(var choice in choices)if(!canonical.ContainsKey(choice))canonical.Add(choice,choice);
            values=values.Where(canonical.ContainsKey).Select(value=>canonical[value]).Distinct(StringComparer.OrdinalIgnoreCase).ToList();
        }
        return JsonSerializer.Serialize(values);
    }

    private static void ClearOwnedAttachmentData(IReadOnlyList<AiAttachment>? attachments)
    {
        if(attachments is null)return;
        foreach(var attachment in attachments)
            if(attachment is {ProviderOwnsData:true,Data:{ } data})CryptographicOperations.ZeroMemory(data);
    }

    private static async Task<byte[]> ReadAttachmentAsync(AiAttachment attachment,long maxBytes,CancellationToken cancellationToken)
    {
        if(attachment.Data is { } data)
        {
            if(data.LongLength>maxBytes)throw new InvalidOperationException("发送给 Hermes 的图片超过 25 MB。");
            return data;
        }
        if(string.IsNullOrWhiteSpace(attachment.FilePath)||!File.Exists(attachment.FilePath))throw new FileNotFoundException("发送给 Hermes 的图片已不可用。",attachment.FilePath);
        var info=new FileInfo(attachment.FilePath);
        if(info.Length>maxBytes)throw new InvalidOperationException("发送给 Hermes 的图片超过 25 MB。");
        return await File.ReadAllBytesAsync(info.FullName,cancellationToken).ConfigureAwait(false);
    }

    private static string ImageFilename(string mime)=>mime.ToLowerInvariant() switch{"image/jpeg"=>"capture.jpg","image/webp"=>"capture.webp","image/gif"=>"capture.gif","image/bmp"=>"capture.bmp",_=>"capture.png"};
    private static IReadOnlyList<string> ReadStringArray(JsonElement element,string name)
    {
        if(!element.TryGetProperty(name,out var value)||value.ValueKind!=JsonValueKind.Array)return [];
        return value.EnumerateArray().Where(item=>item.ValueKind==JsonValueKind.String).Select(item=>item.GetString()).Where(item=>!string.IsNullOrWhiteSpace(item)).Cast<string>().ToArray();
    }
    private static bool TryReadString(JsonElement element,string name,out string value){value=ReadString(element,name,string.Empty);return value.Length>0;}
    private static bool ReadBoolean(JsonElement element,string name)=>element.ValueKind==JsonValueKind.Object&&element.TryGetProperty(name,out var value)&&value.ValueKind==JsonValueKind.True;
    private static string ReadString(JsonElement element,string name,string fallback)=>element.ValueKind==JsonValueKind.Object&&element.TryGetProperty(name,out var value)&&value.ValueKind==JsonValueKind.String?value.GetString()??fallback:fallback;

    public void Dispose()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        _runtime.EventReceived-=OnHermesEvent;
        ActiveTurn? turn;lock(_activeGate){turn=_active;_active=null;}
        turn?.Completion.TrySetCanceled();
        _turnGate.Dispose();_sessionGate.Dispose();
    }

    private sealed class ActiveTurn(long generation,string sessionId,AiRequest request,CancellationToken cancellationToken)
    {
        public long Generation { get; }=generation;
        public string SessionId { get; }=sessionId;
        public AiRequest Request { get; }=request;
        public CancellationToken CancellationToken { get; }=cancellationToken;
        public TaskCompletionSource<HermesTerminalMessage> Completion { get; }=new(TaskCreationOptions.RunContinuationsAsynchronously);
    }
    private sealed record HermesTerminalMessage(string Text,string Reasoning,string Status,string Error);
}
