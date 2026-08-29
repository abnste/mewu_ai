using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using Xunit;
namespace MewuAI.Tests;
public sealed class RecordingIntegrationTests
{
    [Fact] public async Task RecordsSmallRegionToRealMp4()
    {
        using var session=new RecordingSession(new AppSettings{RecordingFps=10,GifFps=2,IncludeRecordingCursor=false},new ScreenRect(0,0,128,128));var done=new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);session.Completed+=p=>done.TrySetResult(p);session.Failed+=e=>done.TrySetException(new InvalidOperationException(e));
        try{var token=TestContext.Current.CancellationToken;session.Start();await Task.Delay(1200,token);session.Stop();var path=await done.Task.WaitAsync(TimeSpan.FromSeconds(20),token);await session.WaitFramesAsync();Assert.True(File.Exists(path));Assert.True(new FileInfo(path).Length>1000);}
        finally{if(File.Exists(session.VideoPath))File.Delete(session.VideoPath);if(Directory.Exists(session.FramesDirectory))Directory.Delete(session.FramesDirectory,true);}
    }
}
