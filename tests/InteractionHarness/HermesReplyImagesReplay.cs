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
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Image=System.Windows.Controls.Image;
using Brushes=System.Windows.Media.Brushes;
using Point=System.Windows.Point;

internal static class HermesReplyImagesReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
    internal static void Run(Application app,CaptureOverlayWindow overlay)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        Program.MarkReplayWindow(overlay,"Hermes 图片交付验收 · 合成画面");
        overlay.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;
            try
            {
                var folder=Path.GetFullPath(".codex-build/hermes-reply-images");Directory.CreateDirectory(folder);
                var file=Path.Combine(folder,"合成 图片.png");
                var visual=new DrawingVisual();using(var dc=visual.RenderOpen())
                {
                    dc.DrawRectangle(Brushes.AliceBlue,null,new Rect(0,0,240,180));
                    dc.DrawEllipse(Brushes.CornflowerBlue,null,new Point(120,84),60,62);
                    dc.DrawEllipse(Brushes.White,null,new Point(98,74),9,12);dc.DrawEllipse(Brushes.White,null,new Point(142,74),9,12);
                    dc.DrawEllipse(Brushes.LightPink,null,new Point(120,106),16,8);
                }
                var bitmap=new RenderTargetBitmap(240,180,96,96,PixelFormats.Pbgra32);bitmap.Render(visual);
                var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));using(var output=File.Create(file))encoder.Save(output);
                var service=typeof(CaptureOverlayWindow).Assembly.GetType("mewu_ai_Assistant.Services.HermesReplyMediaService")!;
                AiResult Complete(string markdown,IReadOnlyList<string> images)=>(AiResult)service.GetMethod("Complete",BindingFlags.NonPublic|BindingFlags.Static)!.Invoke(null,[new AiResult(markdown,[]),images])!;
                var answer=(MarkdownAnswerView)overlay.FindName("AnswerText");
                Invoke("ShowAnswer");
                // A streamed local path was previously rendered as an unreadable placeholder.
                answer.Markdown=$"![自画像](<{new Uri(file).AbsoluteUri}>)";
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                Check("untrusted-local-path-does-not-load",Images().All(image=>image.Source is null));
                var result=Complete($"这是合成自画像。\n\n![自画像](<{new Uri(file).AbsoluteUri}>)\n\nMEDIA:\"{file}\"",[]);
                answer.SetLocalReplyImageSources(result.LocalReplyImageSources);answer.Markdown=result.Answer;Invoke("PositionPromptBar");
                await WaitForImage();
                Check("markdown-and-media-show-one-image",Images().Length==1&&Images()[0].Source is not null);
                Check("local-image-has-visible-layout",Images()[0].ActualWidth>100&&Images()[0].ActualHeight>100);
                answer.SelectAll();Check("copy-retains-description",answer.SelectedPlainText.Contains("自画像"));
                Save(answer,Path.Combine(folder,"reply.png"));
                var snapshot=Invoke("CaptureOverlaySnapshot")!;
                Invoke("ResetAnswerForRequest");Check("new-request-revokes-local-access",answer.LocalReplyImageSources.Count==0);
                Invoke("ApplyOverlaySnapshot",snapshot);await WaitForImage();Check("cancel-restores-image-authorization",answer.LocalReplyImageSources.Count==1&&Images()[0].Source is not null);
                using var packet=JsonDocument.Parse(JsonSerializer.Serialize(new{name="image_generate",result=new{success=true,image=file}}));
                var references=(IReadOnlyList<string>)service.GetMethod("ReadGeneratedImages",BindingFlags.NonPublic|BindingFlags.Static)!.Invoke(null,[packet.RootElement])!;
                result=Complete("已经画好。",references);answer.SetLocalReplyImageSources(result.LocalReplyImageSources);answer.Markdown=result.Answer;
                await WaitForImage();Check("tool-result-displays-without-final-markdown",Images().Length==1&&Images()[0].Source is not null);
                var missing=Path.Combine(folder,"missing.png");result=Complete("MEDIA:"+missing,[]);answer.SetLocalReplyImageSources(result.LocalReplyImageSources);answer.Markdown=result.Answer;
                for(var i=0;i<100&&!Descendants(answer).OfType<TextBlock>().Any(text=>text.Text.Contains("已不存在"));i++)await Task.Delay(25);
                Check("missing-image-has-actionable-message",Descendants(answer).OfType<TextBlock>().Any(text=>text.Text.Contains("已不存在")));
                // Restore the visible successful example for final pixel capture.
                result=Complete("MEDIA:"+file,[]);answer.SetLocalReplyImageSources(result.LocalReplyImageSources);answer.Markdown=result.Answer;await WaitForImage();
                Save(answer,Path.Combine(folder,"media-only.png"));

                Image[] Images()=>Descendants(answer).OfType<Image>().Where(image=>image.Parent is Grid).ToArray();
                async Task WaitForImage()
                {
                    for(var i=0;i<200;i++){overlay.UpdateLayout();if(Images().Any(image=>image.Source is not null)){await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);return;}await Task.Delay(25);}
                    throw new InvalidOperationException("Hermes image did not become visible");
                }
            }
            catch(Exception ex){failure=ex.GetType().Name+": "+ex.Message;Environment.ExitCode=1;}
            finally{Directory.CreateDirectory(".codex-build");File.WriteAllText(".codex-build/hermes-reply-images-result.json",JsonSerializer.Serialize(new{checks,failure}));overlay.Close();app.Shutdown(Environment.ExitCode);}
            object? Invoke(string name,params object[] values)=>typeof(CaptureOverlayWindow).GetMethod(name,Private)!.Invoke(overlay,values);
            void Check(string name,bool condition){if(!condition)throw new InvalidOperationException(name);checks.Add(name);}
        }));
    }
    private static void Save(FrameworkElement element,string path)
    {
        var bitmap=new RenderTargetBitmap((int)Math.Ceiling(element.ActualWidth),(int)Math.Ceiling(element.ActualHeight),96,96,PixelFormats.Pbgra32);bitmap.Render(element);
        var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));using var stream=File.Create(path);encoder.Save(stream);
    }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject parent)
    {
        for(var i=0;i<VisualTreeHelper.GetChildrenCount(parent);i++){var child=VisualTreeHelper.GetChild(parent,i);yield return child;foreach(var nested in Descendants(child))yield return nested;}
    }
}
