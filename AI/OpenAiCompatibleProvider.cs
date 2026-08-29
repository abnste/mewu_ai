using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.AI;
public class OpenAiCompatibleProvider : IAiProvider
{
    private static readonly HttpClient Client=new(){Timeout=TimeSpan.FromMinutes(5)};private readonly AiProviderSettings _settings;private readonly string _apiKey;
    public string Id=>_settings.Id;
    public virtual AiProviderCapabilities Capabilities { get; }=new(true,true,false,false,true,20*1024*1024,TimeSpan.Zero,new HashSet<string>{"image/png","image/jpeg","image/webp"});
    public OpenAiCompatibleProvider(AiProviderSettings settings,string apiKey){_settings=settings;_apiKey=apiKey;}
    public async Task<bool> TestConnectionAsync(CancellationToken token){using var req=Create(HttpMethod.Get,"models");using var res=await Client.SendAsync(req,token);return res.IsSuccessStatusCode;}
    public virtual async Task<AiResult> SendAsync(AiRequest request,CancellationToken token)
    {
        Validate(request);var content=new List<object>();if(!string.IsNullOrWhiteSpace(request.Prompt))content.Add(new{type="text",text=request.Prompt});
        foreach(var a in request.Attachments.Where(x=>x.Type==AiAttachmentType.Image)){var bytes=a.Data??await File.ReadAllBytesAsync(a.FilePath!,token);content.Add(new{type="image_url",image_url=new{url=$"data:{a.MimeType};base64,{Convert.ToBase64String(bytes)}"}});}
        var messages=request.History.Select(x=>(object)new{role=x.Role,content=x.Text}).ToList();messages.Add(new{role="user",content});
        var body=JsonSerializer.Serialize(new{model=_settings.Model,messages,temperature=.2});using var req=Create(HttpMethod.Post,"chat/completions");req.Content=new StringContent(body,Encoding.UTF8,"application/json");using var res=await Client.SendAsync(req,HttpCompletionOption.ResponseHeadersRead,token);var json=await res.Content.ReadAsStringAsync(token);if(!res.IsSuccessStatusCode)throw new InvalidOperationException($"AI 请求失败（HTTP {(int)res.StatusCode}）");
        using var doc=JsonDocument.Parse(json);var answer=doc.RootElement.GetProperty("choices")[0].GetProperty("message").GetProperty("content").GetString()??string.Empty;return StructuredResponseParser.Parse(answer);
    }
    protected HttpRequestMessage Create(HttpMethod method,string relative){var r=new HttpRequestMessage(method,$"{_settings.BaseUrl.TrimEnd('/')}/{relative}");r.Headers.Authorization=new AuthenticationHeaderValue("Bearer",_apiKey);foreach(var h in _settings.CustomHeaders)r.Headers.TryAddWithoutValidation(h.Key,h.Value);return r;}
    private void Validate(AiRequest r){foreach(var a in r.Attachments){if(a.Type==AiAttachmentType.Image&&!Capabilities.SupportsImage)throw new NotSupportedException("当前模型不支持图片");if(a.Type==AiAttachmentType.Video&&!Capabilities.SupportsVideo)throw new NotSupportedException("当前模型不支持视频");var size=a.Data?.LongLength??(a.FilePath is null?0:new FileInfo(a.FilePath).Length);if(size>Capabilities.MaxAttachmentSize)throw new InvalidOperationException("附件超过当前模型限制");}}
}
