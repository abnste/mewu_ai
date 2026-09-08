// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Brushes=System.Windows.Media.Brushes;
using Button=System.Windows.Controls.Button;
using Point=System.Windows.Point;

internal static class CaptureToolsReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic|BindingFlags.DeclaredOnly;
    internal static void Run(Application app,AppHost host)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        host.Settings.RecordSystemAudio=false;host.Settings.RecordMicrophone=false;
        var rows=new StackPanel();
        for(var i=0;i<100;i++)rows.Children.Add(new Border{Height=50,Background=i%2==0?Brushes.LightBlue:Brushes.LightYellow,Child=new TextBlock{Text=$"Capture test row {i:000}",FontSize=24,Margin=new Thickness(35,5,0,0)}});
        var scroll=new ScrollViewer{Content=rows};
        var background=new Window{Title="Mewu capture tools fixture",WindowStyle=WindowStyle.None,WindowState=WindowState.Maximized,Topmost=true,Content=scroll};
        background.Show();
        CaptureOverlayWindow? overlay=null;
        app.Dispatcher.BeginInvoke(DispatcherPriority.Normal,new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;object? state=null;
            try
            {
                background.UpdateLayout();await Dispatcher.Yield(DispatcherPriority.Background);
                overlay=new CaptureOverlayWindow(host);overlay.Show();overlay.UpdateLayout();
                ((FrameworkElement)overlay.FindName("TeachingBadge")).Visibility=Visibility.Visible;
                ((TextBlock)overlay.FindName("TeachingBadgeText")).Text="功能验收窗口 · 完成后自动关闭";
                var item=Invoke("CreateSelection",false)!;item.GetType().GetField("Bounds")!.SetValue(item,new Rect(100,150,360,240));
                ((IList)Get("_selections")).Add(item);Invoke("Select",0);Invoke("UpdatePointerInteraction",new Point(200,220));overlay.UpdateLayout();
                state=new{protectedWindow=Get("_captureExclusionVerified"),recordEnabled=((Button)overlay.FindName("RecordButton")).IsEnabled,longEnabled=((Button)overlay.FindName("LongCaptureButton")).IsEnabled};
                Click("LongCaptureButton");
                await Until(()=>(bool)Get("_longCaptureMode")&&((IList)Get("_longCaptureFrames")).Count>0,"Long screenshot did not start");
                checks.Add("long-capture-button-starts-live-session");
                Invoke("CancelLongCaptureSession","QA complete");
                Require(!(bool)Get("_longCaptureMode"),"Long capture cancellation did not restore the screenshot");
                checks.Add("long-capture-cancels-and-restores-screenshot");
                Invoke("UpdatePointerInteraction",new Point(200,220));overlay.UpdateLayout();Click("RecordButton");
                Require((bool)Get("_recordingCountdownActive"),"Record button did not enter countdown");
                await Until(()=>GetOrNull("_recordingSession") is not null,"Recording did not start after countdown");
                var session=Get("_recordingSession");
                var ready=(Task)session.GetType().GetProperty("RecordingReady",BindingFlags.Instance|BindingFlags.NonPublic)!.GetValue(session)!;
                await ready.WaitAsync(TimeSpan.FromSeconds(25));checks.Add("record-button-counts-down-and-starts-recorder");
                await Task.Delay(1300);
                Invoke("StopRecording",overlay,new RoutedEventArgs());
                await Until(()=>item.GetType().GetProperty("VideoPath")?.GetValue(item) is string||item.GetType().GetField("VideoPath")?.GetValue(item) is string,"Recording did not produce video",30);
                checks.Add("recording-stops-and-produces-video");
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally
            {
                overlay?.Close();background.Close();Directory.CreateDirectory(".codex-build");
                File.WriteAllText(".codex-build/capture-tools-result.json",JsonSerializer.Serialize(new{checks,state,failure}));app.Shutdown(Environment.ExitCode);
            }
            object Get(string field)=>GetOrNull(field)!;
            object? GetOrNull(string field)=>overlay!.GetType().GetField(field,Private)!.GetValue(overlay);
            object? Invoke(string method,params object[] args)=>overlay!.GetType().GetMethod(method,Private)!.Invoke(overlay,args);
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
}
