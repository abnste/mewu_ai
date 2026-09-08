// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Clipboard=System.Windows.Clipboard;
using Point=System.Windows.Point;
using Brushes=System.Windows.Media.Brushes;
using TextBox=System.Windows.Controls.TextBox;
using RichTextBox=System.Windows.Controls.RichTextBox;

internal static class CaptureInputReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic|BindingFlags.DeclaredOnly;
    [DllImport("user32.dll")] private static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window,out uint processId);
    [DllImport("user32.dll")] private static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);

    internal static void Run(Application app,AppHost host,CaptureOverlayWindow first)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        first.Show();
        app.Dispatcher.BeginInvoke(DispatcherPriority.Normal,new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;
            var originalClipboard=Clipboard.GetDataObject();
            try
            {
                var root=(Canvas)first.FindName("Root");
                var a=AddSelection(first,new Rect(100,100,360,180));
                var b=AddSelection(first,new Rect(80,285,700,170));
                Invoke(first,"Select",0);Invoke(first,"ShowToolbar");first.UpdateLayout();
                var toolbar=(FrameworkElement)first.FindName("Toolbar");
                var toolbarBounds=new Rect(Canvas.GetLeft(toolbar),Canvas.GetTop(toolbar),toolbar.ActualWidth,toolbar.ActualHeight);
                SetPublic(b,"Bounds",toolbarBounds);Invoke(first,"UpdateSelection",b);Invoke(first,"Select",0);
                var p=new Point(toolbarBounds.Left+toolbarBounds.Width/2,toolbarBounds.Top+toolbarBounds.Height/2);
                var screen=root.PointToScreen(p);SetCursorPos((int)screen.X,(int)screen.Y);
                await Yield();
                Invoke(first,"OnMouseMove",root,new System.Windows.Input.MouseEventArgs(Mouse.PrimaryDevice,Environment.TickCount));
                Require((int)Get(first,"_activeIndex")==0,"Toolbar switched to the region underneath");checks.Add("overlapping-toolbar-keeps-owner");

                var selection=(Canvas)a.GetType().GetProperty("TextSelection")!.GetValue(a)!;
                var box=new RichTextBox{IsReadOnly=true,Width=100,Height=70};selection.Children.Add(box);selection.IsHitTestVisible=true;
                first.Activate();box.Focus();Require(box.IsKeyboardFocused,"OCR focus setup failed");
                first.GetType().GetMethod("ClearTextSelection",BindingFlags.Static|BindingFlags.NonPublic)!.Invoke(null,[a]);
                Require(root.IsKeyboardFocused,"Removing OCR left focus detached");checks.Add("removed-ocr-focus-restored");

                Invoke(first,"SetPromptBarHidden",false,false);
                ((TextBox)first.FindName("QuickPrompt")).Focus();Invoke(first,"SetPromptBarHidden",true,false);
                Require(root.IsKeyboardFocused,"Hidden composer retained keyboard focus");checks.Add("hidden-composer-releases-focus");

                // Export a visible translation overlay and prove Enter copies
                // rendered content, then closes, even with a toolbar focused.
                var textLayer=(Canvas)a.GetType().GetProperty("TextOverlays")!.GetValue(a)!;
                var source=(BitmapSource)first.GetType().GetMethod("CurrentImage",Private)!.Invoke(first,null)!;
                Invoke(first,"RenderTextOverlays",a,source,Array.Empty<mewu_ai_Assistant.Models.OcrLine>(),Array.Empty<string>(),true);
                textLayer.Children.Add(new System.Windows.Shapes.Rectangle{Width=50,Height=50,Fill=Brushes.Lime});
                first.UpdateLayout();
                ((System.Windows.Controls.Button)first.FindName("ChannelButton")).Focus();
                RaiseKey(first,Key.Enter);
                Require((bool)Get(first,"_closed"),"Enter did not close the screenshot");
                var copied=Clipboard.GetImage();Require(copied is not null&&copied.PixelWidth>0,"Enter did not copy an image");
                var rgb=new FormatConvertedBitmap(copied,PixelFormats.Bgra32,null,0);var pixel=new byte[4];rgb.CopyPixels(new Int32Rect(10,10,1,1),pixel,4,0);
                Require(pixel[1]>200&&pixel[0]<30&&pixel[2]<30,"Copied image omitted the translation layer");checks.Add("enter-copies-rendered-image-and-closes");

                var busy=Create(host);await Yield();using var pending=new CancellationTokenSource();Set(busy,"_overlayRequest",pending);
                ((Canvas)busy.FindName("Root")).Focus();RaiseKey(busy,Key.Escape);
                Require((bool)Get(busy,"_closed")&&pending.IsCancellationRequested,"Escape waited for an unfinished translation");checks.Add("escape-cancels-pending-translation-and-closes");

                var multi=Create(host);await Yield();
                foreach(var bounds in new[]{new Rect(90,90,160,100),new Rect(310,90,160,100)})
                {
                    var item=AddSelection(multi,bounds);var layer=(Canvas)item.GetType().GetProperty("TextSelection")!.GetValue(item)!;
                    var text=new RichTextBox{IsReadOnly=true,Width=100,Height=50};layer.Children.Add(text);layer.IsHitTestVisible=true;text.Focus();
                }
                RaiseKey(multi,Key.Escape);Require((bool)Get(multi,"_closed"),"Escape consumed only one OCR region");checks.Add("escape-exits-multiple-ocr-regions");

                var inactive=Create(host);await Yield();
                using var helper=Process.Start(new ProcessStartInfo(Environment.ProcessPath!,"--capture-input-foreground"){UseShellExecute=false})!;
                var switched=false;
                for(var attempt=0;attempt<80;attempt++)
                {
                    GetWindowThreadProcessId(GetForegroundWindow(),out var pid);
                    if(pid==(uint)helper.Id&&!inactive.IsActive){switched=true;break;}
                    await Task.Delay(50);
                }
                Require(switched,"Foreground helper did not activate");
                keybd_event(0x1B,0,0,UIntPtr.Zero);
                try{await Task.Delay(150);}finally{keybd_event(0x1B,0,2,UIntPtr.Zero);}
                for(var attempt=0;attempt<20&&!(bool)Get(inactive,"_closed");attempt++)await Task.Delay(50);
                Require((bool)Get(inactive,"_closed"),"Escape after switching to another process did not close capture");checks.Add("escape-after-switching-to-another-process");
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally
            {
                try{if(originalClipboard is not null)Clipboard.SetDataObject(originalClipboard,true);else Clipboard.Clear();}catch{}
                foreach(var window in app.Windows.OfType<Window>().ToArray())window.Close();
                var path=Path.GetFullPath(".codex-build/capture-input-result.json");Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                File.WriteAllText(path,JsonSerializer.Serialize(new{checks,failure}),new System.Text.UTF8Encoding(false));
                app.Shutdown(Environment.ExitCode);
            }
        }));
    }

    internal static void RunForegroundHelper()
    {
        var app=new Application();var window=new Window{Title="Mewu Capture Input Test",Width=260,Height=90,Topmost=true,Content="正在验证切屏后的 Esc…"};
        var timer=new DispatcherTimer{Interval=TimeSpan.FromSeconds(6)};timer.Tick+=(_,_)=>{timer.Stop();window.Close();};
        window.Loaded+=(_,_)=>{window.Activate();timer.Start();};app.Run(window);
    }

    private static CaptureOverlayWindow Create(AppHost host){var overlay=new CaptureOverlayWindow(host);overlay.Show();overlay.Activate();return overlay;}
    private static object AddSelection(CaptureOverlayWindow overlay,Rect bounds)
    {
        var item=overlay.GetType().GetMethod("CreateSelection",Private)!.Invoke(overlay,[false])!;SetPublic(item,"Bounds",bounds);
        ((IList)Get(overlay,"_selections")).Add(item);Invoke(overlay,"UpdateSelection",item);return item;
    }
    private static async Task Yield()=>await Task.Delay(100);
    private static void RaiseKey(CaptureOverlayWindow overlay,Key key)
    {
        var args=new System.Windows.Input.KeyEventArgs(Keyboard.PrimaryDevice,PresentationSource.FromVisual(overlay)!,Environment.TickCount,key){RoutedEvent=Keyboard.PreviewKeyDownEvent};
        if(Keyboard.FocusedElement is UIElement focused)focused.RaiseEvent(args);else overlay.RaiseEvent(args);
    }
    private static void Require(bool condition,string message){if(!condition)throw new InvalidOperationException(message);}
    private static object Get(object target,string field)=>target.GetType().GetField(field,Private)!.GetValue(target)!;
    private static void Set(object target,string field,object value)=>target.GetType().GetField(field,Private)!.SetValue(target,value);
    private static void SetPublic(object target,string field,object value)=>target.GetType().GetField(field)!.SetValue(target,value);
    private static void Invoke(object target,string method,params object[] arguments)=>target.GetType().GetMethod(method,Private)!.Invoke(target,arguments);
}
