// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
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
using Brushes=System.Windows.Media.Brushes;
using Brush=System.Windows.Media.Brush;
using Pen=System.Windows.Media.Pen;
using FlowDirection=System.Windows.FlowDirection;
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
        overlay.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle,new Action(()=>
        {
            try
            {
                const BindingFlags flags=BindingFlags.Instance|BindingFlags.NonPublic;
                void Invoke(string name,params object[] args)=>typeof(CaptureOverlayWindow).GetMethod(name,flags)!.Invoke(overlay,args);
                string L(string zh,string en)=>english?en:zh;
                var questions=new[]{L("解方程：3x + 4 = 2(9 − 2x)","Solve: 3x + 4 = 2(9 − 2x)"),L("化简：(2a + 8b) − (3b − 2a)","Simplify: (2a + 8b) − (3b − 2a)"),L("展开：(2x + 1)²","Expand: (2x + 1)²"),L("用科学记数法表示 0.00056","Write 0.00056 in scientific notation")};
                var answers=new[]{"x = 14/3","4a + 5b","4x² + 1","5.6 × 10⁻?"};
                var expected=new[]{"x = 2","4a + 5b","4x² + 4x + 1","5.6 × 10⁻⁴"};
                var visual=new DrawingVisual();
                using(var dc=visual.RenderOpen())
                {
                    dc.DrawRectangle(Brushes.White,null,new Rect(0,0,1000,720));
                    Text(L("代数小练习","Algebra practice"),50,32,34,Brushes.Black);
                    Text(L("演示作答 · 请核对后保存","Demo answers · Review before saving"),50,86,20,Brushes.SlateGray);
                    for(var n=0;n<4;n++)
                    {
                        Text($"{n+1}. {questions[n]}",50,140+n*130,27,Brushes.Black);
                        Text(answers[n],600,184+n*130,32,Brushes.RoyalBlue);
                        if(n<3)dc.DrawLine(new Pen(Brushes.Gainsboro,1),new Point(50,247+n*130),new Point(950,247+n*130));
                    }
                    void Text(string value,double x,double y,double size,Brush brush)=>dc.DrawText(new FormattedText(value,CultureInfo.InvariantCulture,FlowDirection.LeftToRight,new Typeface("Microsoft YaHei"),size,brush,1),new Point(x,y));
                }
                var bitmap=new RenderTargetBitmap(1000,720,96,96,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();
                var page=TeachingImportService.Create(bitmap,L("演示 A","Demo A"),1);
                page.Items=Enumerable.Range(0,4).Select(n=>new GradingItem((n+1).ToString(),answers[n],expected[n],n==1?GradingVerdict.Correct:n==3?GradingVerdict.Uncertain:GradingVerdict.Incorrect,
                    n==0?L("移项得 7x = 14，因此 x = 2。注意右侧 −4x 的符号。","Collecting terms gives 7x = 14, so x = 2. Check the sign of −4x."):n==1?L("去括号并合并同类项，结果正确。","The brackets and like terms are handled correctly."):n==2?L("完全平方公式包含中间项 4x。","The square includes the middle term 4x."):L("指数无法辨认，请回看原卷。","The exponent is unclear. Check the original."),
                    n==1?L("去括号与合并同类项","Collecting like terms"):L("代数基础","Algebra"),.59,(180+n*130)/720d,.32,.065){Confirmed=n<3}).ToArray();
                page.Selected=false;page.Status=L("3 题已核对 · 1 题待核","3 reviewed · 1 uncertain");host.Teaching.AddRange([page]);
                Invoke("ToggleTeaching",overlay,new RoutedEventArgs());Invoke("ShowTeachingPage",page);Invoke("SetPromptBarHidden",true,false);
                var panel=(Border)typeof(CaptureOverlayWindow).GetField("_teachingPanel",flags)!.GetValue(overlay)!;
                var scroll=(ScrollViewer)panel.Child;overlay.UpdateLayout();
                var content=(StackPanel)scroll.Content;
                var review=content.Children.OfType<TextBlock>().First(t=>t.Text.StartsWith(L("逐题核对","Review"),StringComparison.Ordinal));
                scroll.ScrollToVerticalOffset(review.TranslatePoint(new Point(),content).Y);overlay.UpdateLayout();
                // Capture the actual overlay with only the original worksheet and seeded demo review.
                // No model call, saved settings, real desktop or student material is read.
                ((FrameworkElement)overlay.FindName("PointerInspector")).Visibility=Visibility.Collapsed;
                var height=(int)Math.Min(overlay.ActualHeight,Canvas.GetTop(panel)+panel.ActualHeight+24);
                var image=new RenderTargetBitmap((int)overlay.ActualWidth,height,96,96,PixelFormats.Pbgra32);image.Render(overlay);
                var path=Path.GetFullPath("docs/images/teaching-review-"+(english?"en":"zh")+".png");Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(image));using var output=File.Create(path);encoder.Save(output);
            }
            catch(Exception ex){Environment.ExitCode=1;Directory.CreateDirectory(".codex-build");File.WriteAllText(".codex-build/teaching-readme-error.txt",ex.ToString());}
            finally{overlay.Close();host.Teaching.Clear();app.Shutdown(Environment.ExitCode);}
        }));
#endif
    }
}
