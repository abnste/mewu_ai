using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Effects;
using System.Windows.Threading;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public sealed class RecordingControlWindow : Window
{
    private readonly AppHost _host;private readonly ScreenRect _region;private readonly RecordingSession _session;private readonly Action? _recordingEnded;private readonly TextBlock _time=new();private readonly DispatcherTimer _timer=new(){Interval=TimeSpan.FromSeconds(1)};private DateTime _started;private bool _stopping,_paused,_ended;
    public RecordingControlWindow(AppHost host,ScreenRect region,Action? recordingEnded=null)
    {
        _host=host;_region=region;_recordingEnded=recordingEnded;_session=new RecordingSession(host.Settings,region);Title="喵呜AI 录屏";Width=365;Height=76;AllowsTransparency=true;WindowStyle=WindowStyle.None;ResizeMode=ResizeMode.NoResize;Background=Brushes.Transparent;Topmost=true;ShowInTaskbar=NativeMethods.VisualQaCaptureEnabled;
        var display=System.Windows.Forms.Screen.FromPoint(new System.Drawing.Point(region.X+region.Width/2,region.Y+region.Height/2)).WorkingArea;
        var row=new Grid{Margin=new Thickness(14,10,14,10)};row.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});row.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});row.ColumnDefinitions.Add(new ColumnDefinition());row.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});row.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});
        var dot=new Border{Width=10,Height=10,CornerRadius=new CornerRadius(5),Background=new SolidColorBrush(Color.FromRgb(238,78,92)),VerticalAlignment=VerticalAlignment.Center,Effect=new DropShadowEffect{Color=Color.FromRgb(238,78,92),BlurRadius=10,ShadowDepth=0,Opacity=.65}};row.Children.Add(dot);
        var label=new TextBlock{Text="录制中",FontWeight=FontWeights.SemiBold,FontSize=13,Margin=new Thickness(8,0,16,0),VerticalAlignment=VerticalAlignment.Center};Grid.SetColumn(label,1);row.Children.Add(label);_time.Text="00:00";_time.Foreground=new SolidColorBrush(Color.FromRgb(94,107,126));_time.FontFamily=new FontFamily("Consolas");_time.VerticalAlignment=VerticalAlignment.Center;Grid.SetColumn(_time,2);row.Children.Add(_time);
        var pause=IconButton("Ⅱ","暂停");pause.Click+=(_,_)=>{_paused=!_paused;if(_paused){_session.Pause();pause.Content="▶";pause.ToolTip="继续";}else{_session.Resume();pause.Content="Ⅱ";pause.ToolTip="暂停";}};Grid.SetColumn(pause,3);row.Children.Add(pause);var stop=IconButton("■","停止");stop.Foreground=new SolidColorBrush(Color.FromRgb(218,72,86));stop.Margin=new Thickness(6,0,0,0);stop.Click+=async(_,_)=>await StopAsync();Grid.SetColumn(stop,4);row.Children.Add(stop);
        Content=new Border{Background=new SolidColorBrush(Color.FromArgb(252,255,255,255)),BorderBrush=new SolidColorBrush(Color.FromRgb(215,225,238)),BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(18),Child=row,Effect=new DropShadowEffect{Color=Color.FromRgb(54,70,92),BlurRadius=25,ShadowDepth=6,Opacity=.28}};
        _timer.Tick+=(_,_)=>_time.Text=(DateTime.UtcNow-_started).ToString(@"mm\:ss");Loaded+=(_,_)=>Start();Closed+=(_,_)=>{_timer.Stop();if(!_stopping)_session.Stop();_session.Dispose();EndOverlay();};SourceInitialized+=(_,_)=>{var handle=new System.Windows.Interop.WindowInteropHelper(this).Handle;NativeMethods.ExcludeFromCapture(handle);var scale=Math.Max(96,NativeMethods.GetDpiForWindow(handle))/96d;var width=(int)Math.Ceiling(Width*scale);var height=(int)Math.Ceiling(Height*scale);var x=Math.Clamp(region.X,display.Left,Math.Max(display.Left,display.Right-width));var y=region.Y-height-10>=display.Top?region.Y-height-10:Math.Min(display.Bottom-height,region.Bottom+10);NativeMethods.SetWindowPos(handle,new IntPtr(-1),x,y,0,0,0x0011);};KeyDown+=async(_,e)=>{if(e.Key==System.Windows.Input.Key.Escape)await StopAsync();};
    }
    private static Button IconButton(string icon,string tip){var button=new Button{Content=icon,ToolTip=tip,Width=38,Height=38,Padding=new Thickness(0),FontSize=14};button.SetResourceReference(StyleProperty,"IconButton");return button;}
    private void Start(){try{_session.Completed+=path=>Dispatcher.InvokeAsync(()=>CompleteAsync(path));_session.Failed+=error=>Dispatcher.Invoke(()=>{_stopping=true;MessageBox.Show(error,"录屏失败");_session.Dispose();Close();});_session.Start();_started=DateTime.UtcNow;_timer.Start();}catch(Exception ex){_stopping=true;_session.Dispose();MessageBox.Show(ex.Message,"无法开始录屏");Close();}}
    private async Task StopAsync(){if(_stopping)return;_stopping=true;_timer.Stop();_time.Text="处理中…";_session.Stop();await _session.WaitFramesAsync();}
    private async Task CompleteAsync(string path){await _session.WaitFramesAsync();new RecordingPreviewWindow(_host,_region,path,_session.FramesDirectory).Show();_session.Dispose();Close();}
    private void EndOverlay(){if(_ended)return;_ended=true;_recordingEnded?.Invoke();}
}
