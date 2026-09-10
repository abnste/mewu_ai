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
                var page=(await TeachingImportService.ImportAsync(Path.Combine(folder,"student-A.png"),L("模拟作答 A","Simulated A"),"",11,deadline.Token)).Single();
                using var saved=System.Text.Json.JsonDocument.Parse(File.ReadAllText(Path.Combine(folder,"workflow-api-A.json")));
                var rows=System.Text.Json.JsonSerializer.Deserialize<GradingItem[]>(saved.RootElement.GetProperty("Items"))??throw new InvalidDataException("Missing actual grading results");
                if(rows.Length!=6||!rows.Select(r=>r.Question).SequenceEqual(new[]{"24","25","26","27","28","29"}))throw new InvalidDataException("Expected the measured TSA page excerpt");
                page.Items=await mewu_ai_Assistant.AI.TeachingGradingService.RefineBoundsAsync(page,rows,deadline.Token);
                page.Selected=false;page.Status=L("MiniMax 实测结果 · 待老师确认","Recorded MiniMax grading · Teacher review required");host.Teaching.AddRange([page]);
                Invoke("ToggleTeaching",overlay,new RoutedEventArgs());Invoke("ShowTeachingPage",page);Invoke("SetPromptBarHidden",true,false);
                var panel=(Border)typeof(CaptureOverlayWindow).GetField("_teachingPanel",flags)!.GetValue(overlay)!;
                var scroll=(ScrollViewer)panel.Child;overlay.UpdateLayout();
                var content=(StackPanel)scroll.Content;
                var review=content.Children.OfType<TextBlock>().First(t=>t.Text.StartsWith(L("逐题核对","Review"),StringComparison.Ordinal));
                scroll.ScrollToVerticalOffset(review.TranslatePoint(new Point(),content).Y);overlay.UpdateLayout();
                // Capture the official TSA excerpt with the actual saved MiniMax grading output.
                // Blue answers are explicitly simulated; no model verdicts are invented here.
                // Original PDF and raw response files remain outside the repository.
                ((FrameworkElement)overlay.FindName("PointerInspector")).Visibility=Visibility.Collapsed;
                ((FrameworkElement)overlay.FindName("PromptBarHost")).Visibility=Visibility.Collapsed;
                var height=(int)Math.Min(overlay.ActualHeight,Canvas.GetTop(panel)+panel.ActualHeight+24);
                var image=new RenderTargetBitmap((int)overlay.ActualWidth,height,96,96,PixelFormats.Pbgra32);image.Render(overlay);
                var path=Path.GetFullPath("docs/images/tsa-2025-official-grading-"+(english?"en":"zh")+".png");Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(image));using var output=File.Create(path);encoder.Save(output);
            }
            catch(Exception ex){Environment.ExitCode=1;Directory.CreateDirectory(".codex-build");File.WriteAllText(".codex-build/teaching-readme-error.txt",ex.ToString());}
            finally{overlay.Close();host.Teaching.Clear();app.Shutdown(Environment.ExitCode);}
        }));
#endif
    }
}
