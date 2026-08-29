using System.ComponentModel;
using System.Windows;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant;
public partial class MainWindow : Window
{
    private readonly AppHost _host;
    public MainWindow(AppHost host) { _host=host; InitializeComponent(); HotkeyText.Text=host.Settings.CaptureHotkey.DisplayName; ProviderText.Text=string.IsNullOrWhiteSpace(host.Settings.DefaultProviderId)?"尚未配置（基础功能可用）":host.Settings.DefaultProviderId; }
    private void StartCapture(object sender,RoutedEventArgs e)=>_host.BeginCapture();
    private void OpenSettings(object sender,RoutedEventArgs e)=>_host.ShowSettings();
    private void OnClosing(object? sender,CancelEventArgs e) { if(_host.IsExiting)return; e.Cancel=true; Hide(); }
}
