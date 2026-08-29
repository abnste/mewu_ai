using System.ComponentModel;
using System.Windows;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Interop;
namespace mewu_ai_Assistant;
public partial class MainWindow : Window
{
    private readonly AppHost _host;
    public MainWindow(AppHost host) { _host=host; InitializeComponent(); RefreshStatus();SourceInitialized+=(_,_)=>NativeMethods.SetWindowDisplayAffinity(new System.Windows.Interop.WindowInteropHelper(this).Handle,NativeMethods.WdaExcludeFromCapture); }
    public void RefreshStatus(){HotkeyText.Text=_host.Settings.CaptureHotkey.DisplayName;var provider=_host.Settings.Providers.FirstOrDefault(x=>x.Id==_host.Settings.DefaultProviderId)??_host.Settings.Providers.FirstOrDefault();ProviderText.Text=provider is null?"尚未配置（基础功能可用）":$"{provider.Name} · {provider.Model}";}
    private void StartCapture(object sender,RoutedEventArgs e){Hide();_host.BeginCapture();}
    private void OpenSettings(object sender,RoutedEventArgs e)=>_host.ShowSettings();
    private void OpenTextAi(object sender,RoutedEventArgs e)=>_host.ShowTextAi();
    private void OnClosing(object? sender,CancelEventArgs e) { if(_host.IsExiting)return; e.Cancel=true; Hide(); }
}
