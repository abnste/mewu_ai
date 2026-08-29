using Microsoft.Win32;
using System.Windows;
using System.Windows.Input;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media.Imaging;
using System.Windows.Media;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Models;
using Point=System.Windows.Point;
using MouseEventArgs=System.Windows.Input.MouseEventArgs;
using KeyEventArgs=System.Windows.Input.KeyEventArgs;
namespace mewu_ai_Assistant.Views;
public partial class CaptureOverlayWindow : Window
{
    private readonly AppHost _host; private readonly CaptureFrame _frame; private Point _start; private Rect _selection; private bool _selecting; private bool _moving; private Point _moveStart; private Rect _moveOrigin;
    public CaptureOverlayWindow(AppHost host)
    {
        _host=host; _frame=new ScreenCaptureService().CaptureDesktop(host.Settings.IncludeCaptureCursor); InitializeComponent(); DesktopImage.Source=_frame.Image;Dimmer.Fill=new SolidColorBrush(Color.FromArgb((byte)Math.Round(Math.Clamp(host.Settings.OverlayOpacity,.4,.75)*255),0,0,0));
        var area=System.Windows.Forms.SystemInformation.VirtualScreen; Left=area.Left;Top=area.Top;Width=area.Width;Height=area.Height;
        SourceInitialized+=(_,_)=>{var hwnd=new System.Windows.Interop.WindowInteropHelper(this).Handle;NativeMethods.SetWindowPos(hwnd,new IntPtr(-1),area.Left,area.Top,area.Width,area.Height,0x0040);NativeMethods.SetWindowDisplayAffinity(hwnd,NativeMethods.WdaExcludeFromCapture);};Loaded+=(_,_)=>{DesktopImage.Width=Dimmer.Width=Root.ActualWidth;DesktopImage.Height=Dimmer.Height=Root.ActualHeight;Focus();};
    }
    private void OnMouseDown(object s,MouseButtonEventArgs e)
    {
        if(e.OriginalSource is Thumb)return;var p=e.GetPosition(Root); if(!_selection.IsEmpty&&_selection.Contains(p)){_moving=true;_moveStart=p;_moveOrigin=_selection;CaptureMouse();return;}
        _selecting=true;_start=p;_selection=Rect.Empty;Toolbar.Visibility=Visibility.Collapsed;SelectionBorder.Visibility=SelectedImage.Visibility=Visibility.Visible;CaptureMouse();
    }
    private void OnMouseMove(object s,MouseEventArgs e)
    {
        var p=e.GetPosition(Root); if(_selecting){_selection=new Rect(_start,p);} else if(_moving){var d=p-_moveStart;_selection=ClampSelection(new Rect(_moveOrigin.X+d.X,_moveOrigin.Y+d.Y,_moveOrigin.Width,_moveOrigin.Height));} else return; UpdateSelection();
    }
    private void OnMouseUp(object s,MouseButtonEventArgs e)
    {
        if(!_selecting&&!_moving)return; _selecting=_moving=false;ReleaseMouseCapture(); if(_selection.Width<8||_selection.Height<8){Reset();return;} UpdateSelection();ShowToolbar();
    }
    private void UpdateSelection()
    {
        var r=Normalize(_selection);_selection=r;Canvas.SetLeft(SelectionBorder,r.Left);Canvas.SetTop(SelectionBorder,r.Top);SelectionBorder.Width=r.Width;SelectionBorder.Height=r.Height;
        Canvas.SetLeft(SelectedImage,r.Left);Canvas.SetTop(SelectedImage,r.Top);SelectedImage.Width=r.Width;SelectedImage.Height=r.Height;
        var px=ToPixelRect(r);if(px.Width>0&&px.Height>0)SelectedImage.Source=ScreenCaptureService.Crop(_frame.Image,px);
        SizeText.Text=$"{px.Width} × {px.Height}";SizeText.Visibility=Visibility.Visible;Canvas.SetLeft(SizeText,r.Left);Canvas.SetTop(SizeText,Math.Max(0,r.Top-30));
        PositionHandles(r);
    }
    private Int32Rect ToPixelRect(Rect r)
        =>ScreenCoordinateService.ToPixelRect(r,Root.ActualWidth,Root.ActualHeight,_frame.Image.PixelWidth,_frame.Image.PixelHeight);
    private static Rect Normalize(Rect r)=>new(Math.Min(r.Left,r.Right),Math.Min(r.Top,r.Bottom),Math.Abs(r.Width),Math.Abs(r.Height));
    private Rect ClampSelection(Rect value){var width=Math.Min(value.Width,Root.ActualWidth);var height=Math.Min(value.Height,Root.ActualHeight);return new Rect(Math.Clamp(value.X,0,Math.Max(0,Root.ActualWidth-width)),Math.Clamp(value.Y,0,Math.Max(0,Root.ActualHeight-height)),width,height);}
    private Rect MonitorBounds(){var pixels=ToPixelRect(_selection);var center=new System.Drawing.Point(_frame.OriginX+pixels.X+pixels.Width/2,_frame.OriginY+pixels.Y+pixels.Height/2);var bounds=System.Windows.Forms.Screen.FromPoint(center).Bounds;var sx=Root.ActualWidth/_frame.Image.PixelWidth;var sy=Root.ActualHeight/_frame.Image.PixelHeight;return new Rect((bounds.Left-_frame.OriginX)*sx,(bounds.Top-_frame.OriginY)*sy,bounds.Width*sx,bounds.Height*sy);}
    private void ShowToolbar(){Toolbar.Visibility=Visibility.Visible;Toolbar.Measure(new Size(double.PositiveInfinity,double.PositiveInfinity));var w=Toolbar.DesiredSize.Width;var h=Toolbar.DesiredSize.Height;var monitor=MonitorBounds();var x=Math.Clamp(_selection.Left,monitor.Left+8,Math.Max(monitor.Left+8,monitor.Right-w-8));var y=_selection.Bottom+10;if(y+h>monitor.Bottom-8)y=_selection.Top-h-10;y=Math.Clamp(y,monitor.Top+8,Math.Max(monitor.Top+8,monitor.Bottom-h-8));Canvas.SetLeft(Toolbar,x);Canvas.SetTop(Toolbar,y);}
    private BitmapSource CurrentImage()=>ScreenCaptureService.Crop(_frame.Image,ToPixelRect(_selection));
    private void PositionHandles(Rect r){var list=new[]{Nw,N,Ne,W,E,Sw,S,Se};foreach(var t in list){t.Width=t.Height=10;t.Background=new SolidColorBrush(Color.FromRgb(67,168,255));t.Visibility=Visibility.Visible;}Set(Nw,r.Left,r.Top);Set(N,r.Left+r.Width/2,r.Top);Set(Ne,r.Right,r.Top);Set(W,r.Left,r.Top+r.Height/2);Set(E,r.Right,r.Top+r.Height/2);Set(Sw,r.Left,r.Bottom);Set(S,r.Left+r.Width/2,r.Bottom);Set(Se,r.Right,r.Bottom);static void Set(Thumb t,double x,double y){Canvas.SetLeft(t,x-5);Canvas.SetTop(t,y-5);}}
    private void ResizeDelta(object sender,DragDeltaEventArgs e){if(sender is not Thumb t)return;var d=t.Tag?.ToString()??"";var l=_selection.Left;var top=_selection.Top;var r=_selection.Right;var b=_selection.Bottom;if(d.Contains('W'))l=Math.Clamp(l+e.HorizontalChange,0,r-12);if(d.Contains('E'))r=Math.Clamp(r+e.HorizontalChange,l+12,Root.ActualWidth);if(d.Contains('N'))top=Math.Clamp(top+e.VerticalChange,0,b-12);if(d.Contains('S'))b=Math.Clamp(b+e.VerticalChange,top+12,Root.ActualHeight);_selection=new Rect(new Point(l,top),new Point(r,b));UpdateSelection();ShowToolbar();e.Handled=true;}
    private void Reset(){_selection=Rect.Empty;SelectionBorder.Visibility=SelectedImage.Visibility=Toolbar.Visibility=SizeText.Visibility=Visibility.Collapsed;foreach(var t in new[]{Nw,N,Ne,W,E,Sw,S,Se})t.Visibility=Visibility.Collapsed;}
    private void Copy(object s,RoutedEventArgs e){Clipboard.SetImage(CurrentImage());_host.Notify("已复制到剪贴板");Close();}
    private void Save(object s,RoutedEventArgs e){var jpeg=_host.Settings.DefaultImageFormat.Equals("jpg",StringComparison.OrdinalIgnoreCase)||_host.Settings.DefaultImageFormat.Equals("jpeg",StringComparison.OrdinalIgnoreCase);var d=new SaveFileDialog{Filter="PNG 图片|*.png|JPEG 图片|*.jpg;*.jpeg",DefaultExt=jpeg?".jpg":".png",FilterIndex=jpeg?2:1,AddExtension=true};if(d.ShowDialog(this)==true)ScreenCaptureService.Save(CurrentImage(),d.FileName,d.FilterIndex==2);}
    private void Pin(object s,RoutedEventArgs e){new PinnedImageWindow(CurrentImage()).Show();Close();}
    private void Draw(object s,RoutedEventArgs e){var pixels=ToPixelRect(_selection);new DrawingWindow(_host,CurrentImage(),ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY)).Show();Close();}
    private void AskAi(object s,RoutedEventArgs e){var pixels=ToPixelRect(_selection);new AiPromptWindow(_host,CurrentImage(),false,ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY)).Show();Close();}
    private void Translate(object s,RoutedEventArgs e){new TranslationWindow(_host,CurrentImage()).Show();Close();}
    private void Ocr(object s,RoutedEventArgs e){new OcrTextWindow(CurrentImage()).Show();Close();}
    private void Record(object s,RoutedEventArgs e){var px=ToPixelRect(_selection);new RecordingControlWindow(_host,ScreenCoordinateService.ToScreenRect(px,_frame.OriginX,_frame.OriginY)).Show();Close();}
    private void OnKeyDown(object s,KeyEventArgs e)
    {
        if(e.Key==Key.Escape){Close();return;}if(_selection.IsEmpty){if((int)e.Key>=(int)Key.A&&(int)e.Key<=(int)Key.Z){_host.ShowTextAi(e.Key.ToString());Close();}return;}
        var step=Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)?10:1;if(e.Key is Key.Left or Key.Right or Key.Up or Key.Down){_selection=ClampSelection(new Rect(_selection.X+(e.Key==Key.Left?-step:e.Key==Key.Right?step:0),_selection.Y+(e.Key==Key.Up?-step:e.Key==Key.Down?step:0),_selection.Width,_selection.Height));UpdateSelection();ShowToolbar();e.Handled=true;return;}
        if(e.Key==Key.C)Copy(s,new());else if(e.Key==Key.S)Save(s,new());else if(e.Key==Key.P)Pin(s,new());else if(e.Key==Key.D)Draw(s,new());else if(e.Key==Key.T)Translate(s,new());else if(e.Key==Key.O)Ocr(s,new());else if(e.Key==Key.Enter)AskAi(s,new());else if(e.Key==Key.R)Record(s,new());
    }
}
