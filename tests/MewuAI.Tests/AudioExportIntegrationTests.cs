using System.Security.Cryptography;
using System.Text;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using Windows.Media.Editing;
using Windows.Media.MediaProperties;
using Windows.Media.Transcoding;
using Windows.Storage;
using Xunit;

namespace MewuAI.Tests;

public sealed class AudioExportIntegrationTests
{
    [Fact]
    public async Task Mp3AndAnnotatedMp4KeepAudibleSamplesAndDoNotModifySource()
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(TestContext.Current.CancellationToken);
        timeout.CancelAfter(TimeSpan.FromSeconds(90));var token=timeout.Token;
        var root=Path.Combine(Path.GetTempPath(),"MewuAudioExportTests",Guid.NewGuid().ToString("N"));Directory.CreateDirectory(root);
        try
        {
            var wave=Path.Combine(root,"tone.wav");WriteTone(wave);
            var source=Path.Combine(root,"source.mp4");
            await CreateVideoAsync(source,wave,token);
            var originalHash=SHA256.HashData(await File.ReadAllBytesAsync(source,token));
            var mp3=Path.Combine(root,"sound.mp3");
            await Mp3ExportService.ExportAsync(source,mp3,token);
            await AssertAudibleAsync(mp3,Path.Combine(root,"decoded.wav"),token);
            var note=new AiAnnotation(.1,.1,.3,.3,"Synthetic",0,.1,.8,
                [new VideoAnnotationKeyframe(.1,.1,.1,.3,.3),new VideoAnnotationKeyframe(.8,.2,.2,.3,.3)]);
            var annotated=Path.Combine(root,"annotated.mp4");
            await AnnotatedVideoExportService.ExportAsync(source,annotated,null,[note],token);
            await AssertAudibleAsync(annotated,Path.Combine(root,"annotated.wav"),token);
            Assert.Equal(originalHash,SHA256.HashData(await File.ReadAllBytesAsync(source,token)));

            var silent=Path.Combine(root,"silent.mp4");await CreateVideoAsync(silent,null,token);
            var existing=Path.Combine(root,"existing.mp3");await File.WriteAllTextAsync(existing,"retain",token);
            await Assert.ThrowsAsync<InvalidOperationException>(()=>Mp3ExportService.ExportAsync(silent,existing,token));
            Assert.Equal("retain",await File.ReadAllTextAsync(existing,token));
            using var canceled=new CancellationTokenSource();canceled.Cancel();
            await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>Mp3ExportService.ExportAsync(source,existing,canceled.Token));
            Assert.Equal("retain",await File.ReadAllTextAsync(existing,token));
            await Assert.ThrowsAsync<InvalidOperationException>(()=>Mp3ExportService.ExportAsync(source,source,token));
        }
        finally{await DeleteMediaDirectoryAsync(root);}
    }

    internal static async Task DeleteMediaDirectoryAsync(string root)
    {
        // WinRT transcode result wrappers have no IDisposable contract and can
        // retain native readers until finalization, even after closing streams.
        // Collect only in fixture teardown; production must never force GC.
        GC.Collect();GC.WaitForPendingFinalizers();
        var started=System.Diagnostics.Stopwatch.StartNew();
        while(true)
        {
            try{Directory.Delete(root,true);return;}
            catch(IOException) when(started.Elapsed<TimeSpan.FromSeconds(5))
            {await Task.Delay(100,CancellationToken.None);}
        }
    }

    internal static void WriteTone(string path)
    {
        const int rate=44100,count=rate*2;
        using var stream=File.Create(path);using var writer=new BinaryWriter(stream);
        writer.Write(Encoding.ASCII.GetBytes("RIFF"));writer.Write(36+count*2);writer.Write(Encoding.ASCII.GetBytes("WAVEfmt "));
        writer.Write(16);writer.Write((short)1);writer.Write((short)1);writer.Write(rate);writer.Write(rate*2);writer.Write((short)2);writer.Write((short)16);
        writer.Write(Encoding.ASCII.GetBytes("data"));writer.Write(count*2);
        for(var i=0;i<count;i++)writer.Write((short)(Math.Sin(2*Math.PI*440*i/rate)*5000));
    }

    private static async Task CreateVideoAsync(string path,string? wave,CancellationToken token)
    {
        var composition=new MediaComposition();
        composition.Clips.Add(MediaClip.CreateFromColor(Windows.UI.Color.FromArgb(255,30,160,210),TimeSpan.FromSeconds(2)));
        if(wave is not null)composition.BackgroundAudioTracks.Add(await BackgroundAudioTrack.CreateFromFileAsync(await StorageFile.GetFileFromPathAsync(wave).AsTask(token)).AsTask(token));
        var folder=await StorageFolder.GetFolderFromPathAsync(Path.GetDirectoryName(path)!).AsTask(token);
        var output=await folder.CreateFileAsync(Path.GetFileName(path),CreationCollisionOption.FailIfExists).AsTask(token);
        var profile=MediaEncodingProfile.CreateMp4(VideoEncodingQuality.Wvga);profile.Video.Width=128;profile.Video.Height=96;
        if(wave is null)profile.Audio=null;
        Assert.Equal(TranscodeFailureReason.None,await composition.RenderToFileAsync(output,MediaTrimmingPreference.Precise,profile).AsTask(token));
    }

    internal static async Task AssertAudibleAsync(string sourcePath,string wavePath,CancellationToken token)
    {
        var source=await StorageFile.GetFileFromPathAsync(sourcePath).AsTask(token);
        var folder=await StorageFolder.GetFolderFromPathAsync(Path.GetDirectoryName(wavePath)!).AsTask(token);
        var output=await folder.CreateFileAsync(Path.GetFileName(wavePath),CreationCollisionOption.FailIfExists).AsTask(token);
        var profile=MediaEncodingProfile.CreateWav(AudioEncodingQuality.High);profile.Audio=AudioEncodingProperties.CreatePcm(44100,1,16);
        using(var inputStream=await source.OpenAsync(FileAccessMode.Read).AsTask(token))
        using(var outputStream=await output.OpenAsync(FileAccessMode.ReadWrite).AsTask(token))
        {
            var transcode=await new MediaTranscoder().PrepareStreamTranscodeAsync(inputStream,outputStream,profile).AsTask(token);
            Assert.True(transcode.CanTranscode,$"Decode failed: {transcode.FailureReason}");await transcode.TranscodeAsync().AsTask(token);
        }
        using var reader=new BinaryReader(File.OpenRead(wavePath));
        Assert.Equal("RIFF",Encoding.ASCII.GetString(reader.ReadBytes(4)));reader.ReadUInt32();Assert.Equal("WAVE",Encoding.ASCII.GetString(reader.ReadBytes(4)));
        long audible=0,total=0;
        while(reader.BaseStream.Position+8<=reader.BaseStream.Length)
        {
            var id=Encoding.ASCII.GetString(reader.ReadBytes(4));var length=reader.ReadUInt32();var end=reader.BaseStream.Position+length;
            Assert.True(end<=reader.BaseStream.Length);
            if(id=="data")for(uint i=0;i+2<=length;i+=2){if(Math.Abs((int)reader.ReadInt16())>100)audible++;total++;}
            reader.BaseStream.Position=end+(length&1);
        }
        Assert.True(total>22050,"Decoded audio must span at least half a second.");
        Assert.True(audible>total/10,$"Expected audible samples, got {audible}/{total}.");
    }
}
