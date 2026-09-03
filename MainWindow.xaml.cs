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
        AiStatusTitle.Text=BuildAiStatusTitle(_host.Settings,available);
        ProviderText.Text=available?BuildAiStatusText(_host.Settings):LocalizationService.T("截图、OCR、标注和录屏可用","Capture, OCR, annotation, and recording are available");
        var screenAiAvailable=_host.IsScreenAiAvailable(out _);
        CaptureSubtitle.Text=screenAiAvailable?LocalizationService.T("圈选并直接分析","Select an area and analyze it"):LocalizationService.T("截图、OCR、标注和录屏","Capture, OCR, annotate, and record");
    }

    internal static string BuildAiStatusTitle(AppSettings settings,bool available)
    {
        ArgumentNullException.ThrowIfNull(settings);
        if(!available)return LocalizationService.T("暂未设置AI功能","AI features are not set up");
        return settings.HermesEnabled?LocalizationService.T("智能体已接入","Agent connected"):LocalizationService.T("AI模型已接入","AI model connected");
    }

    internal static string BuildAiStatusText(AppSettings settings)
    {
        ArgumentNullException.ThrowIfNull(settings);
        if(settings.HermesEnabled)
        {
            var profile=string.IsNullOrWhiteSpace(settings.HermesProfile)?"default":settings.HermesProfile.Trim();
            var model=string.IsNullOrWhiteSpace(settings.HermesModel)?LocalizationService.T("未选择模型","No model selected"):settings.HermesModel.Trim();
            var reasoning=(settings.HermesReasoningEffort??string.Empty).Trim().ToLowerInvariant() switch
            {
                "none"=>LocalizationService.T("关闭思考","reasoning off"),
                "minimal"=>LocalizationService.T("极简思考","minimal reasoning"),
                "low"=>LocalizationService.T("低度思考","low reasoning"),
                "medium"=>LocalizationService.T("中等思考","medium reasoning"),
                "high"=>LocalizationService.T("高度思考","high reasoning"),
                "xhigh"=>LocalizationService.T("超高思考","extra-high reasoning"),
                "max"=>LocalizationService.T("最大思考","maximum reasoning"),
                "ultra"=>LocalizationService.T("极致思考","ultra reasoning"),
                _=>LocalizationService.T("思考程度待修复","reasoning setting needs attention")
            };
            return $"Hermes · {profile} · {model} · {reasoning}";
        }
        if(settings.Providers.Count==0)return LocalizationService.T("未配置 AI 模型","No AI model configured");
        if(string.IsNullOrWhiteSpace(settings.DefaultProviderId))return LocalizationService.T("默认 Provider 未选择 · AI 不可用","Choose a default provider to enable AI");
        var matches=settings.Providers.Where(provider=>provider.Id==settings.DefaultProviderId).Take(2).ToList();
        return matches.Count switch
        {
            0=>LocalizationService.T("默认 Provider 需重新选择 · AI 不可用","Re-select the default provider to enable AI"),
            >1=>LocalizationService.T("Provider ID 重复 · AI 不可用","Duplicate provider IDs · AI unavailable"),
            _=>BuildProviderDisplayText(matches[0])
        };
    }
    private static string BuildProviderDisplayText(AiProviderSettings provider)
    {
        var name=(provider.Name??string.Empty).Trim();
        var model=(provider.Model??string.Empty).Trim();
        if(name.Length==0)return model.Length==0?LocalizationService.T("AI 模型","AI model"):model;
        if(model.Length==0)return name;
        var normalizedName=new string(name.Where(char.IsLetterOrDigit).ToArray());
        var normalizedModel=new string(model.Where(char.IsLetterOrDigit).ToArray());
        return string.Equals(normalizedName,normalizedModel,StringComparison.OrdinalIgnoreCase)?name:$"{name} · {model}";
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
