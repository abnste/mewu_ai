using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Threading;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;
public sealed class RecordingControlWindow : Window
{
    private readonly AppHost _host;private readonly RecordingSession _session;private readonly TextBlock _time=new();private readonly DispatcherTimer _timer=new(){Interval=TimeSpan.FromSeconds(1)};private readonly DateTime _started=DateTime.UtcNow;private bool _stopping;
    public RecordingControlWindow(AppHost host,ScreenRect region){_host=host;_session=new RecordingSession(host.Settings,region);Title="喵呜AI 录屏";Width=250;Height=82;Left=region.X;Top=Math.Max(0,region.Y-90);Topmost=true;ShowInTaskbar=false;WindowStyle=WindowStyle.None;Background=new SolidColorBrush(Color.FromRgb(18,26,38));Foreground=Brushes.White;var bar=new StackPanel{Orientation=Orientation.Horizontal,Margin=new Thickness(12)};bar.Children.Add(new TextBlock{Text="● REC",Foreground=Brushes.Red,FontWeight=FontWeights.Bold,VerticalAlignment=VerticalAlignment.Center});_time.Text="00:00";_time.Margin=new Thickness(15,0,15,0);_time.VerticalAlignment=VerticalAlignment.Center;bar.Children.Add(_time);var stop=new Button{Content="停止",Padding=new Thickness(12,5,12,5)};stop.Click+=async(_,_)=>await StopAsync();bar.Children.Add(stop);Content=bar;_timer.Tick+=(_,_)=>_time.Text=(DateTime.UtcNow-_started).ToString(@"mm\:ss");Loaded+=(_,_)=>Start();SourceInitialized+=(_,_)=>NativeMethods.SetWindowDisplayAffinity(new System.Windows.Interop.WindowInteropHelper(this).Handle,NativeMethods.WdaExcludeFromCapture);KeyDown+=async(_,e)=>{if(e.Key==System.Windows.Input.Key.Escape)await StopAsync();};}
    private void Start(){try{_session.Completed+=p=>Dispatcher.Invoke(async()=>await CompleteAsync(p));_session.Failed+=e=>Dispatcher.Invoke(()=>{MessageBox.Show(e,"录屏失败");Close();});_session.Start();_timer.Start();}catch(Exception ex){MessageBox.Show(ex.Message,"无法开始录屏");Close();}}
    private async Task StopAsync(){if(_stopping)return;_stopping=true;_timer.Stop();_time.Text="处理中…";_session.Stop();await _session.WaitFramesAsync();}
    private async Task CompleteAsync(string path){await _session.WaitFramesAsync();new RecordingPreviewWindow(_host,path,_session.FramesDirectory).Show();_session.Dispose();Close();}
}
