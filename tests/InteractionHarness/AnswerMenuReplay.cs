// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Clipboard=System.Windows.Clipboard;
using TextBox=System.Windows.Controls.TextBox;
using MenuItem=System.Windows.Controls.MenuItem;
using ContextMenu=System.Windows.Controls.ContextMenu;

internal static class AnswerMenuReplay
{
    internal static void Run(Application app,CaptureOverlayWindow overlay)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        Program.MarkReplayWindow(overlay,"AI 回复复制菜单验收 · 合成内容");
        overlay.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;ContextMenu? open=null;
            var previous=Clipboard.GetDataObject();
            try
            {
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                var answer=(MarkdownAnswerView)overlay.FindName("AnswerText");
                typeof(CaptureOverlayWindow).GetMethod("ShowAnswer",BindingFlags.Instance|BindingFlags.NonPublic)!.Invoke(overlay,null);
                answer.Markdown="最新回复 **复制检查** 😀";
                var fullHistory="历史回复 "+new string('文',1000)+" 完整结尾";
                string? copied=null;
                var type=typeof(CaptureOverlayWindow).Assembly.GetType("mewu_ai_Assistant.Views.HistoryTextBox")!;
                var history=(TextBox)Activator.CreateInstance(type,BindingFlags.Instance|BindingFlags.NonPublic,null,[fullHistory,(Action<string>)(text=>copied=text)],null)!;
                var root=(Canvas)overlay.FindName("Root");history.Width=400;history.Height=70;
                Canvas.SetLeft(history,60);Canvas.SetTop(history,140);root.Children.Add(history);
                overlay.UpdateLayout();
                history.Select(0,4);await Open(history,"answer-menu-history.png");
                Check("history-selection-enabled",((MenuItem)open!.Items[0]).IsEnabled);
                Click(0);Check("history-copy-selection",copied==history.SelectedText);
                await Open(history,"answer-menu-history-full.png");
                Click(1);Check("history-copy-full-untruncated",copied==fullHistory&&history.Text.Length<fullHistory.Length);
                var historyStyle=open.Style;var historyTemplate=open.Template;
                open.IsOpen=false;
                answer.SelectAll();await Open(answer,"answer-menu-latest.png");
                Check("same-menu-style-template-and-labels",ReferenceEquals(historyStyle,open!.Style)&&ReferenceEquals(historyTemplate,open.Template)&&
                    history.ContextMenu.Items.Cast<MenuItem>().Select(i=>i.Header).SequenceEqual(open.Items.Cast<MenuItem>().Select(i=>i.Header)));
                Check("latest-menu-keeps-composer-visible",(bool)typeof(CaptureOverlayWindow).GetMethod("IsInteractingWithPrompt",BindingFlags.Instance|BindingFlags.NonPublic)!.Invoke(overlay,[new System.Windows.Point(-100,-100)])!);
                Click(0);var selectedCopy=Clipboard.GetText();Check("latest-selected-text-preserves-emoji",selectedCopy==answer.SelectedPlainText&&selectedCopy.Contains("😀"));
                await Open(answer,"answer-menu-latest-full.png");
                answer.Markdown+="，流式新增尾句。";
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);Click(1);
                var fullCopy=Clipboard.GetText();var expected=answer.PlainText;
                Check("latest-full-copy-reads-current-document",fullCopy==expected);
                Check("streamed-tail-present",answer.PlainText.EndsWith("流式新增尾句。"));
                open.IsOpen=false;answer.Selection.Select(answer.Document.ContentStart,answer.Document.ContentStart);
                await Open(answer,"answer-menu-unselected.png");
                Check("empty-selection-disabled-full-enabled",!((MenuItem)open!.Items[0]).IsEnabled&&((MenuItem)open.Items[1]).IsEnabled);
                open.IsOpen=false;
                history.Select(0,0);await Open(history,"answer-menu-history-unselected.png");
                Check("history-empty-selection-disabled",!((MenuItem)open!.Items[0]).IsEnabled);
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally
            {
                if(open is not null)open.IsOpen=false;
                try{if(previous is not null)Clipboard.SetDataObject(previous,true);else Clipboard.Clear();}
                catch(Exception ex){failure??=ex.ToString();Environment.ExitCode=1;}
                Directory.CreateDirectory(".codex-build");File.WriteAllText(".codex-build/answer-menu-result.json",JsonSerializer.Serialize(new{checks,failure}));
                overlay.Close();app.Shutdown(Environment.ExitCode);
            }
            async Task Open(FrameworkElement target,string file)
            {
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                open=target.ContextMenu!;open.PlacementTarget=target;open.Placement=PlacementMode.Bottom;open.IsOpen=true;
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);await Task.Delay(220);
                open.UpdateLayout();
                var surface=(Border)open.Template.FindName("MenuSurface",open);
                var origin=surface.TranslatePoint(new System.Windows.Point(),open);
                Check(file+"-shadow-room",origin.X>=27&&origin.Y>=27&&open.ActualWidth-origin.X-surface.ActualWidth>=27&&open.ActualHeight-origin.Y-surface.ActualHeight>=27);
                var bitmap=new RenderTargetBitmap((int)Math.Ceiling(open.ActualWidth),(int)Math.Ceiling(open.ActualHeight),96,96,PixelFormats.Pbgra32);bitmap.Render(open);
                var pixels=new byte[bitmap.PixelWidth*bitmap.PixelHeight*4];bitmap.CopyPixels(pixels,bitmap.PixelWidth*4,0);
                var width=bitmap.PixelWidth;var height=bitmap.PixelHeight;
                byte Alpha(int x,int y)=>pixels[(y*width+x)*4+3];
                Check(file+"-transparent-outer-edge",Enumerable.Range(0,width).All(x=>Alpha(x,0)==0&&Alpha(x,height-1)==0)&&Enumerable.Range(0,height).All(y=>Alpha(0,y)==0&&Alpha(width-1,y)==0));
                Check(file+"-shadow-visible-outside-surface",Alpha(width/2,(int)(origin.Y+surface.ActualHeight+3))>0);
                Directory.CreateDirectory(".codex-build");var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));using var stream=File.Create(Path.Combine(".codex-build",file));encoder.Save(stream);
            }
            void Click(int index)
            {
                ((MenuItem)open!.Items[index]).RaiseEvent(new RoutedEventArgs(MenuItem.ClickEvent));
                // Raising Click directly skips MenuItem.OnClick's normal popup dismissal.
                open.IsOpen=false;
            }
            void Check(string name,bool valid){if(!valid)throw new InvalidOperationException(name);checks.Add(name);}
        }));
    }
}
