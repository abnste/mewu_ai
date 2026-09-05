using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Image=System.Windows.Controls.Image;
using Color=System.Windows.Media.Color;
using Brushes=System.Windows.Media.Brushes;

// Opt-in local replay of the real overlay. Does not start AppHost, load saved
// settings/history, connect to a Provider, or write user configuration.
internal static class Program
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;

    [STAThread]
    private static void Main(string[] args)
    {
#if !DEBUG
        throw new InvalidOperationException("The interaction harness must use Debug capture protection QA mode.");
#else
        Environment.SetEnvironmentVariable("MEWU_QA_CAPTURE_WINDOWS","1");
        var english=args.Contains("--english");
        typeof(AppHost).Assembly.GetType("mewu_ai_Assistant.Services.LocalizationService")!
            .GetMethod("Initialize",BindingFlags.Static|BindingFlags.NonPublic)!
            .Invoke(null,[english?"en-US":"zh-CN",null]);
        var app=new Application { ShutdownMode=ShutdownMode.OnMainWindowClose };
        app.Resources.MergedDictionaries.Add(new ResourceDictionary { Source=new Uri("/MewuAI;component/Themes/LightTheme.xaml",UriKind.Relative) });
        var host=new AppHost(app);
        var area=System.Windows.Forms.SystemInformation.VirtualScreen;
        var image=CreateSyntheticDesktop(area.Width,area.Height);
        var overlay=new CaptureOverlayWindow(host);
        Set(overlay,"_frame",new CaptureFrame(area.Left,area.Top,image));
        ((Image)overlay.FindName("DesktopImage")).Source=image;
        Set(overlay,"_conversationAiAvailable",true);
        ((FrameworkElement)overlay.FindName("PromptBarHost")).Visibility=Visibility.Visible;
        overlay.Title="Mewu Interaction QA";
        if(args.Contains("--verify-lifetime"))
        {
            VerifyResourceLifetime(overlay);
            overlay.Close();
            app.Shutdown();
            return;
        }
        overlay.Loaded+=(_,_)=>
        {
            Set(overlay,"_lastSubmittedPrompt",english?"Summarize these example interaction improvements.":"概括这次示例中的交互优化。");
            Invoke(overlay,"RefreshHistoryPreview");
            var answer=string.Empty;var index=0;
            var timer=new DispatcherTimer(DispatcherPriority.Background){Interval=TimeSpan.FromMilliseconds(500)};
            timer.Tick+=(_,_)=>
            {
                Invoke(overlay,"ShowAnswer");
                answer+=index==0
                    ?english?"## A calmer workspace\n\nThis is synthetic, local QA content. No model request is sent.\n\n":"## 更从容的屏幕工作区\n\n这是本地生成的验收内容，不会发送模型请求。\n\n"
                    :english?$"**Step {index:00}** · Read at your own pace. Scroll up while this response grows; use Jump to latest to resume following.\n\n":$"**第 {index:00} 项** · 按自己的节奏阅读。回复增长时向上滚动，查看之前的内容；点击回到最新回复可恢复跟随。\n\n";
                Invoke(overlay,"RefreshAnswer",answer);
                if(++index==120)timer.Stop();
            };
            timer.Start();
            // Keep the harness bounded if the operator leaves it open.
            var lifetime=new DispatcherTimer{Interval=TimeSpan.FromMinutes(5)};
            lifetime.Tick+=(_,_)=>{lifetime.Stop();timer.Stop();overlay.Close();};lifetime.Start();
            overlay.Closed+=(_,_)=>{timer.Stop();lifetime.Stop();};
        };
        app.Run(overlay);
#endif
    }

    private static void Set(object target,string field,object value)=>target.GetType().GetField(field,Private)!.SetValue(target,value);
    private static void Invoke(object target,string method,params object[] arguments)=>target.GetType().GetMethod(method,Private)!.Invoke(target,arguments);

    private static void VerifyResourceLifetime(CaptureOverlayWindow overlay)
    {
        var selections=(System.Collections.IList)Get(overlay,"_selections");
        var owned=Get(overlay,"_ownedSelections");
        var history=Get(overlay,"_overlayHistory");
        var registryType=typeof(AppHost).Assembly.GetType("mewu_ai_Assistant.Services.TempMediaRegistry")!;
        var registry=registryType.GetProperty("Shared",BindingFlags.Static|BindingFlags.NonPublic)!.GetValue(null)!;
        var initialLeases=(int)registryType.GetProperty("ActiveLeaseCount",Private)!.GetValue(registry)!;
        var layer=(Canvas)overlay.FindName("SelectionLayer");
        object Snapshot()=>overlay.GetType().GetMethod("CaptureOverlaySnapshot",Private)!.Invoke(overlay,null)!;
        int OwnedCount()=>(int)owned.GetType().GetProperty("Count")!.GetValue(owned)!;
        void DrainCleanup()
        {
            Invoke(overlay,"QueueSelectionResourceCleanup");
            var frame=new DispatcherFrame();
            overlay.Dispatcher.BeginInvoke(DispatcherPriority.SystemIdle,new Action(()=>frame.Continue=false));
            Dispatcher.PushFrame(frame);
        }

        for(var index=0;index<60;index++)
        {
            var before=Snapshot();
            var item=overlay.GetType().GetMethod("CreateSelection",Private)!.Invoke(overlay,[false])!;
            item.GetType().GetField("Bounds")!.SetValue(item,new Rect(80,80,120,100));
            var lease=registryType.GetMethod("Acquire",Private)!.Invoke(registry,[System.IO.Path.Combine(System.IO.Path.GetTempPath(),$"mewu-qa-{Guid.NewGuid():N}.mp4")]);
            item.GetType().GetField("VideoLease")!.SetValue(item,lease);
            selections.Add(item);Invoke(overlay,"RecordOverlayOperation",before,"create");
            before=Snapshot();selections.Clear();layer.Children.Clear();Invoke(overlay,"RecordOverlayOperation",before,"delete");
        }
        DrainCleanup();
        var retained=OwnedCount();
        if(retained!=25)throw new InvalidOperationException($"Expected 25 undoable regions; found {retained}.");
        history.GetType().GetMethod("TryUndo",Private)!.Invoke(history,[null,null]);
        DrainCleanup();
        if(OwnedCount()!=retained)throw new InvalidOperationException("Redo resources were released too early.");

        using var request=new CancellationTokenSource();Set(overlay,"_request",request);
        for(var index=0;index<60;index++)Invoke(overlay,"RecordOverlayOperation",Snapshot(),"expire");
        DrainCleanup();
        if(OwnedCount()!=retained)throw new InvalidOperationException("An in-flight request lost its rollback resources.");
        overlay.GetType().GetField("_request",Private)!.SetValue(overlay,null);
        DrainCleanup();
        var finalLeases=(int)registryType.GetProperty("ActiveLeaseCount",Private)!.GetValue(registry)!;
        if(OwnedCount()!=0||finalLeases!=initialLeases)throw new InvalidOperationException("Expired regions or video leases remain retained.");
        var output=System.IO.Path.GetFullPath(".codex-build/interaction-lifetime.json");
        System.IO.Directory.CreateDirectory(System.IO.Path.GetDirectoryName(output)!);
        System.IO.File.WriteAllText(output,System.Text.Json.JsonSerializer.Serialize(new{created=60,retainedForUndo=retained,afterExpiry=OwnedCount(),remainingTestLeases=finalLeases-initialLeases}),new System.Text.UTF8Encoding(false));
    }

    private static object Get(object target,string field)=>target.GetType().GetField(field,Private)!.GetValue(target)!;

    private static BitmapSource CreateSyntheticDesktop(int width,int height)
    {
        var visual=new DrawingVisual();
        using(var dc=visual.RenderOpen())
        {
            dc.DrawRectangle(new SolidColorBrush(Color.FromRgb(231,237,246)),null,new Rect(0,0,width,height));
            for(var row=0;row<3;row++)
                for(var column=0;column<4;column++)
                {
                    var rect=new Rect(60+column*280,60+row*200,250,165);
                    dc.DrawRoundedRectangle(Brushes.White,null,rect,16,16);
                    dc.DrawRoundedRectangle(new SolidColorBrush(Color.FromRgb((byte)(100+row*35),(byte)(130+column*20),220)),null,new Rect(rect.X+20,rect.Y+22,45,45),10,10);
                    dc.DrawRoundedRectangle(new SolidColorBrush(Color.FromRgb(216,223,234)),null,new Rect(rect.X+20,rect.Y+90,180,9),4,4);
                    dc.DrawRoundedRectangle(new SolidColorBrush(Color.FromRgb(232,236,243)),null,new Rect(rect.X+20,rect.Y+112,130,8),4,4);
                }
        }
        var image=new RenderTargetBitmap(width,height,96,96,PixelFormats.Pbgra32);image.Render(visual);image.Freeze();return image;
    }
}
