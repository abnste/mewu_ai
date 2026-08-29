using Microsoft.Win32;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Services;
using ContextMenu=System.Windows.Controls.ContextMenu;
using MenuItem=System.Windows.Controls.MenuItem;
namespace mewu_ai_Assistant.Views;
public sealed class PinnedImageWindow : Window
{
    private readonly BitmapSource _image;
    public PinnedImageWindow(BitmapSource image)
    {
        _image=image;Title="喵呜AI 贴图";WindowStyle=WindowStyle.None;ResizeMode=ResizeMode.CanResizeWithGrip;Topmost=true;ShowInTaskbar=false;Background=Brushes.Transparent;AllowsTransparency=true;
        Width=Math.Min(image.PixelWidth,900);Height=Width*image.PixelHeight/image.PixelWidth;Content=new Image{Source=image,Stretch=Stretch.Uniform};
        MouseLeftButtonDown+=(_,e)=>{if(e.ButtonState==MouseButtonState.Pressed)DragMove();};MouseWheel+=(_,e)=>{var factor=e.Delta>0?1.08:.92;Width=Math.Clamp(Width*factor,120,2400);Height=Width*image.PixelHeight/image.PixelWidth;};
        var menu=new ContextMenu();Add(menu,"复制",()=>Clipboard.SetImage(_image));Add(menu,"保存",Save);Add(menu,"100%",()=>{Width=image.PixelWidth;Height=image.PixelHeight;});Add(menu,"置顶",()=>Topmost=!Topmost);Add(menu,"透明度 80%",()=>Opacity=Opacity<1?1:.8);Add(menu,"关闭",Close);ContextMenu=menu;
        SourceInitialized+=(_,_)=>NativeMethods.SetWindowDisplayAffinity(new System.Windows.Interop.WindowInteropHelper(this).Handle,NativeMethods.WdaExcludeFromCapture);
    }
    private static void Add(ContextMenu menu,string text,Action action){var item=new MenuItem{Header=text};item.Click+=(_,_)=>action();menu.Items.Add(item);}
    private void Save(){var d=new SaveFileDialog{Filter="PNG 图片|*.png|JPEG 图片|*.jpg;*.jpeg",DefaultExt=".png"};if(d.ShowDialog(this)==true)ScreenCaptureService.Save(_image,d.FileName,d.FilterIndex==2);}
}
