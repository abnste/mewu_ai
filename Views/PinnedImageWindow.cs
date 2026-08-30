using Microsoft.Win32;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Effects;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public sealed class PinnedImageWindow : Window
{
    private const int ShadowPixels=16;
    private readonly BitmapSource _image;
    private readonly ScreenRect? _originalRegion;
    private readonly Border _frame;
    private MenuItem? _topmostItem, _opacityItem;
    private bool _adjustingSize;

    public PinnedImageWindow(BitmapSource image,ScreenRect? originalRegion=null)
    {
        _image=image;_originalRegion=originalRegion;Title="喵呜AI 贴图";WindowStyle=WindowStyle.None;ResizeMode=ResizeMode.CanResize;Topmost=true;ShowInTaskbar=NativeMethods.VisualQaCaptureEnabled;Background=Brushes.Transparent;AllowsTransparency=true;
        _frame=new Border{Background=Brushes.White,CornerRadius=new CornerRadius(7),BorderBrush=new SolidColorBrush(Color.FromArgb(120,145,158,177)),BorderThickness=new Thickness(1),Effect=new DropShadowEffect{Color=Color.FromRgb(42,55,72),BlurRadius=26,ShadowDepth=5,Opacity=.38},Child=new Image{Source=image,Stretch=Stretch.Fill,SnapsToDevicePixels=true}};
        Content=_frame;Width=Math.Min(image.PixelWidth,900)+ShadowPixels*2;Height=(Width-ShadowPixels*2)*image.PixelHeight/image.PixelWidth+ShadowPixels*2;
        SizeChanged+=KeepAspectRatio;DpiChanged+=OnDpiChanged;MouseLeftButtonDown+=OnMouseLeftButtonDown;MouseWheel+=OnMouseWheel;ContextMenu=BuildContextMenu();
        SourceInitialized+=(_,_)=>{var handle=new System.Windows.Interop.WindowInteropHelper(this).Handle;NativeMethods.ExcludeFromCapture(handle);if(_originalRegion is { } region)PlaceAtOriginalSize(handle,region);else{ApplyShadowPadding(handle);UpdateHeightForAspectRatio();}};
    }

    private void PlaceAtOriginalSize(IntPtr handle,ScreenRect region)
    {
        _adjustingSize=true;
        try{NativeMethods.SetWindowPos(handle,Topmost?new IntPtr(-1):new IntPtr(-2),region.X-ShadowPixels,region.Y-ShadowPixels,region.Width+ShadowPixels*2,region.Height+ShadowPixels*2,0x0040);ApplyShadowPadding(handle);}
        finally{_adjustingSize=false;}
    }

    private void ApplyShadowPadding(IntPtr handle)
    {
        var dpi=Math.Max(96,NativeMethods.GetDpiForWindow(handle));var padding=ShadowPixels*96d/dpi;_frame.Margin=new Thickness(padding);
    }

    private void OnDpiChanged(object sender,DpiChangedEventArgs e)
    {
        var handle=new System.Windows.Interop.WindowInteropHelper(this).Handle;if(handle==IntPtr.Zero)return;ApplyShadowPadding(handle);UpdateHeightForAspectRatio();
    }

    private void KeepAspectRatio(object? sender,SizeChangedEventArgs e)
    {
        UpdateHeightForAspectRatio();
    }

    private void UpdateHeightForAspectRatio()
    {
        if(_adjustingSize)return;var padding=_frame.Margin.Left*2;var contentWidth=Math.Max(1,ActualWidth-padding);var expected=contentWidth*_image.PixelHeight/_image.PixelWidth+padding;if(Math.Abs(ActualHeight-expected)<1)return;_adjustingSize=true;Height=Math.Min(expected,2400);_adjustingSize=false;
    }

    private void OnMouseLeftButtonDown(object sender,MouseButtonEventArgs e)
    {
        if(e.ClickCount==2){Topmost=false;UpdateTopmostHeader();e.Handled=true;return;}if(e.ButtonState==MouseButtonState.Pressed)DragMove();
    }

    private void OnMouseWheel(object sender,MouseWheelEventArgs e)
    {
        var factor=e.Delta>0?1.08:.92;var minimumWidth=_frame.Margin.Left*2+1;Width=Math.Clamp(Width*factor,minimumWidth,2400);
    }

    private ContextMenu BuildContextMenu()
    {
        var menu=new ContextMenu();menu.SetResourceReference(StyleProperty,typeof(ContextMenu));Add(menu,"复制图片",CopyImage);Add(menu,"保存图片…",Save);AddSeparator(menu);Add(menu,"恢复原位原大小",RestoreOriginal);_topmostItem=Add(menu,"保持置顶 ✓",ToggleTopmost);_opacityItem=Add(menu,"设为 80% 透明度",ToggleOpacity);AddSeparator(menu);Add(menu,"关闭贴图",Close);return menu;
    }

    private static MenuItem Add(ContextMenu menu,string text,Action action){var item=new MenuItem{Header=text};item.SetResourceReference(StyleProperty,typeof(MenuItem));item.Click+=(_,_)=>action();menu.Items.Add(item);return item;}
    private static void AddSeparator(ContextMenu menu){var separator=new Separator();separator.SetResourceReference(StyleProperty,typeof(Separator));menu.Items.Add(separator);}
    private void ToggleTopmost(){Topmost=!Topmost;UpdateTopmostHeader();}
    private void UpdateTopmostHeader(){if(_topmostItem is not null)_topmostItem.Header=Topmost?"保持置顶 ✓":"恢复置顶";}
    private void ToggleOpacity(){Opacity=Opacity<1?1:.8;if(_opacityItem is not null)_opacityItem.Header=Opacity<1?"恢复 100% 不透明度":"设为 80% 透明度";}
    private void RestoreOriginal(){if(_originalRegion is not { } region)return;var handle=new System.Windows.Interop.WindowInteropHelper(this).Handle;PlaceAtOriginalSize(handle,region);}
    private void CopyImage(){if(!ClipboardService.TrySetImage(_image,out var error))MessageBox.Show(this,error??"复制图片失败，请稍后重试","复制失败",MessageBoxButton.OK,MessageBoxImage.Warning);}
    private void Save(){var dialog=new SaveFileDialog{Filter="PNG 图片|*.png|JPEG 图片|*.jpg;*.jpeg",DefaultExt=".png"};if(dialog.ShowDialog(this)!=true)return;try{ScreenCaptureService.Save(_image,dialog.FileName,dialog.FilterIndex==2);}catch(Exception ex){new PrivacyLogger().Error("PinnedImageSave",ex);MessageBox.Show(this,$"图片保存失败：{ex.Message}","保存失败",MessageBoxButton.OK,MessageBoxImage.Warning);}}
}
