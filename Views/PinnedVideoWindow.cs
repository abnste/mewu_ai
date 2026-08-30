using Microsoft.Win32;
using System.Collections.Specialized;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Effects;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Views;

public sealed class PinnedVideoWindow : Window
{
    private const int ShadowPixels=16;
    private readonly string _videoPath;
    private readonly ScreenRect _originalRegion;
    private readonly Border _frame;
    private readonly MediaElement _player;
    private MenuItem? _topmostItem,_playItem;
    private bool _playing=true,_adjustingSize;

    public PinnedVideoWindow(string videoPath,ScreenRect originalRegion)
    {
        _videoPath=videoPath;_originalRegion=originalRegion;Title="喵呜AI 贴视频";WindowStyle=WindowStyle.None;ResizeMode=ResizeMode.CanResize;Topmost=true;ShowInTaskbar=NativeMethods.VisualQaCaptureEnabled;Background=Brushes.Transparent;AllowsTransparency=true;
        _player=new MediaElement{Source=new Uri(videoPath),Stretch=Stretch.Fill,LoadedBehavior=MediaState.Manual,UnloadedBehavior=MediaState.Close};_player.MediaEnded+=(_,_)=>{_player.Position=TimeSpan.Zero;if(_playing)_player.Play();};
        _frame=new Border{Background=Brushes.Black,CornerRadius=new CornerRadius(7),BorderBrush=new SolidColorBrush(Color.FromArgb(120,145,158,177)),BorderThickness=new Thickness(1),ClipToBounds=true,Effect=new DropShadowEffect{Color=Color.FromRgb(42,55,72),BlurRadius=26,ShadowDepth=5,Opacity=.38},Child=_player};Content=_frame;Width=originalRegion.Width+ShadowPixels*2;Height=originalRegion.Height+ShadowPixels*2;MinWidth=160+ShadowPixels*2;MinHeight=90+ShadowPixels*2;
        SizeChanged+=KeepAspectRatio;MouseLeftButtonDown+=OnMouseLeftButtonDown;MouseWheel+=OnMouseWheel;ContextMenu=BuildContextMenu();Loaded+=(_,_)=>_player.Play();Closed+=(_,_)=>_player.Close();SourceInitialized+=(_,_)=>{var handle=new System.Windows.Interop.WindowInteropHelper(this).Handle;NativeMethods.ExcludeFromCapture(handle);PlaceAtOriginalSize(handle);};
    }
    private void PlaceAtOriginalSize(IntPtr handle){NativeMethods.SetWindowPos(handle,new IntPtr(-1),_originalRegion.X-ShadowPixels,_originalRegion.Y-ShadowPixels,_originalRegion.Width+ShadowPixels*2,_originalRegion.Height+ShadowPixels*2,0x0040);var dpi=Math.Max(96,NativeMethods.GetDpiForWindow(handle));_frame.Margin=new Thickness(ShadowPixels*96d/dpi);}
    private void KeepAspectRatio(object? sender,SizeChangedEventArgs e){if(_adjustingSize)return;var padding=_frame.Margin.Left*2;var contentWidth=Math.Max(1,ActualWidth-padding);var expected=contentWidth*_originalRegion.Height/Math.Max(1d,_originalRegion.Width)+padding;if(Math.Abs(ActualHeight-expected)<1)return;_adjustingSize=true;Height=Math.Clamp(expected,MinHeight,2400);_adjustingSize=false;}
    private void OnMouseLeftButtonDown(object sender,MouseButtonEventArgs e){if(e.ClickCount==2){Topmost=false;UpdateTopmostHeader();e.Handled=true;return;}if(e.ButtonState==MouseButtonState.Pressed)DragMove();}
    private void OnMouseWheel(object sender,MouseWheelEventArgs e){Width=Math.Clamp(Width*(e.Delta>0?1.08:.92),MinWidth,2400);}
    private ContextMenu BuildContextMenu(){var menu=new ContextMenu();_playItem=Add(menu,"暂停",TogglePlayback);Add(menu,"复制视频文件",CopyFile);Add(menu,"保存视频…",Save);menu.Items.Add(new Separator());Add(menu,"恢复原位原大小",()=>PlaceAtOriginalSize(new System.Windows.Interop.WindowInteropHelper(this).Handle));_topmostItem=Add(menu,"保持置顶 ✓",ToggleTopmost);Add(menu,"透明度 80%",()=>Opacity=Opacity<1?1:.8);menu.Items.Add(new Separator());Add(menu,"关闭贴视频",Close);return menu;}
    private static MenuItem Add(ContextMenu menu,string text,Action action){var item=new MenuItem{Header=text};item.Click+=(_,_)=>action();menu.Items.Add(item);return item;}
    private void TogglePlayback(){if(_playing){_player.Pause();_playing=false;}else{_player.Play();_playing=true;}if(_playItem is not null)_playItem.Header=_playing?"暂停":"播放";}
    private void CopyFile(){var files=new StringCollection{_videoPath};var data=new DataObject();data.SetFileDropList(files);Clipboard.SetDataObject(data,true);}
    private void Save(){var dialog=new SaveFileDialog{Filter="MP4 视频|*.mp4",DefaultExt=".mp4"};if(dialog.ShowDialog(this)==true)File.Copy(_videoPath,dialog.FileName,true);}
    private void ToggleTopmost(){Topmost=!Topmost;UpdateTopmostHeader();}
    private void UpdateTopmostHeader(){if(_topmostItem is not null)_topmostItem.Header=Topmost?"保持置顶 ✓":"恢复置顶";}
}
