using mewu_ai_Assistant;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class HermesHostIntegrationTests
{
    [Fact]
    public void EnabledHermesBypassesInvalidRemoteProviderAndSharesConversationProvider()
    {
        using var runtime=new HermesRuntimeService();
        var settings=new AppSettings
        {
            HermesEnabled=true,
            HermesProfile="coder",
            HermesProvider="openrouter",
            HermesModel="test-model",
            HermesReasoningEffort="medium",
            Providers=[]
        };

        var text=AppHost.CreateConversationProviderCore(
            HermesConversationKind.Text,
            ()=>settings,
            runtime,
            new AiProviderFactory(null,null),
            out var textError);
        var screen=AppHost.CreateConversationProviderCore(
            HermesConversationKind.Screen,
            ()=>settings,
            runtime,
            new AiProviderFactory(null,null),
            out var screenError);

        Assert.Null(textError);
        Assert.Null(screenError);
        Assert.NotNull(text);
        Assert.Equal("hermes-local",text.Id);
        Assert.Same(text,screen);
    }

    [Fact]
    public void DisabledHermesUsesRemoteProviderValidation()
    {
        using var runtime=new HermesRuntimeService();
        var settings=new AppSettings{HermesEnabled=false,Providers=[]};

        var provider=AppHost.CreateConversationProviderCore(
            HermesConversationKind.Text,
            ()=>settings,
            runtime,
            new AiProviderFactory(null,null),
            out var error);

        Assert.Null(provider);
        Assert.Contains("尚未配置",error,StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowStatusShowsHermesModelAndReasoning()
    {
        var status=MainWindow.BuildAiStatusText(new AppSettings
        {
            HermesEnabled=true,
            HermesModel="MiniMax-M3",
            HermesReasoningEffort="high"
        });

        Assert.Equal("Hermes · default · MiniMax-M3 · 高度思考",status);
    }

    [Fact]
    public void MainWindowStatusUsesBackendSpecificLabels()
    {
        Assert.Equal("智能体已接入",MainWindow.BuildAiStatusTitle(new AppSettings{HermesEnabled=true},true));
        Assert.Equal("AI模型已接入",MainWindow.BuildAiStatusTitle(new AppSettings{HermesEnabled=false},true));
        Assert.Equal("暂未设置AI功能",MainWindow.BuildAiStatusTitle(new AppSettings{HermesEnabled=false},false));
    }

    [Fact]
    public void MainWindowStatusDoesNotRepeatEquivalentProviderNameAndModel()
    {
        var status=MainWindow.BuildAiStatusText(new AppSettings
        {
            DefaultProviderId="minimax",
            Providers=[new AiProviderSettings{Id="minimax",Name="MiniMax M3",Model="MiniMax-M3"}]
        });
        Assert.Equal("MiniMax M3",status);
    }

    [Fact]
    public async Task SpeechFileRemainsLeasedUntilDisposedAndAudioCanBeZeroed()
    {
        var root=TestDirectory();
        try
        {
            var registry=new TempMediaRegistry();
            var store=new HermesSpeechFileStore(new TempFileService(root,registry),registry);
            var bytes=new byte[]{0x52,0x49,0x46,0x46,0x01,0x02,0x03,0x04};
            var audio=new HermesSpeechAudio("audio/wav",".wav",bytes);
            var staged=await store.StageAsync(audio,TestContext.Current.CancellationToken);
            try
            {
                Assert.True(File.Exists(staged.Path));
                Assert.True(registry.IsLeased(staged.Path));
                Assert.Equal(bytes,await File.ReadAllBytesAsync(staged.Path,TestContext.Current.CancellationToken));
                audio.Dispose();
                Assert.All(bytes,value=>Assert.Equal(0,value));
            }
            finally{staged.Dispose();audio.Dispose();}

            Assert.False(File.Exists(staged.Path));
            Assert.Equal(0,registry.ActiveLeaseCount);
        }
        finally{if(Directory.Exists(root))Directory.Delete(root,true);}
    }

    [Fact]
    public async Task SpeechFileRejectsUnsafeExtensionBeforeCreatingOrLeasingFile()
    {
        var root=TestDirectory();
        try
        {
            var registry=new TempMediaRegistry();
            var store=new HermesSpeechFileStore(new TempFileService(root,registry),registry);
            using var audio=new HermesSpeechAudio("audio/wav","../escape.wav",[1,2,3]);

            await Assert.ThrowsAsync<ArgumentException>(()=>store.StageAsync(audio,TestContext.Current.CancellationToken));

            Assert.Empty(Directory.EnumerateFileSystemEntries(root));
            Assert.Equal(0,registry.ActiveLeaseCount);
        }
        finally{if(Directory.Exists(root))Directory.Delete(root,true);}
    }

    private static string TestDirectory()=>Path.Combine(Path.GetTempPath(),"MewuAI-HermesHostTests",Guid.NewGuid().ToString("N"));
}
