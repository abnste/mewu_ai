// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net.Http;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Documents;
using System.Windows.Media;
using MessageBox=mewu_ai_Assistant.Services.LocalizedMessageBox;
using System.Windows.Shell;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Interop;

namespace mewu_ai_Assistant.Views;

public sealed partial class SettingsWindow : Window
{
    private readonly TabItem _aiTab;
    private CodexSettingsPage _codexSettings=null!;
    private WorkBuddySettingsPage _workBuddySettings=null!;
    private MiniMaxCodeSettingsPage _miniMaxCodeSettings=null!;
    private AiSettingsTabs _backendSelector=null!;
    private bool HermesSelected=>_backendSelector?.SelectedBackendIndex==AiSettingsTabs.HermesIndex;
    internal void ShowAiPage()=>_aiTab.IsSelected=true;
    private static readonly (string Value,string Label)[] HermesReasoningChoices=
    [
        ("none","关闭"),("minimal","极少"),("low","较低"),("medium","中等"),
        ("high","较高"),("xhigh","很高"),("max","最大"),("ultra","极致")
    ];
    private static string LocalizedHermesReasoningLabel(string value,string chinese)=>LocalizationService.IsEnglish?value switch
    {
        "none"=>"Off","minimal"=>"Minimal","low"=>"Low","medium"=>"Medium",
        "high"=>"High","xhigh"=>"Extra high","max"=>"Maximum","ultra"=>"Ultra",
        _=>value
    }:chinese;
    private static readonly string[] HermesReasoningValues=HermesReasoningChoices.Select(choice=>choice.Value).ToArray();
    private static readonly Brush PanelBrush = Brushes.White;
    private static readonly Brush ControlBorderBrush = new SolidColorBrush(Color.FromRgb(224, 230, 240));
    private static readonly Brush SecondaryBrush = new SolidColorBrush(Color.FromRgb(99, 112, 137));
    private readonly AppHost _host;
    private readonly ProviderHeaderCredentialService _headerCredentials = new();
    private readonly ComboBox _uiLanguage = new(), _delay = new(), _imageFormat = new(), _overlayOpacity = new(), _recordingFps = new(), _recordingQuality = new(), _gifFps = new(), _tempCleanup = new(), _voiceLanguage = new(), _hermesAgentSelector = new(), _hermesModelSelector = new(), _hermesReasoning = new(), _model = new();
    private readonly TextBox _hotkey = new();
    private readonly TextBox _baseUrl = new(), _customHeaders = new();
    private readonly TextBox _requestParameters = new();
    private readonly PasswordBox _apiKey = new();
    private readonly Button _clearApiKey = new(), _testApiConnection = new();
    private readonly TextBlock _connectionStatus = new() { TextWrapping = TextWrapping.Wrap, FontSize = 12, VerticalAlignment = VerticalAlignment.Center };
    private ApiConnectionsView _apiConnections = null!;
    private readonly Dictionary<AiProviderSettings,ApiConnectionDraft> _providerDrafts = new(ReferenceEqualityComparer.Instance);
    private readonly TextBlock _apiKeyStatus = new(), _windowConfigurationWarning = new(), _aiConfigurationWarning = new(), _hermesStatus = new();
    private readonly CheckBox _history = new(), _voice = new(), _autoVoice = new(), _startup = new(), _captureCursor = new(), _teachingMode = new(), _recordCursor = new(), _hermesAutoReadAloud = new();
    private readonly Button _hermesDetect = new(), _hermesTest = new();
    private readonly CheckBox _recordSystemAudio = new(), _recordMicrophone = new();
    private readonly List<AiProviderSettings> _providers;
    private readonly Dictionary<string, string> _pendingApiKeys = [];
    private readonly HashSet<string> _apiKeysMarkedForDeletion = [];
    // Keep this state keyed by the editable Provider instance, not its ID.
    // RepairIdentities may replace blank/duplicate IDs while the settings page
    // is open; an ID-keyed map could then make two providers share the wrong
    // unavailable-header warning.
    private readonly Dictionary<AiProviderSettings, HashSet<string>> _unavailableSensitiveHeaders = new(ReferenceEqualityComparer.Instance);
    private readonly Dictionary<AiProviderSettings,string> _hydrationErrors = new(ReferenceEqualityComparer.Instance);
    private readonly List<string> _editorWarnings = [];
    private CancellationTokenSource? _connectionTest;
    private CancellationTokenSource? _hermesConnectionTest;
    private CancellationTokenSource? _updateCheck;
    private readonly CancellationTokenSource _windowLifetime=new();
    private HermesInstallation? _hermesInstallation;
    private AiProviderSettings? _selectedProvider;
    private string? _defaultProviderId;
    private readonly int _repairedProviderIdentityCount;
    private bool _loadingProvider;
    private readonly TextBlock _modelStatus = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 9), FontSize = 12 };
    private CancellationTokenSource? _modelLoad;
    private bool _modelLoadPending;
    private readonly System.Windows.Threading.DispatcherTimer _modelLoadDebounce = new() { Interval = TimeSpan.FromMilliseconds(600) };
    private bool _loadingHermes;
    private bool _hermesBusy;
    private bool? _captureProtectionAvailable;
    private System.Windows.Input.Key _capturedHotkeyKey = System.Windows.Input.Key.S;
    private System.Windows.Input.ModifierKeys _capturedHotkeyModifiers = System.Windows.Input.ModifierKeys.Shift | System.Windows.Input.ModifierKeys.Alt;

    public SettingsWindow(AppHost host)
    {
        _host = host;
        _providers=[];
        var unavailableByProvider=new List<(AiProviderSettings Provider,HashSet<string> Headers)>();
        foreach(var stored in host.Settings.Providers ?? [])
        {
            if(stored is null)
            {
                _editorWarnings.Add("Provider 列表包含无效项，已跳过该项；请重新添加 Provider 后保存。");
                continue;
            }
            var unavailable=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            var hydration=ProviderEditorHydrationPolicy.TryHydrate(stored,_headerCredentials,unavailable);
            var editable=hydration.Provider;
            if(hydration.Warning is not null)
            {
                _hydrationErrors[editable]=hydration.Warning;
                _editorWarnings.Add($"{stored.Name ?? "Provider"}：{hydration.Warning}");
            }
            _providers.Add(editable);
            unavailableByProvider.Add((editable,unavailable));
        }
        if (_providers.Count == 0) _providers.Add(new AiProviderSettings());
        var identityResult=ProviderEditingPolicy.RepairIdentities(_providers,host.Settings.DefaultProviderId);
        _defaultProviderId=identityResult.DefaultProviderId;
        _repairedProviderIdentityCount=identityResult.RepairedIdentityCount;
        foreach(var unavailable in unavailableByProvider)
            if(unavailable.Headers.Count>0)_unavailableSensitiveHeaders[unavailable.Provider]=unavailable.Headers;
        Title = "喵呜AI 设置";
        Width = 760;
        // Keep the compact default; users can enlarge or maximize the window
        // when editing longer connection settings.
        Height = Math.Min(574, SystemParameters.WorkArea.Height - 40);
        MinWidth = 600;
        MinHeight = 400;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        // WindowChrome supplies the custom visuals. Keep the native frame
        // style so Windows maximizes to the monitor's working area.
        WindowStyle = WindowStyle.SingleBorderWindow;
        AllowsTransparency = false;
        Background = new SolidColorBrush(Color.FromRgb(245,247,252));
        Foreground = new SolidColorBrush(Color.FromRgb(23,32,51));
        UseLayoutRounding = true;
        SnapsToDevicePixels = true;
        TextOptions.SetTextFormattingMode(this, TextFormattingMode.Display);
        WindowChrome.SetWindowChrome(this,new WindowChrome
        {
            CaptionHeight=HasConfigurationWarnings?62:42,
            ResizeBorderThickness=new Thickness(6),
            GlassFrameThickness=new Thickness(0),
            CornerRadius=new CornerRadius(0),
            UseAeroCaptionButtons=false
        });

        var tabs = new TabControl
        {
            Margin = new Thickness(16, 8, 16, 14),
            TabStripPlacement = Dock.Left,
            Padding = new Thickness(0, 0, 0, 14),
            MinWidth = 0,
            ClipToBounds = false
        };
        tabs.Items.Add(Tab("常规", General()));
        tabs.Items.Add(Tab("捕获", Capture()));
        tabs.Items.Add(Tab("录屏", Recording()));
        _aiTab=Tab("AI", Ai(),scroll:false);
        tabs.Items.Add(_aiTab);
        tabs.Items.Add(Tab("语音", Voice()));
        tabs.Items.Add(Tab("隐私", Privacy()));
        tabs.Items.Add(Tab("关于", About()));

        var save = ActionButton("保存", true);
        save.Margin = new Thickness(0, 0, 18, 16);
        save.HorizontalAlignment = HorizontalAlignment.Right;
        save.Click += (_, _) => Save();
        var grid = new Grid { Background = new SolidColorBrush(Color.FromRgb(245,247,252)), ClipToBounds = false };
        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(HasConfigurationWarnings?68:48) });
        grid.RowDefinitions.Add(new RowDefinition());
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        var header = new Grid { Margin = new Thickness(20,0,12,0) };
        header.ColumnDefinitions.Add(new ColumnDefinition());
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var titleStack=new StackPanel{Orientation=Orientation.Horizontal,VerticalAlignment=VerticalAlignment.Center};
        titleStack.Children.Add(new Border
        {
            Width=26,
            Height=26,
            CornerRadius=new CornerRadius(8),
            Background=new SolidColorBrush(Color.FromRgb(232,245,255)),
            BorderBrush=new SolidColorBrush(Color.FromRgb(211,235,255)),
            BorderThickness=new Thickness(1),
            Padding=new Thickness(3),
            Child=new Image
            {
                Source=new System.Windows.Media.Imaging.BitmapImage(new Uri("pack://application:,,,/Assets/MewuAI.Icon.png")),
                Stretch=Stretch.Uniform
            }
        });
        var titleText=new StackPanel{VerticalAlignment=VerticalAlignment.Center,Margin=new Thickness(8,0,0,0)};
        titleText.Children.Add(new TextBlock { Text="喵呜AI 设置", FontSize=16.5, FontWeight=FontWeights.SemiBold });
        _windowConfigurationWarning.Foreground=new SolidColorBrush(Color.FromRgb(185,93,32));
        _windowConfigurationWarning.FontSize=10.5;
        _windowConfigurationWarning.TextTrimming=TextTrimming.CharacterEllipsis;
        _windowConfigurationWarning.MaxWidth=600;
        titleText.Children.Add(_windowConfigurationWarning);
        titleStack.Children.Add(titleText);
        Grid.SetColumn(titleStack, 0);
        header.Children.Add(titleStack);
        var windowActions = CreateWindowActions();
        Grid.SetColumn(windowActions, 1);
        header.Children.Add(windowActions);
        Grid.SetRow(tabs,1);Grid.SetRow(save,2);
        grid.Children.Add(header);grid.Children.Add(tabs);
        grid.Children.Add(save);
        Content = new Border { BorderBrush=ControlBorderBrush, BorderThickness=new Thickness(1), Background=new SolidColorBrush(Color.FromRgb(245,247,252)), Child=grid };
        RefreshConfigurationWarnings();
        SourceInitialized += (_, _) =>
        {
            var handle=new System.Windows.Interop.WindowInteropHelper(this).Handle;
            NativeMethods.TryUseSystemRoundedCorners(handle);
            // Settings are intentionally screenshotable so UI issues can be
            // reported. Sensitive values remain masked by the PasswordBox.
            _captureProtectionAvailable=NativeMethods.SetWindowCaptureVisibleForDiagnostics(handle);
            if(_captureProtectionAvailable==true){LoadDisplayedApiKey();return;}
            HideSensitiveEditorsAfterCaptureProtectionFailure();
        };
        Closed += (_, _) =>
        {
            _modelLoadDebounce.Stop();
            _modelLoad?.Cancel();
            _windowLifetime.Cancel();
            _connectionTest?.Cancel();
            _hermesConnectionTest?.Cancel();
            _updateCheck?.Cancel();
            _windowLifetime.Dispose();
        };
    }

    private static TabItem Tab(string header, UIElement content,bool scroll=true) => new()
    {
        Header = header,
        Height = 46,
        MinHeight = 46,
        Margin = new Thickness(0, 2, 8, 6),
        Padding = new Thickness(14, 9, 14, 9),
        Content = scroll?new ScrollViewer
        {
            Content = content,
            Background = PanelBrush,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            Padding = new Thickness(3)
        }:content
    };

    private static TextBlock Text(string text, bool secondary = false) => new()
    {
        Text = text,
        Margin = new Thickness(0, 4, 0, 8),
        TextWrapping = TextWrapping.Wrap,
        Foreground = secondary ? SecondaryBrush : new SolidColorBrush(Color.FromRgb(23,32,51))
    };

    private static StackPanel Panel() => new() { Margin = new Thickness(20) };

    private static FrameworkElement Labeled(string name, Control control)
    {
        var panel = new StackPanel();
        panel.Children.Add(Text(name, true));
        System.Windows.Automation.AutomationProperties.SetName(control, name);
        control.Margin = new Thickness(0, 0, 0, 9);
        panel.Children.Add(control);
        return panel;
    }

    private static Button ActionButton(string text, bool primary = false)
    {
        var button=new Button{Content=text,Padding=new Thickness(16,8,16,8),Cursor=System.Windows.Input.Cursors.Hand};
        button.SetResourceReference(StyleProperty,primary?"PrimaryButton":"SecondaryButton");
        return button;
    }

    private static void AddNumericChoices(ComboBox box,IEnumerable<int> values,int selected,string suffix)
    {
        box.SelectedValuePath="Tag";
        foreach(var value in values)
            box.Items.Add(new ComboBoxItem{Content=suffix=="%"?$"{value}%":$"{value} {suffix}",Tag=value});
        box.SelectedValue=selected;
    }

    private static int ReadNumericChoice(ComboBox box,int fallback)=>box.SelectedValue is int value?value:fallback;

    private static System.Windows.Shapes.Path CloseIcon() => new()
    {
        Width=15,
        Height=15,
        Stretch=Stretch.Uniform,
        Stroke=new SolidColorBrush(Color.FromRgb(82,99,122)),
        StrokeThickness=1.8,
        StrokeStartLineCap=PenLineCap.Round,
        StrokeEndLineCap=PenLineCap.Round,
        Data=Geometry.Parse("M3,3 L13,13 M13,3 L3,13")
    };

    private UIElement General()
    {
        var panel = Panel();
        panel.Children.Add(Text("界面语言", true));
        foreach(var item in new[]{("跟随 Windows","system"),("简体中文","zh-CN"),("English","en-US")})
            _uiLanguage.Items.Add(new ComboBoxItem{Content=item.Item1,Tag=item.Item2});
        _uiLanguage.SelectedIndex=_host.Settings.UiLanguage switch{"zh-CN"=>1,"en-US"=>2,_=>0};
        System.Windows.Automation.AutomationProperties.SetName(_uiLanguage,"界面语言");
        panel.Children.Add(_uiLanguage);
        panel.Children.Add(Text("语言设置将在重新启动喵呜AI后生效。",true));
        panel.Children.Add(ThinkingGlowSettings());
        panel.Children.Add(Text("启动与快捷键", true));
        _startup.Content = "登录 Windows 后自动启动";
        _startup.IsChecked = _host.Settings.LaunchAtStartup;
        panel.Children.Add(_startup);
        panel.Children.Add(Text("全局截图快捷键", true));
        _capturedHotkeyKey = _host.Settings.CaptureHotkey.Key;
        _capturedHotkeyModifiers = _host.Settings.CaptureHotkey.Modifiers;
        _hotkey.IsReadOnly = true;
        _hotkey.MinHeight = 36;
        _hotkey.Padding = new Thickness(10, 6, 10, 6);
        _hotkey.VerticalContentAlignment = VerticalAlignment.Center;
        _hotkey.Cursor = System.Windows.Input.Cursors.Hand;
        _hotkey.Text = FormatHotkey(_capturedHotkeyKey, _capturedHotkeyModifiers);
        _hotkey.PreviewKeyDown += CaptureHotkeyKeyDown;
        _hotkey.GotKeyboardFocus += (_, _) => _hotkey.SelectAll();
        System.Windows.Automation.AutomationProperties.SetName(_hotkey, "全局截图快捷键按键");
        panel.Children.Add(_hotkey);
        panel.Children.Add(Text(LocalizationService.T("点击输入框后按组合键设置（至少包含 Shift、Alt 或 Ctrl），按 Delete 清空；保存后生效。", "Click the field and press a shortcut with Shift, Alt or Ctrl; press Delete to clear. Changes take effect after saving."), true));
        var restore = ActionButton("恢复默认 Shift + Alt + S");
        restore.Click += (_, _) => SetCapturedHotkey(System.Windows.Input.Key.S, System.Windows.Input.ModifierKeys.Shift | System.Windows.Input.ModifierKeys.Alt);
        panel.Children.Add(restore);
        panel.Children.Add(Text("关闭主窗口不会退出；请使用托盘菜单退出。", true));
        return panel;
    }

    private static string FormatHotkey(System.Windows.Input.Key key, System.Windows.Input.ModifierKeys modifiers)
    {
        if(key==System.Windows.Input.Key.None)return string.Empty;
        var parts = new List<string>();
        if (modifiers.HasFlag(System.Windows.Input.ModifierKeys.Control)) parts.Add("Ctrl");
        if (modifiers.HasFlag(System.Windows.Input.ModifierKeys.Shift)) parts.Add("Shift");
        if (modifiers.HasFlag(System.Windows.Input.ModifierKeys.Alt)) parts.Add("Alt");
        parts.Add(key.ToString());
        return string.Join(" + ", parts);
    }

    private void SetCapturedHotkey(System.Windows.Input.Key key, System.Windows.Input.ModifierKeys modifiers)
    {
        _capturedHotkeyKey = key; _capturedHotkeyModifiers = modifiers;
        _hotkey.Text = FormatHotkey(key, modifiers);
    }

    private void CaptureHotkeyKeyDown(object sender, System.Windows.Input.KeyEventArgs e)
    {
        e.Handled = true;
        var key = e.Key == System.Windows.Input.Key.System ? e.SystemKey : e.Key;
        if(key==System.Windows.Input.Key.Delete)
        {
            SetCapturedHotkey(System.Windows.Input.Key.None,System.Windows.Input.ModifierKeys.None);
            return;
        }
        if (key is System.Windows.Input.Key.LeftCtrl or System.Windows.Input.Key.RightCtrl or System.Windows.Input.Key.LeftShift or System.Windows.Input.Key.RightShift or System.Windows.Input.Key.LeftAlt or System.Windows.Input.Key.RightAlt) return;
        var modifiers = System.Windows.Input.Keyboard.Modifiers & (System.Windows.Input.ModifierKeys.Control | System.Windows.Input.ModifierKeys.Shift | System.Windows.Input.ModifierKeys.Alt);
        if (modifiers == System.Windows.Input.ModifierKeys.None) return;
        if (key < System.Windows.Input.Key.A || key > System.Windows.Input.Key.Z) return;
        SetCapturedHotkey(key, modifiers);
    }

    private UIElement Capture()
    {
        var panel = Panel();
        panel.Children.Add(Text("延时截图", true));
        foreach (var seconds in new[] { 0, 3, 5 }) _delay.Items.Add($"{seconds} 秒");
        _delay.SelectedIndex = _host.Settings.CaptureDelaySeconds switch { 3 => 1, 5 => 2, _ => 0 };
        System.Windows.Automation.AutomationProperties.SetName(_delay, "延时截图");
        panel.Children.Add(_delay);
        panel.Children.Add(Text("默认图片格式", true));
        _imageFormat.Items.Add("PNG");
        _imageFormat.Items.Add("JPEG");
        _imageFormat.SelectedIndex = _host.Settings.DefaultImageFormat.Equals("jpg", StringComparison.OrdinalIgnoreCase) || _host.Settings.DefaultImageFormat.Equals("jpeg", StringComparison.OrdinalIgnoreCase) ? 1 : 0;
        System.Windows.Automation.AutomationProperties.SetName(_imageFormat, "默认图片格式");
        panel.Children.Add(_imageFormat);
        panel.Children.Add(Text("选区外暗化程度", true));
        var opacityPercent=(int)Math.Round(Math.Clamp(_host.Settings.OverlayOpacity,.4,.75)*100);
        AddNumericChoices(_overlayOpacity,SettingsChoicePolicy.IncludeCurrent(new[] { 40, 45, 50, 55, 60, 65, 70, 75 },opacityPercent),opacityPercent,"%");
        System.Windows.Automation.AutomationProperties.SetName(_overlayOpacity, "选区外暗化程度");
        panel.Children.Add(_overlayOpacity);
        _captureCursor.Content = "截图包含系统鼠标指针";
        _captureCursor.IsChecked = _host.Settings.IncludeCaptureCursor;
        panel.Children.Add(_captureCursor);
        _teachingMode.Content=LocalizationService.T("教学演示模式（允许屏幕共享看到框选和标注）","Teaching mode (show selections and annotations in screen sharing)");
        _teachingMode.IsChecked=_host.Settings.TeachingMode;
        panel.Children.Add(_teachingMode);
        panel.Children.Add(Text(LocalizationService.T("保存后，下次截图及其贴图生效。请在会议或教学软件中共享整个屏幕。支持同时使用区域录屏和滚动长截图，采集区域内会让出实时画面，操作控件放在区域外；F8 停止录屏或完成长截图。设置和密钥仍受防捕获保护。","Applies to your next capture and pinned windows. Share your entire screen in the meeting app. Region recording and scrolling capture remain available: the capture area shows live content, with controls placed outside it. Press F8 to stop recording or finish scrolling capture. Settings and credentials remain protected."),true));
        panel.Children.Add(Text("截图、OCR、复制和保存均在本地完成。", true));
        return panel;
    }

    private UIElement Recording()
    {
        var panel = Panel();
        panel.Children.Add(Text("MP4 帧率", true));
        AddNumericChoices(_recordingFps,SettingsChoicePolicy.IncludeCurrent(new[] { 15, 24, 30, 60 },_host.Settings.RecordingFps),_host.Settings.RecordingFps,"FPS");
        System.Windows.Automation.AutomationProperties.SetName(_recordingFps, "MP4 帧率");
        panel.Children.Add(_recordingFps);
        panel.Children.Add(Text("MP4 质量", true));
        AddNumericChoices(_recordingQuality,SettingsChoicePolicy.IncludeCurrent(new[] { 50, 75, 90 },_host.Settings.RecordingQuality),_host.Settings.RecordingQuality,"%");
        System.Windows.Automation.AutomationProperties.SetName(_recordingQuality, "MP4 质量");
        panel.Children.Add(_recordingQuality);
        panel.Children.Add(Text("GIF 帧率", true));
        AddNumericChoices(_gifFps,SettingsChoicePolicy.IncludeCurrent(new[] { 5, 10, 15 },_host.Settings.GifFps),_host.Settings.GifFps,"FPS");
        System.Windows.Automation.AutomationProperties.SetName(_gifFps, "GIF 帧率");
        panel.Children.Add(_gifFps);
        _recordCursor.Content = "录屏包含系统鼠标指针";
        _recordCursor.IsChecked = _host.Settings.IncludeRecordingCursor;
        panel.Children.Add(_recordCursor);
        _recordSystemAudio.Content=LocalizationService.T("录制电脑声音","Record computer audio");
        _recordSystemAudio.IsChecked=_host.Settings.RecordSystemAudio;
        panel.Children.Add(_recordSystemAudio);
        _recordMicrophone.Content=LocalizationService.T("同时录制麦克风","Record microphone audio");
        _recordMicrophone.IsChecked=_host.Settings.RecordMicrophone;
        panel.Children.Add(_recordMicrophone);
        panel.Children.Add(Text("自动清理临时媒体", true));
        AddNumericChoices(_tempCleanup,SettingsChoicePolicy.IncludeCurrent(new[] { 1, 3, 7, 14, 30 },_host.Settings.TempCleanupDays),_host.Settings.TempCleanupDays,"天");
        System.Windows.Automation.AutomationProperties.SetName(_tempCleanup, "临时媒体保留天数");
        panel.Children.Add(_tempCleanup);
        panel.Children.Add(Text("未保存的录制暂存在本机，并由应用自动清理。", true));
        return panel;
    }

    private UIElement Ai()
    {
        _codexSettings=new CodexSettingsPage(_host.Settings,_windowLifetime.Token);
        _workBuddySettings=new WorkBuddySettingsPage(_host.Settings,_windowLifetime.Token);
        _miniMaxCodeSettings=new MiniMaxCodeSettingsPage(_host.Settings,_windowLifetime.Token);
        var hermes=HermesPage();
        var selected=_host.Settings.MiniMaxCodeEnabled?AiSettingsTabs.MiniMaxCodeIndex:_host.Settings.WorkBuddyEnabled?AiSettingsTabs.WorkBuddyIndex:_host.Settings.CodexEnabled?AiSettingsTabs.CodexIndex:_host.Settings.HermesEnabled?AiSettingsTabs.HermesIndex:AiSettingsTabs.ApiIndex;
        _backendSelector=new AiSettingsTabs(Api(),hermes,_codexSettings,selected,_workBuddySettings,_miniMaxCodeSettings){Margin=new Thickness(12,12,12,4)};
        _backendSelector.BackendChanged+=(_,_)=>UpdateHermesControls();
        UpdateHermesControls();
        return _backendSelector;
    }


    private UIElement HermesPage()
    {
        _loadingHermes=true;

        var form=new AiSettingsForm("Hermes",LocalizationService.T("连接本机 Hermes，沿用所选人格的模型与会话。","Connect to local Hermes using the selected profile’s model and conversation."),_hermesStatus);
        form.AddAction(_hermesTest,LocalizationService.T("测试连接","Test connection"));
        form.AddAction(_hermesDetect,LocalizationService.T("重新检测","Detect again"));

        _hermesAgentSelector.DisplayMemberPath=nameof(HermesAgentOption.Label);
        _hermesAgentSelector.MinWidth=0;
        System.Windows.Automation.AutomationProperties.SetName(_hermesAgentSelector,"Hermes Agent / 人格");
        EnsureStoredHermesAgentItem();

        _hermesModelSelector.DisplayMemberPath=nameof(HermesModelOption.DisplayName);
        _hermesModelSelector.MinWidth=0;
        System.Windows.Automation.AutomationProperties.SetName(_hermesModelSelector,"Hermes 模型");
        System.Windows.Automation.AutomationProperties.SetName(_hermesReasoning,"Hermes 思考程度");
        EnsureStoredHermesModelItem();
        PopulateHermesReasoningChoices(_host.Settings.HermesReasoningEffort);
        form.Fields.Children.Add(AiSettingsForm.Field(LocalizationService.T("Agent / 人格","Agent / Profile"),_hermesAgentSelector));
        form.Fields.Children.Add(AiSettingsForm.Field(LocalizationService.T("模型","Model"),_hermesModelSelector));
        form.Fields.Children.Add(AiSettingsForm.Field(LocalizationService.T("思考程度","Reasoning effort"),_hermesReasoning));

        _hermesAutoReadAloud.Content="回复后自动朗读";
        _hermesAutoReadAloud.IsChecked=_host.Settings.HermesAutoReadAloud;
        _hermesAutoReadAloud.Margin=new Thickness(0,5,0,0);
        System.Windows.Automation.AutomationProperties.SetName(_hermesAutoReadAloud,"Hermes 回复后自动朗读");

        form.Fields.Children.Add(_hermesAutoReadAloud);
        form.Loaded+=HermesPageLoaded;

        _hermesAgentSelector.SelectionChanged+=async (_,_)=>
        {
            if(_loadingHermes||!IsLoaded||!HermesSelected)return;
            _hermesModelSelector.Items.Clear();
            await ConnectHermesAsync(false);
        };
        _hermesModelSelector.SelectionChanged+=(_,_)=>{if(!_loadingHermes)PopulateHermesReasoningChoices(ReadHermesReasoning());};
        _hermesDetect.Click+=async (_,_)=>
        {
            DetectHermes();
            if(HermesSelected&&_hermesInstallation is not null)await ConnectHermesAsync(true);
        };
        _hermesTest.Click+=async (_,_)=>await ConnectHermesAsync(true);
        _loadingHermes=false;
        DetectHermes();
        UpdateHermesControls();
        return form;
    }

    private void ApiModelLoaded(object sender,RoutedEventArgs e)
    {
        if (_modelLoadPending) ScheduleModelLoad();
    }

    private async void HermesPageLoaded(object sender,RoutedEventArgs e)
    {
        if(sender is FrameworkElement page)page.Loaded-=HermesPageLoaded;
        DetectHermes();
        if(HermesSelected&&_hermesInstallation is not null)
            await ConnectHermesAsync(false);
    }

    private void DetectHermes()
    {
        try
        {
            _hermesInstallation=_host.DiscoverHermes();
            if(_hermesInstallation is null)SetHermesStatus("未检测到本机 Hermes",HermesStatusTone.Error);
            else SetHermesStatus($"已检测 · {_hermesInstallation.HomePath}",HermesStatusTone.Ready);
        }
        catch(Exception ex)
        {
            _hermesInstallation=null;
            SetHermesStatus("检测失败，请重试",HermesStatusTone.Error);
            try{new PrivacyLogger().Error("HermesDiscovery",ex);}catch{}
        }
        UpdateHermesControls();
    }

    private async Task ConnectHermesAsync(bool refresh)
    {
        if(_hermesBusy)return;
        DetectHermes();
        if(_hermesInstallation is null)return;
        _hermesBusy=true;
        _hermesConnectionTest?.Cancel();
        using var test=CancellationTokenSource.CreateLinkedTokenSource(_windowLifetime.Token);
        test.CancelAfter(TimeSpan.FromSeconds(55));
        _hermesConnectionTest=test;
        SetHermesStatus("正在连接本机 Hermes…",HermesStatusTone.Working);
        UpdateHermesControls();
        try
        {
            var agents=await _host.GetHermesAgentOptionsAsync(test.Token);
            test.Token.ThrowIfCancellationRequested();
            if(agents.Count==0)throw new InvalidOperationException("Hermes 未返回可用 Agent / 人格。");
            PopulateHermesAgents(agents);
            var profile=(_hermesAgentSelector.SelectedItem as HermesAgentOption)?.Name??"default";
            var options=await _host.GetHermesModelOptionsAsync(profile,refresh,test.Token);
            test.Token.ThrowIfCancellationRequested();
            if(options.Count==0)throw new InvalidOperationException("Hermes 未返回可用模型，请先完成 Hermes 模型配置。");
            PopulateHermesModels(options);
            SetHermesStatus($"连接正常 · {agents.Count} 个 Agent · {options.Count} 个模型",HermesStatusTone.Connected);
        }
        catch(OperationCanceledException) when(_windowLifetime.IsCancellationRequested){}
        catch(OperationCanceledException)
        {
            if(IsVisible)SetHermesStatus("连接超时，请检查 Hermes 配置",HermesStatusTone.Error);
        }
        catch(Exception ex)
        {
            try{new PrivacyLogger().Error("HermesConnectionTest",ex);}catch{}
            if(IsVisible)SetHermesStatus(ex.Message,HermesStatusTone.Error);
        }
        finally
        {
            if(ReferenceEquals(_hermesConnectionTest,test))_hermesConnectionTest=null;
            _hermesBusy=false;
            if(IsVisible)UpdateHermesControls();
        }
    }

    private void PopulateHermesAgents(IReadOnlyList<HermesAgentOption> options)
    {
        var current=(_hermesAgentSelector.SelectedItem as HermesAgentOption)?.Name??_host.Settings.HermesProfile;
        _loadingHermes=true;
        try
        {
            _hermesAgentSelector.Items.Clear();
            foreach(var option in options)_hermesAgentSelector.Items.Add(option);
            _hermesAgentSelector.SelectedItem=options.FirstOrDefault(option=>string.Equals(option.Name,current,StringComparison.Ordinal))
                ??options.FirstOrDefault(option=>option.IsDefault)
                ??options.FirstOrDefault();
        }
        finally{_loadingHermes=false;}
    }

    private void EnsureStoredHermesAgentItem()
    {
        var profile=string.IsNullOrWhiteSpace(_host.Settings.HermesProfile)?"default":_host.Settings.HermesProfile.Trim();
        var saved=new HermesAgentOption(profile,profile,string.Empty,string.Empty,string.Empty,profile=="default");
        _hermesAgentSelector.Items.Add(saved);_hermesAgentSelector.SelectedItem=saved;
    }

    private void PopulateHermesModels(IReadOnlyList<HermesModelOption> options)
    {
        var current=_hermesModelSelector.SelectedItem as HermesModelOption;
        var provider=current?.Provider??_host.Settings.HermesProvider;
        var model=current?.Model??_host.Settings.HermesModel;
        _loadingHermes=true;
        try
        {
            _hermesModelSelector.Items.Clear();
            var unique=options.GroupBy(item=>item.Key,StringComparer.Ordinal).Select(group=>group.First()).ToList();
            foreach(var option in unique)
                _hermesModelSelector.Items.Add(option);
            _hermesModelSelector.SelectedItem=unique.FirstOrDefault(option=>
                string.Equals(option.Provider,provider,StringComparison.Ordinal)&&string.Equals(option.Model,model,StringComparison.Ordinal))
                ??unique.FirstOrDefault(option=>option.IsCurrent)
                ??unique.FirstOrDefault();
            PopulateHermesReasoningChoices(ReadHermesReasoning());
        }
        finally{_loadingHermes=false;}
        UpdateHermesControls();
    }

    private void EnsureStoredHermesModelItem()
    {
        if(string.IsNullOrWhiteSpace(_host.Settings.HermesModel))return;
        var provider=_host.Settings.HermesProvider?.Trim()??string.Empty;
        var model=_host.Settings.HermesModel.Trim();
        var prefix=string.IsNullOrWhiteSpace(provider)?string.Empty:$"{provider} · ";
        var saved=new HermesModelOption(provider,model,$"{prefix}{model}",HermesReasoningValues);
        _hermesModelSelector.Items.Add(saved);_hermesModelSelector.SelectedItem=saved;
    }

    private void PopulateHermesReasoningChoices(string preferred)
    {
        var selectedModel=_hermesModelSelector.SelectedItem as HermesModelOption;
        var allowed=(selectedModel?.ReasoningEfforts??HermesReasoningValues).ToHashSet(StringComparer.Ordinal);
        var normalized=string.IsNullOrWhiteSpace(preferred)?"medium":preferred.Trim().ToLowerInvariant();
        _hermesReasoning.Items.Clear();
        foreach(var choice in HermesReasoningChoices.Where(choice=>allowed.Contains(choice.Value)))
            _hermesReasoning.Items.Add(new ComboBoxItem{Content=LocalizedHermesReasoningLabel(choice.Value,choice.Label),Tag=choice.Value});
        _hermesReasoning.SelectedItem=_hermesReasoning.Items.OfType<ComboBoxItem>().FirstOrDefault(item=>string.Equals(item.Tag?.ToString(),normalized,StringComparison.Ordinal))
            ??_hermesReasoning.Items.OfType<ComboBoxItem>().FirstOrDefault();
    }

    private string ReadHermesReasoning()=>
        (_hermesReasoning.SelectedItem as ComboBoxItem)?.Tag?.ToString()??_host.Settings.HermesReasoningEffort;

    private void UpdateHermesControls()
    {
        var enabled=HermesSelected;
        _hermesDetect.IsEnabled=!_hermesBusy;
        _hermesTest.IsEnabled=!_hermesBusy&&_hermesInstallation is not null;
        _hermesAgentSelector.IsEnabled=enabled&&!_hermesBusy&&_hermesAgentSelector.Items.Count>0;
        _hermesModelSelector.IsEnabled=enabled&&!_hermesBusy&&_hermesModelSelector.Items.Count>0;
        _hermesReasoning.IsEnabled=enabled&&!_hermesBusy&&_hermesReasoning.Items.Count>0;
        _hermesAutoReadAloud.IsEnabled=enabled;
    }

    private void SetHermesStatus(string message,HermesStatusTone tone)
    {
        message=LocalizationService.TranslateUiText(message);_hermesStatus.Text=message;_hermesStatus.ToolTip=message;
        var color=tone switch
        {
            HermesStatusTone.Connected=>Color.FromRgb(34,157,105),
            HermesStatusTone.Ready=>Color.FromRgb(44,137,204),
            HermesStatusTone.Working=>Color.FromRgb(221,143,50),
            _=>Color.FromRgb(201,78,91)
        };
        var brush=new SolidColorBrush(color);_hermesStatus.Foreground=brush;
    }

    private enum HermesStatusTone { Ready,Connected,Working,Error }

    private UIElement Voice()
    {
        var panel = Panel();
        _voice.Content = "启用语音输入";
        _voice.IsChecked = _host.Settings.EnableVoiceInput;
        _autoVoice.Content = "Prompt 出现时自动监听";
        _autoVoice.IsChecked = _host.Settings.AutomaticallyStartListening;
        _autoVoice.IsEnabled = _voice.IsChecked == true;
        _voice.Checked += (_, _) => _autoVoice.IsEnabled = true;
        _voice.Unchecked += (_, _) => { _autoVoice.IsChecked = false; _autoVoice.IsEnabled = false; };
        panel.Children.Add(_voice);
        panel.Children.Add(_autoVoice);
        panel.Children.Add(Text("识别语言", true));
        foreach (var item in new[] { ("跟随 Windows", "system"), ("简体中文", "zh-CN"), ("英语", "en-US") })
            _voiceLanguage.Items.Add(new ComboBoxItem { Content = item.Item1, Tag = item.Item2 });
        _voiceLanguage.SelectedIndex = _host.Settings.VoiceLanguage switch { "zh-CN" => 1, "en-US" => 2, _ => 0 };
        System.Windows.Automation.AutomationProperties.SetName(_voiceLanguage, "识别语言");
        panel.Children.Add(_voiceLanguage);
        panel.Children.Add(Text("识别结果只会填入输入框，不会自动发送。", true));
        return panel;
    }

    private UIElement Privacy()
    {
        var panel = Panel();
        _history.Content = "在本地保存 AI 对话历史";
        _history.IsChecked = _host.Settings.SaveConversationHistory;
        panel.Children.Add(_history);
        panel.Children.Add(Text("关闭后仍可在本次运行中查看；退出应用后不保留。", true));
        panel.Children.Add(Text("媒体默认不永久保存；截图只有明确点击发送后才会上传。", true));
        var clearHistory = ActionButton("清空本地对话历史");
        clearHistory.Margin = new Thickness(0, 8, 0, 0);
        clearHistory.Click += async (_, _) =>
        {
            clearHistory.IsEnabled=false;
            try
            {
                await new ConversationHistoryService().ClearAsync(_windowLifetime.Token);
                _host.ClearSessionConversationHistory();
                if(IsVisible)MessageBox.Show(this,"本地对话历史已清空","喵呜AI");
            }
            catch(OperationCanceledException) when(_windowLifetime.IsCancellationRequested){}
            catch(Exception ex)
            {
                try{new PrivacyLogger().Error("ConversationHistoryClear",ex);}catch{}
                if(IsVisible)MessageBox.Show(this,"本地对话历史清理失败，请稍后重试。","无法清理");
            }
            finally{if(IsVisible)clearHistory.IsEnabled=true;}
        };
        panel.Children.Add(clearHistory);
        var clear = ActionButton("清理临时媒体");
        clear.Margin = new Thickness(0, 6, 0, 0);
        clear.Click += (_, _) => ClearTemporaryMedia();
        panel.Children.Add(clear);
        var open = ActionButton("打开数据目录");
        open.Margin = new Thickness(0, 6, 0, 0);
        open.Click += (_, _) => System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "MewuAI")) { UseShellExecute = true });
        panel.Children.Add(open);
        var appVersion=typeof(SettingsWindow).Assembly.GetName().Version?.ToString(3)??"0.2.0";
        panel.Children.Add(Text(LocalizationService.T($"应用版本：{appVersion}\n.NET：{Environment.Version}\nWindows：{Environment.OSVersion.Version}\n捕获：GDI desktop snapshot / PP-OCRv6（Windows OCR 仅故障降级）\n录屏：Media Foundation H.264",$"App version: {appVersion}\n.NET: {Environment.Version}\nWindows: {Environment.OSVersion.Version}\nCapture: GDI desktop snapshot / PP-OCRv6 (Windows OCR fallback)\nRecording: Media Foundation H.264"), true));
        return panel;
    }

    private UIElement About()
    {
        var panel=Panel();
        panel.Children.Add(new TextBlock{Text="MewuAI",FontSize=24,FontWeight=FontWeights.SemiBold,Foreground=new SolidColorBrush(Color.FromRgb(49,73,126)),Margin=new Thickness(0,0,0,4)});
        var appVersion=typeof(SettingsWindow).Assembly.GetName().Version?.ToString(3)??"0.2.0";
        panel.Children.Add(Text(LocalizationService.T("作者：Abner Stephen\n版本："+appVersion+"\nWindows 截图、OCR、标注、录屏与多模态 AI 工作流","Author: Abner Stephen\nVersion: "+appVersion+"\nA Windows workflow for capture, OCR, annotation, recording, and multimodal AI."), true));
        var updateStatus=Text(LocalizationService.T("从 GitHub Releases 获取正式版本。安装包下载后会校验 SHA-256。","MewuAI checks GitHub Releases for official updates and verifies every installer with SHA-256."),true);
        updateStatus.Margin=new Thickness(0,4,0,8);
        var checkUpdate=ActionButton(LocalizationService.T("检查更新","Check for updates"));
        checkUpdate.HorizontalAlignment=HorizontalAlignment.Left;
        checkUpdate.Click+=async (_,_)=>await CheckForUpdatesAsync(checkUpdate,updateStatus);
        async void CheckOnFirstRender(object? sender,EventArgs e)
        {
            ContentRendered-=CheckOnFirstRender;
            if(IsVisible&&!_windowLifetime.IsCancellationRequested)
                await CheckForUpdatesAsync(checkUpdate,updateStatus);
        }
        ContentRendered+=CheckOnFirstRender;
        panel.Children.Add(checkUpdate);
        panel.Children.Add(updateStatus);
        var repo=new TextBlock{Margin=new Thickness(0,8,0,12),TextWrapping=TextWrapping.Wrap};
        var link=new Hyperlink(new Run(LocalizationService.T("GitHub 开源仓库 · github.com/abnste/mewu_ai","Open-source repository · github.com/abnste/mewu_ai"))){NavigateUri=new Uri("https://github.com/abnste/mewu_ai")};
        link.RequestNavigate+=(_,e)=>{try{System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(e.Uri.AbsoluteUri){UseShellExecute=true});}catch{};e.Handled=true;};repo.Inlines.Add(link);panel.Children.Add(repo);
        panel.Children.Add(new Border{Background=new SolidColorBrush(Color.FromRgb(247,249,253)),BorderBrush=ControlBorderBrush,BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(10),Padding=new Thickness(14,12,14,12),Child=new TextBlock{Text=LocalizationService.T("许可说明 · MPL-2.0\n本项目自有源代码采用 Mozilla Public License 2.0，允许遵守协议的商业使用。对外分发时须按协议提供受 MPL 覆盖的源代码及修改，并保留版权和许可声明。源码：github.com/abnste/mewu_ai；完整条款见随附 LICENSE，第三方组件保留各自许可证。","License · MPL-2.0\nProject-owned source code uses the Mozilla Public License 2.0, which permits compliant commercial use. Distribution requires providing MPL-covered source and modifications and preserving copyright and license notices under the license. Source: github.com/abnste/mewu_ai. See the bundled LICENSE for full terms; third-party components retain their own licenses."),TextWrapping=TextWrapping.Wrap,Foreground=SecondaryBrush,LineHeight=20}});
        var notices=ActionButton(LocalizationService.T("开源许可与第三方声明","Open-source licenses and third-party notices"));
        notices.HorizontalAlignment=HorizontalAlignment.Stretch;notices.Margin=new Thickness(0,12,0,0);
        notices.Click+=(_,_)=>new LicenseNoticesWindow{Owner=this,Topmost=Topmost}.ShowDialog();
        panel.Children.Add(notices);
        return panel;
    }

    private async Task CheckForUpdatesAsync(Button button,TextBlock status)
    {
        if(_updateCheck is not null||!IsVisible||_windowLifetime.IsCancellationRequested)return;
        _updateCheck=CancellationTokenSource.CreateLinkedTokenSource(_windowLifetime.Token);
        var operation=_updateCheck;
        button.IsEnabled=false;
        var service=new ApplicationUpdateService();
        var progress=new Progress<ApplicationUpdateProgress>(value=>
        {
            if(!ReferenceEquals(_updateCheck,operation)||operation.IsCancellationRequested||!IsVisible)return;
            status.Text=value.TotalBytes is >0
                ?$"{value.Message} {Math.Clamp(value.BytesReceived*100d/value.TotalBytes.Value,0,100):0}%"
                :value.Message;
        });
        try
        {
            var current=typeof(SettingsWindow).Assembly.GetName().Version??new Version(0,1,0);
            var result=await service.CheckAndDownloadAsync(current,progress,operation.Token,
                async (_,tag,token)=>await Dispatcher.InvokeAsync(()=>
                {
                    if(token.IsCancellationRequested||!ReferenceEquals(_updateCheck,operation)||!IsVisible)return false;
                    status.Text=LocalizationService.T($"发现新版本 {tag}",$"New version available: {tag}");
                    return MewuDialogWindow.ShowChoice(
                        this,
                        LocalizationService.T("发现新版本","Update available"),
                        LocalizationService.T($"当前版本 v{current.ToString(3)}，发现新版本 {tag}。是否下载更新？下载完成后可选择安装并重启。",$"You're using v{current.ToString(3)}. MewuAI {tag} is available. Download the update? You can choose to install and restart after the download."),
                        LocalizationService.T("下载更新","Download update"),
                        LocalizationService.T("稍后","Later"))==MewuDialogResult.Primary;
                },System.Windows.Threading.DispatcherPriority.Normal,token).Task);
            if(!ReferenceEquals(_updateCheck,operation)||operation.IsCancellationRequested||!IsVisible)return;
            if(!result.IsUpdateAvailable)
            {
                status.Text=LocalizationService.T($"已是最新版 v{current.ToString(3)}",$"You're up to date (v{current.ToString(3)})");
                return;
            }

            if(result.Package is null)
            {
                status.Text=LocalizationService.T($"发现新版本 {result.TagName}，可点击“检查更新”继续。",$"{result.TagName} is available. Click Check for updates when you're ready.");
                return;
            }

            status.Text=LocalizationService.T($"{result.TagName} 已下载并通过校验",$"{result.TagName} downloaded and verified");
            var choice=MewuDialogWindow.ShowChoice(
                this,
                LocalizationService.T("更新已准备好","Update ready"),
                LocalizationService.T($"喵呜AI {result.TagName} 已下载并通过 SHA-256 校验。立即安装并自动重启应用吗？",$"MewuAI {result.TagName} has been downloaded and verified with SHA-256. Install it now and restart MewuAI?"),
                LocalizationService.T("安装并重启","Install and restart"),
                LocalizationService.T("稍后","Later"));
            if(choice!=MewuDialogResult.Primary||operation.IsCancellationRequested||!ReferenceEquals(_updateCheck,operation)||!IsVisible)return;
            status.Text=LocalizationService.T("正在启动安装程序…","Starting the installer…");
            await service.LaunchInstallerAsync(result.Package!,operation.Token);
            _host.Exit();
        }
        catch(OperationCanceledException) when(operation.IsCancellationRequested){}
        catch(Exception ex)
        {
            try{new PrivacyLogger().Error("ApplicationUpdate",ex);}catch{}
            if(ReferenceEquals(_updateCheck,operation)&&IsVisible)
                status.Text=LocalizationService.T($"检查更新失败：{ex.Message}",$"Update failed: {LocalizationService.TranslateUiText(ex.Message)}\nPlease try again.");
        }
        finally
        {
            if(ReferenceEquals(_updateCheck,operation))
            {
                _updateCheck=null;
                operation.Dispose();
                if(IsVisible)button.IsEnabled=true;
            }
        }
    }

    private static AiProviderSettings CloneProvider(AiProviderSettings source) => ProviderHeaderCredentialService.Clone(source);

    private void SelectProvider(AiProviderSettings? provider)
    {
        if (_loadingProvider || provider is null) return;
        InvalidateConnectionTest();
        _modelLoad?.Cancel();
        _modelLoad = null;
        _modelLoadDebounce.Stop();
        _selectedProvider = provider;
        _loadingProvider = true;
        try
        {
            var draft = _providerDrafts.GetValueOrDefault(provider) ?? ApiConnectionDraft.FromProvider(provider);
            _baseUrl.Text = draft.BaseUrl;
            PopulateModelSuggestions(draft.Model);
            _requestParameters.Text = draft.ParametersJson;
            _customHeaders.Text = _captureProtectionAvailable == false
                ? "屏幕防捕获不可用，Custom Headers 已隐藏。" : draft.HeadersJson;
            _apiAdvanced.IsExpanded = ProviderPresetPolicy.Detect(provider).RequiresBaseUrl &&
                string.IsNullOrWhiteSpace(draft.BaseUrl);
            LoadDisplayedApiKey();
        }
        finally { _loadingProvider = false; }
        UpdateApiKeyStatus();
        ScheduleModelLoad();
    }

    private void PopulateModelSuggestions(string currentModel,IEnumerable<string>? liveModels=null)
    {
        var wasLoading = _loadingProvider;
        _loadingProvider = true;
        try
        {
        _model.Items.Clear();var models=new List<string>();
        if (liveModels is not null) models.AddRange(liveModels);
        if(!string.IsNullOrWhiteSpace(currentModel)&&!models.Contains(currentModel,StringComparer.OrdinalIgnoreCase))models.Insert(0,currentModel);
        foreach(var model in models.Distinct(StringComparer.OrdinalIgnoreCase))_model.Items.Add(model);
        _model.SelectedItem = models.FirstOrDefault(m => m.Equals(currentModel, StringComparison.OrdinalIgnoreCase));
        _model.Text=currentModel;
        }
        finally { _loadingProvider = wasLoading; }
    }


    private void ScheduleModelLoad()
    {
        if (_loadingProvider) return;
        _modelLoad?.Cancel();
        _modelLoad = null;
        _modelLoadDebounce.Stop();
        _modelStatus.Text = LocalizationService.T("模型可从列表选择，也可手动输入 ID。", "Select a model from the list or enter its ID.");
        _modelLoadPending = !_model.IsLoaded;
        if (!_modelLoadPending && !_windowLifetime.IsCancellationRequested) _modelLoadDebounce.Start();
    }

    private async Task RefreshModelsAsync()
    {
        if (_selectedProvider is null || _loadingProvider || _windowLifetime.IsCancellationRequested) return;
        _modelLoadDebounce.Stop();
        _modelLoad?.Cancel();
        using var operation = CancellationTokenSource.CreateLinkedTokenSource(_windowLifetime.Token);
        _modelLoad = operation;
        operation.CancelAfter(TimeSpan.FromSeconds(30));
        var provider = _selectedProvider;
        var endpoint = _baseUrl.Text.TrimEnd('/');
        bool IsCurrent() => ReferenceEquals(_modelLoad, operation) && ReferenceEquals(provider, _selectedProvider) && !operation.IsCancellationRequested;
        try
        {
            if (_captureProtectionAvailable != true) return;
            if (string.IsNullOrWhiteSpace(endpoint)) { _modelStatus.Text = LocalizationService.T("请填写 API 地址。", "Enter an API endpoint."); return; }
            var endpointUri = ProviderEndpointPolicy.NormalizeBaseUri(endpoint);
            ValidateSensitiveHeaderAvailability(provider);
            var headers = ParseHeaders();
            var key = !string.IsNullOrWhiteSpace(_apiKey.Password) ? _apiKey.Password : _apiKeysMarkedForDeletion.Contains(provider.Id) ? null : new CredentialService().Read(provider.CredentialId);
            if (string.IsNullOrWhiteSpace(key) && !headers.Keys.Any(ProviderHeaderCredentialService.IsAuthentication) && !endpointUri.IsLoopback)
            { _modelStatus.Text = LocalizationService.T("输入此提供商的 API Key 后自动加载模型。", "Enter this provider's API key to load models automatically."); return; }
            _modelStatus.Text = LocalizationService.T("正在加载模型…", "Loading models…");
            var catalog = new ProviderModelCatalogService();
            var models = await catalog.GetModelsAsync(endpoint, key ?? "", headers, operation.Token);
            if (!IsCurrent()) return;
            var correctedEndpoint = catalog.LastSuccessfulBaseUrl?.TrimEnd('/');
            var endpointWasCorrected = !string.IsNullOrWhiteSpace(correctedEndpoint) &&
                !string.Equals(correctedEndpoint, endpoint, StringComparison.OrdinalIgnoreCase);
            if (endpointWasCorrected)
            {
                // Make the bounded correction visible in the editable draft so
                // the next send uses the working endpoint after the user saves.
                _loadingProvider = true;
                try { _baseUrl.Text = correctedEndpoint!; }
                finally { _loadingProvider = false; }
                CaptureApiDraft();
                _modelLoadDebounce.Stop();
            }
            var currentModel = _model.Text;
            PopulateModelSuggestions(currentModel, models);
            _modelStatus.Text = models.Count == 0
                ? LocalizationService.T("未返回对话模型，可手动输入模型 ID。", "No chat models returned. Enter a model ID manually.")
                : endpointWasCorrected
                    ? LocalizationService.T($"地址已自动修正为 {correctedEndpoint}，加载了 {models.Count} 个对话模型；保存后生效。", $"The endpoint was corrected to {correctedEndpoint}; loaded {models.Count} chat models. Save to apply it.")
                    : LocalizationService.T($"已从服务商加载 {models.Count} 个对话模型；可选择或手动输入。", $"Loaded {models.Count} chat models from the service. Choose one or enter an ID.");
        }
        catch (OperationCanceledException) { if (ReferenceEquals(_modelLoad, operation) && !_windowLifetime.IsCancellationRequested) _modelStatus.Text = LocalizationService.T("加载已取消或超时，可刷新重试或手动输入模型 ID。", "Loading canceled or timed out. Retry or enter a model ID."); }
        catch (InvalidDataException) when (ReferenceEquals(_modelLoad, operation) && !operation.IsCancellationRequested)
        {
            _modelStatus.Text = LocalizationService.T(
                "API 地址或模型列表格式无效；已尝试标准 /v1 和 /api/v1 后缀。请检查地址，或手动输入模型 ID。",
                "The API endpoint or model list format is invalid. Standard /v1 and /api/v1 suffixes were tried. Check the endpoint or enter a model ID manually.");
        }
        catch (Exception ex) when (ex is InvalidOperationException or JsonException or HttpRequestException or IOException)
        { if (IsCurrent()) _modelStatus.Text = ex is InvalidOperationException ? ex.Message : LocalizationService.T("加载模型失败，请检查地址、密钥和网络；可手动输入模型 ID。", "Could not load models. Check the endpoint, key and network, or enter a model ID."); }
        finally { if (ReferenceEquals(_modelLoad, operation)) _modelLoad = null; }
    }

    private bool StoreSelectedProvider(bool showValidationError = false)
    {
        CaptureApiDraft();
        return _selectedProvider is null || ApplyApiDraft(_selectedProvider, showValidationError);
    }

    private bool ApplyApiDraft(AiProviderSettings provider, bool showValidationError)
    {
        if (!_providerDrafts.TryGetValue(provider, out var draft)) return true;
        try { draft.ApplyTo(provider); return true; }
        catch (Exception ex) when (ex is JsonException or InvalidOperationException)
        {
            if (showValidationError)
            {
                SelectProvider(provider);
                RefreshProviderList();
                _apiAdvanced.IsExpanded = true;
                _connectionStatus.Foreground = Brushes.Firebrick;
                _connectionStatus.Text = LocalizationService.T("高级设置中有未完成或无效的 JSON，请检查后重试。", "Advanced settings contain incomplete or invalid JSON. Check them and retry.");
                _connectionStatus.ToolTip = ex is JsonException
                    ? LocalizationService.T("请检查引号、逗号和大括号。", "Check quotes, commas and braces.")
                    : ex.Message;
            }
            return false;
        }
    }

    private async Task TestConnectionAsync(Button button)
    {
        if (!StoreSelectedProvider(true) || _selectedProvider is not { } existing) return;
        InvalidateConnectionTest();
        button.IsEnabled = false;
        using var test = CancellationTokenSource.CreateLinkedTokenSource(_windowLifetime.Token);
        test.CancelAfter(TimeSpan.FromSeconds(25));
        _connectionTest = test;
        bool IsCurrent() => ReferenceEquals(_connectionTest, test) && ReferenceEquals(_selectedProvider, existing) &&
            !_windowLifetime.IsCancellationRequested;
        try
        {
            _connectionStatus.Foreground = SecondaryBrush;
            _connectionStatus.Text = LocalizationService.T("正在测试连接…", "Testing connection…");
            if (_captureProtectionAvailable == false) throw new InvalidOperationException("设置窗口初始化失败，请重新打开设置后测试。");
            ValidateSensitiveHeaderAvailability(existing);
            var key = !string.IsNullOrWhiteSpace(_apiKey.Password) ? _apiKey.Password :
                _apiKeysMarkedForDeletion.Contains(existing.Id) ? null : new CredentialService().Read(existing.CredentialId);
            var settings = CloneProvider(existing);
            ValidateProvider(settings);
            ProviderAuthenticationPolicy.EnsureUsableCredentials(settings, key);
            key ??= string.Empty;
            IAiProvider provider = AiProviderFactory.CreateConfigured(settings, key);
            var ok = await provider.TestConnectionAsync(test.Token);
            if (IsCurrent() && !test.IsCancellationRequested)
            {
                _connectionStatus.Text = ok ? LocalizationService.T("● 连接正常", "● Connected") :
                    LocalizationService.T("连接失败，请检查配置。", "Connection failed. Check the configuration.");
                _connectionStatus.Foreground = ok ? Brushes.SeaGreen : Brushes.Firebrick;
            }
        }
        catch (OperationCanceledException) when (test.IsCancellationRequested)
        {
            if (IsCurrent())
            {
                _connectionStatus.Text = LocalizationService.T("测试超时，请检查网络或重试。", "Test timed out. Check the network or retry.");
                _connectionStatus.Foreground = Brushes.Firebrick;
            }
        }
        catch (Exception ex)
        {
            if (IsCurrent()) { _connectionStatus.Text = LocalizationService.TranslateUiText(ex.Message); _connectionStatus.Foreground = Brushes.Firebrick; }
        }
        finally
        {
            if (ReferenceEquals(_connectionTest, test)) { _connectionTest = null; button.IsEnabled = true; }
        }
    }

    private Dictionary<string, string> ParseHeaders()
    {
        var headers=JsonSerializer.Deserialize<Dictionary<string, string>>(string.IsNullOrWhiteSpace(_customHeaders.Text) ? "{}" : _customHeaders.Text) ?? [];
        ProviderHeaderPolicy.EnsureValid(headers);
        return headers;
    }

    private void Save()
    {
        CaptureApiDraft();
        foreach (var provider in _providers)
            if (!ApplyApiDraft(provider, true)) { _aiTab.IsSelected = true; _backendSelector.Tabs.SelectedIndex = AiSettingsTabs.ApiIndex; return; }
        // Visiting a settings tab configures that channel; saving another tab
        // must not silently disable an already configured route.  The screen
        // assistant chooses between all usable channels at send time.
        var hermesEnabled=_host.Settings.HermesEnabled||HermesSelected;
        var codexEnabled=_host.Settings.CodexEnabled||_backendSelector.SelectedBackendIndex==AiSettingsTabs.CodexIndex;
        var workBuddyEnabled=_host.Settings.WorkBuddyEnabled||_backendSelector.SelectedBackendIndex==AiSettingsTabs.WorkBuddyIndex;
        var miniMaxCodeEnabled=_host.Settings.MiniMaxCodeEnabled||_backendSelector.SelectedBackendIndex==AiSettingsTabs.MiniMaxCodeIndex;
        if(workBuddyEnabled&&_workBuddySettings.SelectedModel is null)
        {
            MewuDialogWindow.ShowMessage(this,LocalizationService.T("无法保存","Cannot save"),LocalizationService.T("请先在 WorkBuddy 页检测并选择可用模型。","Detect and select an available model on the WorkBuddy page first."));return;
        }
        var unchangedCodex=_host.Settings.CodexEnabled&&_codexSettings.SelectedModel?.Model==_host.Settings.CodexModel&&_codexSettings.SelectedEffort==_host.Settings.CodexReasoningEffort;
        if(codexEnabled&&((!_codexSettings.ConnectionVerified&&!unchangedCodex)||_codexSettings.SelectedModel is null))
        {
            MessageBox.Show(this,"请先在 Codex 页检测连接并选择可用模型。","无法保存");return;
        }
        var hermesAgent=_hermesAgentSelector.SelectedItem as HermesAgentOption;
        var hermesSelection=_hermesModelSelector.SelectedItem as HermesModelOption;
        var hermesReasoning=ReadHermesReasoning();
        if(hermesEnabled&&_hermesInstallation is null)
        {
            MessageBox.Show(this,"未检测到可用的本机 Hermes，请重新检测后再启用。","无法保存");
            return;
        }
        if(hermesEnabled&&(hermesAgent is null||string.IsNullOrWhiteSpace(hermesAgent.Name)))
        {
            MessageBox.Show(this,"请先连接 Hermes 并选择 Agent / 人格。","无法保存");
            return;
        }
        if(hermesEnabled&&(hermesSelection is null||string.IsNullOrWhiteSpace(hermesSelection.Provider)||string.IsNullOrWhiteSpace(hermesSelection.Model)))
        {
            MessageBox.Show(this,"请先连接 Hermes 并选择模型。","无法保存");
            return;
        }
        var modifiers = _capturedHotkeyModifiers;
        var parsed = _capturedHotkeyKey;
        if (parsed != System.Windows.Input.Key.None && modifiers == System.Windows.Input.ModifierKeys.None) { MessageBox.Show(this,"快捷键至少需要 Ctrl、Shift 或 Alt 中的一个修饰键。", "无法保存"); return; }
        foreach (var provider in _providers)
        {
            try { ValidateProvider(provider); ValidateSensitiveHeaderAvailability(provider); }
            catch (InvalidOperationException ex)
            {
                SelectProvider(provider);RefreshProviderList();_apiAdvanced.IsExpanded=true;
                _aiTab.IsSelected=true;_backendSelector.Tabs.SelectedIndex=AiSettingsTabs.ApiIndex;
                _connectionStatus.Text=LocalizationService.TranslateUiText(ex.Message);_connectionStatus.Foreground=Brushes.Firebrick;
                return;
            }
        }
        if(string.IsNullOrWhiteSpace(_defaultProviderId)||_providers.All(provider=>provider.Id!=_defaultProviderId))
        {
            _aiTab.IsSelected=true;_backendSelector.Tabs.SelectedIndex=AiSettingsTabs.ApiIndex;
            MewuDialogWindow.ShowMessage(this,LocalizationService.T("无法保存","Cannot save"),LocalizationService.T("请在连接的“⋯”菜单中选择“设为默认”。","Choose Set as default from a connection’s ⋯ menu."));
            return;
        }

        var credentials = new CredentialService();
        var defaultProvider=_providers.Single(provider=>provider.Id==_defaultProviderId);
        _pendingApiKeys.TryGetValue(defaultProvider.Id,out var pendingDefaultKey);
        var effectiveDefaultKey=!string.IsNullOrWhiteSpace(pendingDefaultKey)
            ?pendingDefaultKey
            :_apiKeysMarkedForDeletion.Contains(defaultProvider.Id)
                ?null
                :credentials.Read(defaultProvider.CredentialId);
        if(!hermesEnabled&&!codexEnabled&&!workBuddyEnabled&&!miniMaxCodeEnabled)
        {
            try{ProviderAuthenticationPolicy.EnsureUsableCredentials(defaultProvider,effectiveDefaultKey);}
            catch(InvalidOperationException ex){MessageBox.Show(this,ex.Message,"默认 Provider 无法使用");return;}
        }
        var previousCredentialIds=_host.Settings.Providers
            .SelectMany(provider=>provider.SensitiveHeaderCredentialIds.Values.Append(provider.CredentialId))
            .Where(id=>!string.IsNullOrWhiteSpace(id)).ToHashSet(StringComparer.OrdinalIgnoreCase);
        var storedProviders=_providers.Select(CloneProvider).ToList();
        var committed=false;string? applyWarning=null;
        try
        {
            foreach (var provider in storedProviders)
            {
                _pendingApiKeys.TryGetValue(provider.Id,out var replacement);
                ProviderApiKeyChangePolicy.Apply(provider,replacement,_apiKeysMarkedForDeletion.Contains(provider.Id),credentials);
            }
            foreach (var provider in storedProviders) _headerCredentials.ProtectEditableHeaders(provider);
            var competing=storedProviders.FirstOrDefault(ProviderApiKeyChangePolicy.HasCompetingAuthentication);
            if(competing is not null)throw new InvalidOperationException($"{competing.Name} 同时配置了 API Key 与认证 Custom Header。请使用“清除已保存密钥”后再保存，避免并发发送两套凭据。");
            var overlayOpacityPercent=ReadNumericChoice(_overlayOpacity,(int)Math.Round(Math.Clamp(_host.Settings.OverlayOpacity,.4,.75)*100));
            var candidate=new AppSettings
            {
                CaptureHotkey=new HotkeySetting{Key=parsed,Modifiers=modifiers},
                LaunchAtStartup=_startup.IsChecked==true,
                UiLanguage=(_uiLanguage.SelectedItem as ComboBoxItem)?.Tag?.ToString()??"system",
                ThinkingGlowEnabled=_thinkingGlowEnabled.IsChecked==true,
                ThinkingGlowColor=_thinkingGlowColor,
                OverlayOpacity=overlayOpacityPercent/100d,
                CaptureDelaySeconds=_delay.SelectedIndex switch{1=>3,2=>5,_=>0},
                DefaultImageFormat=_imageFormat.SelectedIndex==1?"jpg":"png",
                IncludeCaptureCursor=_captureCursor.IsChecked==true,
                TeachingMode=_teachingMode.IsChecked==true,
                RecordingFps=ReadNumericChoice(_recordingFps,30),
                RecordingQuality=ReadNumericChoice(_recordingQuality,75),
                GifFps=ReadNumericChoice(_gifFps,15),
                IncludeRecordingCursor=_recordCursor.IsChecked==true,
                RecordSystemAudio=_recordSystemAudio.IsChecked==true,
                RecordMicrophone=_recordMicrophone.IsChecked==true,
                TempCleanupDays=ReadNumericChoice(_tempCleanup,_host.Settings.TempCleanupDays),
                SaveConversationHistory=_history.IsChecked==true,
                EnableVoiceInput=_voice.IsChecked==true,
                AutomaticallyStartListening=_voice.IsChecked==true&&_autoVoice.IsChecked==true,
                VoiceLanguage=(_voiceLanguage.SelectedItem as ComboBoxItem)?.Tag?.ToString()??"system",
                ConversationChannelId=_host.Settings.ConversationChannelId,
                HermesEnabled=hermesEnabled,
                CodexEnabled=codexEnabled,
                WorkBuddyEnabled=workBuddyEnabled,
                CodexExecutablePath=_host.Settings.CodexExecutablePath,
                WorkBuddyExecutablePath=_host.Settings.WorkBuddyExecutablePath,
                MiniMaxCodeEnabled=miniMaxCodeEnabled,
                MiniMaxCodeModel=_miniMaxCodeSettings.SelectedModel?.Model??_host.Settings.MiniMaxCodeModel,
                WorkBuddyModel=_workBuddySettings.SelectedModel?.Model??_host.Settings.WorkBuddyModel,
                WorkBuddyReasoningEffort=_workBuddySettings.SelectedEffort,
                WorkBuddySupportsImage=_workBuddySettings.SelectedModel?.SupportsImage??_host.Settings.WorkBuddySupportsImage,
                CodexModel=_codexSettings.SelectedModel?.Model??_host.Settings.CodexModel,
                CodexReasoningEffort=_codexSettings.SelectedEffort,
                CodexSupportsImage=_codexSettings.SelectedModel?.SupportsImage??_host.Settings.CodexSupportsImage,
                HermesProfile=hermesAgent?.Name??_host.Settings.HermesProfile,
                HermesProvider=hermesSelection?.Provider??_host.Settings.HermesProvider,
                HermesModel=hermesSelection?.Model??_host.Settings.HermesModel,
                HermesReasoningEffort=hermesReasoning,
                HermesAutoReadAloud=_hermesAutoReadAloud.IsChecked==true,
                Providers=storedProviders,
                DefaultProviderId=_defaultProviderId
            };
            if(!_host.TryApplySettings(candidate,out var error,out var warning))throw new InvalidOperationException(error??"设置保存失败");
            committed=true;applyWarning=warning;
        }
        catch (Exception ex)
        {
            if(!committed)
            {
                var createdIds=storedProviders.SelectMany(provider=>provider.SensitiveHeaderCredentialIds.Values.Append(provider.CredentialId)).Where(id=>!string.IsNullOrWhiteSpace(id));
                foreach(var id in createdIds.Where(id=>!previousCredentialIds.Contains(id)).Distinct(StringComparer.OrdinalIgnoreCase))
                    try{credentials.Delete(id);}catch(Exception cleanupError){try{new PrivacyLogger().Error("CredentialRollback",cleanupError);}catch{}}
                MessageBox.Show(this,$"无法安全保存 Provider 配置：{ex.Message}", "无法保存");return;
            }
            try{new PrivacyLogger().Error("SettingsPostCommit",ex);}catch{}
        }
        if(applyWarning is not null)try{MessageBox.Show(this,applyWarning,"喵呜AI 设置");}catch(Exception ex){try{new PrivacyLogger().Error("SettingsWarning",ex);}catch{}}
        var retainedCredentialIds=storedProviders.SelectMany(provider=>provider.SensitiveHeaderCredentialIds.Values.Append(provider.CredentialId)).Where(id=>!string.IsNullOrWhiteSpace(id)).ToHashSet(StringComparer.OrdinalIgnoreCase);
        foreach(var id in previousCredentialIds.Where(id=>!retainedCredentialIds.Contains(id)))
            try{credentials.Delete(id);}catch(Exception cleanupError){try{new PrivacyLogger().Error("CredentialCleanup",cleanupError);}catch{}}
        _pendingApiKeys.Clear();
        _apiKeysMarkedForDeletion.Clear();
        try{Close();}catch(Exception ex){try{new PrivacyLogger().Error("SettingsClose",ex);}catch{}}
    }

    private static void ValidateProvider(AiProviderSettings provider)
    {
        ProviderRequestParameterPolicy.Validate(provider.RequestParameters);
        if(string.IsNullOrWhiteSpace(provider.Name))throw new InvalidOperationException("Provider 名称不能为空");
        if(string.IsNullOrWhiteSpace(provider.Model))throw new InvalidOperationException($"{provider.Name} 的 Model 不能为空");
        try{_ = ProviderEndpointPolicy.NormalizeBaseUri(provider.BaseUrl);}
        catch(InvalidOperationException ex){throw new InvalidOperationException($"{provider.Name}：{ex.Message}",ex);}
        ProviderHeaderPolicy.EnsureValid(provider.CustomHeaders);
    }

    private void ToggleApiKeyDeletion()
    {
        if(_selectedProvider is null)return;
        if(_apiKeysMarkedForDeletion.Remove(_selectedProvider.Id)){InvalidateConnectionTest();LoadDisplayedApiKey();ScheduleModelLoad();return;}
        var hasSaved=!string.IsNullOrWhiteSpace(_selectedProvider.CredentialId);var hasDraft=!string.IsNullOrWhiteSpace(_apiKey.Password)||_pendingApiKeys.ContainsKey(_selectedProvider.Id);
        if(!hasSaved&&!hasDraft){UpdateApiKeyStatus();return;}
        if(MessageBox.Show(this,"保存设置后将删除此 Provider 的 API Key。Custom Headers 中的独立凭据不会受影响。","清除 API Key",MessageBoxButton.YesNo,MessageBoxImage.Warning)!=MessageBoxResult.Yes)return;
        InvalidateConnectionTest();_apiKeysMarkedForDeletion.Add(_selectedProvider.Id);_pendingApiKeys.Remove(_selectedProvider.Id);LoadDisplayedApiKey();ScheduleModelLoad();
    }

    private void LoadDisplayedApiKey()
    {
        if (_selectedProvider is null) return;
        var wasLoading = _loadingProvider;
        _loadingProvider = true;
        try
        {
            _apiKey.Password = ProviderApiKeyEditorPolicy.ReadForDisplay(_selectedProvider, _pendingApiKeys,
                _apiKeysMarkedForDeletion, _captureProtectionAvailable == true, id => new CredentialService().Read(id));
        }
        finally { _loadingProvider = wasLoading; }
        UpdateApiKeyStatus();
    }

    private void UpdateApiKeyStatus()
    {
        if(_selectedProvider is null){_clearApiKey.IsEnabled=false;_apiKeyStatus.Text="";return;}
        if(_captureProtectionAvailable!=true){_clearApiKey.IsEnabled=false;_apiKeyStatus.Text="屏幕防捕获不可用，API Key 与敏感 Header 已隐藏。";_apiKeyStatus.Foreground=new SolidColorBrush(Color.FromRgb(196,76,88));return;}
        var deleting=_apiKeysMarkedForDeletion.Contains(_selectedProvider.Id);var replacement=_pendingApiKeys.ContainsKey(_selectedProvider.Id);var savedReference=!string.IsNullOrWhiteSpace(_selectedProvider.CredentialId);var saved=savedReference&&!string.IsNullOrWhiteSpace(_apiKey.Password);
        _clearApiKey.Content=deleting?LocalizationService.T("撤销清除","Undo clear"):LocalizationService.T("清除已保存密钥","Clear saved key");_clearApiKey.IsEnabled=deleting||replacement||savedReference;
        _apiKeyStatus.Text=deleting?LocalizationService.T("保存后清除密钥，可在高级设置中撤销。","Key will be removed on save. Undo in Advanced settings."):replacement?LocalizationService.T("密钥已修改，保存后生效。", "Key changed. Save to apply."):saved?LocalizationService.T("已配置密钥。", "Key configured."):savedReference?LocalizationService.T("已保存的密钥无法读取，请重新输入。","Saved key unavailable. Enter it again."):LocalizationService.T("未配置 API Key。", "No API key configured.");
        _apiKeyStatus.Foreground=deleting||savedReference&&!saved?new SolidColorBrush(Color.FromRgb(196,76,88)):SecondaryBrush;
    }

    private void HideSensitiveEditorsAfterCaptureProtectionFailure()
    {
        _customHeaders.Text="屏幕防捕获不可用，Custom Headers 已隐藏。";_customHeaders.IsEnabled=false;
        _apiKey.Clear();_apiKey.IsEnabled=false;_clearApiKey.IsEnabled=false;
        const string warning="系统未能启用设置窗口防捕获；API Key 与敏感 Header 已隐藏，请重启应用后重试。";
        _windowConfigurationWarning.Text=warning;_windowConfigurationWarning.Visibility=Visibility.Visible;
        _aiConfigurationWarning.Text=warning;_aiConfigurationWarning.Visibility=Visibility.Visible;
        UpdateApiKeyStatus();
    }

    private void ValidateSensitiveHeaderAvailability(AiProviderSettings provider)
    {
        if(_hydrationErrors.TryGetValue(provider,out var hydrationError))
            throw new InvalidOperationException(hydrationError);
        if(!_unavailableSensitiveHeaders.TryGetValue(provider,out var unavailable))return;
        var unresolved=unavailable.Where(name=>!provider.CustomHeaders.Keys.Any(current=>current.Equals(name,StringComparison.OrdinalIgnoreCase))).ToList();
        if(unresolved.Count>0)throw new InvalidOperationException($"{provider.Name} 的加密 Header 无法读取（{string.Join("、",unresolved)}）。请重新填写这些 Header，或删除该 Provider 后再保存，原凭据尚未被改动。");
    }

    private bool HasConfigurationWarnings=>
        _host.Settings.ConfigurationErrors.Count>0||
        _editorWarnings.Count>0||
        _repairedProviderIdentityCount>0||
        string.IsNullOrWhiteSpace(_defaultProviderId);

    private void RefreshConfigurationWarnings()
    {
        var messages=_host.Settings.ConfigurationErrors.Distinct(StringComparer.Ordinal).ToList();
        messages.AddRange(_editorWarnings);
        if(_repairedProviderIdentityCount>0)
            messages.Add($"已在编辑副本中修复 {_repairedProviderIdentityCount} 个空白或重复的 Provider ID，保存后才会写入设置。");
        if(string.IsNullOrWhiteSpace(_defaultProviderId))
            messages.Add(LocalizationService.T("请在连接的“⋯”菜单中设定默认连接。","Set a default connection from its ⋯ menu."));
        var headerWarning=messages.Count==0?string.Empty:$"AI 配置需要确认：{messages[0]}";
        _windowConfigurationWarning.Text=headerWarning;
        _windowConfigurationWarning.ToolTip=messages.Count==0?null:string.Join("\n",messages);
        System.Windows.Automation.AutomationProperties.SetHelpText(_windowConfigurationWarning, string.Join("\n",messages));
        _windowConfigurationWarning.Visibility=messages.Count==0?Visibility.Collapsed:Visibility.Visible;
        _aiConfigurationWarning.Text=string.Join("\n",messages.Select(message=>$"• {message}"));
        _aiConfigurationWarning.Visibility=messages.Count==0?Visibility.Collapsed:Visibility.Visible;
    }

    private void ClearTemporaryMedia()
    {
        var blockReason=TempMediaCleanupPolicy.GetBlockReason(_host.IsCaptureActive,TempMediaRegistry.Shared.ActiveLeaseCount);
        if(blockReason is not null)
        {
            MessageBox.Show(this,blockReason,"暂时无法清理");
            return;
        }
        try
        {
            var result=new TempFileService().Cleanup(TimeSpan.Zero,true);
            MessageBox.Show(this,result.SkippedLeasedCount>0?$"已清理未使用的临时媒体；另有 {result.SkippedLeasedCount} 个正在使用的文件已安全保留。":"临时媒体已清理","喵呜AI");
        }
        catch(Exception ex)
        {
            try{new PrivacyLogger().Error("TempMediaCleanup",ex);}catch{}
            MessageBox.Show(this,"临时媒体清理失败，请关闭正在使用这些文件的程序后重试。","无法清理");
        }
    }
}
