// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.OCR;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Brushes=System.Windows.Media.Brushes;
using Image=System.Windows.Controls.Image;
using Point=System.Windows.Point;
using FlowDirection=System.Windows.FlowDirection;

internal static class TranslationReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
    internal static void Run(Application app,CaptureOverlayWindow overlay,bool live)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        Program.MarkReplayWindow(overlay,"原位翻译验收 · 合成材料 · 完成后自动关闭");
        overlay.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;string? model=null;
            double ocrMs=0,serialMs=0,parallelMs=0;var ocrLines=0;
            Directory.CreateDirectory(".codex-build/translation");
            try
            {
                // The source is drawn by this fixture; real OCR coordinates,
                // rather than hand-authored boxes, drive the live overlay.
                var source=CreateSource();var watch=Stopwatch.StartNew();
                var document=await new WindowsOcrService(null).RecognizeAsync(source,CancellationToken.None);
                ocrMs=watch.Elapsed.TotalMilliseconds;ocrLines=document.Lines.Count;
                Check("real-ocr-detected-synthetic-lines",ocrLines>=24);
                IReadOnlyList<string> translations;
                if(live)
                {
                    // Read-only snapshot: do not invoke SettingsService.Load,
                    // migrate credentials, save settings or load user history.
                    var path=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","settings.json");
                    using var settingsStream=File.OpenRead(path);
                    var options=new JsonSerializerOptions{Converters={new JsonStringEnumConverter()}};
                    var settings=JsonSerializer.Deserialize<AppSettings>(settingsStream,options)??throw new InvalidDataException("Missing settings");
                    var provider=new AiProviderFactory(null,null).Create(settings)??throw new InvalidOperationException("Configured translation provider unavailable");
                    model=settings.Providers.Single(p=>p.Id==settings.DefaultProviderId).Model;
                    var lines=document.Lines.Select(line=>line.Text).ToArray();
                    var service=new InPlaceTranslationService();using var timeout=new CancellationTokenSource(TimeSpan.FromMinutes(5));
                    watch.Restart();translations=await service.TranslateAsync(provider,lines,"Simplified Chinese",null,timeout.Token);
                    parallelMs=watch.Elapsed.TotalMilliseconds;
                    watch.Restart();
                    foreach(var batch in CaptureOverlayPolicy.CreateTranslationBatches(lines,12,1600))
                        await service.TranslateAsync(provider,batch.Lines,"Simplified Chinese",null,timeout.Token);
                    serialMs=watch.Elapsed.TotalMilliseconds;
                    Check("live-provider-returned-every-line",translations.Count==ocrLines&&translations.All(value=>!string.IsNullOrWhiteSpace(value)));
                    File.WriteAllText(".codex-build/translation/synthetic-translations.json",JsonSerializer.Serialize(document.Lines.Zip(translations,(line,text)=>new{source=line.Text,translation=text,line.X,line.Y,line.Width,line.Height}),new JsonSerializerOptions{WriteIndented=true}));
                }
                else translations=document.Lines.Select((line,i)=>$"第{i+1}条完整译文：在原文的位置显示，检查文字不会漂移。").ToArray();

                var item=Invoke("CreateSelection",false)!;var type=item.GetType();
                ((IList)typeof(CaptureOverlayWindow).GetField("_selections",Private)!.GetValue(overlay)!).Add(item);
                foreach(var scale in new[]{1d,.8d,.5d})
                {
                    type.GetField("Bounds")!.SetValue(item,new Rect(80,85,source.PixelWidth*scale,source.PixelHeight*scale));
                    Invoke("UpdateSelection",item);((Image)type.GetProperty("Image")!.GetValue(item)!).Source=source;
                    Invoke("RenderTextOverlays",item,source,document.Lines,translations,true);overlay.UpdateLayout();
                    var canvas=(Canvas)type.GetProperty("TextOverlays")!.GetValue(item)!;
                    var hosts=canvas.Children.OfType<Grid>().Where(grid=>grid.Children.Count==1&&grid.Children[0] is OutlinedTextVisual).ToArray();
                    Check($"{scale}-all-rows-visible",hosts.Length==ocrLines);
                    for(var i=0;i<hosts.Length;i++)
                    {
                        var original=document.Lines[i];var host=hosts[i];
                        Check($"{scale}-{i}-horizontal-anchor",Math.Abs(Canvas.GetLeft(host)+3-original.X*scale)<.1);
                        Check($"{scale}-{i}-vertical-anchor",Math.Abs(Canvas.GetTop(host)+1-original.Y*scale)<.1);
                        var bounds=new Rect(Canvas.GetLeft(host),Canvas.GetTop(host),host.Width,host.Height);
                        for(var j=0;j<i;j++)
                        {
                            var other=hosts[j];var intersection=Rect.Intersect(bounds,new Rect(Canvas.GetLeft(other),Canvas.GetTop(other),other.Width,other.Height));
                            Check($"{scale}-{i}-{j}-no-overlap",intersection.IsEmpty||intersection.Width*intersection.Height<.001);
                        }
                        var rows=(IReadOnlyList<string>)typeof(OutlinedTextVisual).GetField("_lines",Private)!.GetValue(host.Children[0])!;
                        Check($"{scale}-{i}-complete-text",string.Concat(rows).Replace(" ","")==translations[i].Replace(" ",""));
                    }
                    var bitmap=(BitmapSource)typeof(CaptureOverlayWindow).GetMethod("RenderTranslationOverlay",BindingFlags.Static|BindingFlags.NonPublic)!.Invoke(null,[item,source.PixelWidth,source.PixelHeight])!;
                    Check($"{scale}-export-physical-size",bitmap.PixelWidth==source.PixelWidth&&bitmap.PixelHeight==source.PixelHeight);
                    var corner=new byte[4];bitmap.CopyPixels(new Int32Rect(1,1,1,1),corner,4,0);
                    Check($"{scale}-export-preserves-empty-top-left-margin",corner[3]==0);
                    Save(bitmap,$"translation-overlay-{scale}.png");
                    if(scale==1)SaveComparison(source,bitmap);
                }
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
            }
            catch(Exception ex){failure=ex.GetType().Name+": "+(ex is TargetInvocationException?ex.InnerException?.GetType().Name:"Replay did not finish");}
            finally
            {
                File.WriteAllText(".codex-build/translation/result.json",JsonSerializer.Serialize(new{checks,failure,live,model,ocrLines,ocrMs,serialMs,parallelMs},new JsonSerializerOptions{WriteIndented=true}));
                overlay.Close();app.Shutdown(failure is null?0:1);
            }
            object? Invoke(string name,params object[] values)=>typeof(CaptureOverlayWindow).GetMethod(name,Private)!.Invoke(overlay,values);
            void Check(string name,bool passed){if(!passed)throw new InvalidOperationException(name);checks.Add(name);}
        }));
    }

    private static BitmapSource CreateSource()
    {
        var visual=new DrawingVisual();using(var dc=visual.RenderOpen())
        {
            dc.DrawRectangle(Brushes.White,null,new Rect(0,0,1040,700));
            for(var i=0;i<28;i++)
            {
                var sentence=$"Item {i+1}: Read the instructions carefully.";
                dc.DrawText(new FormattedText(sentence,CultureInfo.InvariantCulture,FlowDirection.LeftToRight,new Typeface("Segoe UI"),20,Brushes.Black,1),new Point(25+(i/14)*520,20+(i%14)*47));
            }
        }
        var image=new RenderTargetBitmap(1040,700,96,96,PixelFormats.Pbgra32);image.Render(visual);image.Freeze();return image;
    }
    private static void SaveComparison(BitmapSource source,BitmapSource translated)
    {
        var visual=new DrawingVisual();using(var dc=visual.RenderOpen())
        {
            dc.DrawImage(source,new Rect(0,0,1040,700));dc.DrawImage(source,new Rect(1060,0,1040,700));dc.DrawImage(translated,new Rect(1060,0,1040,700));
        }
        var result=new RenderTargetBitmap(2100,700,96,96,PixelFormats.Pbgra32);result.Render(visual);Save(result,"comparison.png");
    }
    private static void Save(BitmapSource image,string name)
    {
        var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(image));using var file=File.Create(Path.Combine(".codex-build/translation",name));encoder.Save(file);
    }
}
