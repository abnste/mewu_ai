// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Size=System.Windows.Size;

internal static class ScreenEntityScanReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;

    internal static void Run()
    {
        var directory=ReplayOutputDirectory.PrepareWorkingDirectory("screen-entity-scan");
        LocalizationService.Initialize("en-US",null);
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(directory,"logs"));
        var app=new Application{ShutdownMode=ShutdownMode.OnExplicitShutdown};
        var previousContext=SynchronizationContext.Current;
        SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext(app.Dispatcher));
        app.Resources.MergedDictionaries.Add(new ResourceDictionary{Source=new Uri("/MewuAI;component/Themes/LightTheme.xaml",UriKind.Relative)});
        using var host=new AppHost(app,null,"MewuAI-ScreenEntityScan-"+Guid.NewGuid().ToString("N"));
        host.Settings.EnableVoiceInput=false;host.Settings.AutomaticallyStartListening=false;host.Settings.SaveConversationHistory=false;
        var source=BitmapSource.Create(400,300,96,96,PixelFormats.Bgra32,null,new byte[400*300*4],400*4);source.Freeze();
        var frame=new CaptureFrame(0,0,source);
        var overlay=new CaptureOverlayWindow(host,null,frame);
        var checks=new List<string>();string? failure=null;
        object? Invoke(string name,params object[] values)=>typeof(CaptureOverlayWindow).GetMethod(name,Private)!.Invoke(overlay,values);
        void Set(string name,object value)=>typeof(CaptureOverlayWindow).GetField(name,Private)!.SetValue(overlay,value);
        void Check(bool value,string label){if(!value)throw new InvalidOperationException(label);checks.Add(label);}
        try
        {
            var root=(Canvas)overlay.FindName("Root");root.Width=400;root.Height=300;
            root.Measure(new Size(400,300));root.Arrange(new Rect(0,0,400,300));root.UpdateLayout();
            var item=Invoke("CreateSelection",false)!;
            var fields=item.GetType();
            var selections=(IList)typeof(CaptureOverlayWindow).GetField("_selections",Private)!.GetValue(overlay)!;
            var pending=(IDictionary)typeof(CaptureOverlayWindow).GetField("_screenTextInFlight",Private)!.GetValue(overlay)!;
            var bounds=new Rect(20,20,180,120);
            selections.Add(item);Set("_activeIndex",0);fields.GetField("Bounds")!.SetValue(item,bounds);
            string? Text()=>fields.GetField("SnapshotText")!.GetValue(item) as string;
            void Clear()=>fields.GetField("SnapshotText")!.SetValue(item,null);
            Task Start(TaskCompletionSource<string?> completion,Action<CancellationToken>? observe=null)
            {
                Func<CancellationToken,Task<string?>> reader=token=>{observe?.Invoke(token);return completion.Task;};
                return (Task)Invoke("BeginScreenEntityScan",item,reader)!;
            }
            static TaskCompletionSource<string?> NewResult()=>new(TaskCreationOptions.RunContinuationsAsynchronously);

            var first=NewResult();CancellationToken firstToken=default;var firstTask=Start(first,token=>firstToken=token);
            Invoke("ClearImageDerivedLayers",item);
            Check(firstToken.IsCancellationRequested,"content invalidation cancels the old scan");
            var second=NewResult();var secondTask=Start(second);
            first.SetResult("old@example.org");Complete(firstTask);
            Check(Text() is null&&pending.Count==1,"old completion neither publishes stale text nor removes the replacement request");
            second.SetResult("current text");Complete(secondTask);
            Check(Text()=="current text"&&pending.Count==0,"replacement completes and releases only its own request");

            Clear();var changedBounds=NewResult();var boundsTask=Start(changedBounds);
            fields.GetField("Bounds")!.SetValue(item,new Rect(30,20,180,120));
            changedBounds.SetResult("old bounds");Complete(boundsTask);
            Check(Text() is null,"geometry changed without a matching source identity rejects old text");
            fields.GetField("Bounds")!.SetValue(item,bounds);

            var changedFrame=NewResult();var frameTask=Start(changedFrame);
            Set("_frame",new CaptureFrame(0,0,source));changedFrame.SetResult("old frame");Complete(frameTask);
            Check(Text() is null,"desktop refresh rejects a result from the previous frame");Set("_frame",frame);

            var changedImage=NewResult();var imageTask=Start(changedImage);
            fields.GetField("CapturedImageOverride")!.SetValue(item,source);changedImage.SetResult("old source");Complete(imageTask);
            Check(Text() is null,"replacement image rejects a result from the prior source");fields.GetField("CapturedImageOverride")!.SetValue(item,null);

            var oldSnapshot=NewResult();CancellationToken snapshotToken=default;var snapshotTask=Start(oldSnapshot,token=>snapshotToken=token);
            var snapshot=Invoke("CaptureOverlaySnapshot")!;Invoke("ApplyOverlaySnapshot",snapshot);
            Check(snapshotToken.IsCancellationRequested,"restoring an identical snapshot invalidates the old request identity");
            oldSnapshot.SetResult("old snapshot");Complete(snapshotTask);Check(Text() is null,"same-geometry history restore cannot accept an earlier scan");

            var inactive=NewResult();var inactiveTask=Start(inactive);Set("_activeIndex",-1);
            inactive.SetResult("inactive valid text");Complete(inactiveTask);
            Check(Text()=="inactive valid text","valid inactive selection retains its own text without becoming the active selection");
            Check((int)typeof(CaptureOverlayWindow).GetField("_activeIndex",Private)!.GetValue(overlay)! == -1,"background completion does not change active selection");
            Set("_activeIndex",0);Clear();

            var removed=NewResult();var removedTask=Start(removed);selections.Remove(item);
            removed.SetResult("removed selection");Complete(removedTask);Check(Text() is null,"removed selection never receives a late result");selections.Add(item);

            var deliveries=0;var sentCallbacks=0;
            var compose=new MailComposeWindow(null,"sample@example.org","Synthetic compose",(_,_,_)=>
                Task.FromResult(++deliveries==1?new MailDeliveryResult(false,"synthetic refusal"):new MailDeliveryResult(false,"synthetic unconfirmed",true)));
            try
            {
                var composeType=typeof(MailComposeWindow);
                ((System.Windows.Controls.TextBox)composeType.GetField("_body",Private)!.GetValue(compose)!).Text="Synthetic body";
                var sendButton=(System.Windows.Controls.Button)composeType.GetField("_send",Private)!.GetValue(compose)!;
                var status=(TextBlock)composeType.GetField("_status",Private)!.GetValue(compose)!;
                compose.OnSent=()=>sentCallbacks++;
                Task Send()=>(Task)composeType.GetMethod("SendAsync",Private)!.Invoke(compose,null)!;
                Complete(Send());Check(sendButton.IsEnabled&&deliveries==1&&sentCallbacks==0,"explicit mail refusal allows correction without claiming success");
                Complete(Send());Check(!sendButton.IsEnabled&&deliveries==2&&sentCallbacks==0&&status.Text=="synthetic unconfirmed","unknown delivery remains neutral and disables repeat send without OnSent");
                Complete(Send());Check(deliveries==2,"unknown delivery cannot be resent through the command handler");
                Check(new WindowInteropHelper(compose).Handle==IntPtr.Zero&&!compose.IsVisible,"mail-state checks use an unshown window and a fake sender");
            }
            finally{compose.Close();}

            var crawlResult=NewResult();var crawlCalls=0;var displayedResults=0;CancellationToken crawlToken=default;
            Func<CancellationToken,Task> crawl=async token=>
            {
                crawlCalls++;crawlToken=token;
                await crawlResult.Task;
                token.ThrowIfCancellationRequested();
                displayedResults++;
            };
            var crawlTask=(Task)Invoke("RunScreenEntityCrawlAsync",crawl)!;
            var duplicateCrawl=(Task)Invoke("RunScreenEntityCrawlAsync",crawl)!;Complete(duplicateCrawl);
            Check(crawlCalls==1,"the crawl gate includes the initial asynchronous operation and rejects duplicate clicks");
            crawlResult.SetResult("basic result");Complete(crawlTask);
            Check(displayedResults==1&&!(bool)typeof(CaptureOverlayWindow).GetField("_scraplingCrawlInFlight",Private)!.GetValue(overlay)!,"completed crawl releases its gate exactly once");
            crawlResult=NewResult();var closingCrawl=(Task)Invoke("RunScreenEntityCrawlAsync",crawl)!;

            var shutdown=NewResult();CancellationToken shutdownToken=default;var shutdownTask=Start(shutdown,token=>shutdownToken=token);
            Check(new WindowInteropHelper(overlay).Handle==IntPtr.Zero&&!overlay.IsVisible,"all scan checks run without a native window or desktop input");
            overlay.Close();Check(shutdownToken.IsCancellationRequested,"real overlay Close cancels its pending scan");
            Check(crawlToken.IsCancellationRequested,"real overlay Close cancels the initial crawl operation");
            crawlResult.SetResult("late crawl result");Complete(closingCrawl);
            Check(displayedResults==1&&!(bool)typeof(CaptureOverlayWindow).GetField("_scraplingCrawlInFlight",Private)!.GetValue(overlay)!,"cancelled crawl releases gate and does not consume a late result");
            Complete((Task)Invoke("RunScreenEntityCrawlAsync",crawl)!);
            Check(crawlCalls==2,"closed overlay cannot start another crawl");
            shutdown.SetResult("after close");Complete(shutdownTask);
            Check(Text() is null&&pending.Count==0,"late closed-window completion releases its request without publishing text");
            Check(host.IsIsolatedReplay,"replay uses an isolated host without loading production settings");
        }
        catch(Exception error){failure=error.ToString();}
        finally{overlay.Close();app.Shutdown();SynchronizationContext.SetSynchronizationContext(previousContext);}
        using var assembly=File.OpenRead(typeof(CaptureOverlayWindow).Assembly.Location);
        File.WriteAllText(Path.Combine(directory,"screen-entity-result.json"),JsonSerializer.Serialize(new
        {
            passed=failure is null,checks,failure,productSha256=Convert.ToHexString(SHA256.HashData(assembly)),
            actualDesktopInput=false,nativeWindowCreated=false,realOcrOrAccountsUsed=false
        },new JsonSerializerOptions{WriteIndented=true}),new UTF8Encoding(false));
        Environment.ExitCode=failure is null?0:1;
    }

    private static void Complete(Task task)
    {
        if(task.IsCompleted){task.GetAwaiter().GetResult();return;}
        var loop=new DispatcherFrame();var watch=Stopwatch.StartNew();
        var timer=new DispatcherTimer(DispatcherPriority.Send){Interval=TimeSpan.FromMilliseconds(20)};
        timer.Tick+=(_,_)=>{if(task.IsCompleted||watch.Elapsed>TimeSpan.FromSeconds(10))loop.Continue=false;};
        timer.Start();try{Dispatcher.PushFrame(loop);}finally{timer.Stop();}
        if(!task.IsCompleted)throw new TimeoutException("Synthetic screen-entity request did not settle.");
        task.GetAwaiter().GetResult();
    }
}
