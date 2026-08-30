using System.Drawing;
using System.Drawing.Imaging;
using ScreenRecorderLib;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Forms=System.Windows.Forms;
using MewuScreenRect=mewu_ai_Assistant.Models.ScreenRect;
namespace mewu_ai_Assistant.Recording;
public sealed class RecordingSession : IDisposable
{
    private readonly AppSettings _settings;private readonly MewuScreenRect _region;private readonly TempFileService _temp=new();private readonly CancellationTokenSource _framesStop=new();private Recorder? _recorder;private Task? _frameTask;private string _framesDirectory=string.Empty;private volatile bool _paused;private int _disposed;
    public string VideoPath { get; private set; }=string.Empty;public string FramesDirectory=>_framesDirectory;public event Action<string>? Completed;public event Action<string>? Failed;
    public RecordingSession(AppSettings settings,MewuScreenRect region){_settings=settings;if(region.Width<2||region.Height<2)throw new ArgumentOutOfRangeException(nameof(region),"录屏区域至少需要 2 × 2 像素");_region=new MewuScreenRect(region.X,region.Y,region.Width&~1,region.Height&~1);}
    public void Start()
    {
        var root=Path.GetPathRoot(_temp.DirectoryPath)!;if(new DriveInfo(root).AvailableFreeSpace<500L*1024*1024)throw new IOException("磁盘剩余空间不足 500 MB，无法安全开始录屏");VideoPath=_temp.NewFile(".mp4");_framesDirectory=_temp.NewDirectory();
        var screens=Forms.Screen.AllScreens;var displayRects=screens.Select(x=>new MewuScreenRect(x.Bounds.X,x.Bounds.Y,x.Bounds.Width,x.Bounds.Height)).ToArray();var slices=RecordingLayoutService.CreateSlices(_region,displayRects);var sources=new List<RecordingSourceBase>();foreach(var slice in slices){var screen=screens[Array.IndexOf(displayRects,slice.Display)];sources.Add(new DisplayRecordingSource(screen.DeviceName){SourceRect=new ScreenRecorderLib.ScreenRect(slice.Source.X-slice.Display.X,slice.Source.Y-slice.Display.Y,slice.Source.Width,slice.Source.Height),Position=new ScreenPoint(slice.Output.X,slice.Output.Y),OutputSize=new ScreenSize(slice.Output.Width,slice.Output.Height),IsCursorCaptureEnabled=_settings.IncludeRecordingCursor,IsBorderRequired=false});}if(sources.Count==0)throw new InvalidOperationException("选区不在可录制显示器范围内");
        var options=new RecorderOptions{SourceOptions=new SourceOptions{RecordingSources=sources},OutputOptions=new OutputOptions{RecorderMode=RecorderMode.Video,OutputFrameSize=new ScreenSize(_region.Width,_region.Height)},VideoEncoderOptions=new VideoEncoderOptions{Encoder=new H264VideoEncoder(),Framerate=Math.Clamp(_settings.RecordingFps,10,60),Quality=Math.Clamp(_settings.RecordingQuality,20,100),IsHardwareEncodingEnabled=true,IsFixedFramerate=true},AudioOptions=new AudioOptions{IsAudioEnabled=false},MouseOptions=new MouseOptions{IsMousePointerEnabled=_settings.IncludeRecordingCursor}};
        _recorder=Recorder.CreateRecorder(options);_recorder.OnRecordingComplete+=(_,e)=>Completed?.Invoke(e.FilePath);_recorder.OnRecordingFailed+=(_,e)=>Failed?.Invoke(e.Error);_recorder.Record(VideoPath);_frameTask=CaptureGifFramesAsync(_framesStop.Token);
    }
    private async Task CaptureGifFramesAsync(CancellationToken token)
    {
        var delay=Math.Max(67,1000/Math.Clamp(_settings.GifFps,1,15));var index=0;while(!token.IsCancellationRequested&&index<900){try{if(_paused){await Task.Delay(delay,token);continue;}using var bmp=new Bitmap(_region.Width,_region.Height,PixelFormat.Format24bppRgb);using(var g=Graphics.FromImage(bmp))g.CopyFromScreen(_region.X,_region.Y,0,0,bmp.Size,CopyPixelOperation.SourceCopy);bmp.Save(Path.Combine(_framesDirectory,$"{index++:D5}.png"),System.Drawing.Imaging.ImageFormat.Png);await Task.Delay(delay,token);}catch(OperationCanceledException){break;}catch{await Task.Delay(delay,token);}}
    }
    public void Stop(){_framesStop.Cancel();_recorder?.Stop();}
    public void Pause(){_paused=true;_recorder?.Pause();}public void Resume(){_recorder?.Resume();_paused=false;}
    public async Task WaitFramesAsync(){if(_frameTask is not null)try{await _frameTask;}catch(OperationCanceledException){}}
    public void Dispose(){if(Interlocked.Exchange(ref _disposed,1)!=0)return;_framesStop.Cancel();_recorder?.Dispose();_framesStop.Dispose();}
}
