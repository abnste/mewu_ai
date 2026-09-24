// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Interop;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.WindowsRuntime;
using Windows.Media.Editing;
using Windows.Storage;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Brushes=System.Windows.Media.Brushes;
using Color=System.Windows.Media.Color;
using Button=System.Windows.Controls.Button;
using Point=System.Windows.Point;

internal static class CaptureToolsReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic|BindingFlags.DeclaredOnly;
    [DllImport("user32.dll")] private static extern bool PostMessage(IntPtr handle,int message,IntPtr wParam,IntPtr lParam);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr handle);
    [DllImport("user32.dll")] private static extern IntPtr WindowFromPoint(NativePoint point);
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr handle,uint flags);
    [DllImport("user32.dll")] private static extern bool GetCursorPos(out NativePoint point);
    [DllImport("user32.dll")] private static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] private static extern void mouse_event(uint flags,uint dx,uint dy,uint data,UIntPtr extraInfo);
    [DllImport("user32.dll",EntryPoint="GetWindowLongPtrW")] private static extern IntPtr GetWindowLongPtr(IntPtr handle,int index);
    [StructLayout(LayoutKind.Sequential)] private struct NativePoint
    {
        internal int X,Y;
        internal NativePoint(int x,int y){X=x;Y=y;}
    }
    internal static void Run(Application app,AppHost host)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        host.Settings.RecordSystemAudio=false;host.Settings.RecordMicrophone=false;
        var rows=new StackPanel();
        for(var i=0;i<100;i++)rows.Children.Add(new Border{Height=50,Background=new SolidColorBrush(Color.FromRgb((byte)(80+i*37%170),(byte)(70+i*53%180),(byte)(60+i*29%190))),Child=new TextBlock{Text=$"Capture test row {i:000} — synthetic content — {i*7919}",FontSize=24,Margin=new Thickness(35,5,0,0)}});
        var scroll=new ScrollViewer{Content=rows};
        var background=new Window{Title="Mewu capture tools fixture",WindowStyle=WindowStyle.None,WindowState=WindowState.Maximized,Content=scroll};
        background.Show();
        var dragTarget=new Window{Owner=background,Title="Mewu recording drag target",WindowStyle=WindowStyle.None,ResizeMode=ResizeMode.NoResize,Width=260,Height=70,Left=700,Top=20,Background=Brushes.DarkSlateBlue,Content=new TextBlock{Text="Drag target",Foreground=Brushes.White,FontSize=22,HorizontalAlignment=System.Windows.HorizontalAlignment.Center,VerticalAlignment=System.Windows.VerticalAlignment.Center}};
        var dragging=false;double dragStartLeft=0;System.Drawing.Point dragStartCursor=default;
        dragTarget.MouseLeftButtonDown+=(_,_)=>{dragging=true;dragStartLeft=dragTarget.Left;dragStartCursor=System.Windows.Forms.Cursor.Position;dragTarget.CaptureMouse();};
        dragTarget.MouseMove+=(_,_)=>{if(!dragging)return;var current=System.Windows.Forms.Cursor.Position;dragTarget.Left=dragStartLeft+current.X-dragStartCursor.X;};
        dragTarget.MouseLeftButtonUp+=(_,_)=>{dragging=false;dragTarget.ReleaseMouseCapture();};
        dragTarget.Show();
        CaptureOverlayWindow? overlay=null;
        app.Dispatcher.BeginInvoke(DispatcherPriority.Normal,new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;object? state=null;
            try
            {
                background.UpdateLayout();await Dispatcher.Yield(DispatcherPriority.Background);
                Require(IsWindowVisible(new WindowInteropHelper(background).Handle),"Synthetic background is hidden; run this interactive replay without hidden-window startup");
                overlay=new CaptureOverlayWindow(host);overlay.Show();overlay.UpdateLayout();
                Program.MarkReplayWindow(overlay,"功能验收窗口 · 完成后自动关闭");
                var item=Invoke("CreateSelection",false)!;item.GetType().GetField("Bounds")!.SetValue(item,new Rect(100,300,360,240));
                ((IList)Get("_selections")).Add(item);Invoke("Select",0);Invoke("UpdatePointerInteraction",new Point(200,360));overlay.UpdateLayout();
                state=new{protectedWindow=Get("_captureExclusionVerified"),recordEnabled=((Button)overlay.FindName("RecordButton")).IsEnabled,longEnabled=((Button)overlay.FindName("LongCaptureButton")).IsEnabled};
                var frame=(CaptureFrame)Get("_frame");
                var pixels=(Int32Rect)Invoke("ToPixelRect",new Rect(100,300,360,240))!;
                var expected=ScreenCaptureService.Crop(frame.Image,pixels);
                if(host.Settings.TeachingMode)Require(!(bool)Invoke("IsTeachingAcquisitionClear",item)!,"Teaching capture was considered safe without a native hole");
                Click("LongCaptureButton");
                await Until(()=>(bool)Get("_longCaptureMode")&&Get("_longCaptureAccumulator") is not null,"Long screenshot did not start");
                checks.Add("long-capture-button-starts-live-session");
                if(host.Settings.TeachingMode)
                {
                    Require((bool)Native("IsVisibleToCapture",new WindowInteropHelper(overlay).Handle),"Teaching sharing was disabled during long capture");
                    Require((bool)Invoke("IsTeachingAcquisitionClear",item)!,"Long capture native hole covers some acquired pixels");
                    ComparePixels(expected,(BitmapSource)Get("_longCaptureComposite"),"Long capture contains overlay pixels");
                    checks.Add("teaching-sharing-visible-and-long-capture-pixels-clean");
                    Invoke("RememberLongCaptureWheelDirection",-120);scroll.ScrollToVerticalOffset(80);background.UpdateLayout();
                    await Until(()=>scroll.VerticalOffset>=79,"Fixture did not scroll");
                    Invoke("ScheduleLongCaptureSample",true);
                    await Until(()=>((BitmapSource)Get("_longCaptureComposite")).PixelHeight>expected.PixelHeight,"Teaching scrolling capture did not append content");
                    Require(((BitmapSource)Get("_longCaptureComposite")).PixelHeight>expected.PixelHeight,"Scrolling result did not grow");
                    checks.Add("teaching-scrolling-capture-appends-content");
                }
                Invoke("CancelLongCaptureSession","QA complete");
                scroll.ScrollToVerticalOffset(0);background.UpdateLayout();
                Require(!(bool)Get("_longCaptureMode"),"Long capture cancellation did not restore the screenshot");
                if(host.Settings.TeachingMode)
                {
                    Require(!(bool)Get("_teachingCaptureFinishRegistered")&&!(bool)Invoke("IsTeachingAcquisitionClear",item)!&&(bool)Native("IsVisibleToCapture",new WindowInteropHelper(overlay).Handle),"Long capture cancellation failed to restore sharing, geometry or F8");
                    Invoke("Record",overlay,new RoutedEventArgs());
                    Require((bool)Get("_recordingCountdownActive"),"Teaching countdown did not start");
                    Require(PostMessage(new WindowInteropHelper(overlay).Handle,0x312,new IntPtr(0x6D38),IntPtr.Zero),"Countdown cancel dispatch failed");
                    await Until(()=>!(bool)Get("_recordingCountdownActive"),"F8 did not cancel countdown");
                    Require(GetOrNull("_recordingSession") is null&&!(bool)Get("_teachingCaptureFinishRegistered"),"Countdown cancel created a recorder or leaked F8");
                    checks.Add("countdown-F8-cancels-without-starting-recorder");
                }
                checks.Add("long-capture-cancels-and-restores-screenshot");
                Invoke("UpdatePointerInteraction",new Point(200,360));overlay.UpdateLayout();Click("RecordButton");
                Require((bool)Get("_recordingCountdownActive"),"Record button did not enter countdown");
                await Until(()=>GetOrNull("_recordingSession") is not null,"Recording did not start after countdown");
                var session=Get("_recordingSession");
                var ready=(Task)session.GetType().GetProperty("RecordingReady",BindingFlags.Instance|BindingFlags.NonPublic)!.GetValue(session)!;
                await ready.WaitAsync(TimeSpan.FromSeconds(25));checks.Add("record-button-counts-down-and-starts-recorder");
                var overlayHandle=new WindowInteropHelper(overlay).Handle;var backgroundHandle=new WindowInteropHelper(background).Handle;
                var desktopPoint=overlay.PointToScreen(new Point(200,360));
                var recordingBar=(FrameworkElement)overlay.FindName("RecordingBar");
                GetCursorPos(out var savedCursor);
                try
                {
                    Require(SetCursorPos((int)desktopPoint.X,(int)desktopPoint.Y),"Could not position pointer over the recording desktop");
                    Invoke("UpdateRecordingInputRouting");await Task.Delay(40);
                    Require((GetWindowLongPtr(overlayHandle,-20).ToInt64()&0x20)!=0,"Recording overlay did not become mouse-transparent outside its control strip");
                    Require(GetAncestor(WindowFromPoint(new NativePoint((int)desktopPoint.X,(int)desktopPoint.Y)),2)==backgroundHandle,"Recording overlay still owns desktop input outside its control strip");
                    if(recordingBar.Visibility==Visibility.Visible)
                    {
                        var barPoint=recordingBar.PointToScreen(new Point(recordingBar.ActualWidth/2,recordingBar.ActualHeight/2));
                        Require(SetCursorPos((int)barPoint.X,(int)barPoint.Y),"Could not position pointer over recording controls");
                        Invoke("UpdateRecordingInputRouting");await Task.Delay(40);
                        Require((GetWindowLongPtr(overlayHandle,-20).ToInt64()&0x20)==0,"Recording control strip remained mouse-transparent");
                        Require(GetAncestor(WindowFromPoint(new NativePoint((int)barPoint.X,(int)barPoint.Y)),2)==overlayHandle,"Recording control strip is not interactive");
                        SaveRecordingBar(overlay,recordingBar,host.Settings.TeachingMode?"teaching-recording-bar.png":"recording-bar.png");
                        var barOrigin=recordingBar.PointToScreen(new Point(0,0));var desiredLeft=barOrigin.X-340;
                        var fromDevice=PresentationSource.FromVisual(dragTarget)!.CompositionTarget!.TransformFromDevice;
                        var targetOrigin=fromDevice.Transform(new Point(desiredLeft,barOrigin.Y));
                        dragTarget.Left=targetOrigin.X;dragTarget.Top=targetOrigin.Y;dragTarget.UpdateLayout();await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                        var beforeLeft=dragTarget.Left;var startX=(int)Math.Round(desiredLeft+130);var dragY=(int)Math.Round(barOrigin.Y+recordingBar.ActualHeight/2);var endX=(int)Math.Round(barOrigin.X+recordingBar.ActualWidth+180);
                        Require(SetCursorPos(startX,dragY),"Could not position pointer on the drag fixture");Invoke("UpdateRecordingInputRouting");await Task.Delay(40);
                        var dragHandle=new WindowInteropHelper(dragTarget).Handle;
                        var hitHandle=GetAncestor(WindowFromPoint(new NativePoint(startX,dragY)),2);
                        Require(hitHandle==dragHandle,$"Drag fixture is not the hit target at {startX},{dragY} (hit=0x{hitHandle.ToInt64():X}, drag=0x{dragHandle.ToInt64():X}, overlay=0x{overlayHandle.ToInt64():X}, background=0x{backgroundHandle.ToInt64():X})");
                        mouse_event(0x0002,0,0,0,UIntPtr.Zero);
                        for(var step=1;step<=14;step++){SetCursorPos(startX+(endX-startX)*step/14,dragY);await Task.Delay(25);}
                        mouse_event(0x0004,0,0,0,UIntPtr.Zero);await Task.Delay(120);
                        Require(dragTarget.Left>beforeLeft+150,$"Dragging an underlying window across the recording control strip was interrupted ({beforeLeft:F0} -> {dragTarget.Left:F0})");
                        checks.Add("recording-drag-crosses-control-strip");
                    }
                }
                finally{SetCursorPos(savedCursor.X,savedCursor.Y);Invoke("UpdateRecordingInputRouting");}
                checks.Add("recording-window-region-leaves-desktop-interactive");
                if(host.Settings.TeachingMode)Require((bool)Native("IsVisibleToCapture",new WindowInteropHelper(overlay).Handle)&&(bool)Invoke("IsTeachingAcquisitionClear",item)!,"Recording changed sharing or covered acquired pixels");
                await Task.Delay(1300);
                if(host.Settings.TeachingMode)Require(PostMessage(new WindowInteropHelper(overlay).Handle,0x312,new IntPtr(0x6D38),IntPtr.Zero),"F8 dispatch failed");
                else Invoke("StopRecording",overlay,new RoutedEventArgs());
                await Until(()=>item.GetType().GetProperty("VideoPath")?.GetValue(item) is string||item.GetType().GetField("VideoPath")?.GetValue(item) is string,"Recording did not produce video",30);
                checks.Add("recording-stops-and-produces-video");
                var video=(string)(item.GetType().GetProperty("VideoPath")?.GetValue(item)??item.GetType().GetField("VideoPath")!.GetValue(item))!;
                await Until(()=>item.GetType().GetField("VideoPreview")!.GetValue(item) is not null,"Video preview surface was not created");
                await Task.Delay(200);
                Invoke("RefreshToolbar",new Point(200,360));overlay.UpdateLayout();
                Click("VideoPlayButton");
                Require(!(bool)Get("_recordingMode")&&!(bool)item.GetType().GetField("VideoPlaying")!.GetValue(item)!,"Video pause button did not pause the preview");
                Click("VideoPlayButton");
                await Until(()=>{var player=item.GetType().GetField("VideoPreview")!.GetValue(item)!;return (long)player.GetType().GetProperty("PresentedFrameCount",BindingFlags.Instance|BindingFlags.NonPublic)!.GetValue(player)!>0;},"Video play button did not resume the preview");
                checks.Add("video-play-pause-button-controls-in-place-preview");
                var clip=await MediaClip.CreateFromFileAsync(await StorageFile.GetFileFromPathAsync(video));
                var composition=new MediaComposition();composition.Clips.Add(clip);
                foreach(var fraction in new[]{0.05,0.5,0.9})
                {
                    using var thumbnail=await composition.GetThumbnailAsync(TimeSpan.FromSeconds(clip.OriginalDuration.TotalSeconds*fraction),expected.PixelWidth,expected.PixelHeight,VideoFramePrecision.NearestFrame);
                    using var stream=thumbnail.AsStreamForRead();var decoder=BitmapDecoder.Create(stream,BitmapCreateOptions.PreservePixelFormat,BitmapCacheOption.OnLoad);
                    ComparePixels(expected,decoder.Frames[0],"Video contains overlay pixels");
                }
                composition.Clips.Clear();checks.Add("video-first-middle-last-frames-clean");
                if(host.Settings.TeachingMode)
                {
                    Require(!(bool)Get("_teachingCaptureFinishRegistered"),"F8 was not released after recording");
                    overlay.Close();overlay=new CaptureOverlayWindow(host);overlay.Show();overlay.UpdateLayout();
                    var monitor=(Rect)Invoke("MonitorBounds",new Rect(100,300,360,240))!;
                    var full=Invoke("CreateSelection",false)!;full.GetType().GetField("Bounds")!.SetValue(full,monitor);
                    ((IList)Get("_selections")).Add(full);Invoke("Select",0);
                    Invoke("CaptureLongScreenshot",overlay,new RoutedEventArgs());
                    await Until(()=>(bool)Get("_longCaptureMode")&&Get("_longCaptureAccumulator") is not null,"Full screen long capture did not start");
                    Require(((FrameworkElement)overlay.FindName("LongCaptureBar")).Visibility!=Visibility.Visible,"Full screen capture still shows overlapping controls");
                    Require((bool)Invoke("IsTeachingAcquisitionClear",full)!,"Full screen region is not clear");
                    Require(PostMessage(new WindowInteropHelper(overlay).Handle,0x312,new IntPtr(0x6D38),IntPtr.Zero),"Full screen F8 dispatch failed");
                    await Until(()=>!(bool)Get("_longCaptureMode"),"F8 did not finish full screen long capture");
                    Require(!(bool)Get("_teachingCaptureFinishRegistered"),"F8 was not released after long capture");
                    checks.Add("full-screen-long-capture-finishes-with-F8");
                    // Finishing a tall long capture can fit its preview back into
                    // the monitor and change the selection bounds. Restore the
                    // exact monitor rectangle before exercising full-screen video.
                    full.GetType().GetField("Bounds")!.SetValue(full,monitor);Invoke("Select",0);overlay.UpdateLayout();
                    Invoke("Record",overlay,new RoutedEventArgs());
                    await Until(()=>GetOrNull("_recordingSession") is not null,"Full screen recording did not start");
                    var fullSession=Get("_recordingSession");
                    await ((Task)fullSession.GetType().GetProperty("RecordingReady",BindingFlags.Instance|BindingFlags.NonPublic)!.GetValue(fullSession)!).WaitAsync(TimeSpan.FromSeconds(25));
                    Require(((FrameworkElement)overlay.FindName("RecordingBar")).Visibility!=Visibility.Visible&&(bool)Invoke("IsTeachingAcquisitionClear",full)!,"Full screen recording controls cover its image");
                    await Task.Delay(1100);Require(PostMessage(new WindowInteropHelper(overlay).Handle,0x312,new IntPtr(0x6D38),IntPtr.Zero),"Full screen stop dispatch failed");
                    await Until(()=>full.GetType().GetField("VideoPath")?.GetValue(full) is string,"Full screen recording did not complete",30);
                    checks.Add("full-screen-recording-stops-with-F8");
                }
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally
            {
                overlay?.Close();dragTarget.Close();background.Close();Directory.CreateDirectory(".codex-build");
                File.WriteAllText(host.Settings.TeachingMode?".codex-build/teaching-capture-tools-result.json":".codex-build/capture-tools-result.json",JsonSerializer.Serialize(new{checks,state,failure}));app.Shutdown(Environment.ExitCode);
            }
            object Get(string field)=>GetOrNull(field)!;
            object? GetOrNull(string field)=>overlay!.GetType().GetField(field,Private)!.GetValue(overlay);
            object? Invoke(string method,params object[] args)=>overlay!.GetType().GetMethod(method,Private)!.Invoke(overlay,args);
            object Native(string method,params object[] args)=>typeof(AppHost).Assembly.GetType("mewu_ai_Assistant.Interop.NativeMethods")!.GetMethod(method,BindingFlags.Static|BindingFlags.NonPublic)!.Invoke(null,args)!;
            void Click(string name)
            {
                var button=(Button)overlay!.FindName(name);Require(button.IsVisible&&button.IsEnabled,$"{name} unavailable");
                var root=(Canvas)overlay.FindName("Root");var point=button.TranslatePoint(new Point(button.ActualWidth/2,button.ActualHeight/2),root);
                Invoke("UpdatePointerInteraction",point);
                button.RaiseEvent(new RoutedEventArgs(Button.ClickEvent,button));
            }
            async Task Until(Func<bool> condition,string message,int seconds=12)
            {
                var deadline=DateTime.UtcNow.AddSeconds(seconds);
                while(DateTime.UtcNow<deadline){if(condition())return;await Task.Delay(50);}
                throw new InvalidOperationException(message+"; status="+((TextBlock)overlay!.FindName("PromptStatus")).Text);
            }
        }));
        app.Run();
    }
    private static void Require(bool condition,string message){if(!condition)throw new InvalidOperationException(message);}
    private static void ComparePixels(BitmapSource expected,BitmapSource actual,string message)
    {
        Require(expected.PixelWidth==actual.PixelWidth&&expected.PixelHeight==actual.PixelHeight,message+": dimensions differ");
        var a=new FormatConvertedBitmap(expected,PixelFormats.Bgra32,null,0);var b=new FormatConvertedBitmap(actual,PixelFormats.Bgra32,null,0);
        var first=new byte[a.PixelWidth*a.PixelHeight*4];var second=new byte[first.Length];a.CopyPixels(first,a.PixelWidth*4,0);b.CopyPixels(second,b.PixelWidth*4,0);
        var different=0;
        for(var i=0;i<first.Length;i+=4)if(Math.Abs(first[i]-second[i])>35||Math.Abs(first[i+1]-second[i+1])>35||Math.Abs(first[i+2]-second[i+2])>35)different++;
        Require(different/(double)(a.PixelWidth*a.PixelHeight)<.035,message+$": changed fraction {different/(double)(a.PixelWidth*a.PixelHeight):F4}");
    }
    private static void SaveRecordingBar(CaptureOverlayWindow overlay,FrameworkElement bar,string file)
    {
        var root=(Canvas)overlay.FindName("Root");root.UpdateLayout();
        var full=new RenderTargetBitmap(Math.Max(1,(int)Math.Ceiling(root.ActualWidth)),Math.Max(1,(int)Math.Ceiling(root.ActualHeight)),96,96,PixelFormats.Pbgra32);full.Render(root);
        var origin=bar.TranslatePoint(new Point(0,0),root);const int margin=28;
        var left=Math.Max(0,(int)Math.Floor(origin.X)-margin);var top=Math.Max(0,(int)Math.Floor(origin.Y)-margin);
        var right=Math.Min(full.PixelWidth,(int)Math.Ceiling(origin.X+bar.ActualWidth)+margin);var bottom=Math.Min(full.PixelHeight,(int)Math.Ceiling(origin.Y+bar.ActualHeight)+margin);
        var crop=new CroppedBitmap(full,new Int32Rect(left,top,Math.Max(1,right-left),Math.Max(1,bottom-top)));crop.Freeze();
        Directory.CreateDirectory(".codex-build");var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(crop));using var stream=File.Create(Path.Combine(".codex-build",file));encoder.Save(stream);
    }
}
