// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Globalization;
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
using Point=System.Windows.Point;
using Brushes=System.Windows.Media.Brushes;
using FlowDirection=System.Windows.FlowDirection;

internal static class IssueTwoReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
    internal static void Run(Application app,CaptureOverlayWindow overlay)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        Program.MarkReplayWindow(overlay,"Issue #2 验收 · 合成译文和公开图片");
        overlay.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;
            try
            {
                var lines=Enumerable.Range(0,12).Select(i=>new OcrLine("Original line "+(i+1),20+(i/6)*380,30+(i%6)*43,265,20,[])).ToArray();
                var texts=Enumerable.Range(0,12).Select(i=>$"第{i+1}行译文：这是一段比原文更长的翻译，用来检查相邻行与双栏内容不会重叠。"+(i%3==0?"还要保留这段补充说明。":"")).ToArray();
                var visual=new DrawingVisual();using(var dc=visual.RenderOpen())
                {
                    dc.DrawRectangle(Brushes.White,null,new Rect(0,0,760,320));
                    foreach(var line in lines)dc.DrawText(new FormattedText(line.Text,CultureInfo.InvariantCulture,FlowDirection.LeftToRight,new Typeface("Segoe UI"),17,Brushes.Black,1),new Point(line.X,line.Y));
                }
                var source=new RenderTargetBitmap(760,320,96,96,PixelFormats.Pbgra32);source.Render(visual);source.Freeze();
                var item=Invoke("CreateSelection",false)!;
                item.GetType().GetField("Bounds")!.SetValue(item,new Rect(65,110,760,320));
                ((IList)typeof(CaptureOverlayWindow).GetField("_selections",Private)!.GetValue(overlay)!).Add(item);
                Invoke("UpdateSelection",item);((Image)item.GetType().GetProperty("Image")!.GetValue(item)!).Source=source;
                foreach(var scale in new[]{1d,.8d})
                {
                    item.GetType().GetField("Bounds")!.SetValue(item,new Rect(65,110,760*scale,320*scale));Invoke("UpdateSelection",item);
                    ((Image)item.GetType().GetProperty("Image")!.GetValue(item)!).Source=source;
                    Invoke("RenderTextOverlays",item,source,lines,texts,true);overlay.UpdateLayout();
                    var canvas=(Canvas)item.GetType().GetProperty("TextOverlays")!.GetValue(item)!;
                    var hosts=canvas.Children.OfType<Grid>().Where(grid=>grid.Children.Count==1&&grid.Children[0].GetType().Name=="OutlinedTextVisual").ToArray();
                    Check("all-translation-lines-visible-"+scale,hosts.Length==lines.Length);
                    for(var i=0;i<hosts.Length;i++)
                    {
                        var bounds=new Rect(Canvas.GetLeft(hosts[i]),Canvas.GetTop(hosts[i]),hosts[i].Width,hosts[i].Height);
                        Check($"translation-{scale}-{i}-inside-selection",new Rect(0,0,760*scale,320*scale).Contains(bounds));
                        for(var j=0;j<i;j++){var other=new Rect(Canvas.GetLeft(hosts[j]),Canvas.GetTop(hosts[j]),hosts[j].Width,hosts[j].Height);var overlap=Rect.Intersect(bounds,other);Check($"translation-{scale}-{i}-{j}-no-overlap",overlap.IsEmpty||overlap.Width*overlap.Height<.001);}
                        var text=hosts[i].Children[0];var rows=(IReadOnlyList<string>)text.GetType().GetField("_lines",Private)!.GetValue(text)!;
                        var lineHeight=(double)text.GetType().GetField("_lineHeight",Private)!.GetValue(text)!;
                        Check($"translation-{scale}-{i}-full-text-fits",string.Concat(rows)==texts[i]&&rows.Count*lineHeight+2<=hosts[i].Height+.01);
                    }
                    Save(canvas,$"issue-two-translation-{scale}.png");
                    var exported=(BitmapSource)typeof(CaptureOverlayWindow).GetMethod("RenderTranslationOverlay",BindingFlags.Static|BindingFlags.NonPublic)!.Invoke(null,[item,760,320])!;
                    Check("translation-export-has-pixels-"+scale,exported.PixelWidth==760&&exported.PixelHeight==320);
                }
                Invoke("ShowAnswer");var answer=(MarkdownAnswerView)overlay.FindName("AnswerText");
                var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(source));using var memory=new MemoryStream();encoder.Save(memory);
                answer.Markdown="图片展示检查\n\n![合成双栏图片](data:image/png;base64,"+Convert.ToBase64String(memory.ToArray())+")\n\n![公开项目图标](https://raw.githubusercontent.com/abnste/mewu_ai/master/Assets/MewuAI.Icon.png)";
                Invoke("PositionPromptBar");await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                var ready=false;
                for(var attempt=0;attempt<500;attempt++)
                {
                    var images=Descendants(answer).OfType<Image>().Where(image=>image.Parent is Grid).ToArray();
                    if(images.Length>=2&&images.All(image=>image.Source is not null)){ready=true;break;}
                    await Task.Delay(50);
                }
                Check("inline-and-real-public-web-images-decoded",ready);Save(answer,"issue-two-reply-images.png");
                answer.SelectAll();Check("copy-keeps-image-description",answer.SelectedPlainText.Contains("合成双栏图片"));
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally
            {
                Directory.CreateDirectory(".codex-build");File.WriteAllText(".codex-build/issue-two-result.json",JsonSerializer.Serialize(new{checks,failure}));overlay.Close();app.Shutdown(Environment.ExitCode);
            }
            object? Invoke(string name,params object[] values)=>typeof(CaptureOverlayWindow).GetMethod(name,Private)!.Invoke(overlay,values);
            void Check(string name,bool value){if(!value)throw new InvalidOperationException(name);checks.Add(name);}
        }));
    }
    private static void Save(FrameworkElement element,string name)
    {
        var bitmap=new RenderTargetBitmap((int)Math.Ceiling(element.ActualWidth),(int)Math.Ceiling(element.ActualHeight),96,96,PixelFormats.Pbgra32);bitmap.Render(element);
        var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));Directory.CreateDirectory(".codex-build");using var stream=File.Create(Path.Combine(".codex-build",name));encoder.Save(stream);
    }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject parent)
    {
        for(var i=0;i<VisualTreeHelper.GetChildrenCount(parent);i++){var child=VisualTreeHelper.GetChild(parent,i);yield return child;foreach(var nested in Descendants(child))yield return nested;}
    }
}
