using System.ComponentModel;
using System.Windows;
using System.Windows.Controls.Primitives;
using System.Windows.Media;
using System.Windows.Media.Media3D;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Interop;
using System.Windows.Input;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant;
public partial class MainWindow : Window
{
    private const double ShellCornerRadius = 14;
    private readonly AppHost _host;
    public MainWindow(AppHost host) { _host=host; InitializeComponent(); RefreshStatus();SourceInitialized+=(_,_)=>NativeMethods.ExcludeFromCapture(new System.Windows.Interop.WindowInteropHelper(this).Handle); }
    private void OnLoaded(object sender,RoutedEventArgs e)=>UpdateShellClip();
    private void OnSizeChanged(object sender,SizeChangedEventArgs e)=>UpdateShellClip();
    private void OnDpiChanged(object sender,DpiChangedEventArgs e)
    {
        UpdateShellClip();
        // DpiChanged can arrive before WPF publishes the final layout size.
        // Recompute once at render priority so the rounded clip cannot keep a
        // one-frame rectangle from the previous monitor scale.
        _=Dispatcher.BeginInvoke(DispatcherPriority.Render,new Action(UpdateShellClip));
    }
    private void UpdateShellClip()
    {
        if (Shell.ActualWidth <= 0 || Shell.ActualHeight <= 0) return;
        // Border's CornerRadius paints rounded pixels, but FrameworkElement's
        // ClipToBounds is rectangular.  Clip the complete content explicitly
        // so no child/background can leak through as square corners on a
        // transparent WPF window (especially after a DPI or resize pass).
        Shell.Clip = new RectangleGeometry(
            new Rect(0, 0, Shell.ActualWidth, Shell.ActualHeight),
            ShellCornerRadius,
            ShellCornerRadius);
    }
    public void RefreshStatus()
    {
        var available=_host.IsConversationAvailable(out var error);
        ProviderText.Text=available?BuildAiStatusText(_host.Settings):error??BuildAiStatusText(_host.Settings);
        CaptureSubtitle.Text=available?"圈选并直接分析":"截图、OCR、标注和录屏";
    }

    internal static string BuildAiStatusText(AppSettings settings)
    {
        ArgumentNullException.ThrowIfNull(settings);
        if(settings.HermesEnabled)
        {
            var profile=string.IsNullOrWhiteSpace(settings.HermesProfile)?"default":settings.HermesProfile.Trim();
            var model=string.IsNullOrWhiteSpace(settings.HermesModel)?"未选择模型":settings.HermesModel.Trim();
            var reasoning=(settings.HermesReasoningEffort??string.Empty).Trim().ToLowerInvariant() switch
            {
                "none"=>"关闭思考",
                "minimal"=>"极简思考",
                "low"=>"低度思考",
                "medium"=>"中等思考",
                "high"=>"高度思考",
                "xhigh"=>"超高思考",
                "max"=>"最大思考",
                "ultra"=>"极致思考",
                _=>"思考程度待修复"
            };
            return $"本机 Hermes · {profile} · {model} · {reasoning}";
        }
        if(settings.Providers.Count==0)return "尚未配置（基础功能可用）";
        if(string.IsNullOrWhiteSpace(settings.DefaultProviderId))return "默认 Provider 未选择 · AI 不可用";
        var matches=settings.Providers.Where(provider=>provider.Id==settings.DefaultProviderId).Take(2).ToList();
        return matches.Count switch
        {
            0=>"默认 Provider 需重新选择 · AI 不可用",
            >1=>"Provider ID 重复 · AI 不可用",
            _=>$"{matches[0].Name} · {matches[0].Model}"
        };
    }
    private void StartCapture(object sender,RoutedEventArgs e){Hide();_host.BeginCapture();}
    private void OpenSettings(object sender,RoutedEventArgs e)=>_host.ShowSettings();
    private void DragWindow(object sender,MouseButtonEventArgs e){if(e.ButtonState==MouseButtonState.Pressed&&!IsInsideButton(e.OriginalSource))DragMove();}
    private static bool IsInsideButton(object? source)
    {
        var current=source as DependencyObject;
        while(current is not null)
        {
            if(current is ButtonBase)return true;
            current=current switch
            {
                Visual or Visual3D=>VisualTreeHelper.GetParent(current),
                FrameworkContentElement content=>content.Parent,
                _=>LogicalTreeHelper.GetParent(current)
            };
        }
        return false;
    }
    private void MinimizeWindow(object sender,RoutedEventArgs e)=>WindowState=WindowState.Minimized;
    private void HideWindow(object sender,RoutedEventArgs e)=>Hide();
    private void OnClosing(object? sender,CancelEventArgs e) { if(_host.IsExiting)return; e.Cancel=true; Hide(); }
}
