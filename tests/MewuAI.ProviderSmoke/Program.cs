using System.Text.Json;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

var settingsService=new SettingsService();
var settings=settingsService.Load();
if(new EnvironmentProviderBootstrap().Import(settings))settingsService.Save(settings);
var requested=Environment.GetEnvironmentVariable("MEWU_SMOKE_PROVIDER");
if(!string.IsNullOrWhiteSpace(requested))settings.DefaultProviderId=settings.Providers.Last(x=>x.Name.Contains(requested,StringComparison.OrdinalIgnoreCase)).Id;
var path=await new ProviderVerificationService().VerifyAsync(settings,CancellationToken.None);
var videoPath=Environment.GetEnvironmentVariable("MEWU_SMOKE_VIDEO_PATH");
var videoRequested=!string.IsNullOrWhiteSpace(videoPath);
var video=false;
var videoAnswer=string.Empty;
if(videoRequested)
{
    var provider=new AiProviderFactory().Create(settings);
    if(provider?.Capabilities.SupportsVideo==true&&File.Exists(videoPath))
    {
        var result=await provider.SendAsync(new AiRequest{Prompt="请用一句中文说明这段视频中的主要角色和动作。",Attachments=[new AiAttachment(AiAttachmentType.Video,"video/mp4",FilePath:videoPath)]},CancellationToken.None);
        videoAnswer=result.Answer;
        video=!string.IsNullOrWhiteSpace(videoAnswer);
    }
}
using var report=JsonDocument.Parse(await File.ReadAllTextAsync(path));
var root=report.RootElement;
Console.WriteLine(JsonSerializer.Serialize(new{
    connection=root.GetProperty("connection").GetBoolean(),
    text=root.GetProperty("text").GetBoolean(),
    streaming=root.GetProperty("streaming").GetBoolean(),
    image=root.GetProperty("image").GetBoolean(),
    video,
    videoAnswer,
    errors=root.GetProperty("errors").EnumerateArray().Select(x=>x.GetString()).ToArray()
},new JsonSerializerOptions{WriteIndented=true}));
return root.GetProperty("connection").GetBoolean()&&root.GetProperty("text").GetBoolean()&&root.GetProperty("streaming").GetBoolean()&&root.GetProperty("image").GetBoolean()&&(!videoRequested||video)?0:1;
