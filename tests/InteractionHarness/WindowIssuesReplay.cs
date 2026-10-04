// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Brushes=System.Windows.Media.Brushes;

internal static class WindowIssuesReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr window,uint command);
    [DllImport("user32.dll")] private static extern bool GetWindowDisplayAffinity(IntPtr window,out uint affinity);
    [DllImport("user32.dll")] private static extern IntPtr WindowFromPoint(NativePoint point);
    [StructLayout(LayoutKind.Sequential)] private struct NativePoint { public int X,Y; }

    internal static void Run(Application app,AppHost host,bool sharedOnly=false)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        var startedUtc=DateTimeOffset.UtcNow;
        var checks=new Dictionary<string,bool>();
        var windows=new List<Window>();
        var area=System.Windows.Forms.SystemInformation.VirtualScreen;
        var background=new Window {Title="Mewu QA · Issue 3 / 4 · Synthetic desktop",WindowStyle=WindowStyle.None,
            Background=Brushes.LightSteelBlue,Topmost=true,ShowInTaskbar=true,
            Content=new TextBlock{Text="窗口层级验收 · 合成桌面 · 完成后自动关闭",FontSize=30,Margin=new Thickness(40)}};
        windows.Add(background);
        background.SourceInitialized+=(_,_)=>NativeMethods.SetWindowPos(Handle(background),new IntPtr(-1),area.Left,area.Top,area.Width,area.Height,0x0040);
        background.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            string? failure=null;
            try
            {
                background.Topmost=false;
                var launcher=new MainWindow(host){Title="Mewu QA · Launcher protection"};windows.Add(launcher);
                launcher.Show();await Idle();
                Check("launcher-visible-has-no-recording-block",Affinity(launcher)==0);
                launcher.Hide();await Idle();
                Check("launcher-hidden-has-no-recording-block",Affinity(launcher)==0);
                launcher.Show();await Idle();
                Check("launcher-reopened-has-no-recording-block",Affinity(launcher)==0);
                launcher.Hide();
                var settings=new SettingsWindow(host){Title="Mewu QA · Synthetic settings"};windows.Add(settings);
                using var suppressUpdate=new CancellationTokenSource();
                typeof(SettingsWindow).GetField("_updateCheck",Private)!.SetValue(settings,suppressUpdate);
                settings.Show();await Idle();
                Check("settings-capture-allowed-as-requested",Affinity(settings)==0);
                VerifyDialogPolicy(settings,"settings",0,Check);
                var licenses=new LicenseNoticesWindow{Owner=settings};windows.Add(licenses);
                licenses.Show();await Idle();
                Check("public-licenses-have-no-recording-block",Affinity(licenses)==0);
                licenses.Close();
                var settingsHandle=Handle(settings);settings.Close();await Idle();
                Check("closed-settings-no-protected-hwnd",!GetWindowDisplayAffinity(settingsHandle,out _));

                // The protected-mode half deliberately blocks desktop recorders.
                // Skip it when checking a real NVIDIA Instant Replay session.
                foreach(var teaching in sharedOnly?new[]{true}:new[]{true,false})
                {
                    host.Settings.TeachingMode=teaching;
                    var prefix=teaching?"teaching":"protected";
                    var firstOverlay=new CaptureOverlayWindow(host){Title="Mewu QA · First pin",ShowInTaskbar=true};windows.Add(firstOverlay);
                    Program.MarkReplayWindow(firstOverlay,"置顶验收 · 当前贴图立即在上 · 完成后自动关闭");
                    firstOverlay.Show();firstOverlay.Activate();await Idle();
                    VerifyDialogPolicy(firstOverlay,prefix,teaching?0:NativeMethods.WdaExcludeFromCapture,Check);
                    if(teaching)
                    {
                        // A visibly different frozen frame proves refresh captures
                        // the desktop underneath, rather than the overlay itself.
                        ((System.Windows.Controls.Image)firstOverlay.FindName("DesktopImage")).Source=MagentaDesktop(area.Width,area.Height);
                        await Idle();NativeMethods.FlushComposition();
                        Check("visible-overlay-differs-from-clean-desktop",!HasBackgroundPixel(new ScreenCaptureService().CaptureDesktop(false)));
                        var handle=Handle(firstOverlay);
                        var hiddenFrame=NativeMethods.WithWindowCloaked(handle,()=>
                        {
                            Check("snapshot-keeps-affinity-unprotected",Affinity(firstOverlay)==0);
                            Check("snapshot-is-cloaked",!NativeMethods.IsWindowUncloaked(handle));
                            NativeMethods.WithWindowCloaked(handle,()=>Affinity(firstOverlay));
                            Check("nested-snapshot-preserves-outer-cloak",!NativeMethods.IsWindowUncloaked(handle));
                            return new ScreenCaptureService().CaptureDesktop(false);
                        });
                        Check("cloaked-snapshot-has-clean-desktop",HasBackgroundPixel(hiddenFrame));
                        Check("snapshot-restores-window",NativeMethods.IsWindowUncloaked(handle)&&Affinity(firstOverlay)==0);
                        var expected=new InvalidOperationException("Synthetic snapshot failure");
                        try{NativeMethods.WithWindowCloaked<int>(handle,()=>throw expected);Check("snapshot-failure-propagated",false);}
                        catch(InvalidOperationException ex) when(ReferenceEquals(ex,expected)){Check("snapshot-failure-propagated",true);}
                        Check("failed-snapshot-restores-window",NativeMethods.IsWindowUncloaked(handle)&&Affinity(firstOverlay)==0);
                        var refreshed=(CaptureFrame)typeof(CaptureOverlayWindow).GetMethod("CaptureCleanDesktopForRefresh",Private)!.Invoke(firstOverlay,null)!;
                        Check("overlay-refresh-has-clean-desktop",HasBackgroundPixel(refreshed));
                        Check("overlay-refresh-restores-shared-window",NativeMethods.IsWindowUncloaked(handle)&&Affinity(firstOverlay)==0);
                    }
                    var firstPin=PinSelection(app,firstOverlay);windows.Add(firstPin);await Idle();
                    Check(prefix+"-first-pin-above-current-overlay",Above(firstPin,firstOverlay));
                    Check(prefix+"-first-pin-receives-input",HitsCenter(firstPin));
                    firstPin.Activate();await Idle();firstOverlay.Activate();await Idle();
                    Check(prefix+"-first-pin-survives-overlay-reactivation",Above(firstPin,firstOverlay)&&HitsCenter(firstPin));
                    firstOverlay.Close();await Idle();
                    Check(prefix+"-first-pin-survives-capture-close",firstPin.IsVisible&&firstPin.Topmost);
                    firstPin.Close();await Idle();
                    var pin=new PinnedImageWindow(SolidImage(),new ScreenRect(area.Left+120,area.Top+140,400,260),teaching);
                    windows.Add(pin);pin.Show();await Idle();
                    var second=new PinnedImageWindow(SolidImage(),new ScreenRect(area.Left+340,area.Top+270,350,220),teaching);
                    windows.Add(second);second.Show();await Idle();
                    for(var round=0;round<2;round++)
                    {
                        var secondWasAbove=Above(second,pin);
                        var overlay=new CaptureOverlayWindow(host){Title="Mewu QA · Screenshot below pinned images",ShowInTaskbar=true};windows.Add(overlay);
                        Program.MarkReplayWindow(overlay,"置顶验收 · 再次截图在已有贴图下方 · 完成后自动关闭");
                        overlay.Show();overlay.Activate();await Idle();
                        var label=$"{prefix}-{round}";
                        Check(label+"-later-overlay-above-both-old-pins",Above(overlay,pin)&&Above(overlay,second));
                        Check(label+"-old-pin-area-input-goes-to-overlay",WindowFromPoint(new NativePoint{X=area.Left+200,Y=area.Top+210})==Handle(overlay));
                        Check(label+"-existing-pin-order-retained",Above(second,pin)==secondWasAbove);
                        Check(label+"-overlay-protection",Affinity(overlay)==(teaching?0:NativeMethods.WdaExcludeFromCapture));
                        Check(label+"-pins-retain-topmost",pin.Topmost&&second.Topmost&&pin.IsVisible&&second.IsVisible);
                        Check(label+"-pin-protection",Affinity(pin)==(teaching?0:NativeMethods.WdaExcludeFromCapture));
                        var frame=(CaptureFrame)typeof(CaptureOverlayWindow).GetField("_frame",Private)!.GetValue(overlay)!;
                        var sample=new byte[4];
                        frame.Image.CopyPixels(new Int32Rect(200,210,1,1),sample,4,0);
                        Check(label+"-pin-pixels-preserved-in-frozen-desktop",sample[2]>sample[1]&&sample[1]>sample[0]);
                        Array.Clear(sample);
                        second.Activate();await Idle();overlay.Activate();await Idle();
                        Check(label+"-reactivation-keeps-overlay-above-old-pins",Above(overlay,pin)&&Above(overlay,second));
                        var refreshed=(CaptureFrame)typeof(CaptureOverlayWindow).GetMethod("CaptureCleanDesktopForRefresh",Private)!.Invoke(overlay,null)!;
                        refreshed.Image.CopyPixels(new Int32Rect(200,210,1,1),sample,4,0);
                        Check(label+"-refresh-preserves-pin-pixels",sample[2]>sample[1]&&sample[1]>sample[0]);
                        Check(label+"-refresh-preserves-capture-policy",Affinity(overlay)==(teaching?0:NativeMethods.WdaExcludeFromCapture));
                        Array.Clear(sample);
                        // Exercise the actual pin command in this same overlay,
                        // including its desktop refresh and focus restoration.
                        var created=PinSelection(app,overlay);
                        await Idle();
                        windows.Add(created);
                        Check(label+"-new-pin-above-current-overlay",Above(created,overlay));
                        Check(label+"-new-pin-receives-input",HitsCenter(created));
                        Check(label+"-new-pin-preserved",created.IsVisible&&created.Topmost);
                        pin.Activate();await Idle();overlay.Activate();await Idle();
                        Check(label+"-mixed-generation-reactivation",Above(created,overlay)&&Above(overlay,pin)&&Above(overlay,second));
                        overlay.Close();await Idle();
                        Check(label+"-closing-overlay-preserves-pins",pin.IsVisible&&second.IsVisible&&created.IsVisible&&pin.Topmost&&second.Topmost);
                        created.Close();
                        await Idle();
                        Check(label+"-pin-input-restored",WindowFromPoint(new NativePoint{X=area.Left+200,Y=area.Top+300})==Handle(pin));
                    }
                    pin.Close();second.Close();
                }
            }
            catch(Exception ex){failure=ex.ToString();}
            finally
            {
                foreach(var window in windows.AsEnumerable().Reverse())try{window.Close();}catch{}
                host.Dispose();
                Directory.CreateDirectory(".codex-build/issue-3-4");
                var report=sharedOnly?"window-replay-shared.json":"window-replay.json";
                File.WriteAllText(Path.Combine(".codex-build/issue-3-4",report),JsonSerializer.Serialize(new{sharedOnly,startedUtc,completedUtc=DateTimeOffset.UtcNow,checks,failure},new JsonSerializerOptions{WriteIndented=true}));
                app.Shutdown(failure is null&&checks.Values.All(value=>value)?0:1);
            }
        }));
        app.Run(background);
        void Check(string name,bool value)=>checks.Add(name,value);
        async Task Idle()
        {
            await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
            await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
            // Give an external recorder time to acquire each visible state.
            if(sharedOnly)await Task.Delay(200);
        }
    }

    private static IntPtr Handle(Window window)=>new WindowInteropHelper(window).Handle;
    private static void VerifyDialogPolicy(Window owner,string prefix,uint expected,Action<string,bool> check)
    {
        ObserveModal<MewuDialogWindow>(()=>MewuDialogWindow.ShowMessage(owner,"Mewu QA","Synthetic dialog"),dialog=>
        {
            check(prefix+"-message-inherits-owner-policy",Affinity(dialog)==expected);
            ObserveModal<MewuColorDialog>(()=>MewuColorDialog.TryChoose(dialog,Colors.Red,out _),color=>
                check(prefix+"-nested-color-inherits-owner-policy",Affinity(color)==expected));
        });
        ObserveModal<MewuColorDialog>(()=>MewuColorDialog.TryChoose(owner,Colors.Red,out _),color=>
            check(prefix+"-color-inherits-owner-policy",Affinity(color)==expected));
    }
    private static void ObserveModal<T>(Action show,Action<T> inspect) where T:Window
    {
        Exception? failure=null;
        Application.Current.Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle,new Action(()=>
        {
            T? dialog=null;
            try{dialog=Application.Current.Windows.OfType<T>().Single();inspect(dialog);}
            catch(Exception ex){failure=ex;}
            finally{dialog?.Close();}
        }));
        show();
        if(failure is not null)throw new InvalidOperationException("Modal capture-policy replay failed",failure);
    }
    private static bool HasBackgroundPixel(CaptureFrame frame)
    {
        var pixel=new byte[4];
        frame.Image.CopyPixels(new Int32Rect(frame.Image.PixelWidth-100,frame.Image.PixelHeight-100,1,1),pixel,4,0);
        return pixel[0]==Colors.LightSteelBlue.B&&pixel[1]==Colors.LightSteelBlue.G&&pixel[2]==Colors.LightSteelBlue.R;
    }
    private static BitmapSource MagentaDesktop(int width,int height)
    {
        var pixels=new byte[width*height*4];
        for(var index=0;index<pixels.Length;index+=4){pixels[index]=255;pixels[index+2]=255;pixels[index+3]=255;}
        var image=BitmapSource.Create(width,height,96,96,PixelFormats.Bgra32,null,pixels,width*4);image.Freeze();Array.Clear(pixels);return image;
    }
    private static bool HitsCenter(Window window)
    {
        if(!NativeMethods.GetWindowRect(Handle(window),out var bounds))return false;
        return WindowFromPoint(new NativePoint{X=(bounds.Left+bounds.Right)/2,Y=(bounds.Top+bounds.Bottom)/2})==Handle(window);
    }
    private static PinnedImageWindow PinSelection(Application app,CaptureOverlayWindow overlay)
    {
        var previous=app.Windows.OfType<PinnedImageWindow>().ToHashSet();
        var selection=typeof(CaptureOverlayWindow).GetMethod("CreateSelection",Private)!.Invoke(overlay,[false])!;
        selection.GetType().GetField("Bounds")!.SetValue(selection,new Rect(80,80,180,120));
        var selections=(System.Collections.IList)typeof(CaptureOverlayWindow).GetField("_selections",Private)!.GetValue(overlay)!;
        selections.Add(selection);
        typeof(CaptureOverlayWindow).GetField("_activeIndex",Private)!.SetValue(overlay,selections.Count-1);
        typeof(CaptureOverlayWindow).GetMethod("UpdateSelection",Private)!.Invoke(overlay,[selection]);
        typeof(CaptureOverlayWindow).GetMethod("Pin",Private)!.Invoke(overlay,[overlay,new RoutedEventArgs()]);
        return app.Windows.OfType<PinnedImageWindow>().Single(window=>!previous.Contains(window));
    }
    private static uint Affinity(Window window)=>GetWindowDisplayAffinity(Handle(window),out var value)?value:uint.MaxValue;
    private static bool Above(Window front,Window back)
    {
        var target=Handle(front);var current=Handle(back);
        for(var count=0;count<2048&&current!=IntPtr.Zero;count++)
        {
            current=GetWindow(current,3); // GW_HWNDPREV: preceding window in actual Z order.
            if(current==target)return true;
        }
        return false;
    }
    private static BitmapSource SolidImage()
    {
        var pixels=new byte[400*260*4];
        for(var index=0;index<pixels.Length;index+=4){pixels[index]=80;pixels[index+1]=160;pixels[index+2]=230;pixels[index+3]=255;}
        var image=BitmapSource.Create(400,260,96,96,PixelFormats.Bgra32,null,pixels,400*4);image.Freeze();Array.Clear(pixels);return image;
    }
}
