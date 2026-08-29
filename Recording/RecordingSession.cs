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
    private readonly AppSettings _settings;private readonly MewuScreenRect _region;private readonly TempFileService _temp=new();private readonly CancellationTokenSource _framesStop=new();private Recorder? _recorder;private Task? _frameTask;private string _framesDirectory=string.Empty;
    public string VideoPath { get; private set; }=string.Empty;public string FramesDirectory=>_framesDirectory;public event Action<string>? Completed;public event Action<string>? Failed;
    public RecordingSession(AppSettings settings,MewuScreenRect region){_settings=settings;_region=region;}
    public void Start()
    {
        var root=Path.GetPathRoot(_temp.DirectoryPath)!;if(new DriveInfo(root).AvailableFreeSpace<500L*1024*1024)throw new IOException("磁盘剩余空间不足 500 MB，无法安全开始录屏");VideoPath=_temp.NewFile(".mp4");_framesDirectory=_temp.NewDirectory();
        var point=new System.Drawing.Point(_region.X+_region.Width/2,_region.Y+_region.Height/2);var screen=Forms.Screen.FromPoint(point);var local=new ScreenRecorderLib.ScreenRect(_region.X-screen.Bounds.Left,_region.Y-screen.Bounds.Top,_region.Width,_region.Height);var display=new DisplayRecordingSource(screen.DeviceName){SourceRect=local,IsCursorCaptureEnabled=_settings.IncludeRecordingCursor,IsBorderRequired=false};
        var width=_region.Width-( _region.Width%2);var height=_region.Height-(_region.Height%2);var options=new RecorderOptions{SourceOptions=new SourceOptions{RecordingSources=[display]},OutputOptions=new OutputOptions{RecorderMode=RecorderMode.Video,OutputFrameSize=new ScreenSize(width,height)},VideoEncoderOptions=new VideoEncoderOptions{Encoder=new H264VideoEncoder(),Framerate=_settings.RecordingFps,Quality=75,IsHardwareEncodingEnabled=true,IsFixedFramerate=true},AudioOptions=new AudioOptions{IsAudioEnabled=false},MouseOptions=new MouseOptions{IsMousePointerEnabled=_settings.IncludeRecordingCursor}};
        _recorder=Recorder.CreateRecorder(options);_recorder.OnRecordingComplete+=(_,e)=>Completed?.Invoke(e.FilePath);_recorder.OnRecordingFailed+=(_,e)=>Failed?.Invoke(e.Error);_recorder.Record(VideoPath);_frameTask=CaptureGifFramesAsync(_framesStop.Token);
    }
    private async Task CaptureGifFramesAsync(CancellationToken token)
    {
        var delay=Math.Max(67,1000/Math.Clamp(_settings.GifFps,1,15));var index=0;while(!token.IsCancellationRequested&&index<900){try{using var bmp=new Bitmap(_region.Width,_region.Height,PixelFormat.Format24bppRgb);using(var g=Graphics.FromImage(bmp))g.CopyFromScreen(_region.X,_region.Y,0,0,bmp.Size,CopyPixelOperation.SourceCopy);bmp.Save(Path.Combine(_framesDirectory,$"{index++:D5}.png"),System.Drawing.Imaging.ImageFormat.Png);await Task.Delay(delay,token);}catch(OperationCanceledException){break;}catch{await Task.Delay(delay,token);}}
    }
    public void Stop(){_framesStop.Cancel();_recorder?.Stop();}
    public async Task WaitFramesAsync(){if(_frameTask is not null)try{await _frameTask;}catch(OperationCanceledException){}}
    public void Dispose(){_framesStop.Cancel();_recorder?.Dispose();_framesStop.Dispose();}
}
