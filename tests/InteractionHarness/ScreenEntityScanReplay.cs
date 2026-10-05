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
using QRCoder;
using Application=System.Windows.Application;
using Size=System.Windows.Size;
using Button=System.Windows.Controls.Button;
using Panel=System.Windows.Controls.Panel;
using Point=System.Windows.Point;
using Color=System.Windows.Media.Color;

internal static class ScreenEntityScanReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;

    internal static void Run(string[] args)
    {
        var directory=ReplayOutputDirectory.PrepareWorkingDirectory("screen-entity-scan");
        var english=!args.Contains("--chinese");
        var nextLabel=english?"Next code":"下一个二维码";
        var previousLabel=english?"Previous code":"上一个二维码";
        LocalizationService.Initialize(english?"en-US":"zh-CN",null);
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

            const string qrValue="https://example.invalid/automatic-qr";
            using(var generator=new QRCodeGenerator())
            using(var qrData=generator.CreateQrCode(qrValue,QRCodeGenerator.ECCLevel.M))
            using(var qrCode=new PngByteQRCode(qrData))
            using(var qrStream=new MemoryStream(qrCode.GetGraphic(5)))
            {
                var qrImage=new BitmapImage();qrImage.BeginInit();qrImage.CacheOption=BitmapCacheOption.OnLoad;qrImage.StreamSource=qrStream;qrImage.EndInit();qrImage.Freeze();
                fields.GetField("CapturedImageOverride")!.SetValue(item,qrImage);fields.GetField("Bounds")!.SetValue(item,new Rect(20,20,qrImage.PixelWidth,qrImage.PixelHeight));Clear();
                var barcodeResults=(IDictionary)typeof(CaptureOverlayWindow).GetField("_screenBarcodes",Private)!.GetValue(overlay)!;
                var delayedText=NewResult();var automaticTask=Start(delayedText);
                WaitUntil(()=>barcodeResults.Contains(item));
                var bar=(FrameworkElement)overlay.FindName("ScreenEntityBar");
                var content=(Panel)overlay.FindName("ScreenEntityBarContent");
                Check(bar.Visibility==Visibility.Visible&&Text() is null,"QR hint appears automatically before the text reader completes");
                Check(Buttons(content).Any(button=>System.Windows.Automation.AutomationProperties.GetName(button)==qrValue),"automatic QR hint exposes the decoded content without a toolbar click");
                delayedText.SetResult("sample@example.org");Complete(automaticTask);
                Check(Buttons(content).Any(button=>System.Windows.Automation.AutomationProperties.GetName(button)==qrValue)&&Buttons(content).Any(button=>System.Windows.Automation.AutomationProperties.GetName(button)=="sample@example.org"),"email and QR actions coexist after either recognition completes");
                fields.GetField("Bounds")!.SetValue(item,new Rect(1,0,qrImage.PixelWidth-1,qrImage.PixelHeight));Invoke("UpdateScreenEntityBar",item);
                Check(!Buttons(content).Any(button=>System.Windows.Automation.AutomationProperties.GetName(button)==qrValue),"changed geometry never reuses a previous QR hint");
                Invoke("ClearImageDerivedLayers",item);Check(!barcodeResults.Contains(item),"content invalidation releases decoded QR content");
                fields.GetField("CapturedImageOverride")!.SetValue(item,null);fields.GetField("Bounds")!.SetValue(item,bounds);Clear();
            }

            var multiValues=new[]{"https://example.invalid/first","second-code","https://example.invalid/third"};
            var multiImage=MultipleCodes(multiValues);
            // Use real monitor metadata with synthetic pixels, so a monitor
            // above/left of the primary cannot place the hint outside the fake frame.
            var desktop=System.Windows.Forms.SystemInformation.VirtualScreen;
            var primary=System.Windows.Forms.Screen.PrimaryScreen!.WorkingArea;
            var syntheticDesktop=BitmapSource.Create(desktop.Width,desktop.Height,96,96,PixelFormats.Bgra32,null,new byte[desktop.Width*desktop.Height*4],desktop.Width*4);syntheticDesktop.Freeze();
            Set("_frame",new CaptureFrame(desktop.X,desktop.Y,syntheticDesktop));
            var stageLeft=primary.Left-desktop.Left;var stageTop=primary.Top-desktop.Top;
            fields.GetField("CapturedImageOverride")!.SetValue(item,multiImage);
            fields.GetField("Bounds")!.SetValue(item,new Rect(stageLeft+40,stageTop+150,Math.Min(700,multiImage.PixelWidth),multiImage.PixelHeight));Clear();
            root.Width=desktop.Width;root.Height=desktop.Height;root.Measure(new Size(desktop.Width,desktop.Height));root.Arrange(new Rect(0,0,desktop.Width,desktop.Height));root.UpdateLayout();
            var multiText=NewResult();var multiTask=Start(multiText);
            var results=(IDictionary)typeof(CaptureOverlayWindow).GetField("_screenBarcodes",Private)!.GetValue(overlay)!;
            WaitUntil(()=>results.Contains(item));
            var multiBar=(FrameworkElement)overlay.FindName("ScreenEntityBar");var multiContent=(Panel)overlay.FindName("ScreenEntityBarContent");
            var decoded=(IReadOnlyList<string>)results[item]!.GetType().GetProperty("Values")!.GetValue(results[item])!;
            Check(decoded.Count==3&&multiValues.All(decoded.Contains),"three actual QR codes are decoded and none is lost by the compact presentation");
            Check(multiContent.Children.Count==1&&Buttons(multiContent).Count(button=>decoded.Contains(System.Windows.Automation.AutomationProperties.GetName(button)))<=2,"multiple QR codes show one action group instead of repeated buttons");
            var seen=new HashSet<string>();
            for(var index=0;index<3;index++)
            {
                var current=Buttons(multiContent).First(button=>decoded.Contains(System.Windows.Automation.AutomationProperties.GetName(button)));
                seen.Add(System.Windows.Automation.AutomationProperties.GetName(current));
                Buttons(multiContent).Single(button=>System.Windows.Automation.AutomationProperties.GetName(button)==nextLabel).RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
            }
            Check(seen.SetEquals(multiValues),"next navigation reaches every code and wraps without selecting a screenshot");
            var firstSelected=Buttons(multiContent).First(button=>decoded.Contains(System.Windows.Automation.AutomationProperties.GetName(button)));
            var selectedValue=System.Windows.Automation.AutomationProperties.GetName(firstSelected);
            Buttons(multiContent).Single(button=>System.Windows.Automation.AutomationProperties.GetName(button)==previousLabel).RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
            var previousValue=System.Windows.Automation.AutomationProperties.GetName(Buttons(multiContent).First(button=>decoded.Contains(System.Windows.Automation.AutomationProperties.GetName(button))));
            Check(previousValue!=selectedValue,"previous navigation changes the code independently of the text reader");
            multiText.SetResult("sample@example.org");Complete(multiTask);
            Check(Buttons(multiContent).Any(button=>System.Windows.Automation.AutomationProperties.GetName(button)==previousValue),"late text completion preserves the selected QR code");
            Invoke("UpdateSelection",item);root.Background=new SolidColorBrush(Color.FromRgb(232,236,242));
            var toolbar=(FrameworkElement)overlay.FindName("Toolbar");toolbar.Visibility=Visibility.Visible;
            Invoke("PositionFloatingBar",toolbar,item);root.UpdateLayout();
            Rect Bounds(FrameworkElement element)=>new(Canvas.GetLeft(element),Canvas.GetTop(element),element.DesiredSize.Width,element.DesiredSize.Height);
            Check(multiBar.Visibility==Visibility.Visible&&!Bounds(multiBar).IntersectsWith(Bounds(toolbar)),"the rendered hint avoids the main toolbar after toolbar reflow");
            Check(multiBar.DesiredSize.Width<=500&&multiBar.DesiredSize.Height<100,"three codes and a mailbox remain compact rather than stretching across the screen");
            Check((bool)Invoke("PointerInToolbarInteractionZone",new Point(Bounds(multiBar).Left+10,Bounds(multiBar).Top+10))!,"the hint owns pointer hover and cannot select an underlying region");
            var nextButton=Buttons(multiContent).Single(button=>System.Windows.Automation.AutomationProperties.GetName(button)==nextLabel);
            nextButton.RaiseEvent(new System.Windows.Input.MouseButtonEventArgs(System.Windows.Input.Mouse.PrimaryDevice,Environment.TickCount,System.Windows.Input.MouseButton.Left){RoutedEvent=System.Windows.Input.Mouse.PreviewMouseDownEvent});
            Check(!(bool)typeof(CaptureOverlayWindow).GetField("_selecting",Private)!.GetValue(overlay)!&&!(bool)typeof(CaptureOverlayWindow).GetField("_moving",Private)!.GetValue(overlay)!&&selections.Count==1,"real preview mouse routing through a QR button never starts selection or movement");
            var sizeLabel=(FrameworkElement)overlay.FindName("SizeText");
            Check(sizeLabel.Visibility!=Visibility.Visible||!Bounds(multiBar).IntersectsWith(Bounds(sizeLabel)),"the hint also avoids the screenshot dimension badge");
            var scale=english?1:1.75;
            Check(new Rect(stageLeft,stageTop,primary.Width,primary.Height).Contains(Bounds(multiBar)),"the actual hint lies inside the active physical monitor's viewport");
            var stage=new DrawingVisual();
            using(var drawing=stage.RenderOpen())drawing.DrawRectangle(new VisualBrush(root){ViewboxUnits=BrushMappingMode.Absolute,Viewbox=new Rect(stageLeft,stageTop,900,620),Stretch=Stretch.Fill},null,new Rect(0,0,900,620));
            var rendered=new RenderTargetBitmap((int)(900*scale),(int)(620*scale),96*scale,96*scale,PixelFormats.Pbgra32);rendered.Render(stage);
            var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(rendered));
            using(var output=File.Create(Path.Combine(directory,"multiple-qr-layout.png")))encoder.Save(output);
            Invoke("ClearImageDerivedLayers",item);fields.GetField("CapturedImageOverride")!.SetValue(item,null);fields.GetField("Bounds")!.SetValue(item,bounds);Clear();
            Set("_frame",frame);root.Width=400;root.Height=300;root.Measure(new Size(400,300));root.Arrange(new Rect(0,0,400,300));root.UpdateLayout();

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
            actualDesktopInput=false,nativeWindowCreated=false,realOcrOrAccountsUsed=false,realBarcodeDecode=true,language=english?"en-US":"zh-CN",renderedDpi=english?96:168
        },new JsonSerializerOptions{WriteIndented=true}),new UTF8Encoding(false));
        Environment.ExitCode=failure is null?0:1;
    }

    private static IEnumerable<Button> Buttons(Panel panel)
        =>panel.Children.OfType<Button>().Concat(panel.Children.OfType<Panel>().SelectMany(Buttons));

    private static BitmapSource MultipleCodes(string[] values)
    {
        var images=values.Select(value=>
        {
            using var generator=new QRCodeGenerator();using var data=generator.CreateQrCode(value,QRCodeGenerator.ECCLevel.M);
            using var code=new PngByteQRCode(data);using var stream=new MemoryStream(code.GetGraphic(4));
            var image=new BitmapImage();image.BeginInit();image.CacheOption=BitmapCacheOption.OnLoad;image.StreamSource=stream;image.EndInit();image.Freeze();return image;
        }).ToArray();
        var width=images.Sum(image=>image.PixelWidth)+40*(images.Length-1);var height=images.Max(image=>image.PixelHeight);
        var pixels=Enumerable.Repeat((byte)255,width*height*4).ToArray();var offset=0;
        foreach(var image in images)
        {
            var converted=new FormatConvertedBitmap(image,PixelFormats.Bgra32,null,0);var rows=new byte[image.PixelWidth*image.PixelHeight*4];converted.CopyPixels(rows,image.PixelWidth*4,0);
            for(var y=0;y<image.PixelHeight;y++)Array.Copy(rows,y*image.PixelWidth*4,pixels,(y*width+offset)*4,image.PixelWidth*4);
            offset+=image.PixelWidth+40;
        }
        var result=BitmapSource.Create(width,height,96,96,PixelFormats.Bgra32,null,pixels,width*4);result.Freeze();return result;
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

    private static void WaitUntil(Func<bool> condition)
    {
        var loop=new DispatcherFrame();var watch=Stopwatch.StartNew();
        var timer=new DispatcherTimer(DispatcherPriority.Send){Interval=TimeSpan.FromMilliseconds(20)};
        timer.Tick+=(_,_)=>{if(condition()||watch.Elapsed>TimeSpan.FromSeconds(10))loop.Continue=false;};
        timer.Start();try{Dispatcher.PushFrame(loop);}finally{timer.Stop();}
        if(!condition())throw new TimeoutException("Automatic QR scan did not publish its hint.");
    }
}
