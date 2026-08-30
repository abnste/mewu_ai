using System.ComponentModel;
using System.Windows;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Interop;
using System.Windows.Input;
namespace mewu_ai_Assistant;
public partial class MainWindow : Window
{
    private readonly AppHost _host;
    public MainWindow(AppHost host) { _host=host; InitializeComponent(); RefreshStatus();SourceInitialized+=(_,_)=>NativeMethods.ExcludeFromCapture(new System.Windows.Interop.WindowInteropHelper(this).Handle); }
    public void RefreshStatus()
    {
        if(_host.Settings.Providers.Count==0){ProviderText.Text="尚未配置（基础功能可用）";return;}
        if(string.IsNullOrWhiteSpace(_host.Settings.DefaultProviderId)){ProviderText.Text="默认 Provider 未选择 · AI 不可用";return;}
        var matches=_host.Settings.Providers.Where(provider=>provider.Id==_host.Settings.DefaultProviderId).Take(2).ToList();
        ProviderText.Text=matches.Count switch
        {
            0=>"默认 Provider 需重新选择 · AI 不可用",
            >1=>"Provider ID 重复 · AI 不可用",
            _=>$"{matches[0].Name} · {matches[0].Model}"
        };
    }
    private void StartCapture(object sender,RoutedEventArgs e){Hide();_host.BeginCapture();}
    private void OpenSettings(object sender,RoutedEventArgs e)=>_host.ShowSettings();
    private void OpenTextAi(object sender,RoutedEventArgs e)=>_host.ShowTextAi();
    private void DragWindow(object sender,MouseButtonEventArgs e){if(e.ButtonState==MouseButtonState.Pressed&&e.OriginalSource is not System.Windows.Controls.Button)DragMove();}
    private void MinimizeWindow(object sender,RoutedEventArgs e)=>WindowState=WindowState.Minimized;
    private void HideWindow(object sender,RoutedEventArgs e)=>Hide();
    private void OnClosing(object? sender,CancelEventArgs e) { if(_host.IsExiting)return; e.Cancel=true; Hide(); }
}
