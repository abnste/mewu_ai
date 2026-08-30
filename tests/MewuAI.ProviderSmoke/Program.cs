using System.Text.Json;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;
using MewuAI.ProviderSmoke;

using var singleInstance=new SingleInstanceService();
if(!singleInstance.IsPrimary)
{
    Console.Error.WriteLine("喵呜AI 主程序或另一个 ProviderSmoke 正在运行；为避免竞争设置与凭据，本次验收已停止。");
    return 2;
}
string? requested,videoPath,recordRectJson;int recordingSeconds;string[] expectedTerms,expectedAll;bool requireFrameChanges,videoConfigurationPresent;
try
{
    requested=Environment.GetEnvironmentVariable("MEWU_SMOKE_PROVIDER");
    videoPath=Environment.GetEnvironmentVariable("MEWU_SMOKE_VIDEO_PATH");
    var recordingSecondsValue=Environment.GetEnvironmentVariable("MEWU_SMOKE_RECORD_SECONDS");
    recordRectJson=Environment.GetEnvironmentVariable("MEWU_SMOKE_RECORD_RECT_JSON");
    var expectedTermsJson=Environment.GetEnvironmentVariable("MEWU_SMOKE_VIDEO_EXPECTED_ANY_JSON");
    var expectedAllJson=Environment.GetEnvironmentVariable("MEWU_SMOKE_VIDEO_EXPECTED_ALL_JSON");
    var requireFrameChangesValue=Environment.GetEnvironmentVariable("MEWU_SMOKE_VIDEO_REQUIRE_FRAME_CHANGES");
    videoConfigurationPresent=new[]{videoPath,recordingSecondsValue,recordRectJson,expectedTermsJson,expectedAllJson,requireFrameChangesValue}.Any(value=>value is not null);
    recordingSeconds=ParseRecordingSeconds(recordingSecondsValue);
    expectedTerms=ParseExpectedTerms(expectedTermsJson);
    expectedAll=ParseExpectedTerms(expectedAllJson);
    requireFrameChanges=ParseOptionalBoolean(requireFrameChangesValue,true,"MEWU_SMOKE_VIDEO_REQUIRE_FRAME_CHANGES");
}
catch(Exception ex)when(ex is InvalidOperationException or JsonException)
{
    Console.Error.WriteLine($"ProviderSmoke 配置无效：{ex.Message}");
    return 2;
}
var semanticExpectationConfigured=expectedTerms.Length>0||expectedAll.Length>0;
var videoSourceConfigured=recordingSeconds>0||!string.IsNullOrWhiteSpace(videoPath);
if(videoConfigurationPresent&&!videoSourceConfigured)
{
    Console.Error.WriteLine("ProviderSmoke 配置无效：设置了视频验收参数，但未提供有效的 MEWU_SMOKE_VIDEO_PATH 或 MEWU_SMOKE_RECORD_SECONDS");
    return 2;
}
if(videoSourceConfigured&&!semanticExpectationConfigured)
{
    Console.Error.WriteLine("ProviderSmoke 配置无效：视频验收必须配置至少一个非空预期语义词");
    return 2;
}
var settingsService=new SettingsService();
var settings=settingsService.Load();
var changedRequestedProvider=false;
RecordingSession? recordingSession=null;
try
{
if(recordingSeconds>0)
{
    var bounds=System.Windows.Forms.SystemInformation.VirtualScreen;
    var recordRect=string.IsNullOrWhiteSpace(recordRectJson)?new ScreenRect(bounds.X,bounds.Y,bounds.Width,bounds.Height):JsonSerializer.Deserialize<ScreenRect>(recordRectJson);
    recordingSession=new RecordingSession(settings,recordRect);
    var recorded=new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
    recordingSession.Completed+=path=>recorded.TrySetResult(path);
    recordingSession.Failed+=error=>recorded.TrySetException(new InvalidOperationException(error));
    recordingSession.Start();
    await Task.Delay(TimeSpan.FromSeconds(recordingSeconds));
    recordingSession.Stop();
    videoPath=await recorded.Task.WaitAsync(TimeSpan.FromSeconds(30));
}
var bootstrapResult=await new EnvironmentProviderBootstrap().ImportAndCommitAsync(settingsService,true,CancellationToken.None);
settings=settingsService.Load();
if(!string.IsNullOrWhiteSpace(requested))
{
    var requestedId=settings.Providers.Last(x=>x.Name.Contains(requested,StringComparison.OrdinalIgnoreCase)).Id;
    changedRequestedProvider=settings.DefaultProviderId!=requestedId;settings.DefaultProviderId=requestedId;
}
var videoRequested=!string.IsNullOrWhiteSpace(videoPath);
var video=false;
var videoAnswer=string.Empty;
var videoSemanticMatch=false;
var videoStreamingUsed=false;
var videoStreamDeltaCount=0;
var recordedVideoBytes=recordingSession is null||string.IsNullOrWhiteSpace(videoPath)||!File.Exists(videoPath)?0:new FileInfo(videoPath).Length;
string? videoSourceHashBefore=null,videoSourceHashAfter=null,videoError=null,videoQaError=null;
VideoQaEvidence? videoEvidence=null;
if(videoRequested&&File.Exists(videoPath))
{
    try{videoSourceHashBefore=await VideoQaSampler.HashFileAsync(videoPath!,CancellationToken.None);}
    catch(Exception ex){videoQaError=$"源视频发送前哈希失败：{ex.Message}";}
    try{videoEvidence=await VideoQaSampler.CaptureAsync(videoPath!,CancellationToken.None);}
    catch(Exception ex){videoQaError=string.IsNullOrWhiteSpace(videoQaError)?$"验收帧抽取失败：{ex.Message}":$"{videoQaError}；验收帧抽取失败：{ex.Message}";}
}
else if(videoRequested)videoQaError="指定的视频文件不存在";
if(videoRequested)
{
    try
    {
        var provider=new AiProviderFactory().Create(settings);
        if(!semanticExpectationConfigured)videoError="视频验收必须配置至少一个预期语义词，避免任意非空回答误判通过";
        else if(provider is null)videoError="没有可用的 AI Provider";
        else if(!provider.Capabilities.SupportsVideo)videoError="当前 Provider 不支持视频理解";
        else if(!File.Exists(videoPath))videoError="指定的视频文件不存在";
        else
        {
            videoStreamingUsed=provider.Capabilities.SupportsStreaming;
            var progress=videoStreamingUsed?new InlineProgress<AiStreamDelta>(_=>Interlocked.Increment(ref videoStreamDeltaCount)):null;
            var result=await provider.SendAsync(new AiRequest{Prompt="请用一句中文说明这段视频中的主要角色和动作。",Attachments=[new AiAttachment(AiAttachmentType.Video,"video/mp4",FilePath:videoPath,Duration:videoEvidence?.Duration)],StreamingProgress=progress},CancellationToken.None);
            videoAnswer=result.Answer;
            videoSemanticMatch=(expectedTerms.Length==0||expectedTerms.Any(term=>videoAnswer.Contains(term,StringComparison.OrdinalIgnoreCase)))&&expectedAll.All(term=>videoAnswer.Contains(term,StringComparison.OrdinalIgnoreCase));
            video=!string.IsNullOrWhiteSpace(videoAnswer)&&videoSemanticMatch;
        }
    }
    catch(Exception ex){videoError=ex.Message;}
    finally
    {
        if(File.Exists(videoPath))try{videoSourceHashAfter=await VideoQaSampler.HashFileAsync(videoPath!,CancellationToken.None);}catch(Exception ex){videoQaError=string.IsNullOrWhiteSpace(videoQaError)?$"源视频发送后哈希失败：{ex.Message}":$"{videoQaError}；源视频发送后哈希失败：{ex.Message}";}
    }
}
var videoSourceHashUnchanged=!videoRequested||videoSourceHashBefore is not null&&string.Equals(videoSourceHashBefore,videoSourceHashAfter,StringComparison.Ordinal);
var videoFrameDifferences=videoEvidence is null?Array.Empty<VideoFrameDifference>():VideoQaSampler.CompareFrames(videoEvidence.Samples).ToArray();
var videoFramesChanged=!videoRequested||!requireFrameChanges||videoFrameDifferences.Any(difference=>difference.Meaningful);
var videoDurationReasonable=!videoRequested||videoEvidence is not null&&videoEvidence.Duration>TimeSpan.Zero&&(recordingSeconds==0||videoEvidence.Duration>=TimeSpan.FromSeconds(Math.Max(1,recordingSeconds-2))&&videoEvidence.Duration<=TimeSpan.FromSeconds(recordingSeconds+5));
var videoFrameSamplesReady=!videoRequested||videoEvidence?.Samples.Count==3&&videoEvidence.Samples.All(sample=>sample.Bytes>0&&File.Exists(sample.Path))&&videoFramesChanged;
var videoStreamingCompleted=!videoRequested||!videoStreamingUsed||videoStreamDeltaCount>0;
var path=!changedRequestedProvider&&bootstrapResult.VerificationReportPath is not null
    ?bootstrapResult.VerificationReportPath
    :await new ProviderVerificationService().VerifyAsync(settings,CancellationToken.None);
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
    semanticExpectationConfigured,
    videoSemanticMatch,
    videoStreamingUsed,
    videoStreamDeltaCount,
    videoStreamingCompleted,
    videoAnswer,
    videoError,
    videoSourceHashBefore,
    videoSourceHashAfter,
    videoSourceHashUnchanged,
    videoFramesChanged,
    videoDurationReasonable,
    videoFrameSamplesReady,
    videoFrameSampleDirectory=videoEvidence?.Directory,
    videoDurationMs=videoEvidence?.Duration.TotalMilliseconds,
    videoFrameHashBasis="BGRA8 pixels",
    videoFrameDifferenceThreshold=new{cellLumaDelta=12,minimumChangedCellRatio=.03},
    videoFrameDifferences=videoFrameDifferences.Select(difference=>new{first=difference.First,second=difference.Second,changedCellRatio=difference.ChangedCellRatio,meanAbsoluteLumaDelta=difference.MeanAbsoluteLumaDelta,meaningful=difference.Meaningful}).ToArray(),
    videoFrameSamples=videoEvidence?.Samples.Select(sample=>new{label=sample.Label,timestampMs=sample.Timestamp.TotalMilliseconds,path=sample.Path,bytes=sample.Bytes,sha256=sample.Sha256,contentType=sample.ContentType}).ToArray(),
    videoQaError,
    errors=root.GetProperty("errors").EnumerateArray().Select(x=>x.GetString()).ToArray()
},new JsonSerializerOptions{WriteIndented=true}));
var exitCode=root.GetProperty("connection").GetBoolean()&&root.GetProperty("text").GetBoolean()&&root.GetProperty("streaming").GetBoolean()&&root.GetProperty("image").GetBoolean()&&(!videoRequested||video&&videoSourceHashUnchanged&&videoFrameSamplesReady&&videoDurationReasonable&&videoStreamingCompleted)?0:1;
return exitCode;
}

