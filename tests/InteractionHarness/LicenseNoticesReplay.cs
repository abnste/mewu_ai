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
using Button=System.Windows.Controls.Button;
using ComboBox=System.Windows.Controls.ComboBox;
using TextBox=System.Windows.Controls.TextBox;
using TabControl=System.Windows.Controls.TabControl;

internal static class LicenseNoticesReplay
{
    internal static void Run(Application app,AppHost host)
    {
        const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
        var checks=new List<string>();string? failure=null;
        var settings=new SettingsWindow(host){Title="许可信息验收 · 不读取用户设置",Topmost=true};
        ((CancellationTokenSource)typeof(SettingsWindow).GetField("_windowLifetime",Private)!.GetValue(settings)!).Cancel();
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        settings.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            try
            {
                var tabs=Descendants(settings).OfType<TabControl>().First();tabs.SelectedIndex=tabs.Items.Count-1;
                await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                var entry=Descendants(settings).OfType<Button>().Single(b=>b.Content?.ToString() is "开源许可与第三方声明" or "Open-source licenses and third-party notices");
                entry.BringIntoView();settings.UpdateLayout();Save(settings,"license-about.png");
                _=app.Dispatcher.BeginInvoke(new Action(async()=>
                {
                    Window? viewer=null;
                    try
                    {
                        viewer=app.Windows.Cast<Window>().Single(w=>w.GetType().Name=="LicenseNoticesWindow");
                        await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                        Check("viewer-owned-by-settings",viewer.Owner==settings);
                        var type=viewer.GetType();var picker=(ComboBox)type.GetField("_documents",Private)!.GetValue(viewer)!;
                        var text=(TextBox)type.GetField("_text",Private)!.GetValue(viewer)!;
                        var overview=(MarkdownAnswerView)type.GetField("_overview",Private)!.GetValue(viewer)!;
                        Check("all-bundled-documents-listed",picker.Items.Count==3+Directory.GetFiles(Path.Combine(AppContext.BaseDirectory,"Licenses"),"*.txt").Length);
                        Save(viewer,"license-overview.png");
                        for(var index=0;index<picker.Items.Count;index++)
                        {
                            picker.SelectedIndex=index;
                            var doc=picker.SelectedItem!;var path=(string)doc.GetType().GetProperty("Path")!.GetValue(doc)!;
                            var markdown=(bool)doc.GetType().GetProperty("Markdown")!.GetValue(doc)!;
                            Check("exact-original-"+Path.GetFileName(path),(markdown?overview.Markdown:text.Text)==File.ReadAllText(path));
                        }
                        picker.SelectedIndex=1;Save(viewer,"license-mpl.png");
                        viewer.Width=600;viewer.Height=400;await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);viewer.UpdateLayout();Save(viewer,"license-narrow.png");
                        Check("narrow-reader-remains-visible",text.ActualHeight>140&&picker.ActualWidth<=600);
                        var close=Descendants(viewer).OfType<Button>().Single(b=>b.Content?.ToString()=="×");
                        Check("narrow-close-button-within-window",close.TranslatePoint(new System.Windows.Point(close.ActualWidth,0),viewer).X<=viewer.ActualWidth);
                    }
                    catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
                    finally{viewer?.Close();}
                }));
                entry.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally
            {
                Directory.CreateDirectory(".codex-build");File.WriteAllText(".codex-build/license-notices-result.json",JsonSerializer.Serialize(new{checks,failure}));
                settings.Close();app.Shutdown(Environment.ExitCode);
            }
            void Check(string name,bool valid){if(!valid)throw new InvalidOperationException(name);checks.Add(name);}
        }));
        app.Run(settings);
    }
    private static void Save(FrameworkElement element,string name)
    {
        element.UpdateLayout();var bitmap=new RenderTargetBitmap((int)Math.Ceiling(element.ActualWidth),(int)Math.Ceiling(element.ActualHeight),96,96,PixelFormats.Pbgra32);bitmap.Render(element);
        var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));Directory.CreateDirectory(".codex-build");using var stream=File.Create(Path.Combine(".codex-build",name));encoder.Save(stream);
    }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject parent)
    {
        for(var i=0;i<VisualTreeHelper.GetChildrenCount(parent);i++){var child=VisualTreeHelper.GetChild(parent,i);yield return child;foreach(var nested in Descendants(child))yield return nested;}
    }
}
