// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Media;
using System.Security.Cryptography;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using Xunit;

namespace MewuAI.Tests;

public sealed class RecordingAudioLiveIntegrationTests
{
    public static bool LiveAudioEnabled=>Environment.GetEnvironmentVariable("MEWU_AUDIO_LIVE")=="1";

    [Fact(Skip="Requires an interactive Windows audio output; set MEWU_AUDIO_LIVE=1.",SkipUnless=nameof(LiveAudioEnabled))]
    public async Task ComputerAudioReachesRecordedMp4AndExportedMp3()
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(TestContext.Current.CancellationToken);
        timeout.CancelAfter(TimeSpan.FromSeconds(90));var token=timeout.Token;
        var root=Path.Combine(Path.GetTempPath(),"MewuAudioLiveTests",Guid.NewGuid().ToString("N"));Directory.CreateDirectory(root);
        var session=new RecordingSession(new AppSettings{RecordingFps=10,IncludeRecordingCursor=false},new ScreenRect(0,0,128,128),null);
        var done=new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
        session.Completed+=path=>done.TrySetResult(path);
        session.Failed+=error=>done.TrySetException(new InvalidOperationException(error));
        try
        {
            var tone=Path.Combine(root,"tone.wav");AudioExportIntegrationTests.WriteTone(tone);
            using var player=new SoundPlayer(tone);player.Load();
            session.Start();
            await session.RecordingReady.WaitAsync(TimeSpan.FromSeconds(25),token);
            player.PlayLooping();
            try{await Task.Delay(TimeSpan.FromSeconds(3),token);}
            finally{player.Stop();}
            session.Stop();var source=await done.Task.WaitAsync(TimeSpan.FromSeconds(25),token);
            using var retained=session.RetainCompletedVideo();await session.DisposeAsync();
            var originalHash=SHA256.HashData(await File.ReadAllBytesAsync(source,token));
            await AudioExportIntegrationTests.AssertAudibleAsync(source,Path.Combine(root,"recorded.wav"),token);
            var mp3=Path.Combine(root,"recorded.mp3");await Mp3ExportService.ExportAsync(source,mp3,token);
            await AudioExportIntegrationTests.AssertAudibleAsync(mp3,Path.Combine(root,"exported.wav"),token);
            Assert.Equal(originalHash,SHA256.HashData(await File.ReadAllBytesAsync(source,token)));
        }
        catch(Exception error)
        {
            TestContext.Current.TestOutputHelper!.WriteLine(error.ToString());
            throw;
        }
        finally
        {
            await session.DisposeAsync();
            if(File.Exists(session.VideoPath))File.Delete(session.VideoPath);
            await AudioExportIntegrationTests.DeleteMediaDirectoryAsync(root);
        }
    }
}
