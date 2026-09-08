// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Security.Cryptography;
using System.Runtime.InteropServices.WindowsRuntime;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using Windows.Graphics.Imaging;
using Windows.Media.Editing;
using Windows.Media.MediaProperties;
using Windows.Storage;
using Xunit;

namespace MewuAI.Tests;

public sealed class VideoAnalysisTimebaseTests
{
    [Theory]
    [InlineData(30,1,27)]
    [InlineData(2,1,41)]
    [InlineData(2,0,27)]
    public void RejectsCompressedFrameClockAndEncoderTailPadding(uint numerator,uint denominator,double outputSeconds)
        =>Assert.Throws<InvalidDataException>(()=>VideoAnalysisTimebaseService.EnsureTimebase(numerator,denominator,TimeSpan.FromSeconds(27),TimeSpan.FromSeconds(outputSeconds)));

    [Theory]
    [InlineData(.1,1)]
    [InlineData(.5,1)]
    [InlineData(1,1)]
    [InlineData(1.01,2)]
    [InlineData(15.5,16)]
    [InlineData(15.5333,16)]
    [InlineData(27,27)]
    public void AnalysisTransportCompletesFramePairsWithoutShorteningSource(double source,double expected)
        =>Assert.Equal(TimeSpan.FromSeconds(expected),VideoAnalysisTimebaseService.GetEncodedDuration(TimeSpan.FromSeconds(source)));

    [Theory]
    [InlineData(2)]
    [InlineData(1.5)]
    [InlineData(1.5333)]
    public async Task CalibratedVideoKeepsEventsDurationAndOriginalBytes(double tailSeconds)
    {
        var root=Path.Combine(Path.GetTempPath(),"MewuVideoClockTests",Guid.NewGuid().ToString("N"));Directory.CreateDirectory(root);
        string? preparedPath=null;
        try
        {
            var token=TestContext.Current.CancellationToken;
            var folder=await StorageFolder.GetFolderFromPathAsync(root).AsTask(token);
            var source=await folder.CreateFileAsync("synthetic.mp4").AsTask(token);
            var composition=new MediaComposition();
            composition.Clips.Add(MediaClip.CreateFromColor(Windows.UI.Color.FromArgb(255,255,0,0),TimeSpan.FromSeconds(2)));
            composition.Clips.Add(MediaClip.CreateFromColor(Windows.UI.Color.FromArgb(255,0,0,255),TimeSpan.FromSeconds(2)));
            composition.Clips.Add(MediaClip.CreateFromColor(Windows.UI.Color.FromArgb(255,0,255,0),TimeSpan.FromSeconds(tailSeconds)));
            var profile=MediaEncodingProfile.CreateMp4(VideoEncodingQuality.HD720p);profile.Video.Width=320;profile.Video.Height=180;profile.Video.FrameRate.Numerator=30;profile.Video.FrameRate.Denominator=1;profile.Audio=null;
            Assert.Equal(Windows.Media.Transcoding.TranscodeFailureReason.None,await composition.RenderToFileAsync(source,MediaTrimmingPreference.Precise,profile).AsTask(token));
            var before=SHA256.HashData(await File.ReadAllBytesAsync(source.Path,token));
            using(var prepared=await VideoAnalysisTimebaseService.CreateAsync(new(AiAttachmentType.Video,"video/mp4",FilePath:source.Path),token))
            {
                preparedPath=prepared.Path;Assert.InRange(prepared.Duration.TotalSeconds,4+tailSeconds-.05,4+tailSeconds+.05);
                var result=await StorageFile.GetFileFromPathAsync(prepared.Path).AsTask(token);
                var encodedProperties=await result.Properties.GetVideoPropertiesAsync().AsTask(token);
                Assert.InRange(encodedProperties.Duration.TotalSeconds,5.95,6.05);
                var clip=await MediaClip.CreateFromFileAsync(result).AsTask(token);var output=new MediaComposition();output.Clips.Add(clip);
                var sourceClip=await MediaClip.CreateFromFileAsync(source).AsTask(token);var input=new MediaComposition();input.Clips.Add(sourceClip);
                foreach(var (seconds,channel) in new[]{(.5,2),(2.5,0),(4.5,1),(5.4,1)})
                {
                    foreach(var video in new[]{input,output})
                    {
                        using var thumbnail=await video.GetThumbnailAsync(TimeSpan.FromSeconds(seconds),32,18,VideoFramePrecision.NearestFrame).AsTask(token);
                        var decoder=await BitmapDecoder.CreateAsync(thumbnail).AsTask(token);var data=(await decoder.GetPixelDataAsync().AsTask(token)).DetachPixelData();
                        try{Assert.True(data[channel]>230&&data[(channel+1)%3]<25&&data[(channel+2)%3]<25,$"Event moved at {seconds} seconds (source={ReferenceEquals(video,input)} BGR={data[0]},{data[1]},{data[2]})");}
                        finally{CryptographicOperations.ZeroMemory(data);}
                    }
                }
                output.Clips.Clear();input.Clips.Clear();
            }
            var after=SHA256.HashData(await File.ReadAllBytesAsync(source.Path,token));Assert.Equal(before,after);
            Assert.False(mewu_ai_Assistant.Services.TempMediaRegistry.Shared.IsLeased(preparedPath!));
        }
        finally{try{Directory.Delete(root,true);}catch(IOException){}}
    }
}
