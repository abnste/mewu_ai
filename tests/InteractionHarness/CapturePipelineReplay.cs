// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Diagnostics;
using System.IO;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Brushes=System.Windows.Media.Brushes;
internal static class CapturePipelineReplay
{
    [System.Runtime.InteropServices.DllImport("dwmapi.dll")]private static extern int DwmSetWindowAttribute(IntPtr window,int attribute,ref int value,int size);
    internal static void Run(Application app)
    {
        using var host=new AppHost(app,null,"MewuAI-IsolatedPipeline-"+Guid.NewGuid().ToString("N"));
        host.Settings.EnableVoiceInput=false;host.Settings.AutomaticallyStartListening=false;
        host.Settings.TeachingMode=false;
        var marker=new Border{Background=Brushes.Lime};
        var fixture=new Window{Title="Mewu capture measurement fixture",Width=600,Height=400,WindowStartupLocation=WindowStartupLocation.CenterScreen,Topmost=true,ShowActivated=false,Content=marker};
        fixture.SourceInitialized+=(_,_)=>{var disabled=1;DwmSetWindowAttribute(new System.Windows.Interop.WindowInteropHelper(fixture).Handle,3,ref disabled,4);};
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        var samples=new List<object>();string? failure=null;CaptureOverlayWindow? overlay=null;
        fixture.Loaded+=async(_,_)=>
        {
            try
            {
                await app.Dispatcher.InvokeAsync(()=>{},DispatcherPriority.ApplicationIdle);
                var center=marker.PointToScreen(new System.Windows.Point(marker.ActualWidth/2,marker.ActualHeight/2));
                for(var i=0;i<6;i++)
                {
                    marker.Background=i%2==0?Brushes.Lime:Brushes.Red;
                    await app.Dispatcher.InvokeAsync(()=>{},DispatcherPriority.ApplicationIdle);
                    // Fixture preparation is outside all measurements. Dispatcher idle
                    // alone does not establish that DWM has presented this color.
                    await Task.Delay(150);
                    mewu_ai_Assistant.Interop.NativeMethods.FlushComposition();
                    var timer=Stopwatch.StartNew();var frame=new ScreenCaptureService().CaptureDesktop();var captureMs=timer.Elapsed.TotalMilliseconds;
                    var pixel=new byte[4];frame.Image.CopyPixels(new Int32Rect((int)center.X-frame.OriginX,(int)center.Y-frame.OriginY,1,1),pixel,4,0);
                    if(i%2==0?pixel[1]<240||pixel[2]>10:pixel[2]<240||pixel[1]>10)throw new InvalidOperationException($"Capture did not contain the synthetic fixture color: sample BGR={pixel[0]},{pixel[1]},{pixel[2]}, point={center.X},{center.Y}, origin={frame.OriginX},{frame.OriginY}, size={frame.Image.PixelWidth}x{frame.Image.PixelHeight}, fixtureVisible={fixture.IsVisible}.");
                    timer.Restart();overlay=new CaptureOverlayWindow(host,null,frame){ShowActivated=false,IsHitTestVisible=false};var constructionMs=timer.Elapsed.TotalMilliseconds;
                    timer.Restart();overlay.Show();await app.Dispatcher.InvokeAsync(()=>{},DispatcherPriority.ApplicationIdle);var showThroughIdleMs=timer.Elapsed.TotalMilliseconds;
                    samples.Add(new{iteration=i,cold=i==0,frame.Image.PixelWidth,frame.Image.PixelHeight,captureMs,constructionMs,showThroughIdleMs,fixtureColorVerified=true});
                    overlay.Close();overlay=null;
                    await app.Dispatcher.InvokeAsync(()=>{},DispatcherPriority.ApplicationIdle);
                }
            }
            catch(Exception error){failure=error.ToString();}
            finally
            {
                try{overlay?.Close();}catch(Exception error){failure??=error.ToString();}
                try{fixture.Close();}catch(Exception error){failure??=error.ToString();}
                try
                {
                    Directory.CreateDirectory(".codex-build/capture-pipeline");
                    File.WriteAllText(".codex-build/capture-pipeline/result.json",JsonSerializer.Serialize(new{passed=failure is null,samples,failure,desktopPixelsSaved=false,globalInputInjected=false,providerDiscoveryExcluded=true,hotkeyLatencyMeasured=false,gpuPresentationLatencyMeasured=false},new JsonSerializerOptions{WriteIndented=true}));
                }
                catch(Exception error){failure??=error.ToString();}
                finally{app.Shutdown(failure is null?0:1);}
            }
        };
        app.Run(fixture);
    }
}
