using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Speech;
using System.Globalization;
using System.Text.Json;
using System.Runtime.InteropServices;
using Xunit;
namespace MewuAI.Tests;
public sealed class ServicesTests
{
    [Fact] public void CredentialService_RoundTripsWithCurrentUserDpapi(){var root=TestDirectory();try{var service=new CredentialService(root);service.Save("credential","secret-value");Assert.Equal("secret-value",service.Read("credential"));service.Delete("credential");Assert.Null(service.Read("credential"));}finally{Directory.Delete(root,true);}}
    [Fact] public void CredentialService_RejectsPathTraversalIdentifiers(){var root=TestDirectory();try{var service=new CredentialService(root);Assert.Throws<ArgumentException>(()=>service.Save("..\\outside","secret"));Assert.False(File.Exists(Path.Combine(Directory.GetParent(root)!.FullName,"outside.bin")));}finally{Directory.Delete(root,true);}}
    [Fact] public void SettingsService_RoundTripsProvidersAndEnums(){var root=TestDirectory();try{var path=Path.Combine(root,"settings.json");var settings=new AppSettings{CaptureDelaySeconds=5,IncludeCaptureCursor=true,VoiceLanguage="zh-CN",CaptureHotkey=new(){Key=System.Windows.Input.Key.Z,Modifiers=System.Windows.Input.ModifierKeys.Control|System.Windows.Input.ModifierKeys.Alt},Providers=[new(){Name="私有模型",Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model-a",CustomHeaders=new(){{"X-Tenant","demo"}}}]};settings.DefaultProviderId=settings.Providers[0].Id;var service=new SettingsService(path);service.Save(settings);var loaded=service.Load();Assert.Equal(5,loaded.CaptureDelaySeconds);Assert.Equal(System.Windows.Input.Key.Z,loaded.CaptureHotkey.Key);Assert.Equal("model-a",loaded.Providers.Single().Model);Assert.Equal("demo",loaded.Providers.Single().CustomHeaders["X-Tenant"]);using var document=JsonDocument.Parse(File.ReadAllText(path));Assert.False(document.RootElement.ToString().Contains("secret",StringComparison.OrdinalIgnoreCase));}finally{Directory.Delete(root,true);}}
    [Fact] public void SettingsService_NormalizesUnsafeNumericValues(){var root=TestDirectory();try{var path=Path.Combine(root,"settings.json");File.WriteAllText(path,"{\"RecordingFps\":999,\"RecordingQuality\":-1,\"GifFps\":0,\"TempCleanupDays\":999,\"OverlayOpacity\":9}");var loaded=new SettingsService(path).Load();Assert.Equal(60,loaded.RecordingFps);Assert.Equal(20,loaded.RecordingQuality);Assert.Equal(1,loaded.GifFps);Assert.Equal(30,loaded.TempCleanupDays);Assert.Equal(.75,loaded.OverlayOpacity);}finally{Directory.Delete(root,true);}}
    [Fact] public void TempFileService_CleansOnlyExpiredEntries(){var root=TestDirectory();try{var service=new TempFileService(root);var old=Path.Combine(root,"old.tmp");var fresh=Path.Combine(root,"fresh.tmp");File.WriteAllText(old,"old");File.WriteAllText(fresh,"fresh");File.SetLastWriteTimeUtc(old,DateTime.UtcNow-TimeSpan.FromDays(5));service.Cleanup(TimeSpan.FromDays(3));Assert.False(File.Exists(old));Assert.True(File.Exists(fresh));}finally{Directory.Delete(root,true);}}
    [Fact] public void PrivacyLogger_RedactsAuthorizationAndApiKeys(){var root=TestDirectory();try{new PrivacyLogger(root).Error("AI",new InvalidOperationException("Authorization: Bearer token-123 api_key=secret-456"));var text=File.ReadAllText(Directory.GetFiles(root,"*.log").Single());Assert.DoesNotContain("token-123",text);Assert.DoesNotContain("secret-456",text);Assert.Contains("[REDACTED]",text);}finally{Directory.Delete(root,true);}}
    [Fact] public void MiniMaxM3Provider_EnablesNativeImageAndVideoUnderstanding(){var provider=new MiniMaxProvider(new AiProviderSettings{Type="MiniMax",BaseUrl="https://api.minimax.io/v1",Model="MiniMax-M3"},"unused");Assert.True(provider.Capabilities.SupportsImage);Assert.True(provider.Capabilities.SupportsVideo);Assert.Contains("image/png",provider.Capabilities.AcceptedMimeTypes);Assert.Contains("video/mp4",provider.Capabilities.AcceptedMimeTypes);Assert.Equal(50L*1024*1024,provider.Capabilities.MaxAttachmentSize);}
    [Fact] public void OlderMiniMaxModel_DoesNotClaimM3MultimodalProtocol(){var provider=new MiniMaxProvider(new AiProviderSettings{Type="MiniMax",BaseUrl="https://api.minimax.io/v1",Model="MiniMax-M2.7"},"unused");Assert.False(provider.Capabilities.SupportsImage);Assert.False(provider.Capabilities.SupportsVideo);}
    [Fact] public void OpenAiProvider_DeclaresSupportedImageMimeTypes(){var provider=new OpenAiCompatibleProvider(new AiProviderSettings(),"unused");Assert.Contains("image/png",provider.Capabilities.AcceptedMimeTypes);Assert.False(provider.Capabilities.SupportsVideo);}
    [Fact] public async Task GenericOpenAiProvider_RejectsVideoBeforeNetwork(){var provider=new OpenAiCompatibleProvider(new AiProviderSettings(),"unused");await Assert.ThrowsAsync<NotSupportedException>(()=>provider.SendAsync(new AiRequest{Attachments=[new(AiAttachmentType.Video,"video/mp4",[1,2,3])]},TestContext.Current.CancellationToken));}
    [Fact] public async Task OpenAiProvider_RejectsUnsupportedMimeBeforeNetwork(){var provider=new OpenAiCompatibleProvider(new AiProviderSettings(),"unused");await Assert.ThrowsAsync<NotSupportedException>(()=>provider.SendAsync(new AiRequest{Attachments=[new(AiAttachmentType.Image,"image/bmp",[1,2,3])]},TestContext.Current.CancellationToken));}
    [Fact] public void SpeechFailureMapper_MapsDesktopRecognizerAndAudioFailures(){Assert.Equal("当前语言缺少可用的语音识别器",SpeechRecognitionFailureMapper.FromException(new COMException("missing",SpeechRecognitionFailureMapper.RecognizerNotFound),SpeechRecognitionFailureContext.RecognizerInitialization));Assert.Equal("未检测到可用麦克风",SpeechRecognitionFailureMapper.FromException(new COMException("no device",SpeechRecognitionFailureMapper.AudioDeviceNotFound),SpeechRecognitionFailureContext.AudioInput));Assert.Equal("麦克风权限未开启，无法使用语音输入",SpeechRecognitionFailureMapper.FromException(new UnauthorizedAccessException(),SpeechRecognitionFailureContext.AudioInput));Assert.Equal("麦克风正被其他应用占用，请稍后重试",SpeechRecognitionFailureMapper.FromException(new COMException("busy",SpeechRecognitionFailureMapper.DeviceBusy),SpeechRecognitionFailureContext.AudioInput));Assert.Equal("没有听到语音，请重试",SpeechRecognitionFailureMapper.FromException(new COMException("timeout",SpeechRecognitionFailureMapper.RecognitionTimeout),SpeechRecognitionFailureContext.Recognition));}
    [Fact] public void SpeechLanguageSelector_PrefersExactAndCompatibleInstalledRecognizers(){CultureInfo[] installed=[CultureInfo.GetCultureInfo("en-GB"),CultureInfo.GetCultureInfo("zh-CN")];Assert.Equal("zh-CN",SpeechRecognizerLanguageSelector.SelectBestCulture(" zh-CN ",installed,CultureInfo.GetCultureInfo("en-US"))?.Name);Assert.Equal("en-GB",SpeechRecognizerLanguageSelector.SelectBestCulture("en-US",installed,CultureInfo.GetCultureInfo("zh-CN"))?.Name);Assert.Null(SpeechRecognizerLanguageSelector.SelectBestCulture("ja-JP",installed,CultureInfo.GetCultureInfo("zh-CN")));}
    [Fact] public void SpeechLanguageSelector_SystemUsesWindowsCultureThenSafeInstalledFallback(){CultureInfo[] installed=[CultureInfo.GetCultureInfo("en-US"),CultureInfo.GetCultureInfo("zh-CN")];Assert.Equal("zh-CN",SpeechRecognizerLanguageSelector.SelectBestCulture("system",installed,CultureInfo.GetCultureInfo("zh-CN"))?.Name);Assert.Equal("en-US",SpeechRecognizerLanguageSelector.SelectBestCulture("system",installed,CultureInfo.GetCultureInfo("ja-JP"))?.Name);CultureInfo[] familyFirst=[CultureInfo.GetCultureInfo("zh-CN"),CultureInfo.GetCultureInfo("en-GB")];Assert.Equal("en-GB",SpeechRecognizerLanguageSelector.SelectBestCulture("system",familyFirst,CultureInfo.GetCultureInfo("en-US"),CultureInfo.GetCultureInfo("zh-CN"))?.Name);Assert.Null(SpeechRecognizerLanguageSelector.SelectBestCulture("system",Array.Empty<CultureInfo>(),CultureInfo.GetCultureInfo("zh-CN")));}
    [Fact] public async Task SpeechService_PreCanceledRequestDoesNotInitializeDesktopRecognizer(){using var cancellation=new CancellationTokenSource();cancellation.Cancel();await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>new WindowsSpeechToTextService().RecognizeOnceAsync("system",cancellation.Token));}
    [Fact]
    public async Task SpeechService_ReturnsTaskBeforeSynchronousRecognizerInitializationCompletes()
    {
        var testCancellation = TestContext.Current.CancellationToken;
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        using var returned = new ManualResetEventSlim();
        var callerThread = 0;
        var coreThread = 0;
        Task<string?>? recognition = null;
        Exception? callerError = null;
        var service = new WindowsSpeechToTextService((_, _) =>
        {
            coreThread = Environment.CurrentManagedThreadId;
            entered.Set();
            release.Wait(testCancellation);
            return Task.FromResult<string?>("完成");
        });
        var caller = new Thread(() =>
        {
            try
            {
                callerThread = Environment.CurrentManagedThreadId;
                recognition = service.RecognizeOnceAsync("system", testCancellation);
                returned.Set();
            }
            catch (Exception ex)
            {
                callerError = ex;
                returned.Set();
            }
        });
        caller.SetApartmentState(ApartmentState.STA);
        caller.Start();

        try
        {
            Assert.True(entered.Wait(TimeSpan.FromSeconds(2), testCancellation), "后台识别核心没有启动");
            Assert.True(returned.Wait(TimeSpan.FromMilliseconds(500), testCancellation), "公开方法被同步识别初始化阻塞");
            Assert.NotEqual(callerThread, coreThread);
        }
        finally
        {
            release.Set();
            Assert.True(caller.Join(TimeSpan.FromSeconds(2)), "调用线程没有正常结束");
        }

        Assert.Null(callerError);
        Assert.Equal(
            "完成",
            await (recognition ?? throw new InvalidOperationException("未返回识别任务"))
                .WaitAsync(TimeSpan.FromSeconds(2), testCancellation));
    }
    private static string TestDirectory(){var path=Path.Combine(Path.GetTempPath(),"MewuAI.Tests",Guid.NewGuid().ToString("N"));Directory.CreateDirectory(path);return path;}
}
