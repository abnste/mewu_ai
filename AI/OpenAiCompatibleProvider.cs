using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.AI;

public class OpenAiCompatibleProvider : IAiProvider
{
    private static readonly HttpClient Client=new(){Timeout=TimeSpan.FromMinutes(5)};
    private readonly AiProviderSettings _settings;
    private readonly string _apiKey;

    public string Id=>_settings.Id;
    public virtual AiProviderCapabilities Capabilities { get; }=new(true,true,false,true,true,20*1024*1024,TimeSpan.Zero,new HashSet<string>{"image/png","image/jpeg","image/webp"});

    public OpenAiCompatibleProvider(AiProviderSettings settings,string apiKey){_settings=settings;_apiKey=apiKey;}

    public async Task<bool> TestConnectionAsync(CancellationToken token)
    {
        var result=await SendAsync(new AiRequest{Prompt="Reply with OK."},token);
        return !string.IsNullOrWhiteSpace(result.Answer);
    }

    public virtual async Task<AiResult> SendAsync(AiRequest request,CancellationToken token)
    {
        Validate(request);
        var content=new List<object>();
        if(!string.IsNullOrWhiteSpace(request.Prompt))content.Add(new{type="text",text=request.Prompt});
        foreach(var attachment in request.Attachments.Where(x=>x.Type==AiAttachmentType.Image))
        {
            var bytes=attachment.Data??await File.ReadAllBytesAsync(attachment.FilePath!,token);
            content.Add(new{type="image_url",image_url=new{url=$"data:{attachment.MimeType};base64,{Convert.ToBase64String(bytes)}"}});
        }

        var messages=request.History.Select(x=>(object)new{role=x.Role,content=x.Text}).ToList();
        messages.Add(new{role="user",content});
        var streaming=request.StreamingProgress is not null&&Capabilities.SupportsStreaming;
        var bodyValues=new Dictionary<string,object?>{{"model",_settings.Model},{"messages",messages},{"temperature",.2},{"stream",streaming}};
        if(request.MaxOutputTokens is { } maxTokens)bodyValues["max_tokens"]=maxTokens;
        if(request.DisableReasoning&&_settings.BaseUrl.Contains("volces.com",StringComparison.OrdinalIgnoreCase)){bodyValues["thinking"]=new{type="disabled"};bodyValues["reasoning_effort"]="minimal";}
        var body=JsonSerializer.Serialize(bodyValues);
        using var httpRequest=Create(HttpMethod.Post,"chat/completions");
        httpRequest.Content=new StringContent(body,Encoding.UTF8,"application/json");
        using var response=await Client.SendAsync(httpRequest,HttpCompletionOption.ResponseHeadersRead,token);
        if(!response.IsSuccessStatusCode)throw new InvalidOperationException($"AI 请求失败（HTTP {(int)response.StatusCode}）");

        if(streaming)
        {
            using var responseStream=await response.Content.ReadAsStreamAsync(token);
            using var reader=new StreamReader(responseStream);
            var answer=new StringBuilder();var reasoning=new StringBuilder();
            while(await reader.ReadLineAsync(token) is { } line)
            {
                if(!StreamingResponseParser.TryParse(line,out var delta,out var done))continue;
                if(done)break;
                if(delta.Content.Length>0)answer.Append(delta.Content);
                if(delta.ReasoningContent.Length>0)reasoning.Append(delta.ReasoningContent);
                if(delta.Content.Length>0||delta.ReasoningContent.Length>0)request.StreamingProgress!.Report(delta);
                if(request.StreamingCompletionPredicate?.Invoke(answer.ToString())==true)break;
            }
            return StructuredResponseParser.Parse(answer.ToString(),reasoning.ToString());
        }

        var json=await response.Content.ReadAsStringAsync(token);
        using var document=JsonDocument.Parse(json);
        var message=document.RootElement.GetProperty("choices")[0].GetProperty("message");
        var answerText=ReadString(message,"content");
        var reasoningText=ReadString(message,"reasoning_content");
        if(reasoningText.Length==0)reasoningText=ReadString(message,"thinking_content");
        return StructuredResponseParser.Parse(answerText,reasoningText);
    }

    protected HttpRequestMessage Create(HttpMethod method,string relative)
    {
        var request=new HttpRequestMessage(method,$"{_settings.BaseUrl.TrimEnd('/')}/{relative}");
        request.Headers.Authorization=new AuthenticationHeaderValue("Bearer",_apiKey);
        foreach(var header in _settings.CustomHeaders)request.Headers.TryAddWithoutValidation(header.Key,header.Value);
        return request;
    }

    private static string ReadString(JsonElement value,string name)=>value.TryGetProperty(name,out var property)&&property.ValueKind==JsonValueKind.String?property.GetString()??string.Empty:string.Empty;

    private void Validate(AiRequest request)
    {
        foreach(var attachment in request.Attachments)
        {
            if(attachment.Type==AiAttachmentType.Image&&!Capabilities.SupportsImage)throw new NotSupportedException("当前模型不支持图片");
            if(attachment.Type==AiAttachmentType.Video&&!Capabilities.SupportsVideo)throw new NotSupportedException("当前模型不支持视频");
            if(Capabilities.AcceptedMimeTypes.Count>0&&!Capabilities.AcceptedMimeTypes.Contains(attachment.MimeType))throw new NotSupportedException($"当前模型不接受 {attachment.MimeType} 附件");
            var size=attachment.Data?.LongLength??(attachment.FilePath is null?0:new FileInfo(attachment.FilePath).Length);
            if(size>Capabilities.MaxAttachmentSize)throw new InvalidOperationException("附件超过当前模型限制");
            if(attachment.Duration is { } duration&&Capabilities.MaxVideoDuration>TimeSpan.Zero&&duration>Capabilities.MaxVideoDuration)throw new InvalidOperationException("视频时长超过当前模型限制");
        }
    }
}