finally
{
    if(recordingSession is not null)
    {
        try{await recordingSession.DisposeAsync();}catch{}
        try{if(File.Exists(recordingSession.VideoPath))File.Delete(recordingSession.VideoPath);}catch{}
    }
}

static int ParseRecordingSeconds(string? value)
{
    if(value is null)return 0;
    if(!int.TryParse(value,out var seconds)||seconds is <2 or >20)throw new InvalidOperationException("MEWU_SMOKE_RECORD_SECONDS 必须是 2–20 的整数");
    return seconds;
}

static string[] ParseExpectedTerms(string? json)
{
    if(string.IsNullOrWhiteSpace(json))return [];
    return (JsonSerializer.Deserialize<string[]>(json)??[])
        .Where(term=>!string.IsNullOrWhiteSpace(term))
        .Select(term=>term.Trim())
        .Distinct(StringComparer.OrdinalIgnoreCase)
        .ToArray();
}

static bool ParseOptionalBoolean(string? value,bool defaultValue,string variable)
{
    if(value is null)return defaultValue;
    if(!bool.TryParse(value,out var parsed))throw new InvalidOperationException($"{variable} 必须是 true 或 false");
    return parsed;
}

file sealed class InlineProgress<T>(Action<T> handler):IProgress<T>
{
    public void Report(T value)=>handler(value);
}
