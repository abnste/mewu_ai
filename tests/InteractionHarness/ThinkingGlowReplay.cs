// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using TextBox=System.Windows.Controls.TextBox;
using CheckBox=System.Windows.Controls.CheckBox;
using Button=System.Windows.Controls.Button;
using Point=System.Windows.Point;

internal static class ThinkingGlowReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
    internal static void Run(Application app,AppHost host,CaptureOverlayWindow overlay)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        Program.MarkReplayWindow(overlay,"AI 呼吸光效验收 · 合成界面 · 不发送请求");
        overlay.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;
            try
            {
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                var glow=(FrameworkElement)overlay.FindName("BottomThinkingGlow");
                var prompt=(TextBox)overlay.FindName("QuickPrompt");
                var answer=(FrameworkElement)overlay.FindName("AnswerScroll");
                Check("idle-hidden",glow.Visibility==Visibility.Collapsed);
                prompt.Text="保留输入光标";prompt.Focus();prompt.CaretIndex=3;
                using var first=new CancellationTokenSource();Start(first);
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                Check("working-visible-without-answer-or-focus-change",glow.Visibility==Visibility.Visible&&answer.Visibility==Visibility.Collapsed&&prompt.IsKeyboardFocused&&prompt.CaretIndex==3);
                Check("visual-does-not-hit-test",!glow.IsHitTestVisible&&!glow.Focusable);
                Check("positioned-at-screen-bottom",glow.Width>0&&glow.Height>0&&double.IsFinite(Canvas.GetTop(glow)));
                var screen=(System.Windows.Forms.Screen)typeof(CaptureOverlayWindow).GetMethod("PromptMonitor",Private)!.Invoke(overlay,null)!;
                var frame=(CaptureFrame)typeof(CaptureOverlayWindow).GetField("_frame",Private)!.GetValue(overlay)!;
                var root=(FrameworkElement)overlay.FindName("Root");
                var display=screen.Bounds;
                var full=ScreenCoordinateService.ToLocalDipRect(new mewu_ai_Assistant.Models.ScreenRect(display.X,display.Y,display.Width,display.Height),frame.OriginX,frame.OriginY,root.ActualWidth,root.ActualHeight,frame.Image.PixelWidth,frame.Image.PixelHeight);
                Check("glow-reaches-full-display-edge-including-taskbar",Math.Abs(Canvas.GetTop(glow)+glow.Height-full.Bottom)<.01&&Math.Abs(Canvas.GetLeft(glow)-full.Left)<.01&&Math.Abs(glow.Width-full.Width)<.01);
                var work=(Rect)typeof(CaptureOverlayWindow).GetMethod("PromptMonitorBounds",Private)!.Invoke(overlay,null)!;
                var composer=(FrameworkElement)overlay.FindName("PromptBarHost");
                Check("composer-still-avoids-taskbar",Canvas.GetTop(composer)+composer.ActualHeight<=work.Bottom+1);
                var edge=new Point(full.Left+full.Width/2,full.Bottom-2);
                var lit=Sample(root,edge);
                glow.Visibility=Visibility.Collapsed;var unlit=Sample(root,edge);glow.Visibility=Visibility.Visible;
                Check("bottom-edge-pixels-contain-glow",lit!=unlit);
                await Task.Delay(1900);Save(glow.Parent as FrameworkElement??overlay,"thinking-glow-blue.png");
                first.Cancel();
                using var second=new CancellationTokenSource();host.Settings.ThinkingGlowColor="#FFB8D0";Start(second);
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                Check("late-cancel-does-not-stop-new-request",glow.Visibility==Visibility.Visible);
                var gradient=(RadialGradientBrush)((Border)glow).Background;
                Check("custom-color-applied",gradient.GradientStops[0].Color.R==255&&gradient.GradientStops[0].Color.G==184);
                await Task.Delay(1900);Save(glow.Parent as FrameworkElement??overlay,"thinking-glow-pink.png");
                Stop(first);Check("stale-completion-ignored",glow.Visibility==Visibility.Visible);
                second.Cancel();await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                Check("cancel-stops-animation",glow.Visibility==Visibility.Collapsed&&!glow.HasAnimatedProperties);
                using var third=new CancellationTokenSource();Start(third);Stop(third);
                Check("completion-stops-animation",glow.Visibility==Visibility.Collapsed&&!glow.HasAnimatedProperties);
                host.Settings.ThinkingGlowEnabled=false;Start(third);Check("disabled-hidden",glow.Visibility==Visibility.Collapsed);
                host.Settings.ThinkingGlowEnabled=true;Start(third);overlay.Close();
                Check("closing-stops-animation",glow.Visibility==Visibility.Collapsed&&!glow.HasAnimatedProperties);
                var settings=new SettingsWindow(host){Title="呼吸光效设置验收 · 合成配置",Topmost=true};
                try
                {
                    ((CancellationTokenSource)typeof(SettingsWindow).GetField("_windowLifetime",Private)!.GetValue(settings)!).Cancel();
                    settings.Show();await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                    var toggle=(CheckBox)typeof(SettingsWindow).GetField("_thinkingGlowEnabled",Private)!.GetValue(settings)!;
                    var choose=Descendants(settings).OfType<Button>().Single(b=>b.Content?.ToString() is "更改光效颜色…" or "Change glow color…");
                    var settingPreview=Descendants(settings).OfType<ThinkingGlow>().Single();
                    var before=typeof(SettingsWindow).GetField("_thinkingGlowColor",Private)!.GetValue(settings);
                    Pick(false);Check("color-dialog-cancel-keeps-draft",Equals(before,typeof(SettingsWindow).GetField("_thinkingGlowColor",Private)!.GetValue(settings)));
                    Pick(true);Check("color-dialog-applies-rgb",Equals("#88DDBB",typeof(SettingsWindow).GetField("_thinkingGlowColor",Private)!.GetValue(settings)));
                    Check("color-is-draft-until-save",host.Settings.ThinkingGlowColor=="#FFB8D0");
                    toggle.IsChecked=false;Check("settings-disable-stops-preview",settingPreview.Visibility==Visibility.Collapsed&&!choose.IsEnabled);
                    toggle.IsChecked=true;await Task.Delay(1900);Save(settings,"thinking-glow-settings.png");
                    settings.Width=600;await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);Save(settings,"thinking-glow-settings-narrow.png");
                    Check("narrow-settings-button-accessible",choose.ActualWidth>0&&choose.ActualWidth<600);
                    void Pick(bool accept)
                    {
                        app.Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle,new Action(()=>
                        {
                            var dialog=Application.Current.Windows.OfType<MewuColorDialog>().Single();
                            Check("picker-has-settings-owner",dialog.Owner==settings);
                            ((TextBox)dialog.FindName("RedValue")).Text="136";((TextBox)dialog.FindName("GreenValue")).Text="221";((TextBox)dialog.FindName("BlueValue")).Text="187";
                            dialog.DialogResult=accept;
                        }));
                        choose.RaiseEvent(new RoutedEventArgs(System.Windows.Controls.Primitives.ButtonBase.ClickEvent));
                    }
                }
                finally{settings.Close();}
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally
            {
                Directory.CreateDirectory(".codex-build");File.WriteAllText(".codex-build/thinking-glow-result.json",JsonSerializer.Serialize(new{checks,failure}));
                overlay.Close();app.Shutdown(Environment.ExitCode);
            }
            void Start(CancellationTokenSource request)=>typeof(CaptureOverlayWindow).GetMethod("StartThinkingGlow",Private)!.Invoke(overlay,[request]);
            void Stop(CancellationTokenSource request)=>typeof(CaptureOverlayWindow).GetMethod("StopThinkingGlow",Private)!.Invoke(overlay,[request]);
            void Check(string name,bool valid){if(!valid)throw new Exception(name);checks.Add(name);}
        }));
    }
    private static void Save(FrameworkElement element,string name)
    {
        var bitmap=new RenderTargetBitmap((int)Math.Ceiling(element.ActualWidth),(int)Math.Ceiling(element.ActualHeight),96,96,PixelFormats.Pbgra32);bitmap.Render(element);
        var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));Directory.CreateDirectory(".codex-build");using var stream=File.Create(Path.Combine(".codex-build",name));encoder.Save(stream);
    }
    private static int Sample(FrameworkElement element,Point point)
    {
        var bitmap=new RenderTargetBitmap((int)Math.Ceiling(element.ActualWidth),(int)Math.Ceiling(element.ActualHeight),96,96,PixelFormats.Pbgra32);bitmap.Render(element);
        var pixel=new byte[4];bitmap.CopyPixels(new Int32Rect((int)point.X,(int)point.Y,1,1),pixel,4,0);return BitConverter.ToInt32(pixel);
    }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject parent)
    {
        for(var i=0;i<VisualTreeHelper.GetChildrenCount(parent);i++){var child=VisualTreeHelper.GetChild(parent,i);yield return child;foreach(var nested in Descendants(child))yield return nested;}
    }
}
