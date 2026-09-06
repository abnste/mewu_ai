using System.Text.Json;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using ScreenRecorderLib;
using Xunit;

namespace MewuAI.Tests;

public sealed class RecordingAudioTests
{
    [Fact]
    public void ExistingSettingsDefaultToComputerAudioWithoutOpeningMicrophone()
    {
        var settings=JsonSerializer.Deserialize<AppSettings>("{}")!;
        var options=RecordingAudioPolicy.Create(settings,()=>new LoopbackAudioSource("output"),
            ()=>throw new InvalidOperationException("Microphone must not be opened"));
        Assert.True(options.IsAudioEnabled);
        Assert.IsType<LoopbackAudioSource>(Assert.Single(options.AudioSources));
        Assert.Equal(1,options.AudioSources[0].Volume);
        Assert.False(settings.RecordMicrophone);
    }

    [Fact]
    public void ExplicitSilentRecordingDoesNotAccessEitherDevice()
    {
        var options=RecordingAudioPolicy.Create(new AppSettings{RecordSystemAudio=false},
            ()=>throw new InvalidOperationException(),()=>throw new InvalidOperationException());
        Assert.False(options.IsAudioEnabled);
        Assert.Empty(options.AudioSources);
    }

    [Fact]
    public void MixedSourcesHaveHeadroomAndRemainEnabled()
    {
        var options=RecordingAudioPolicy.Create(new AppSettings{RecordMicrophone=true},
            ()=>new LoopbackAudioSource("output"),()=>new CaptureAudioSource("input"));
        Assert.Equal(2,options.AudioSources.Count);
        Assert.All(options.AudioSources,source=>{Assert.True(source.IsAudioCaptureEnabled);Assert.Equal(.5f,source.Volume);});
    }

    [Theory]
    [InlineData(true,false)]
    [InlineData(false,true)]
    public void MissingRequestedDeviceDoesNotSilentlyProduceAVideoWithoutSound(bool computer,bool mic)
    {
        Assert.Throws<InvalidOperationException>(()=>RecordingAudioPolicy.Create(
            new AppSettings{RecordSystemAudio=computer,RecordMicrophone=mic},()=>null,()=>null));
    }

    [Theory]
    [InlineData(1,".mp4")]
    [InlineData(2,".mp3")]
    [InlineData(3,".gif")]
    public void ExportSelectionMapsToItsActualEncoding(int index,string extension)
        =>Assert.Equal(extension,VideoExportFormats.Extension(VideoExportFormats.FromFilterIndex(index)));
}
