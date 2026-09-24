// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices.WindowsRuntime;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using Windows.Media.Editing;
using Windows.Storage;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Image=System.Windows.Controls.Image;
using Color=System.Windows.Media.Color;
using Brushes=System.Windows.Media.Brushes;

internal static class RecordingDurationReplay
{
    private const BindingFlags Instance=BindingFlags.Instance|BindingFlags.Public|BindingFlags.NonPublic;
    internal static void Run(Application app,AppHost host,string[] args)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        host.Settings.RecordSystemAudio=args.Contains("--system-audio");host.Settings.RecordMicrophone=false;host.Settings.IncludeRecordingCursor=false;
        var inspect=args.FirstOrDefault(a=>a.StartsWith("--inspect-recording=",StringComparison.Ordinal))?[20..];
        var full=args.Contains("--full-screen");
        var pause=args.Contains("--pause-resume");
        var name=inspect is not null?"inspect-"+Path.GetFileNameWithoutExtension(inspect):$"{(host.Settings.TeachingMode?"teaching":"normal")}-{(full?"full":"region")}-{(host.Settings.RecordSystemAudio?"audio":"silent")}{(pause?"-pause":"")}";
        var directory=Path.GetFullPath(".codex-build/recording-duration");Directory.CreateDirectory(directory);
        var report=Path.Combine(directory,name+".json");
        File.WriteAllText(report,"{\"stage\":\"starting\"}");
        var samples=new List<object>();var playback=new List<object>();
        CaptureOverlayWindow? overlay=null;Window? background=null;Window? viewer=null;object? preview=null;DispatcherTimer? timer=null;
        app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            string? failure=null;object? properties=null;string stage="starting";
            void Stage(string value){stage=value;File.WriteAllText(report,JsonSerializer.Serialize(new{stage,samples,playback,properties,failure}));}
            try
            {
                var video=inspect;
                Image previewImage;
                if(video is null)
                {
                    var label=new TextBlock{Text="录屏持续性验收 · 仅合成画面",FontSize=40,Foreground=Brushes.White,Margin=new Thickness(140)};
                    var scene=new Border{Background=Brushes.Red,Child=label};
                    background=new Window{Title="录屏持续性验收 · 完成后自动关闭",WindowStyle=WindowStyle.None,WindowState=WindowState.Maximized,Content=scene,Topmost=true};background.Show();
                    await Task.Delay(300);
                    overlay=new CaptureOverlayWindow(host);overlay.Show();overlay.UpdateLayout();
                    Program.MarkReplayWindow(overlay,"12 秒持续动态录制验收 · 完成后自动关闭");
                    object? Get(string field)=>typeof(CaptureOverlayWindow).GetField(field,Instance)!.GetValue(overlay);
                    object? Invoke(string method,params object[] values)=>typeof(CaptureOverlayWindow).GetMethod(method,Instance)!.Invoke(overlay,values);
                    var monitor=(Rect)Invoke("MonitorBounds",new Rect(100,100,300,200))!;
                    if(full)
                    {
                        var screen=(System.Windows.Forms.Screen)Invoke("SelectionMonitor",monitor)!;
                        var frame=(CaptureFrame)Get("_frame")!;var root=(Canvas)overlay.FindName("Root");var pixels=screen.Bounds;
                        monitor=ScreenCoordinateService.ToLocalDipRect(new ScreenRect(pixels.X,pixels.Y,pixels.Width,pixels.Height),frame.OriginX,frame.OriginY,root.ActualWidth,root.ActualHeight,frame.Image.PixelWidth,frame.Image.PixelHeight);
                    }
                    var bounds=full?monitor:new Rect(monitor.X+60,monitor.Y+100,monitor.Width-120,monitor.Height-200);
                    var item=Invoke("CreateSelection",false)!;item.GetType().GetField("Bounds")!.SetValue(item,bounds);
                    ((IList)Get("_selections")!).Add(item);Invoke("Select",0);
                    Stage("countdown");Invoke("Record",overlay,new RoutedEventArgs());
                    await Until(()=>Get("_recordingSession") is not null,15);
                    var session=Get("_recordingSession")!;
                    await ((Task)session.GetType().GetProperty("RecordingReady",Instance)!.GetValue(session)!).WaitAsync(TimeSpan.FromSeconds(25));
                    Stage("recording");var clock=Stopwatch.StartNew();
                    timer=new DispatcherTimer(DispatcherPriority.Render){Interval=TimeSpan.FromMilliseconds(25)};
                    timer.Tick+=(_,_)=>
                    {
                        var n=(int)(clock.Elapsed.TotalSeconds*4);
                        scene.Background=new SolidColorBrush(Color.FromRgb((byte)(40+n*73%200),(byte)(40+n*113%200),(byte)(40+n*151%200)));
                        label.Text=$"录屏持续性验收 · {clock.Elapsed.TotalSeconds:F2} 秒 · 帧 {n}";
                    };
                    timer.Start();await Task.Delay(5500);
                    if(pause)
                    {
                        Invoke("PauseRecording",overlay,new RoutedEventArgs());
                        var before=(TimeSpan)session.GetType().GetProperty("Elapsed")!.GetValue(session)!;
                        await Task.Delay(1000);
                        var after=(TimeSpan)session.GetType().GetProperty("Elapsed")!.GetValue(session)!;
                        if(after-before>TimeSpan.FromMilliseconds(100))throw new InvalidOperationException("Pause did not stop the recording clock");
                        Invoke("PauseRecording",overlay,new RoutedEventArgs());
                    }
                    await Task.Delay(7000);timer.Stop();
                    Stage("stopping");Invoke("StopRecording",overlay,new RoutedEventArgs());
                    await Until(()=>!(bool)Get("_recordingMode")!&&item.GetType().GetField("VideoPreview")!.GetValue(item) is not null,40);
                    video=(string)item.GetType().GetField("VideoPath")!.GetValue(item)!;
                    preview=item.GetType().GetField("VideoPreview")!.GetValue(item)!;
                    previewImage=(Image)item.GetType().GetProperty("Video")!.GetValue(item)!;
                    File.Copy(video,Path.Combine(directory,name+".mp4"),true);
                }
                else
                {
                    previewImage=new Image();viewer=new Window{Title="录屏播放诊断 · 完成后自动关闭",Width=960,Height=650,Content=previewImage};viewer.Show();
                    var type=typeof(AppHost).Assembly.GetType("mewu_ai_Assistant.Services.VideoPreviewSurface")!;
                    preview=Activator.CreateInstance(type,Instance,null,[previewImage,app.Dispatcher],null)!;
                    type.GetMethod("Load",Instance)!.Invoke(preview,[video,true]);
                }
                Stage("playback");
                preview!.GetType().GetMethod("Stop",Instance)!.Invoke(preview,null);
                preview.GetType().GetMethod("Play",Instance)!.Invoke(preview,null);
                byte[]? previous=null;
                for(var i=0;i<12;i++)
                {
                    await Task.Delay(1000);
                    var pixels=previewImage.Source is BitmapSource bitmap?Signature(bitmap):null;
                    var position=((TimeSpan)preview.GetType().GetProperty("LastPresentedPosition",Instance)!.GetValue(preview)!).TotalSeconds;
                    var change=previous is null||pixels is null?0:Difference(previous,pixels);
                    playback.Add(new{second=i+1,position,frames=preview.GetType().GetProperty("PresentedFrameCount",Instance)!.GetValue(preview),change});
                    if(inspect is null&&i>1&&(change<10||position<i-.5))throw new InvalidOperationException($"Preview froze in second {i+1}");
                    previous=pixels;Stage("playback");
                }
                preview.GetType().GetMethod("Pause",Instance)!.Invoke(preview,null);
                Stage("decode-file");
                var clip=await MediaClip.CreateFromFileAsync(await StorageFile.GetFileFromPathAsync(video)).AsTask().WaitAsync(TimeSpan.FromSeconds(15));
                var encoding=clip.GetVideoEncodingProperties();properties=new{encoding.Width,encoding.Height,duration=clip.OriginalDuration.TotalSeconds,encoding.Bitrate,fps=encoding.FrameRate.Numerator/(double)encoding.FrameRate.Denominator};
                if(inspect is null&&(clip.OriginalDuration.TotalSeconds<12||clip.OriginalDuration.TotalSeconds>14))throw new InvalidOperationException("Recording duration is incomplete or includes the pause");
                var composition=new MediaComposition();composition.Clips.Add(clip);previous=null;
                for(var t=.6;t<clip.OriginalDuration.TotalSeconds-.2;t+=.5)
                {
                    using var thumbnail=await composition.GetThumbnailAsync(TimeSpan.FromSeconds(t),320,180,VideoFramePrecision.NearestFrame).AsTask().WaitAsync(TimeSpan.FromSeconds(10));
                    using var stream=thumbnail.AsStreamForRead();var bitmap=BitmapDecoder.Create(stream,BitmapCreateOptions.PreservePixelFormat,BitmapCacheOption.OnLoad).Frames[0];
                    if(inspect is null&&samples.Count<2){var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));using var imageFile=File.Create(Path.Combine(directory,name+$"-{samples.Count}.png"));encoder.Save(imageFile);}
                    var pixels=Signature(bitmap);var change=previous is null?0:Difference(previous,pixels);
                    samples.Add(new{time=t,change});
                    if(inspect is null&&previous is not null&&change<10)throw new InvalidOperationException($"Synthetic video froze at {t:F1}s (change={change:F2})");
                    previous=pixels;Stage("decode-file");
                }
                composition.Clips.Clear();Stage("complete");
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally
            {
                timer?.Stop();if(inspect is not null&&preview is IDisposable disposable)disposable.Dispose();viewer?.Close();overlay?.Close();background?.Close();
                File.WriteAllText(report,JsonSerializer.Serialize(new{stage,samples,playback,properties,failure}));app.Shutdown(Environment.ExitCode);
            }
        }));app.Run();
    }
    private static async Task Until(Func<bool> condition,int seconds)
    {
        var clock=Stopwatch.StartNew();while(!condition()){if(clock.Elapsed.TotalSeconds>seconds)throw new TimeoutException();await Task.Delay(50);}
    }
    private static byte[] Signature(BitmapSource source)
    {
        var bitmap=new FormatConvertedBitmap(source,PixelFormats.Bgra32,null,0);var raw=new byte[bitmap.PixelWidth*bitmap.PixelHeight*4];bitmap.CopyPixels(raw,bitmap.PixelWidth*4,0);
        var result=new byte[32*18*3];
        for(var y=0;y<18;y++)for(var x=0;x<32;x++)for(var c=0;c<3;c++)result[(y*32+x)*3+c]=raw[((y*bitmap.PixelHeight/18)*bitmap.PixelWidth+x*bitmap.PixelWidth/32)*4+c];
        return result;
    }
    private static double Difference(byte[] a,byte[] b)=>a.Zip(b,(x,y)=>Math.Abs(x-y)).Average();
}
