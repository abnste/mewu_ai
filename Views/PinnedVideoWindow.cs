using Microsoft.Win32;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Effects;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public sealed class PinnedVideoWindow : Window
{
    private const int ShadowPixels=12;
    private readonly string _videoPath;
    private readonly TempMediaLease _videoLease;
    private readonly ScreenRect _originalRegion;
    private readonly Border _frame;
    private readonly MediaElement _player;
    private MenuItem? _topmostItem,_playItem,_opacityItem,_copyItem,_saveItem;
    private bool _playing=true,_adjustingSize,_mediaOperationBusy;

    public PinnedVideoWindow(string videoPath,ScreenRect originalRegion)
    {
        _videoPath=Path.GetFullPath(videoPath);_originalRegion=originalRegion;_videoLease=TempMediaRegistry.Shared.AcquireExistingFile(_videoPath);
        try
        {
            Title="喵呜AI 贴视频";WindowStyle=WindowStyle.None;ResizeMode=ResizeMode.CanResize;Topmost=true;ShowInTaskbar=NativeMethods.VisualQaCaptureEnabled;Background=Brushes.Transparent;AllowsTransparency=true;UseLayoutRounding=true;SnapsToDevicePixels=true;
            _player=new MediaElement{Source=new Uri(_videoPath),Stretch=Stretch.Fill,LoadedBehavior=MediaState.Manual,UnloadedBehavior=MediaState.Close};_player.MediaEnded+=(_,_)=>{_player.Position=TimeSpan.Zero;if(_playing)_player.Play();};
            _frame=new Border{Background=Brushes.Black,CornerRadius=new CornerRadius(10),BorderBrush=new SolidColorBrush(Color.FromArgb(110,189,208,226)),BorderThickness=new Thickness(1),ClipToBounds=true,Effect=new DropShadowEffect{Color=Color.FromRgb(42,55,72),BlurRadius=22,ShadowDepth=4,Opacity=.3},Child=_player};Content=_frame;Width=originalRegion.Width+ShadowPixels*2;Height=originalRegion.Height+ShadowPixels*2;
            SizeChanged+=KeepAspectRatio;DpiChanged+=OnDpiChanged;MouseLeftButtonDown+=OnMouseLeftButtonDown;MouseWheel+=OnMouseWheel;ContextMenu=BuildContextMenu();Loaded+=(_,_)=>_player.Play();Closed+=(_,_)=>{try{_player.Close();}finally{_videoLease.Dispose();}};SourceInitialized+=(_,_)=>{var handle=new System.Windows.Interop.WindowInteropHelper(this).Handle;NativeMethods.ExcludeFromCapture(handle);PlaceAtOriginalSize(handle);};
        }
        catch
        {
            _videoLease.Dispose();
            throw;
        }
    }
    private void PlaceAtOriginalSize(IntPtr handle)
    {
        _adjustingSize=true;
        try
        {
            var dpi=Math.Max(96u,NativeMethods.GetDpiForWindow(handle));
            var outerWidth=_originalRegion.Width+ShadowPixels*2;var outerHeight=_originalRegion.Height+ShadowPixels*2;
            Width=ScreenCoordinateService.PixelsToDip(outerWidth,dpi);Height=ScreenCoordinateService.PixelsToDip(outerHeight,dpi);
            NativeMethods.SetWindowPos(handle,Topmost?new IntPtr(-1):new IntPtr(-2),_originalRegion.X-ShadowPixels,_originalRegion.Y-ShadowPixels,outerWidth,outerHeight,0x0040);ApplyShadowPadding(handle);
        }
        finally{_adjustingSize=false;}
    }
    private void ApplyShadowPadding(IntPtr handle){var dpi=Math.Max(96u,NativeMethods.GetDpiForWindow(handle));_frame.Margin=new Thickness(ScreenCoordinateService.PixelsToDip(ShadowPixels,dpi));}
    private void OnDpiChanged(object sender,DpiChangedEventArgs e){var handle=new System.Windows.Interop.WindowInteropHelper(this).Handle;if(handle==IntPtr.Zero)return;ApplyShadowPadding(handle);UpdateHeightForAspectRatio();}
    private void KeepAspectRatio(object? sender,SizeChangedEventArgs e)=>UpdateHeightForAspectRatio();
    private void UpdateHeightForAspectRatio(){if(_adjustingSize)return;var padding=_frame.Margin.Left*2;var contentWidth=Math.Max(1,ActualWidth-padding);var expected=contentWidth*_originalRegion.Height/Math.Max(1d,_originalRegion.Width)+padding;if(Math.Abs(ActualHeight-expected)<1)return;_adjustingSize=true;Height=Math.Min(expected,2400);_adjustingSize=false;}
    private void OnMouseLeftButtonDown(object sender,MouseButtonEventArgs e){if(e.ClickCount==2){Topmost=false;UpdateTopmostHeader();e.Handled=true;return;}if(e.ButtonState==MouseButtonState.Pressed)DragMove();}
    private void OnMouseWheel(object sender,MouseWheelEventArgs e){var minimumWidth=_frame.Margin.Left*2+1;Width=Math.Clamp(Width*(e.Delta>0?1.08:.92),minimumWidth,2400);}
    private ContextMenu BuildContextMenu(){var menu=new ContextMenu();menu.SetResourceReference(StyleProperty,typeof(ContextMenu));_playItem=Add(menu,"暂停",TogglePlayback);_copyItem=Add(menu,"复制",()=>_ = CopyFileAsync());_saveItem=Add(menu,"保存…",()=>_ = SaveAsync());AddSeparator(menu);Add(menu,"回到原位",()=>PlaceAtOriginalSize(new System.Windows.Interop.WindowInteropHelper(this).Handle));_topmostItem=Add(menu,"置顶",ToggleTopmost);_opacityItem=Add(menu,"80% 透明度",ToggleOpacity);AddSeparator(menu);Add(menu,"关闭",Close);UpdateTopmostHeader();return menu;}
    private static MenuItem Add(ContextMenu menu,string text,Action action){var item=new MenuItem{Header=text};item.SetResourceReference(StyleProperty,typeof(MenuItem));item.Click+=(_,_)=>action();menu.Items.Add(item);return item;}
    private static void AddSeparator(ContextMenu menu){var separator=new Separator();separator.SetResourceReference(StyleProperty,typeof(Separator));menu.Items.Add(separator);}
    private void TogglePlayback(){if(_playing){_player.Pause();_playing=false;}else{_player.Play();_playing=true;}if(_playItem is not null)_playItem.Header=_playing?"暂停":"播放";}
    private async Task CopyFileAsync()
    {
        if(!TryBeginMediaOperation())return;
        try
        {
            var result=await ClipboardService.TrySetFileDropListAsync(_videoPath);
            if(!result.Success)ShowOperationError(result.Error??"复制视频失败，请稍后重试","复制失败");
        }
        catch(Exception ex)
        {
            new PrivacyLogger().Error("PinnedVideoCopy",ex);ShowOperationError($"复制视频失败：{ex.Message}","复制失败");
        }
        finally{EndMediaOperation();}
    }
    private async Task SaveAsync()
    {
        if(_mediaOperationBusy)return;
        var dialog=new SaveFileDialog{Filter="MP4 视频|*.mp4",DefaultExt=".mp4"};if(dialog.ShowDialog(this)!=true)return;
        if(!TryBeginMediaOperation())return;
        try{await Task.Run(()=>AtomicFileService.Copy(_videoPath,dialog.FileName));}
        catch(Exception ex){new PrivacyLogger().Error("PinnedVideoSave",ex);ShowOperationError($"视频保存失败：{ex.Message}","保存失败");}
        finally{EndMediaOperation();}
    }
    private bool TryBeginMediaOperation(){if(_mediaOperationBusy)return false;_mediaOperationBusy=true;if(_copyItem is not null)_copyItem.IsEnabled=false;if(_saveItem is not null)_saveItem.IsEnabled=false;return true;}
    private void EndMediaOperation(){_mediaOperationBusy=false;if(_copyItem is not null)_copyItem.IsEnabled=true;if(_saveItem is not null)_saveItem.IsEnabled=true;}
    private void ShowOperationError(string message,string title){if(IsVisible)MessageBox.Show(this,message,title,MessageBoxButton.OK,MessageBoxImage.Warning);else MessageBox.Show(message,title,MessageBoxButton.OK,MessageBoxImage.Warning);}
    private void ToggleTopmost(){Topmost=!Topmost;UpdateTopmostHeader();}
    private void UpdateTopmostHeader(){if(_topmostItem is not null)_topmostItem.Header=Topmost?"取消置顶":"置顶";}
    private void ToggleOpacity(){Opacity=Opacity<1?1:.8;if(_opacityItem is not null)_opacityItem.Header=Opacity<1?"100% 不透明度":"80% 透明度";}
}
