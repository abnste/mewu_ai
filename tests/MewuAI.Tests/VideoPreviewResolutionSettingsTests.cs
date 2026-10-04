// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class VideoPreviewResolutionSettingsTests
{
    [Fact]
    public void MissingSettingsAndLegacySettingsKeepOriginalPreviewResolution()
    {
        using var fixture=new SettingsFixture();
        Assert.Equal(100,fixture.Service.Load().VideoPreviewResolutionPercent);
        Assert.False(File.Exists(fixture.Path));
        var legacy=JsonSerializer.SerializeToNode(CreateSettings(100))!.AsObject();
        Assert.True(legacy.Remove(nameof(AppSettings.VideoPreviewResolutionPercent)));
        File.WriteAllText(fixture.Path,legacy.ToJsonString(),new UTF8Encoding(false));
        var bytes=File.ReadAllBytes(fixture.Path);

        var loaded=fixture.Service.Load();

        Assert.Equal(100,loaded.VideoPreviewResolutionPercent);
        Assert.Empty(loaded.ConfigurationErrors);
        Assert.Equal(bytes,File.ReadAllBytes(fixture.Path));
    }

    [Theory]
    [InlineData(100)]
    [InlineData(75)]
    [InlineData(50)]
    public void PreviewChoiceRoundTripsWithoutChangingRecording(int percent)
    {
        using var fixture=new SettingsFixture();
        fixture.Service.Save(CreateSettings(percent));
        var loaded=fixture.Service.Load();

        Assert.Equal(percent,loaded.VideoPreviewResolutionPercent);
        Assert.Equal(60,loaded.RecordingFps);
        Assert.Equal(90,loaded.RecordingQuality);
        Assert.Equal(10,loaded.GifFps);
        Assert.True(loaded.RecordMicrophone);
        Assert.Empty(loaded.ConfigurationErrors);
        using var document=JsonDocument.Parse(File.ReadAllText(fixture.Path));
        Assert.Equal(percent,document.RootElement.GetProperty(nameof(AppSettings.VideoPreviewResolutionPercent)).GetInt32());
    }

    [Theory]
    [InlineData(-1)]
    [InlineData(0)]
    [InlineData(1)]
    [InlineData(49)]
    [InlineData(74)]
    [InlineData(99)]
    [InlineData(101)]
    [InlineData(int.MaxValue)]
    public void UnsupportedStoredChoiceFallsBackToOriginalWithoutRewritingFile(int percent)
    {
        using var fixture=new SettingsFixture();
        File.WriteAllText(fixture.Path,JsonSerializer.Serialize(CreateSettings(percent)),new UTF8Encoding(false));
        var bytes=File.ReadAllBytes(fixture.Path);

        var loaded=fixture.Service.Load();

        Assert.Equal(100,loaded.VideoPreviewResolutionPercent);
        Assert.Equal(60,loaded.RecordingFps);
        Assert.Equal(90,loaded.RecordingQuality);
        Assert.Empty(loaded.ConfigurationErrors);
        Assert.Equal(bytes,File.ReadAllBytes(fixture.Path));
    }

    [Theory]
    [InlineData(100)]
    [InlineData(75)]
    [InlineData(50)]
    public async Task SyntheticEnvironmentImportPreservesPreviewAndRecordingChoices(int percent)
    {
        using var fixture=new SettingsFixture();
        var current=CreateSettings(percent);
        fixture.Service.Save(current);
        var verified=false;
        var bootstrap=new EnvironmentProviderBootstrap(fixture.Credentials,
            name=>name=="MINIMAX_CN_API_KEY"?"synthetic-preview-regression":null,
            (candidate,token)=>
            {
                token.ThrowIfCancellationRequested();
                Assert.Equal(percent,candidate.VideoPreviewResolutionPercent);
                Assert.Equal(60,candidate.RecordingFps);
                Assert.Equal(90,candidate.RecordingQuality);
                verified=true;
                return Task.FromResult("synthetic-verification");
            });

        var result=await bootstrap.ImportAndCommitAsync(fixture.Service,current,true,TestContext.Current.CancellationToken);

        Assert.True(result.Changed);
        Assert.True(verified);
        Assert.Equal(percent,current.VideoPreviewResolutionPercent);
        var loaded=fixture.Service.Load();
        Assert.Equal(percent,loaded.VideoPreviewResolutionPercent);
        Assert.Equal(60,loaded.RecordingFps);
        Assert.Equal(90,loaded.RecordingQuality);
        Assert.Empty(loaded.ConfigurationErrors);
    }

    private static AppSettings CreateSettings(int percent)
    {
        var provider=new AiProviderSettings{Id="preview-local",Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="synthetic",AuthMode="none"};
        return new(){VideoPreviewResolutionPercent=percent,RecordingFps=60,RecordingQuality=90,GifFps=10,RecordMicrophone=true,
            Providers=[provider],DefaultProviderId=provider.Id};
    }

    private sealed class SettingsFixture:IDisposable
    {
        private readonly string _directory=System.IO.Path.Combine(System.IO.Path.GetTempPath(),"MewuAI-PreviewSettings-"+Guid.NewGuid().ToString("N"));
        internal string Path {get;}
        internal CredentialService Credentials {get;}
        internal SettingsService Service {get;}
        internal SettingsFixture()
        {
            Directory.CreateDirectory(_directory);
            File.WriteAllText(System.IO.Path.Combine(_directory,".mini-temp"),"synthetic preview settings regression",new UTF8Encoding(false));
            Path=System.IO.Path.Combine(_directory,"settings.json");
            Credentials=new CredentialService(System.IO.Path.Combine(_directory,"Credentials"));
            Service=new SettingsService(Path,new ProviderHeaderCredentialService(Credentials),null);
        }
        public void Dispose()=>Directory.Delete(_directory,true);
    }
}
