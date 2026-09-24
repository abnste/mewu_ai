// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Button=System.Windows.Controls.Button;
using TextBox=System.Windows.Controls.TextBox;
using ScrollBar=System.Windows.Controls.Primitives.ScrollBar;

internal static class HistoryMenuReplay
{
    internal static void Run(Application app,CaptureOverlayWindow overlay,bool english)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        Program.MarkReplayWindow(overlay,"上拉菜单布局验收 · 合成内容 · 自动关闭");
        overlay.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;
            var folder=Path.GetFullPath(".codex-build/history-menu");Directory.CreateDirectory(folder);
            var prefix=english?"en":"zh";
            try
            {
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                var button=(Button)overlay.FindName("NewConversationButton");
                var toggle=(Button)overlay.FindName("HistoryToggle");
                var panel=(Border)overlay.FindName("HistoryPanel");
                var bar=(Border)overlay.FindName("PromptBar");
                var scroll=(ScrollViewer)overlay.FindName("HistoryScroll");
                typeof(CaptureOverlayWindow).GetField("_conversationChannels",BindingFlags.NonPublic|BindingFlags.Instance)!.SetValue(overlay,new List<ConversationChannel>{new("header-test","课堂助手","header-test","MiniMax-M3",ConversationChannelKind.Api,true,true)});
                typeof(CaptureOverlayWindow).GetField("_selectedConversationChannelId",BindingFlags.NonPublic|BindingFlags.Instance)!.SetValue(overlay,"header-test");
                Invoke("UpdateChannelPickerItems");
                Invoke("SetPromptBarHidden",false,false);
                await Layout();Check("collapsed-hides-new-chat",!button.IsVisible);Save("collapsed");
                var host=(FrameworkElement)overlay.FindName("PromptBarHost");
                var hit=(Button)overlay.FindName("HistoryToggleHitTarget");
                var drag=(FrameworkElement)overlay.FindName("PromptDragHandle");
                var center=new System.Windows.Point(host.ActualWidth/2,4);
                Check("upper-center-hits-toggle",hit.IsAncestorOf(VisualTreeHelper.HitTest(host,center)!.VisualHit));
                Check("upper-side-keeps-dragging",drag.IsAncestorOf(VisualTreeHelper.HitTest(host,new System.Windows.Point(35,4))!.VisualHit));
                var size=host.RenderSize;
                hit.Visibility=Visibility.Collapsed;overlay.UpdateLayout();
                Check("transparent-target-adds-no-layout",host.RenderSize==size);
                hit.Visibility=Visibility.Visible;overlay.UpdateLayout();
                CheckBlankDrag("bottom-padding",new System.Windows.Point(host.ActualWidth/2,host.ActualHeight-3));
                CheckBlankDrag("side-padding",new System.Windows.Point(3,host.ActualHeight/2));
                var input=(FrameworkElement)overlay.FindName("PromptInputBorder");
                var inputOrigin=input.TranslatePoint(new System.Windows.Point(),host);
                CheckBlankDrag("space-above-input",new System.Windows.Point(host.ActualWidth/2,inputOrigin.Y-4));
                foreach(var name in new[]{"QuickPrompt","PromptInputBorder","HistoryToggle","HistoryToggleHitTarget","UploadButton","ChannelButton","SendButton"})
                    CheckProtectedControl((UIElement)overlay.FindName(name));
                hit.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));await Layout();
                Check("upper-target-expands-history",panel.IsVisible);
                hit.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));await Layout();
                Check("upper-target-collapses-history",!panel.IsVisible);
                var history=(List<AiMessage>)typeof(CaptureOverlayWindow).GetField("_history",BindingFlags.NonPublic|BindingFlags.Instance)!.GetValue(overlay)!;
                history.Add(new("user",english?"Explain the second question.":"帮我解释一下第 2 题。"));
                history.Add(new("assistant",english?"Start with the known conditions, then check each step.":"先找出题目给出的条件，再逐步检查解题过程。"));
                toggle.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));await Layout();
                Check("expanded-shows-new-chat",button.IsVisible&&panel.IsAncestorOf(button));
                Check("header-outside-scrolling-list",!scroll.IsAncestorOf(button));
                var answer=(MarkdownAnswerView)overlay.FindName("AnswerText");
                typeof(CaptureOverlayWindow).GetField("_lastSubmittedPrompt",BindingFlags.NonPublic|BindingFlags.Instance)!.SetValue(overlay,"继续解释第二步");
                Invoke("ResetAnswerForRequest");answer.Markdown="第二步先代入已知条件，再检查单位。";Invoke("ShowAnswer");await Layout();
                Check("inplace-live-answer-in-same-scroll",scroll.IsAncestorOf(answer)&&((ScrollViewer)overlay.FindName("ResponseScroll")).Content is null);
                CheckBlankDrag("expanded-bottom-padding",new System.Windows.Point(host.ActualWidth/2,host.ActualHeight-3));
                CheckProtectedControl(answer);CheckProtectedControl(button);
                CheckProtectedControl(Descendants((HistoryPreviewPanel)overlay.FindName("HistoryItems")).OfType<TextBox>().First());
                foreach(var scrollbar in Descendants(scroll).OfType<ScrollBar>())CheckProtectedControl(scrollbar);
                toggle.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));await Layout();
                Check("collapse-restores-live-answer",((ScrollViewer)overlay.FindName("ResponseScroll")).IsAncestorOf(answer));
                toggle.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));await Layout();
                Check("reexpand-restores-single-scroll",scroll.IsAncestorOf(answer));
                history.Add(new("user","继续解释第二步"));history.Add(new("assistant",answer.Markdown));
                typeof(CaptureOverlayWindow).GetField("_lastSubmittedTurnRecorded",BindingFlags.NonPublic|BindingFlags.Instance)!.SetValue(overlay,true);
                Invoke("RefreshHistoryPreview");
                var before=Invoke("CaptureOverlaySnapshot");
                Invoke("ResetAnswerForRequest");Invoke("ApplyOverlaySnapshot",before!);await Layout();
                Check("cancel-restores-answer",answer.Markdown.Contains("检查单位"));
                Check("cancel-preserves-expanded-stream",scroll.IsAncestorOf(answer));
                Check("cancel-does-not-duplicate-answer",!Descendants((HistoryPreviewPanel)overlay.FindName("HistoryItems")).OfType<TextBox>().Any(text=>text.Text.Contains("检查单位")));
                var bubbleRows=Descendants((HistoryPreviewPanel)overlay.FindName("HistoryItems")).OfType<Border>().Where(border=>border.Child is StackPanel&&border.CornerRadius.TopLeft>=14).ToArray();
                Check("expanded-history-uses-left-right-bubbles",bubbleRows.Any(border=>border.HorizontalAlignment==System.Windows.HorizontalAlignment.Left)&&bubbleRows.Any(border=>border.HorizontalAlignment==System.Windows.HorizontalAlignment.Right));
                CheckBounds();Save("expanded");
                scroll.ScrollToEnd();await Layout();Check("action-stays-visible-after-scroll",button.IsVisible);CheckBounds();
                // Inspect the actual production panel at a narrow width too.
                typeof(CaptureOverlayWindow).GetField("_positioningPromptBar",BindingFlags.NonPublic|BindingFlags.Instance)!.SetValue(overlay,true);
                bar.Width=320;bar.UpdateLayout();CheckBounds();Save("narrow");
                typeof(CaptureOverlayWindow).GetField("_positioningPromptBar",BindingFlags.NonPublic|BindingFlags.Instance)!.SetValue(overlay,false);
                button.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));await Layout();
                Check("new-chat-clears-context-and-collapses",history.Count==1&&history[0].Role=="system"&&!button.IsVisible);
                Save("new-chat");
                void CheckBlankDrag(string name,System.Windows.Point point)
                {
                    var source=VisualTreeHelper.HitTest(host,point)!.VisualHit;
                    while(source is not UIElement)source=VisualTreeHelper.GetParent(source);
                    var thumb=(Thumb)drag;
                    var origin=new System.Windows.Point(Canvas.GetLeft(host),Canvas.GetTop(host));
                    var wasExpanded=panel.IsVisible;
                    var down=new MouseButtonEventArgs(Mouse.PrimaryDevice,Environment.TickCount,MouseButton.Left){RoutedEvent=Mouse.PreviewMouseDownEvent};
                    ((UIElement)source).RaiseEvent(down);
                    if(!down.Handled||!thumb.IsDragging||!thumb.IsMouseCaptured)throw new InvalidOperationException($"{name}: source={source.GetType().Name}, eligible={Invoke("IsPromptDragBackground",source)}, handled={down.Handled}, dragging={thumb.IsDragging}, captured={thumb.IsMouseCaptured}");
                    Check(name+"-captures-existing-drag-handle",true);
                    thumb.RaiseEvent(new DragDeltaEventArgs(100,-150){RoutedEvent=Thumb.DragDeltaEvent});
                    Check(name+"-moves-and-expands",panel.IsVisible&&(new System.Windows.Point(Canvas.GetLeft(host),Canvas.GetTop(host))-origin).Length>1);
                    thumb.CancelDrag();overlay.UpdateLayout();
                    Check(name+"-cancel-restores-state",!thumb.IsDragging&&!thumb.IsMouseCaptured&&panel.IsVisible==wasExpanded&&(new System.Windows.Point(Canvas.GetLeft(host),Canvas.GetTop(host))-origin).Length<1);
                }
                void CheckProtectedControl(UIElement control)
                {
                    var down=new MouseButtonEventArgs(Mouse.PrimaryDevice,Environment.TickCount,MouseButton.Left){RoutedEvent=Mouse.PreviewMouseDownEvent};
                    control.RaiseEvent(down);
                    Check("interactive-control-keeps-input-"+control.GetType().Name,!((Thumb)drag).IsDragging);
                }
                void CheckBounds()
                {
                    var bounds=new Rect(button.TranslatePoint(new System.Windows.Point(),panel),button.RenderSize);
                    Check("action-fits-"+bar.ActualWidth,bounds.Left>=0&&bounds.Right<=panel.ActualWidth+.1&&bounds.Bottom<=panel.ActualHeight+.1);
                    var scrollBounds=new Rect(scroll.TranslatePoint(new System.Windows.Point(),panel),scroll.RenderSize);
                    Check("list-fits-"+bar.ActualWidth,scrollBounds.Bottom<=panel.ActualHeight+.1&&scrollBounds.Top>=bounds.Bottom);
                }
                async Task Layout(){await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);await Task.Delay(260);overlay.UpdateLayout();}
                void Save(string name)
                {
                    var visual=new DrawingVisual();using(var dc=visual.RenderOpen())dc.DrawRectangle(new VisualBrush(bar){Stretch=Stretch.None,AlignmentX=AlignmentX.Left,AlignmentY=AlignmentY.Top},null,new Rect(bar.RenderSize));
                    var image=new RenderTargetBitmap((int)Math.Ceiling(bar.ActualWidth),(int)Math.Ceiling(bar.ActualHeight),96,96,PixelFormats.Pbgra32);image.Render(visual);
                    var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(image));using var file=File.Create(Path.Combine(folder,prefix+"-"+name+".png"));encoder.Save(file);
                }
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally{File.WriteAllText(Path.Combine(folder,prefix+"-result.json"),JsonSerializer.Serialize(new{checks,failure}));overlay.Close();app.Shutdown(Environment.ExitCode);}
            void Check(string name,bool success){if(!success)throw new InvalidOperationException(name);checks.Add(name);}
            IEnumerable<DependencyObject> Descendants(DependencyObject root){for(var i=0;i<VisualTreeHelper.GetChildrenCount(root);i++){var child=VisualTreeHelper.GetChild(root,i);yield return child;foreach(var nested in Descendants(child))yield return nested;}}
            object? Invoke(string name,params object[] values)=>typeof(CaptureOverlayWindow).GetMethod(name,BindingFlags.Instance|BindingFlags.NonPublic)!.Invoke(overlay,values);
        }));
    }
}
