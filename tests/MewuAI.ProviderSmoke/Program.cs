using System.Text.Json;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;

var settingsService=new SettingsService();
var settings=settingsService.Load();
if(new EnvironmentProviderBootstrap().Import(settings))settingsService.Save(settings);
var requested=Environment.GetEnvironmentVariable("MEWU_SMOKE_PROVIDER");
if(!string.IsNullOrWhiteSpace(requested))settings.DefaultProviderId=settings.Providers.Last(x=>x.Name.Contains(requested,StringComparison.OrdinalIgnoreCase)).Id;
var videoPath=Environment.GetEnvironmentVariable("MEWU_SMOKE_VIDEO_PATH");
var recordingSeconds=int.TryParse(Environment.GetEnvironmentVariable("MEWU_SMOKE_RECORD_SECONDS"),out var seconds)&&seconds>0?Math.Clamp(seconds,2,20):0;
RecordingSession? recordingSession=null;
try
{
if(recordingSeconds>0)
{
    var bounds=System.Windows.Forms.SystemInformation.VirtualScreen;
    var recordRectJson=Environment.GetEnvironmentVariable("MEWU_SMOKE_RECORD_RECT_JSON");
    var recordRect=string.IsNullOrWhiteSpace(recordRectJson)?new ScreenRect(bounds.X,bounds.Y,bounds.Width,bounds.Height):JsonSerializer.Deserialize<ScreenRect>(recordRectJson);
    recordingSession=new RecordingSession(settings,recordRect);
    var recorded=new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
    recordingSession.Completed+=path=>recorded.TrySetResult(path);
    recordingSession.Failed+=error=>recorded.TrySetException(new InvalidOperationException(error));
    recordingSession.Start();
    await Task.Delay(TimeSpan.FromSeconds(recordingSeconds));
    recordingSession.Stop();
    videoPath=await recorded.Task.WaitAsync(TimeSpan.FromSeconds(30));
    await recordingSession.WaitFramesAsync();
}
var videoRequested=!string.IsNullOrWhiteSpace(videoPath);
var video=false;
var videoAnswer=string.Empty;
var expectedTermsJson=Environment.GetEnvironmentVariable("MEWU_SMOKE_VIDEO_EXPECTED_ANY_JSON");
var expectedTerms=string.IsNullOrWhiteSpace(expectedTermsJson)?Array.Empty<string>():JsonSerializer.Deserialize<string[]>(expectedTermsJson)??Array.Empty<string>();
var expectedAllJson=Environment.GetEnvironmentVariable("MEWU_SMOKE_VIDEO_EXPECTED_ALL_JSON");
var expectedAll=string.IsNullOrWhiteSpace(expectedAllJson)?Array.Empty<string>():JsonSerializer.Deserialize<string[]>(expectedAllJson)??Array.Empty<string>();
var videoSemanticMatch=expectedTerms.Length==0&&expectedAll.Length==0;
var recordedVideoBytes=recordingSession is null||string.IsNullOrWhiteSpace(videoPath)?0:new FileInfo(videoPath).Length;
if(videoRequested)
{
    var provider=new AiProviderFactory().Create(settings);
    if(provider?.Capabilities.SupportsVideo==true&&File.Exists(videoPath))
    {
        var result=await provider.SendAsync(new AiRequest{Prompt="请用一句中文说明这段视频中的主要角色和动作。",Attachments=[new AiAttachment(AiAttachmentType.Video,"video/mp4",FilePath:videoPath)]},CancellationToken.None);
        videoAnswer=result.Answer;
        videoSemanticMatch=(expectedTerms.Length==0||expectedTerms.Any(term=>videoAnswer.Contains(term,StringComparison.OrdinalIgnoreCase)))&&expectedAll.All(term=>videoAnswer.Contains(term,StringComparison.OrdinalIgnoreCase));
        video=!string.IsNullOrWhiteSpace(videoAnswer)&&videoSemanticMatch;
    }
}
var path=await new ProviderVerificationService().VerifyAsync(settings,CancellationToken.None);
using var report=JsonDocument.Parse(await File.ReadAllTextAsync(path));
var root=report.RootElement;
Console.WriteLine(JsonSerializer.Serialize(new{
    connection=root.GetProperty("connection").GetBoolean(),
    text=root.GetProperty("text").GetBoolean(),
    streaming=root.GetProperty("streaming").GetBoolean(),
    image=root.GetProperty("image").GetBoolean(),
    video,
    recordedVideo=recordingSession is not null,
    recordedVideoBytes,
    videoSemanticMatch,
    videoAnswer,
    errors=root.GetProperty("errors").EnumerateArray().Select(x=>x.GetString()).ToArray()
},new JsonSerializerOptions{WriteIndented=true}));
var exitCode=root.GetProperty("connection").GetBoolean()&&root.GetProperty("text").GetBoolean()&&root.GetProperty("streaming").GetBoolean()&&root.GetProperty("image").GetBoolean()&&(!videoRequested||video)?0:1;
return exitCode;
}
finally
{
    if(recordingSession is not null)
    {
        try{recordingSession.Dispose();}catch{}
        try{if(File.Exists(recordingSession.VideoPath))File.Delete(recordingSession.VideoPath);}catch{}
        try{if(Directory.Exists(recordingSession.FramesDirectory))Directory.Delete(recordingSession.FramesDirectory,true);}catch{}
    }
}
