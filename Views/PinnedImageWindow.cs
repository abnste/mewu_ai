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
    private const int ShadowPixels=12;
    private readonly BitmapSource _image;
    private readonly ScreenRect? _originalRegion;
    private readonly Border _frame;
    private MenuItem? _topmostItem, _opacityItem;
    private bool _adjustingSize;
    private readonly PinnedWindowDragController _drag;
    private readonly int _initialContentWidthPixels;

    public PinnedImageWindow(BitmapSource image,ScreenRect? originalRegion=null)
    {
        _image=image;_originalRegion=originalRegion;_initialContentWidthPixels=Math.Min(image.PixelWidth,900);_drag=new PinnedWindowDragController(this);Title="喵呜AI 贴图";WindowStyle=WindowStyle.None;ResizeMode=ResizeMode.CanResize;Topmost=true;ShowInTaskbar=NativeMethods.VisualQaCaptureEnabled;Background=Brushes.Transparent;AllowsTransparency=true;UseLayoutRounding=true;SnapsToDevicePixels=true;
        _frame=new Border{Background=Brushes.White,CornerRadius=new CornerRadius(10),BorderBrush=new SolidColorBrush(Color.FromArgb(110,189,208,226)),BorderThickness=new Thickness(1),Effect=new DropShadowEffect{Color=Color.FromRgb(42,55,72),BlurRadius=22,ShadowDepth=4,Opacity=.3},Child=new Image{Source=image,Stretch=Stretch.Fill,SnapsToDevicePixels=true}};
        Content=_frame;Width=_initialContentWidthPixels+ShadowPixels*2;Height=_initialContentWidthPixels*(double)image.PixelHeight/Math.Max(1,image.PixelWidth)+ShadowPixels*2;
        SizeChanged+=KeepAspectRatio;DpiChanged+=OnDpiChanged;MouseLeftButtonDown+=OnMouseLeftButtonDown;MouseLeftButtonUp+=OnMouseLeftButtonUp;MouseMove+=OnMouseMove;MouseWheel+=OnMouseWheel;ContextMenu=BuildContextMenu();
        SourceInitialized+=(_,_)=>
        {
            var handle=new System.Windows.Interop.WindowInteropHelper(this).Handle;NativeMethods.ExcludeFromCapture(handle);
            var dpi=Math.Max(96u,NativeMethods.GetDpiForWindow(handle));
            if(_originalRegion is { } region)PlaceAtOriginalSize(handle,region);
            else
            {
                // Window dimensions are DIPs while the captured image size is
                // physical pixels.  Convert the initial preview once so a
                // 175% display does not make a 900 px image render at 1575 px.
                var outerWidth=_initialContentWidthPixels+ShadowPixels*2;
                var outerHeight=_initialContentWidthPixels*(double)image.PixelHeight/Math.Max(1,image.PixelWidth)+ShadowPixels*2;
                Width=ScreenCoordinateService.PixelsToDip(outerWidth,dpi);
                Height=ScreenCoordinateService.PixelsToDip(outerHeight,dpi);
                ApplyShadowPadding(handle);UpdateHeightForAspectRatio();
            }
        };
    }

    private void PlaceAtOriginalSize(IntPtr handle,ScreenRect region)
    {
        _adjustingSize=true;
        try
        {
            var dpi=Math.Max(96u,NativeMethods.GetDpiForWindow(handle));
            var outerWidth=region.Width+ShadowPixels*2;var outerHeight=region.Height+ShadowPixels*2;
            Width=ScreenCoordinateService.PixelsToDip(outerWidth,dpi);Height=ScreenCoordinateService.PixelsToDip(outerHeight,dpi);
            NativeMethods.SetWindowPos(handle,Topmost?new IntPtr(-1):new IntPtr(-2),region.X-ShadowPixels,region.Y-ShadowPixels,outerWidth,outerHeight,0x0040);ApplyShadowPadding(handle);
        }
        finally{_adjustingSize=false;}
    }

    private void ApplyShadowPadding(IntPtr handle)
    {
        var dpi=Math.Max(96u,NativeMethods.GetDpiForWindow(handle));var padding=ScreenCoordinateService.PixelsToDip(ShadowPixels,dpi);_frame.Margin=new Thickness(padding);
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
        if(e.ClickCount>=2){_drag.End();Topmost=false;UpdateTopmostHeader();e.Handled=true;return;}
        if(e.ButtonState==MouseButtonState.Pressed)_drag.Begin(e.GetPosition(this));
    }

    private void OnMouseLeftButtonUp(object sender,MouseButtonEventArgs e)=>_drag.End();
    private void OnMouseMove(object sender,MouseEventArgs e)=>_drag.Move(e.LeftButton,e.GetPosition(this));

    private void OnMouseWheel(object sender,MouseWheelEventArgs e)
    {
        var factor=e.Delta>0?1.08:.92;var minimumWidth=_frame.Margin.Left*2+1;Width=Math.Clamp(Width*factor,minimumWidth,2400);
    }

    private ContextMenu BuildContextMenu()
    {
        var menu=new ContextMenu();menu.SetResourceReference(StyleProperty,typeof(ContextMenu));Add(menu,"复制",CopyImage);Add(menu,"保存…",Save);AddSeparator(menu);Add(menu,"回到原位",RestoreOriginal);_topmostItem=Add(menu,"置顶",ToggleTopmost);_opacityItem=Add(menu,"80% 透明度",ToggleOpacity);AddSeparator(menu);Add(menu,"关闭",Close);UpdateTopmostHeader();return menu;
    }

    private static MenuItem Add(ContextMenu menu,string text,Action action){var item=new MenuItem{Header=text};item.SetResourceReference(StyleProperty,typeof(MenuItem));item.Click+=(_,_)=>action();menu.Items.Add(item);return item;}
    private static void AddSeparator(ContextMenu menu){var separator=new Separator();separator.SetResourceReference(StyleProperty,typeof(Separator));menu.Items.Add(separator);}
    private void ToggleTopmost(){Topmost=!Topmost;UpdateTopmostHeader();}
    private void UpdateTopmostHeader(){if(_topmostItem is not null)_topmostItem.Header=Topmost?"取消置顶":"置顶";}
    private void ToggleOpacity(){Opacity=Opacity<1?1:.8;if(_opacityItem is not null)_opacityItem.Header=Opacity<1?"100% 不透明度":"80% 透明度";}
    private void RestoreOriginal(){if(_originalRegion is not { } region)return;var handle=new System.Windows.Interop.WindowInteropHelper(this).Handle;PlaceAtOriginalSize(handle,region);}
    private void CopyImage(){if(!ClipboardService.TrySetImage(_image,out var error))MessageBox.Show(this,error??"复制图片失败，请稍后重试","复制失败",MessageBoxButton.OK,MessageBoxImage.Warning);}
    private void Save(){var dialog=new SaveFileDialog{Filter="PNG 图片|*.png|JPEG 图片|*.jpg;*.jpeg",DefaultExt=".png"};if(dialog.ShowDialog(this)!=true)return;try{ScreenCaptureService.Save(_image,dialog.FileName,dialog.FilterIndex==2);}catch(Exception ex){new PrivacyLogger().Error("PinnedImageSave",ex);MessageBox.Show(this,$"图片保存失败：{ex.Message}","保存失败",MessageBoxButton.OK,MessageBoxImage.Warning);}}
}
