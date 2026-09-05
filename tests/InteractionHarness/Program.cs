using System.Reflection;
using System.IO;
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
        var verifyTeaching=args.Contains("--verify-teaching");
        var teaching=args.Contains("--teaching")||verifyTeaching;
#if !DEBUG
        if(!teaching)throw new InvalidOperationException("Release replay requires explicit --teaching or --verify-teaching.");
#else
        Environment.SetEnvironmentVariable("MEWU_QA_CAPTURE_WINDOWS",teaching?null:"1");
#endif
        var english=args.Contains("--english");
        typeof(AppHost).Assembly.GetType("mewu_ai_Assistant.Services.LocalizationService")!
            .GetMethod("Initialize",BindingFlags.Static|BindingFlags.NonPublic)!
            .Invoke(null,[english?"en-US":"zh-CN",null]);
        var app=new Application { ShutdownMode=ShutdownMode.OnMainWindowClose };
        app.Resources.MergedDictionaries.Add(new ResourceDictionary { Source=new Uri("/MewuAI;component/Themes/LightTheme.xaml",UriKind.Relative) });
        var host=new AppHost(app);
        host.Settings.TeachingMode=teaching;
        var area=System.Windows.Forms.SystemInformation.VirtualScreen;
        var image=CreateSyntheticDesktop(area.Width,area.Height);
        Window? background=null;
        if(verifyTeaching)
        {
            app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
            background=new Window { Title="Mewu Teaching QA Background",WindowStyle=WindowStyle.None,WindowState=WindowState.Maximized,Background=Brushes.Lime,ShowInTaskbar=false };
            background.Show();
            image=CreateSolidDesktop(area.Width,area.Height,Brushes.Magenta);
        }
        var overlay=new CaptureOverlayWindow(host);
        Set(overlay,"_frame",new CaptureFrame(area.Left,area.Top,image));
        ((Image)overlay.FindName("DesktopImage")).Source=image;
        Set(overlay,"_conversationAiAvailable",true);
        ((FrameworkElement)overlay.FindName("PromptBarHost")).Visibility=Visibility.Visible;
        overlay.Title="Mewu Interaction QA";
        overlay.ShowInTaskbar=true;
        if(args.Contains("--verify-lifetime"))
        {
            VerifyResourceLifetime(overlay);
            overlay.Close();
            app.Shutdown();
            return;
        }
        if(args.Contains("--verify-answer-alignment"))
        {
            VerifyAnswerAlignment(app,overlay);app.Run(overlay);return;
        }
        if(args.Contains("--benchmark"))
        {
            BenchmarkAnswer(app,overlay,args.Contains("--after"));app.Run(overlay);return;
        }
        if(verifyTeaching)
        {
            VerifyTeachingOnLoad(app,host,overlay,background!);
            app.Run(overlay);
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
    }

    private static void VerifyAnswerAlignment(Application app,CaptureOverlayWindow overlay)
    {
        overlay.Loaded+=(_,_)=>overlay.Dispatcher.BeginInvoke(DispatcherPriority.ContextIdle,new Action(()=>
        {
            try
            {
                Invoke(overlay,"ShowAnswer");
                Invoke(overlay,"RefreshAnswer",string.Concat(Enumerable.Repeat("用于测量滚动条与小箭头中心的合成回答。\n\n",100)));
                var answer=(MarkdownAnswerView)overlay.FindName("AnswerText");
                var button=(System.Windows.Controls.Button)overlay.FindName("LatestAnswerButton");
                var host=(FrameworkElement)overlay.FindName("PromptBarHost");
                overlay.UpdateLayout();
                foreach(var width in new[]{574d,520d,640d})
                {
                    host.Width=width;
                    button.Visibility=Visibility.Visible;
                    overlay.UpdateLayout();
                    var bar=Descendants(answer).OfType<System.Windows.Controls.Primitives.ScrollBar>().Single(b=>b.Orientation==System.Windows.Controls.Orientation.Vertical);
                    var track=(System.Windows.Controls.Primitives.Track)bar.Template.FindName("PART_Track",bar);
                    var glyph=(FrameworkElement)button.Template.FindName("ArrowGlyph",button);
                    double Center(FrameworkElement element)=>element.TransformToAncestor(overlay).Transform(new System.Windows.Point(element.ActualWidth/2,element.ActualHeight/2)).X;
                    Rect Bounds(FrameworkElement element)=>element.TransformToAncestor(overlay).TransformBounds(new Rect(element.RenderSize));
                    var delta=Math.Abs(Center(track.Thumb)-Center(glyph));
                    Console.WriteLine($"width={width}; thumbCenter={Center(track.Thumb):F3}; arrowCenter={Center(glyph):F3}; delta={delta:F3} DIP");
                    if(delta>.1)throw new InvalidOperationException("小箭头与实际滚动滑块中心不一致。");
                    var beforeAnswer=Bounds(answer);var beforeHost=Bounds(host);
                    button.Visibility=Visibility.Collapsed;overlay.UpdateLayout();
                    if(Bounds(answer)!=beforeAnswer||Bounds(host)!=beforeHost)throw new InvalidOperationException("箭头显隐改变正文或对话条布局。");
                }
                Console.WriteLine("PASS: arrow alignment and stable layout at three widths.");
            }
            catch(Exception ex){Console.Error.WriteLine(ex.Message);Environment.ExitCode=1;}
            finally {overlay.Close();app.Shutdown();}
        }));
    }

    private static IEnumerable<DependencyObject> Descendants(DependencyObject root)
    {
        for(var i=0;i<VisualTreeHelper.GetChildrenCount(root);i++)
        {
            var child=VisualTreeHelper.GetChild(root,i);yield return child;
            foreach(var descendant in Descendants(child))yield return descendant;
        }
    }

    private static void BenchmarkAnswer(Application app,CaptureOverlayWindow overlay,bool after)
    {
        overlay.Loaded+=(_,_)=>
        {
            var calls=new List<double>();var gaps=new List<double>();var text=new System.Text.StringBuilder();
            var watch=System.Diagnostics.Stopwatch.StartNew();var previous=watch.Elapsed.TotalMilliseconds;var index=0;var worstGap=0d;var worstGapUpdate=0;
            var heartbeat=new DispatcherTimer(DispatcherPriority.Input){Interval=TimeSpan.FromMilliseconds(16)};
            heartbeat.Tick+=(_,_)=>{var now=watch.Elapsed.TotalMilliseconds;var gap=now-previous;gaps.Add(gap);if(gap>worstGap){worstGap=gap;worstGapUpdate=index;}previous=now;};
            var timer=new DispatcherTimer(DispatcherPriority.Background){Interval=TimeSpan.FromMilliseconds(80)};
            timer.Tick+=(_,_)=>
            {
                var start=watch.Elapsed.TotalMilliseconds;
                text.Append($"**示例 {index}**：这是合成测试文字，用来测量长回答更新时的输入延迟。保留已显示的段落，继续阅读和选择文字。\n\n");
                Invoke(overlay,"ShowAnswer");Invoke(overlay,"RefreshAnswer",text.ToString());
                calls.Add(watch.Elapsed.TotalMilliseconds-start);
                if(++index<160)return;
                timer.Stop();heartbeat.Stop();
                double P95(List<double> values)=>values.Order().ElementAt((int)((values.Count-1)*.95));
                var result=new {Updates=index,Characters=text.Length,UpdateP95Ms=P95(calls),UpdateMaxMs=calls.Max(),HeartbeatP95Ms=P95(gaps),HeartbeatMaxMs=gaps.Max(),WorstGapAtUpdate=worstGapUpdate,ElapsedMs=watch.Elapsed.TotalMilliseconds};
                Directory.CreateDirectory(".codex-build");File.WriteAllText(Path.Combine(".codex-build",after?"answer-after.json":"answer-before.json"),System.Text.Json.JsonSerializer.Serialize(result));
                overlay.Close();app.Shutdown();
            };
            heartbeat.Start();timer.Start();
        };
    }

    private static object Native(string name,params object[] args)=>typeof(AppHost).Assembly
        .GetType("mewu_ai_Assistant.Interop.NativeMethods")!
        .GetMethod(name,BindingFlags.Static|BindingFlags.NonPublic)!.Invoke(null,args)!;

    private static void VerifyTeachingOnLoad(Application app,AppHost host,CaptureOverlayWindow overlay,Window background)
    {
        overlay.Loaded+=(_,_)=>
        {
            var timer=new DispatcherTimer {Interval=TimeSpan.FromMilliseconds(500)};
            timer.Tick+=(_,_)=>
            {
                timer.Stop();
                var results=new Dictionary<string,object>();
                void Check(string name,bool passed){results[name]=passed;if(!passed)throw new InvalidOperationException(name);}
                try
                {
                    var handle=new System.Windows.Interop.WindowInteropHelper(overlay).Handle;
                    var screen=System.Windows.Forms.Screen.PrimaryScreen!.Bounds;
                    var x=screen.Left+screen.Width/2;var y=screen.Top+screen.Height/2;
                    bool IsGreen(CaptureFrame frame)
                    {
                        var pixel=new byte[4];frame.Image.CopyPixels(new Int32Rect(x-frame.OriginX,y-frame.OriginY,1,1),pixel,4,0);
                        return pixel[1]>240&&pixel[0]<12&&pixel[2]<12;
                    }
                    Check("teachingOverlayVisible",(bool)Native("IsVisibleToCapture",handle));
                    Check("recordingProtectionFalse",!(bool)Get(overlay,"_captureExclusionVerified"));
                    Native("FlushComposition");
                    Check("desktopCaptureIncludesOverlay",!IsGreen(new ScreenCaptureService().CaptureDesktop()));
                    var clean=(CaptureFrame)overlay.GetType().GetMethod("CaptureCleanDesktopForRefresh",Private)!.Invoke(overlay,null)!;
                    Check("refreshExcludesOwnOverlay",IsGreen(clean));
                    Check("sharingRestoredAfterRefresh",(bool)Native("IsVisibleToCapture",handle));
                    Invoke(overlay,"Record",overlay,new RoutedEventArgs());
                    Check("recordingBlocked",Get(overlay,"_recordingSession") is null&&!(bool)Get(overlay,"_recordingCountdownActive"));
                    var settings=new SettingsWindow(host);
                    try
                    {
                        var settingsHandle=new System.Windows.Interop.WindowInteropHelper(settings).EnsureHandle();
                        Check("settingsStillProtected",(bool)Native("IsExcludedFromCapture",settingsHandle));
                    }
                    finally { settings.Close(); }
                    var image=CreateSolidDesktop(80,80,Brushes.SkyBlue);
                    var pin=new PinnedImageWindow(image,new ScreenRect(screen.Left+80,screen.Top+80,80,80),true);
                    try
                    {
                        pin.Show();
                        Check("pinVisibleToSharing",(bool)Native("IsVisibleToCapture",new System.Windows.Interop.WindowInteropHelper(pin).Handle));
                        Check("visiblePinNotCompositedTwice",pin.GetType().GetMethod("CreateCaptureSnapshot",Private)!.Invoke(pin,null) is null);
                    }
                    finally { pin.Close(); }
                    var protectedPin=new PinnedImageWindow(image);
                    try
                    {
                        var pinHandle=new System.Windows.Interop.WindowInteropHelper(protectedPin).EnsureHandle();
                        Check("ordinaryPinStillProtected",(bool)Native("IsExcludedFromCapture",pinHandle));
                    }
                    finally { protectedPin.Close(); }
                    Check("allPassed",true);
                }
                catch(Exception error){results["allPassed"]=false;results["error"]=error.ToString();Environment.ExitCode=1;}
                finally
                {
                    var directory=Path.Combine(Environment.CurrentDirectory,".codex-build");Directory.CreateDirectory(directory);
                    File.WriteAllText(Path.Combine(directory,"teaching-verification.json"),System.Text.Json.JsonSerializer.Serialize(results,new System.Text.Json.JsonSerializerOptions{WriteIndented=true}),new System.Text.UTF8Encoding(false));
                    overlay.Close();background.Close();app.Shutdown(Environment.ExitCode);
                }
            };
            timer.Start();
        };
    }

    private static BitmapSource CreateSolidDesktop(int width,int height,System.Windows.Media.Brush brush)
    {
        var visual=new DrawingVisual();using(var dc=visual.RenderOpen())dc.DrawRectangle(brush,null,new Rect(0,0,width,height));
        var image=new RenderTargetBitmap(width,height,96,96,PixelFormats.Pbgra32);image.Render(visual);image.Freeze();return image;
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
