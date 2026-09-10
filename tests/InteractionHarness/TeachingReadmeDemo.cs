// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Point=System.Windows.Point;

internal static class TeachingReadmeDemo
{
    internal static void Run(Application app,AppHost host,CaptureOverlayWindow overlay,bool english)
    {
#if !DEBUG
        throw new InvalidOperationException("README images require the Debug QA capture switch.");
#else
        if(Environment.GetEnvironmentVariable("MEWU_QA_CAPTURE_WINDOWS")!="1")throw new InvalidOperationException("Explicit QA capture switch required.");
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        overlay.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle,new Action(async()=>
        {
            try
            {
                const BindingFlags flags=BindingFlags.Instance|BindingFlags.NonPublic;
                void Invoke(string name,params object[] args)=>typeof(CaptureOverlayWindow).GetMethod(name,flags)!.Invoke(overlay,args);
                string L(string zh,string en)=>english?en:zh;
                var folder=Path.GetFullPath(".codex-build/teaching-evaluation");
                using var deadline=new CancellationTokenSource(TimeSpan.FromSeconds(60));
                var page=(await TeachingImportService.ImportAsync(Path.Combine(folder,"real-handwriting-level2.pdf"),L("HKDSE 公开手写答卷","HKDSE handwritten script"),"3",1,deadline.Token)).Single();
                using var saved=System.Text.Json.JsonDocument.Parse(File.ReadAllText(Path.Combine(folder,"workflow-api-handwriting-Official handwriting.json")));
                var rows=System.Text.Json.JsonSerializer.Deserialize<GradingItem[]>(saved.RootElement.GetProperty("Items"))??throw new InvalidDataException("Missing actual grading results");
                if(rows.Length!=2||!rows.Select(r=>r.Question).SequenceEqual(new[]{"1","2"}))throw new InvalidDataException("Expected the measured HKDSE handwritten script");
                page.Items=await mewu_ai_Assistant.AI.TeachingGradingService.RefineBoundsAsync(page,rows,deadline.Token);
                page.Selected=false;page.Status=L("MiniMax 实测结果 · 待老师确认","Recorded MiniMax grading · Teacher review required");host.Teaching.AddRange([page]);
                Invoke("ToggleTeaching",overlay,new RoutedEventArgs());Invoke("ShowTeachingPage",page);Invoke("SetPromptBarHidden",true,false);
                var panel=(Border)typeof(CaptureOverlayWindow).GetField("_teachingPanel",flags)!.GetValue(overlay)!;
                var scroll=(ScrollViewer)panel.Child;overlay.UpdateLayout();
                var content=(StackPanel)scroll.Content;
                // Reviewer corrections made by reading the official scan, not
                // generated model output. Exercise the real edit/confirm/box UI.
                var firstEditor=Card("1").Children.OfType<System.Windows.Controls.TextBox>().First();
                Card("1").Children.OfType<System.Windows.Controls.Button>().First(b=>Equals(b.Content,L("编辑原文","Edit source"))).RaiseEvent(new RoutedEventArgs(System.Windows.Controls.Button.ClickEvent));overlay.UpdateLayout();
                if(!firstEditor.AcceptsReturn||firstEditor.TextWrapping!=TextWrapping.Wrap||!firstEditor.Text.Contains('\n')||firstEditor.LineCount<3)throw new InvalidDataException("Calculation steps were flattened");
                var corrections=new[]
                {
                    new{Question="1",Observed="= (x⁴y⁻⁵)³ / (xy²)\n= x¹²y⁻¹⁵ / (xy²)\n= x^(12−1)y^(−15−2)\n= x¹³y⁻¹⁷\n= x¹³/y¹⁷",Expected="x¹¹/y¹⁷",Reason=L("12−1 应为 11，原卷后两行写成了 13。","12−1 is 11; the final two lines use 13."),Box=new Rect(.18,.18,.30,.25)},
                    new{Question="2",Observed="= 1/(3d−4) − 2/(6d+5)\n= [(6d+5) − 2(3d−4)] / [(6d−5)(3d−4)]\n= (6d+5−6d+8) / [(6d−5)(3d−4)]\n= 13 / [(6d−5)(3d−4)]",Expected="13 / [(3d−4)(6d+5)]",Reason=L("通分后的分母把 6d+5 写成了 6d−5。\n分子 13 算对了，但最终分式错误。","The common denominator changes 6d+5 to 6d−5.\nThe numerator 13 is correct, but the final fraction is not."),Box=new Rect(.16,.56,.42,.25)}
                };
                foreach(var correction in corrections)
                {
                    var preview=typeof(CaptureOverlayWindow).GetField("_teachingPreview",flags)!.GetValue(overlay)!;
                    var bounds=(Rect)preview.GetType().GetField("Bounds")!.GetValue(preview)!;
                    typeof(CaptureOverlayWindow).GetField("_teachingRepositionQuestion",flags)!.SetValue(overlay,correction.Question);
                    var start=new Point(bounds.X+correction.Box.X*bounds.Width,bounds.Y+correction.Box.Y*bounds.Height);
                    var end=new Point(start.X+correction.Box.Width*bounds.Width,start.Y+correction.Box.Height*bounds.Height);
                    Invoke("TeachingRepositionDown",start);Invoke("TeachingRepositionMove",end);Invoke("TeachingRepositionUp",end);
                    var card=Card(correction.Question);var editors=card.Children.OfType<System.Windows.Controls.TextBox>().ToArray();
                    editors[0].Text=correction.Observed;editors[1].Text=correction.Expected;editors[2].Text=correction.Reason;
                    editors[3].Text=correction.Question=="1"?L("指数定律","Laws of indices"):L("分式运算","Algebraic fractions");
                    card.Children.OfType<System.Windows.Controls.ComboBox>().Single().SelectedValue=GradingVerdict.Incorrect;
                    card.Children.OfType<System.Windows.Controls.Button>().Single(b=>Equals(b.Content,L("确认这道题","Confirm this question"))).RaiseEvent(new RoutedEventArgs(System.Windows.Controls.Button.ClickEvent));
                    var accepted=page.Items.Single(i=>i.Question==correction.Question);
                    if(!accepted.Confirmed||accepted.Verdict!=GradingVerdict.Incorrect||accepted.Observed!=correction.Observed||accepted.Reason!=correction.Reason)throw new InvalidDataException("Review edits were not saved");
                }
                var annotations=TeachingSession.Annotations(page,page.Id);
                if(page.Status!=L("已完成逐题核对","Review complete"))throw new InvalidDataException("Completed review still has a stale pending status");
                if(corrections.Any(c=>!annotations.Any(a=>a.Text.Contains(c.Expected,StringComparison.Ordinal)&&a.Text.Contains(c.Reason,StringComparison.Ordinal))))throw new InvalidDataException("Original-page feedback is missing");
                if(english&&page.Items.Any(i=>i.Skill.Any(c=>c is >= '\u4e00' and <= '\u9fff')||i.Reason.Any(c=>c is >= '\u4e00' and <= '\u9fff')))throw new InvalidDataException("English review contains untranslated generated content");
                var export=Path.Combine(folder,"reviewed-handwriting-steps.zip");Invoke("ExportTeachingTo",export);
                using(var zip=System.IO.Compression.ZipFile.OpenRead(export))
                using(var reader=new StreamReader(zip.GetEntry("review.csv")!.Open()))
                {
                    var csv=reader.ReadToEnd();if(corrections.Any(c=>!csv.Contains(c.Observed,StringComparison.Ordinal)||!csv.Contains(c.Reason,StringComparison.Ordinal)))throw new InvalidDataException("Export lost reviewed step breaks");
                }
                overlay.UpdateLayout();
                var review=content.Children.OfType<TextBlock>().First(t=>t.Text.StartsWith(L("逐题核对 · ","Review · "),StringComparison.Ordinal));
                scroll.ScrollToVerticalOffset(review.TranslatePoint(new Point(),content).Y);overlay.UpdateLayout();
                // Preserve the full official scan, including genuine handwriting and page number.
                // Show the completed review, explicitly identified in README as reviewer-corrected.
                // Original PDF and raw response files remain outside the repository.
                ((FrameworkElement)overlay.FindName("PointerInspector")).Visibility=Visibility.Collapsed;
                ((FrameworkElement)overlay.FindName("PromptBarHost")).Visibility=Visibility.Collapsed;
                var height=(int)Math.Min(overlay.ActualHeight,Canvas.GetTop(panel)+panel.ActualHeight+24);
                var image=new RenderTargetBitmap((int)overlay.ActualWidth*2,height*2,192,192,PixelFormats.Pbgra32);image.Render(overlay);
                var path=Path.GetFullPath("docs/images/hkdse-2025-inplace-math-"+(english?"en":"zh")+".png");Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(image));using(var output=File.Create(path))encoder.Save(output);
                content.Children.OfType<System.Windows.Controls.Button>().Single(b=>Equals(b.Content,L("查看原卷批注","View annotations on paper"))).RaiseEvent(new RoutedEventArgs(System.Windows.Controls.Button.ClickEvent));overlay.UpdateLayout();
                if(panel.Visibility!=Visibility.Collapsed)throw new InvalidDataException("Paper view did not close the review panel");
                var enlarged=typeof(CaptureOverlayWindow).GetField("_teachingPreview",flags)!.GetValue(overlay)!;
                var enlargedHost=(FrameworkElement)enlarged.GetType().GetProperty("Host")!.GetValue(enlarged)!;
                var enlargedBounds=(Rect)enlarged.GetType().GetField("Bounds")!.GetValue(enlarged)!;
                var whole=new RenderTargetBitmap((int)Math.Ceiling(overlay.ActualWidth*2),(int)Math.Ceiling(overlay.ActualHeight*2),192,192,PixelFormats.Pbgra32);whole.Render(overlay);
                var paperShot=new CroppedBitmap(whole,new Int32Rect((int)Math.Ceiling(enlargedBounds.X*2),(int)Math.Ceiling(enlargedBounds.Y*2),(int)Math.Floor(enlargedHost.ActualWidth*2),(int)Math.Floor(enlargedHost.ActualHeight*2)));
                var paperEncoder=new PngBitmapEncoder();paperEncoder.Frames.Add(BitmapFrame.Create(paperShot));using(var paperOutput=File.Create(Path.GetFullPath("docs/images/hkdse-2025-on-paper-"+(english?"en":"zh")+".png")))paperEncoder.Save(paperOutput);
                var back=(System.Windows.Controls.Button)typeof(CaptureOverlayWindow).GetField("_teachingReturnButton",flags)!.GetValue(overlay)!;
                back.RaiseEvent(new RoutedEventArgs(System.Windows.Controls.Button.ClickEvent));overlay.UpdateLayout();if(panel.Visibility!=Visibility.Visible)throw new InvalidDataException("Cannot return to the review");
                StackPanel Card(string question)=>content.Children.OfType<Border>().Select(b=>b.Child).OfType<StackPanel>().Single(c=>c.Children.OfType<TextBlock>().First().Text.StartsWith(question+" ·",StringComparison.Ordinal));
            }
            catch(Exception ex){Environment.ExitCode=1;Directory.CreateDirectory(".codex-build");File.WriteAllText(".codex-build/teaching-readme-error.txt",ex.ToString());}
            finally{overlay.Close();host.Teaching.Clear();app.Shutdown(Environment.ExitCode);}
        }));
#endif
    }
}
