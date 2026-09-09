// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Security.Cryptography;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Brushes=System.Windows.Media.Brushes;
using Point=System.Windows.Point;
using TextBox=System.Windows.Controls.TextBox;
using HorizontalAlignment=System.Windows.HorizontalAlignment;
using KeyEventArgs=System.Windows.Input.KeyEventArgs;

// Explicit opt-in: a real region recording and the real overlay send path.
// Only this synthetic background is recorded. No user history/settings writes.
internal static class VideoWorkflowReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
    internal static void RunBackground(Application app)
    {
        var scene=new Border{Background=Brushes.Red};
        var label=new TextBlock{Text="视频验收：红 → 蓝 → 绿",FontSize=32,Foreground=Brushes.White,Margin=new Thickness(40)};scene.Child=label;
        var window=new Window{Title="视频验收背景（仅合成内容）",WindowState=WindowState.Maximized,WindowStyle=WindowStyle.None,Content=scene,Topmost=true};
        var clock=Stopwatch.StartNew();var timer=new DispatcherTimer{Interval=TimeSpan.FromMilliseconds(100)};
        timer.Tick+=(_,_)=>{var time=clock.Elapsed.TotalSeconds%12;scene.Background=time<4?Brushes.Red:time<8?Brushes.Blue:Brushes.Lime;label.Text=$"视频验收 · 合成色块 · 每四秒变色 · {time:0.0}";};
        window.KeyDown+=(_,e)=>{if(e.Key==Key.Escape)window.Close();};window.Closed+=(_,_)=>timer.Stop();timer.Start();app.Run(window);
    }
    internal static void Run(Application app,AppHost host,bool liveProvider,bool verifyPin=false)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        Directory.CreateDirectory(".codex-build");
        File.WriteAllText(".codex-build/video-workflow-result.json",JsonSerializer.Serialize(new{checks=Array.Empty<string>(),failure="Replay has not completed"}));
        if(liveProvider)
        {
            using var json=JsonDocument.Parse(File.ReadAllText(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","settings.json")));
            var options=new JsonSerializerOptions();options.Converters.Add(new JsonStringEnumConverter());
            var provider=json.RootElement.GetProperty("Providers").EnumerateArray().First(p=>p.GetProperty("Type").GetString()=="MiniMax"&&p.GetProperty("Model").GetString()=="MiniMax-M3").Deserialize<AiProviderSettings>(options)!;
            host.Settings.Providers=[provider];host.Settings.DefaultProviderId=provider.Id;host.Settings.ConversationChannelId="api:"+provider.Id;
        }
        host.Settings.RecordSystemAudio=false;host.Settings.RecordMicrophone=false;host.Settings.SaveConversationHistory=false;host.Settings.IncludeRecordingCursor=false;
        var scene=new Border{Background=Brushes.Red};
        var label=new TextBlock{Text="视频验收 · 合成色块",FontSize=26,Foreground=Brushes.White,HorizontalAlignment=HorizontalAlignment.Center,VerticalAlignment=VerticalAlignment.Center};scene.Child=label;
        var background=new Window{Title="喵呜AI 视频验收背景",WindowState=WindowState.Maximized,WindowStyle=WindowStyle.None,Content=scene,Topmost=true};background.Show();
        CaptureOverlayWindow? overlay=null;
        app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;
            try
            {
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);background.UpdateLayout();
                overlay=new CaptureOverlayWindow(host){Title="喵呜AI 视频录制验收"};overlay.Show();overlay.UpdateLayout();
                Program.MarkReplayWindow(overlay,"视频验收：真实录制 → 输入 → 时间回答 → 复制");
                if(!liveProvider){Set("_conversationAiAvailable",true);((FrameworkElement)overlay.FindName("PromptBarHost")).Visibility=Visibility.Visible;}
                var item=Invoke("CreateSelection",false)!;item.GetType().GetField("Bounds")!.SetValue(item,new Rect(160,120,480,280));((IList)Get("_selections")!).Add(item);Invoke("Select",0);Invoke("UpdateSelection",item);
                Invoke("Record",overlay,new RoutedEventArgs());
                await Until(()=>Get("_recordingSession") is not null,"recorder-start",30);
                var session=Get("_recordingSession")!;
                await ((Task)session.GetType().GetProperty("RecordingReady",BindingFlags.Instance|BindingFlags.NonPublic)!.GetValue(session)!).WaitAsync(TimeSpan.FromSeconds(25));
                var clock=Stopwatch.StartNew();
                await Task.Delay(3000);scene.Background=Brushes.Blue;label.Text="视频验收 · 已变蓝";
                await Task.Delay(3000);scene.Background=Brushes.Lime;label.Text="视频验收 · 已变绿";
                await Task.Delay(2000);Invoke("StopRecording",overlay,new RoutedEventArgs());
                await Until(()=>Get("_recordingSession") is null&&item.GetType().GetField("VideoPreview")!.GetValue(item) is not null,"recording-completed",35);
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                var prompt=(TextBox)overlay.FindName("QuickPrompt");
                Invoke("UpdatePointerInteraction",new Point(300,240));
                Require(prompt.IsKeyboardFocused&&!(bool)Get("_promptBarHidden")!,"recording stop + video hover lost input focus");checks.Add("recording-stop-focus-survives-hover");
                prompt.RaiseEvent(new TextCompositionEventArgs(Keyboard.PrimaryDevice,new TextComposition(InputManager.Current,prompt,"输入验收")){RoutedEvent=TextCompositionManager.TextInputEvent});
                Require(prompt.Text=="输入验收","typing after stop failed");checks.Add("typing-after-stop");
                Invoke("ToggleVideoPlayback",overlay,new RoutedEventArgs());await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                Require(prompt.IsKeyboardFocused&&!(bool)Get("_promptBarHidden")!,"pause preview lost focus");checks.Add("pause-preview-focus");
                var video=(string)item.GetType().GetField("VideoPath")!.GetValue(item)!;
                var before=SHA256.HashData(await File.ReadAllBytesAsync(video));
                if(verifyPin){await VideoPinReplay.VerifyAsync(overlay,item,video);checks.Add("raw-annotated-pin-play-resize-topmost-close-source-unchanged");}
                // Pause here only for the explicit GUI keyboard/clipboard check.
                prompt.Text="请分别说明画面何时从红变蓝、从蓝变绿，时间必须为视频开头起算的秒数，并为这两个变色事件各返回一条单点时间轴批注。";
                if(liveProvider)
                {
                    ((System.Windows.Controls.Button)overlay.FindName("SendButton")).RaiseEvent(new RoutedEventArgs(System.Windows.Controls.Primitives.ButtonBase.ClickEvent));
                    await Until(()=>Get("_request") is null,"video-answer",210);
                    var answer=(MarkdownAnswerView)overlay.FindName("AnswerText");
                    Require(!string.IsNullOrWhiteSpace(answer.PlainText),"empty video answer");
                    var notes=(IEnumerable)item.GetType().GetProperty("AnnotationNotes")!.GetValue(item)!;
                    var times=notes.Cast<AiAnnotation>().Where(n=>n.IsVideoTimeline).Select(n=>n.StartTime!.Value).ToArray();
                    Require(times.Any(t=>t>=2&&t<=4.5)&&times.Any(t=>t>=5&&t<=7.5),"video timeline did not locate both changes: "+string.Join(',',times));
                    checks.Add("live-video-answer-two-correct-event-times");
                    var after=SHA256.HashData(await File.ReadAllBytesAsync(video));Require(before.SequenceEqual(after),"source MP4 changed");checks.Add("source-hash-unchanged");
                    // Validate the window's tunnel routing while a request is
                    // pending; OS Ctrl+C is separately exercised via the UI.
                    answer.Focus();answer.SelectAll();using var pending=new CancellationTokenSource();Set("_request",pending);
                    var key=new KeyEventArgs(Keyboard.PrimaryDevice,PresentationSource.FromVisual(overlay)!,Environment.TickCount,Key.C){RoutedEvent=Keyboard.PreviewKeyDownEvent};answer.RaiseEvent(key);
                    Require(!key.Handled,"window swallowed answer copy shortcut while busy");Set("_request",null);
                    Require(answer.SelectedPlainText==answer.PlainText,"selected answer lost text");checks.Add("answer-copy-routing-while-busy");
                    Directory.CreateDirectory(".codex-build");File.WriteAllText(".codex-build/video-workflow-answer.txt",answer.PlainText);
                }
                label.Text="视频验收完成";
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally
            {
                Directory.CreateDirectory(".codex-build");File.WriteAllText(".codex-build/video-workflow-result.json",JsonSerializer.Serialize(new{checks,failure}));
                foreach(var pin in Application.Current.Windows.OfType<PinnedVideoWindow>().ToArray())pin.Close();
                overlay?.Close();background.Close();app.Shutdown(Environment.ExitCode);
            }
            object? Get(string name)=>typeof(CaptureOverlayWindow).GetField(name,Private)!.GetValue(overlay);
            void Set(string name,object? value)=>typeof(CaptureOverlayWindow).GetField(name,Private)!.SetValue(overlay,value);
            object? Invoke(string name,params object[] values)=>typeof(CaptureOverlayWindow).GetMethod(name,Private)!.Invoke(overlay,values);
            async Task Until(Func<bool> condition,string stage,int seconds)
            {
                var timer=Stopwatch.StartNew();while(!condition()){if(timer.Elapsed.TotalSeconds>seconds)throw new TimeoutException(stage);await Task.Delay(50);}
            }
        }));
        app.Run();
    }
    private static void Require(bool condition,string reason){if(!condition)throw new InvalidOperationException(reason);}
}
