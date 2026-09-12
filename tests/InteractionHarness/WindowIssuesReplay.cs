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

    internal static void Run(Application app,AppHost host)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
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
                Check("settings-still-excluded",Affinity(settings)==NativeMethods.WdaExcludeFromCapture);
                var settingsHandle=Handle(settings);settings.Close();await Idle();
                Check("closed-settings-no-protected-hwnd",!GetWindowDisplayAffinity(settingsHandle,out _));

                foreach(var teaching in new[]{true,false})
                {
                    host.Settings.TeachingMode=teaching;
                    var prefix=teaching?"teaching":"protected";
                    var pin=new PinnedImageWindow(SolidImage(),new ScreenRect(area.Left+120,area.Top+140,400,260),teaching);
                    windows.Add(pin);pin.Show();await Idle();
                    var second=new PinnedImageWindow(SolidImage(),new ScreenRect(area.Left+340,area.Top+270,350,220),teaching);
                    windows.Add(second);second.Show();await Idle();
                    for(var round=0;round<2;round++)
                    {
                        var overlay=new CaptureOverlayWindow(host){Title="Mewu QA · Screenshot over pinned images",ShowInTaskbar=true};windows.Add(overlay);
                        Program.MarkReplayWindow(overlay,"Issue #4 验收 · 贴图不能挡住截图操作 · 完成后自动关闭");
                        overlay.Show();overlay.Activate();await Idle();
                        var label=$"{prefix}-{round}";
                        Check(label+"-overlay-above-both-pins",Above(overlay,pin)&&Above(overlay,second));
                        Check(label+"-pin-area-input-goes-to-overlay",WindowFromPoint(new NativePoint{X=area.Left+200,Y=area.Top+210})==Handle(overlay));
                        Check(label+"-overlay-protection",Affinity(overlay)==(teaching?0:NativeMethods.WdaExcludeFromCapture));
                        Check(label+"-pins-retain-topmost",pin.Topmost&&second.Topmost&&pin.IsVisible&&second.IsVisible);
                        Check(label+"-pin-protection",Affinity(pin)==(teaching?0:NativeMethods.WdaExcludeFromCapture));
                        var frame=(CaptureFrame)typeof(CaptureOverlayWindow).GetField("_frame",Private)!.GetValue(overlay)!;
                        var sample=new byte[4];
                        frame.Image.CopyPixels(new Int32Rect(200,210,1,1),sample,4,0);
                        Check(label+"-pin-pixels-preserved-in-frozen-desktop",sample[2]>sample[1]&&sample[1]>sample[0]);
                        Array.Clear(sample);
                        second.Activate();await Idle();overlay.Activate();await Idle();
                        Check(label+"-reactivation-keeps-overlay-above-pins",Above(overlay,pin)&&Above(overlay,second));
                        // Exercise the actual pin command in this same overlay,
                        // including its desktop refresh and focus restoration.
                        var selection=typeof(CaptureOverlayWindow).GetMethod("CreateSelection",Private)!.Invoke(overlay,[false])!;
                        var boundsProperty=selection.GetType().GetField("Bounds")!;
                        boundsProperty.SetValue(selection,new Rect(80,80,180,120));
                        var selections=(System.Collections.IList)typeof(CaptureOverlayWindow).GetField("_selections",Private)!.GetValue(overlay)!;
                        selections.Add(selection);
                        typeof(CaptureOverlayWindow).GetField("_activeIndex",Private)!.SetValue(overlay,selections.Count-1);
                        typeof(CaptureOverlayWindow).GetMethod("UpdateSelection",Private)!.Invoke(overlay,[selection]);
                        typeof(CaptureOverlayWindow).GetMethod("Pin",Private)!.Invoke(overlay,[overlay,new RoutedEventArgs()]);
                        await Idle();
                        var created=app.Windows.OfType<PinnedImageWindow>().Single(w=>w!=pin&&w!=second);
                        windows.Add(created);
                        Check(label+"-new-pin-does-not-cover-overlay",Above(overlay,created));
                        Check(label+"-new-pin-preserved",created.IsVisible&&created.Topmost);
                        if(teaching&&round==0)await Task.Delay(20000);
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
                File.WriteAllText(".codex-build/issue-3-4/window-replay.json",JsonSerializer.Serialize(new{checks,failure},new JsonSerializerOptions{WriteIndented=true}));
                app.Shutdown(failure is null&&checks.Values.All(value=>value)?0:1);
            }
        }));
        app.Run(background);
        void Check(string name,bool value)=>checks.Add(name,value);
    }

    private static IntPtr Handle(Window window)=>new WindowInteropHelper(window).Handle;
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
    private static async Task Idle(){await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);}
    private static BitmapSource SolidImage()
    {
        var pixels=new byte[400*260*4];
        for(var index=0;index<pixels.Length;index+=4){pixels[index]=80;pixels[index+1]=160;pixels[index+2]=230;pixels[index+3]=255;}
        var image=BitmapSource.Create(400,260,96,96,PixelFormats.Bgra32,null,pixels,400*4);image.Freeze();Array.Clear(pixels);return image;
    }
}
