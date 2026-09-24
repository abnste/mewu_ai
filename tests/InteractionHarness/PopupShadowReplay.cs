// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using ToolTip=System.Windows.Controls.ToolTip;
using ComboBox=System.Windows.Controls.ComboBox;
using Button=System.Windows.Controls.Button;
using Point=System.Windows.Point;

internal static class PopupShadowReplay
{
    internal static void Run(Application app,CaptureOverlayWindow overlay)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        Program.MarkReplayWindow(overlay,"悬浮层阴影验收 · 合成内容 · 自动关闭");
        overlay.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;
            var folder=Path.GetFullPath(".codex-build/popup-shadows");Directory.CreateDirectory(folder);
            ToolTip? tip=null;Popup? open=null;ComboBox? combo=null;
            try
            {
                await Layout();
                var toggle=(Button)overlay.FindName("HistoryToggle");
                tip=new ToolTip{Content=toggle.ToolTip,PlacementTarget=toggle,Placement=PlacementMode.Top,StaysOpen=true};
                foreach(var placement in new[]{PlacementMode.Top,PlacementMode.Bottom,PlacementMode.Left,PlacementMode.Right})
                {
                    tip.Placement=placement;tip.IsOpen=true;await Layout();
                    Verify(tip,(Border)tip.Template.FindName("ToolTipSurface",tip),"history-tooltip-"+placement);
                    tip.IsOpen=false;
                }
                foreach(var name in new[]{"ChannelPickerPopup","ReferencePicker"})
                {
                    var items=(StackPanel)overlay.FindName(name=="ChannelPickerPopup"?"ChannelPickerItems":"ReferencePickerItems");
                    items.Children.Add(new Button{Content=name=="ChannelPickerPopup"?"示例模型 · 当前对话":"@图片1 · 当前截图",Padding=new Thickness(12,8,12,8)});
                    open=(Popup)overlay.FindName(name);open.IsOpen=true;await Layout();
                    var child=(FrameworkElement)open.Child;
                    var surface=Descendants(child).OfType<Border>().First(b=>b.BorderThickness.Left==1);
                    Verify(child,surface,name);open.IsOpen=false;
                }
                combo=new ComboBox{Width=260,ItemsSource=new[]{"示例选项一","示例选项二","示例选项三"},SelectedIndex=0};
                var root=(Canvas)overlay.FindName("Root");Canvas.SetLeft(combo,80);Canvas.SetTop(combo,120);root.Children.Add(combo);
                await Layout();combo.IsDropDownOpen=true;await Layout();
                open=(Popup)combo.Template.FindName("PART_Popup",combo);
                Verify((FrameworkElement)open.Child,(Border)combo.Template.FindName("DropDownSurface",combo),"combo-dropdown");
                combo.IsDropDownOpen=false;
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally
            {
                if(tip is not null)tip.IsOpen=false;
                if(open is not null)open.IsOpen=false;
                if(combo is not null)combo.IsDropDownOpen=false;
                File.WriteAllText(Path.Combine(folder,"result.json"),JsonSerializer.Serialize(new{checks,failure}));
                overlay.Close();app.Shutdown(Environment.ExitCode);
            }
            async Task Layout(){await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);await Task.Delay(220);overlay.UpdateLayout();}
            void Verify(FrameworkElement element,Border surface,string name)
            {
                element.UpdateLayout();var origin=surface.TranslatePoint(new Point(),element);
                Check(name+"-surface-inset",origin.X>=20&&origin.Y>=20&&element.ActualWidth-origin.X-surface.ActualWidth>=20&&element.ActualHeight-origin.Y-surface.ActualHeight>=20);
                foreach(var scale in new[]{1.0,1.75,2.0})
                {
                    var width=(int)Math.Ceiling(element.ActualWidth*scale);var height=(int)Math.Ceiling(element.ActualHeight*scale);
                    var bitmap=new RenderTargetBitmap(width,height,96*scale,96*scale,PixelFormats.Pbgra32);bitmap.Render(element);
                    var pixels=new byte[width*height*4];bitmap.CopyPixels(pixels,width*4,0);
                    byte Alpha(int x,int y)=>pixels[(y*width+x)*4+3];
                    Check(name+"-"+scale+"-transparent-outer-edge",Enumerable.Range(0,width).All(x=>Alpha(x,0)==0&&Alpha(x,height-1)==0)&&Enumerable.Range(0,height).All(y=>Alpha(0,y)==0&&Alpha(width-1,y)==0));
                    Check(name+"-"+scale+"-visible-shadow",Alpha(width/2,(int)((origin.Y+surface.ActualHeight+3)*scale))>0);
                    var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));using var file=File.Create(Path.Combine(folder,name+"-"+scale+".png"));encoder.Save(file);
                }
            }
            void Check(string name,bool passed){if(!passed)throw new InvalidOperationException(name);checks.Add(name);}
        }));
    }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject root)
    {
        for(var i=0;i<VisualTreeHelper.GetChildrenCount(root);i++)
        {
            var child=VisualTreeHelper.GetChild(root,i);yield return child;
            foreach(var nested in Descendants(child))yield return nested;
        }
    }
}
