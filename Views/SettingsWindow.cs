using System.Net.Http;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media;
using System.Windows.Media.Media3D;
using System.Windows.Shell;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Interop;

namespace mewu_ai_Assistant.Views;

public sealed class SettingsWindow : Window
{
    private static readonly (string Value,string Label)[] HermesReasoningChoices=
    [
        ("none","关闭"),("minimal","极少"),("low","较低"),("medium","中等"),
        ("high","较高"),("xhigh","很高"),("max","最大"),("ultra","极致")
    ];
    private static readonly string[] HermesReasoningValues=HermesReasoningChoices.Select(choice=>choice.Value).ToArray();
    private static readonly Brush PanelBrush = Brushes.White;
    private static readonly Brush ControlBorderBrush = new SolidColorBrush(Color.FromRgb(224, 230, 240));
    private static readonly Brush SecondaryBrush = new SolidColorBrush(Color.FromRgb(99, 112, 137));
    private readonly AppHost _host;
    private readonly ProviderHeaderCredentialService _headerCredentials = new();
    private readonly ComboBox _delay = new(), _imageFormat = new(), _overlayOpacity = new(), _providerSelector = new(), _providerType = new(), _hotkey = new(), _recordingFps = new(), _recordingQuality = new(), _gifFps = new(), _tempCleanup = new(), _voiceLanguage = new(), _hermesAgentSelector = new(), _hermesModelSelector = new(), _hermesReasoning = new(), _model = new();
    private readonly TextBox _providerName = new(), _baseUrl = new(), _customHeaders = new();
    private readonly PasswordBox _apiKey = new();
    private readonly Button _clearApiKey = new();
    private readonly TextBlock _apiKeyStatus = new(), _windowConfigurationWarning = new(), _aiConfigurationWarning = new(), _hermesStatus = new();
    private readonly CheckBox _history = new(), _voice = new(), _autoVoice = new(), _startup = new(), _ctrl = new(), _shift = new(), _alt = new(), _captureCursor = new(), _recordCursor = new(), _defaultProvider = new(), _hermesEnabled = new(), _hermesAutoReadAloud = new();
    private readonly Button _hermesDetect = new(), _hermesTest = new();
    private readonly System.Windows.Shapes.Ellipse _hermesStatusDot = new();
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
    private readonly CancellationTokenSource _windowLifetime=new();
    private HermesInstallation? _hermesInstallation;
    private AiProviderSettings? _selectedProvider;
    private string? _defaultProviderId;
    private readonly int _repairedProviderIdentityCount;
    private bool _loadingProvider;
    private bool _loadingHermes;
    private bool _hermesBusy;
    private bool? _captureProtectionAvailable;

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
        // Keep the short settings pages dense while leaving the AI editor
        // enough room to scroll within its own card.
        Height = 430;
        MinWidth = 600;
        MinHeight = 400;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        WindowStyle = WindowStyle.None;
        AllowsTransparency = false;
        Background = new SolidColorBrush(Color.FromRgb(245,247,252));
        Foreground = new SolidColorBrush(Color.FromRgb(23,32,51));
        UseLayoutRounding = true;
        SnapsToDevicePixels = true;
        TextOptions.SetTextFormattingMode(this, TextFormattingMode.Display);
        WindowChrome.SetWindowChrome(this,new WindowChrome
        {
            CaptionHeight=0,
            ResizeBorderThickness=new Thickness(6),
            GlassFrameThickness=new Thickness(0),
            CornerRadius=new CornerRadius(0),
            UseAeroCaptionButtons=false
        });

        var tabs = new TabControl
        {
            Margin = new Thickness(16, 8, 16, 14),
            TabStripPlacement = Dock.Left
        };
        tabs.Items.Add(Tab("常规", General()));
        tabs.Items.Add(Tab("捕获", Capture()));
        tabs.Items.Add(Tab("录屏", Recording()));
        tabs.Items.Add(Tab("AI", Ai()));
        tabs.Items.Add(Tab("语音", Voice()));
        tabs.Items.Add(Tab("隐私", Privacy()));

        var save = ActionButton("保存", true);
        save.Margin = new Thickness(0, 0, 18, 16);
        save.HorizontalAlignment = HorizontalAlignment.Right;
        save.Click += (_, _) => Save();
        var grid = new Grid { Background = new SolidColorBrush(Color.FromRgb(245,247,252)), ClipToBounds = true };
        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(HasConfigurationWarnings?68:48) });
        grid.RowDefinitions.Add(new RowDefinition());
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        var header = new Grid { Margin = new Thickness(20,0,12,0) };
        header.ColumnDefinitions.Add(new ColumnDefinition());
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.MouseLeftButtonDown += (_,e) => { if(e.ButtonState==System.Windows.Input.MouseButtonState.Pressed&&!IsInsideButton(e.OriginalSource)) DragMove(); };
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
        // Keep the settings chrome aligned with the compact shell used by
        // the quick-question window; a full 42-DIP circle made the close
        // affordance visually heavier than the title bar around it.
        var close=ActionButton(string.Empty);close.Content=CloseIcon();close.ToolTip="关闭设置";System.Windows.Automation.AutomationProperties.SetName(close,"关闭设置窗口");close.Width=34;close.Height=34;close.MinWidth=34;close.MinHeight=34;close.Padding=new Thickness(0);close.SetResourceReference(StyleProperty,"RoundIconButton");close.HorizontalAlignment=HorizontalAlignment.Right;close.VerticalAlignment=VerticalAlignment.Center;Grid.SetColumn(close, 1);close.Click+=(_,_)=>Close();header.Children.Add(close);
        Grid.SetRow(tabs,1);Grid.SetRow(save,2);
        grid.Children.Add(header);grid.Children.Add(tabs);
        grid.Children.Add(save);
        Content = new Border { BorderBrush=ControlBorderBrush, BorderThickness=new Thickness(1), Background=new SolidColorBrush(Color.FromRgb(245,247,252)), Child=grid };
        RefreshConfigurationWarnings();
        SourceInitialized += (_, _) =>
        {
            var handle=new System.Windows.Interop.WindowInteropHelper(this).Handle;
            NativeMethods.TryUseSystemRoundedCorners(handle);
            _captureProtectionAvailable=NativeMethods.ExcludeFromCapture(handle);
            if(_captureProtectionAvailable==true)return;
            try{new PrivacyLogger().Error("SettingsCaptureProtection",new InvalidOperationException("设置窗口无法启用防捕获"));}catch{}
            HideSensitiveEditorsAfterCaptureProtectionFailure();
        };
        Loaded += SettingsWindowLoaded;
        Closed += (_, _) =>
        {
            _windowLifetime.Cancel();
            _connectionTest?.Cancel();
            _hermesConnectionTest?.Cancel();
            _windowLifetime.Dispose();
        };
    }

    private static TabItem Tab(string header, UIElement content) => new()
    {
        Header = header,
        Content = new ScrollViewer
        {
            Content = content,
            Background = PanelBrush,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            Padding = new Thickness(3)
        }
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
        panel.Children.Add(Text("启动与快捷键", true));
        _startup.Content = "登录 Windows 后自动启动";
        _startup.IsChecked = _host.Settings.LaunchAtStartup;
        panel.Children.Add(_startup);
        panel.Children.Add(Text("全局截图快捷键", true));
        var modifiers = new StackPanel { Orientation = Orientation.Horizontal };
        _ctrl.Content = "Ctrl";
        _shift.Content = "Shift";
        _alt.Content = "Alt";
        _ctrl.Margin = new Thickness(0, 5, 18, 5);
        _shift.Margin = new Thickness(0, 5, 18, 5);
        _ctrl.IsChecked = _host.Settings.CaptureHotkey.Modifiers.HasFlag(System.Windows.Input.ModifierKeys.Control);
        _shift.IsChecked = _host.Settings.CaptureHotkey.Modifiers.HasFlag(System.Windows.Input.ModifierKeys.Shift);
        _alt.IsChecked = _host.Settings.CaptureHotkey.Modifiers.HasFlag(System.Windows.Input.ModifierKeys.Alt);
        modifiers.Children.Add(_ctrl);
        modifiers.Children.Add(_shift);
        modifiers.Children.Add(_alt);
        panel.Children.Add(modifiers);
        foreach (var key in Enumerable.Range('A', 26).Select(x => ((char)x).ToString())) _hotkey.Items.Add(key);
        _hotkey.SelectedItem = _host.Settings.CaptureHotkey.Key.ToString();
        System.Windows.Automation.AutomationProperties.SetName(_hotkey, "全局截图快捷键按键");
        panel.Children.Add(_hotkey);
        var restore = ActionButton("恢复默认 Ctrl + Shift + A");
        restore.Click += (_, _) => { _ctrl.IsChecked = true; _shift.IsChecked = true; _alt.IsChecked = false; _hotkey.SelectedItem = "A"; };
        panel.Children.Add(restore);
        panel.Children.Add(Text("关闭主窗口不会退出；请使用托盘菜单退出。", true));
        return panel;
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
        panel.Children.Add(Text("自动清理临时媒体", true));
        AddNumericChoices(_tempCleanup,SettingsChoicePolicy.IncludeCurrent(new[] { 1, 3, 7, 14, 30 },_host.Settings.TempCleanupDays),_host.Settings.TempCleanupDays,"天");
        System.Windows.Automation.AutomationProperties.SetName(_tempCleanup, "临时媒体保留天数");
        panel.Children.Add(_tempCleanup);
        panel.Children.Add(Text("未保存的录制暂存在本机，并由应用自动清理。", true));
        return panel;
    }

    private UIElement Ai()
    {
        var panel = Panel();
        panel.Children.Add(HermesCard());
        _aiConfigurationWarning.Foreground=new SolidColorBrush(Color.FromRgb(185,93,32));
        _aiConfigurationWarning.Background=new SolidColorBrush(Color.FromRgb(255,247,235));
        _aiConfigurationWarning.Padding=new Thickness(12,9,12,9);
        _aiConfigurationWarning.Margin=new Thickness(0,0,0,12);
        _aiConfigurationWarning.TextWrapping=TextWrapping.Wrap;
        panel.Children.Add(_aiConfigurationWarning);
        panel.Children.Add(Text("API Key 与敏感 Custom Header 使用 Windows DPAPI 加密，仅保存在本机当前用户目录。", true));
        panel.Children.Add(Text("Provider 配置", true));
        var providerRow = new Grid();
        providerRow.ColumnDefinitions.Add(new ColumnDefinition());
        providerRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        providerRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        _providerSelector.DisplayMemberPath = nameof(AiProviderSettings.Name);
        System.Windows.Automation.AutomationProperties.SetName(_providerSelector, "Provider 配置");
        _providerSelector.Margin = new Thickness(0, 0, 8, 8);
        foreach (var configured in _providers) _providerSelector.Items.Add(configured);
        var add = ActionButton("新增");
        add.Margin = new Thickness(0, 0, 8, 8);
        add.Click += (_, _) => AddProvider();
        var remove = ActionButton("删除");
        remove.Margin = new Thickness(0, 0, 0, 8);
        remove.Click += (_, _) => RemoveProvider();
        Grid.SetColumn(add, 1);
        Grid.SetColumn(remove, 2);
        providerRow.Children.Add(_providerSelector);
        providerRow.Children.Add(add);
        providerRow.Children.Add(remove);
        panel.Children.Add(providerRow);
        panel.Children.Add(Labeled("Provider 名称", _providerName));
        _defaultProvider.Content = "设为默认 Provider";
        _defaultProvider.Checked+=(_,_)=>
        {
            if(_loadingProvider||_selectedProvider is null)return;
            _defaultProviderId=_selectedProvider.Id;
            RefreshConfigurationWarnings();
        };
        _defaultProvider.Unchecked+=(_,_)=>
        {
            if(_loadingProvider||_selectedProvider is null||_defaultProviderId!=_selectedProvider.Id)return;
            _defaultProviderId=null;
            RefreshConfigurationWarnings();
        };
        panel.Children.Add(_defaultProvider);
        _providerType.SelectedValuePath = "Tag";
        _providerType.Items.Add(new ComboBoxItem { Content = "OpenAI 兼容", Tag = "OpenAICompatible" });
        _providerType.Items.Add(new ComboBoxItem { Content = "MiniMax M3", Tag = "MiniMax" });
        panel.Children.Add(Labeled("Provider 类型", _providerType));
        panel.Children.Add(Labeled("Base URL", _baseUrl));
        panel.Children.Add(Text("Model",true));var modelRow=new Grid{Margin=new Thickness(0,0,0,9)};modelRow.ColumnDefinitions.Add(new ColumnDefinition());modelRow.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});_model.IsEditable=true;_model.IsTextSearchEnabled=true;System.Windows.Automation.AutomationProperties.SetName(_model,"Model");var refreshModels=ActionButton("获取火山模型");refreshModels.Margin=new Thickness(8,0,0,0);refreshModels.Click+=async (_,_)=>await RefreshVolcengineModelsAsync(refreshModels);modelRow.Children.Add(_model);Grid.SetColumn(refreshModels,1);modelRow.Children.Add(refreshModels);panel.Children.Add(modelRow);
        panel.Children.Add(Text("API Key（留空则保留现有密钥）", true));
        var apiKeyRow=new Grid{Margin=new Thickness(0,0,0,4)};apiKeyRow.ColumnDefinitions.Add(new ColumnDefinition());apiKeyRow.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});
        _clearApiKey.Content="清除已保存密钥";_clearApiKey.Padding=new Thickness(14,9,14,9);_clearApiKey.Margin=new Thickness(8,0,0,0);_clearApiKey.SetResourceReference(StyleProperty,"SecondaryButton");_clearApiKey.Click+=(_,_)=>ToggleApiKeyDeletion();
        System.Windows.Automation.AutomationProperties.SetName(_apiKey,"API Key");
        apiKeyRow.Children.Add(_apiKey);Grid.SetColumn(_clearApiKey,1);apiKeyRow.Children.Add(_clearApiKey);panel.Children.Add(apiKeyRow);
        _apiKeyStatus.Foreground=SecondaryBrush;_apiKeyStatus.FontSize=11;_apiKeyStatus.Margin=new Thickness(0,0,0,12);_apiKeyStatus.TextWrapping=TextWrapping.Wrap;panel.Children.Add(_apiKeyStatus);
        _apiKey.PasswordChanged+=(_,_)=>{if(_loadingProvider)return;if(_apiKey.Password.Length>0&&_selectedProvider is not null)_apiKeysMarkedForDeletion.Remove(_selectedProvider.Id);UpdateApiKeyStatus();};
        _customHeaders.AcceptsReturn = true;
        _customHeaders.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
        _customHeaders.HorizontalScrollBarVisibility = ScrollBarVisibility.Auto;
        _customHeaders.TextWrapping = TextWrapping.NoWrap;
        _customHeaders.MinHeight = 80;
        _customHeaders.FontFamily = new FontFamily("Cascadia Mono, Consolas");
        panel.Children.Add(Labeled("Custom Headers JSON（高级，敏感值保存时自动加密）", _customHeaders));
        var test = ActionButton("测试连接");
        test.Click += async (_, _) => await TestConnectionAsync(test);
        panel.Children.Add(test);
        _providerSelector.SelectionChanged += ProviderSelectionChanged;
        var initial = _defaultProviderId is null?_providers[0]:_providers.FirstOrDefault(x => x.Id == _defaultProviderId) ?? _providers[0];
        _providerSelector.SelectedItem = initial;
        return panel;
    }

    private UIElement HermesCard()
    {
        _loadingHermes=true;
        _hermesEnabled.Content="使用本机 Hermes";
        _hermesEnabled.IsChecked=_host.Settings.HermesEnabled;
        _hermesEnabled.FontWeight=FontWeights.SemiBold;
        System.Windows.Automation.AutomationProperties.SetName(_hermesEnabled,"使用本机 Hermes");

        var badge=new Border
        {
            Width=34,Height=34,CornerRadius=new CornerRadius(11),
            Background=new SolidColorBrush(Color.FromRgb(228,243,255)),
            BorderBrush=new SolidColorBrush(Color.FromRgb(199,226,248)),BorderThickness=new Thickness(1),
            Child=new TextBlock{Text="H",FontSize=16,FontWeight=FontWeights.Bold,Foreground=new SolidColorBrush(Color.FromRgb(31,126,200)),HorizontalAlignment=HorizontalAlignment.Center,VerticalAlignment=VerticalAlignment.Center}
        };
        var heading=new StackPanel{Margin=new Thickness(10,0,0,0),VerticalAlignment=VerticalAlignment.Center};
        heading.Children.Add(new TextBlock{Text="本机 Hermes",FontSize=14.5,FontWeight=FontWeights.SemiBold});
        heading.Children.Add(new TextBlock{Text="普通对话与屏幕对话",FontSize=11,Foreground=SecondaryBrush,Margin=new Thickness(0,2,0,0)});
        var header=new Grid();
        header.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});
        header.ColumnDefinitions.Add(new ColumnDefinition());
        header.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});
        header.Children.Add(badge);Grid.SetColumn(heading,1);header.Children.Add(heading);Grid.SetColumn(_hermesEnabled,2);header.Children.Add(_hermesEnabled);

        _hermesStatusDot.Width=8;_hermesStatusDot.Height=8;_hermesStatusDot.Margin=new Thickness(0,0,8,0);_hermesStatusDot.VerticalAlignment=VerticalAlignment.Center;
        _hermesStatus.FontSize=11.5;_hermesStatus.Foreground=SecondaryBrush;_hermesStatus.TextWrapping=TextWrapping.NoWrap;_hermesStatus.TextTrimming=TextTrimming.CharacterEllipsis;_hermesStatus.VerticalAlignment=VerticalAlignment.Center;
        var statusContent=new Grid{VerticalAlignment=VerticalAlignment.Center,Margin=new Thickness(0,0,12,0)};
        statusContent.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});statusContent.ColumnDefinitions.Add(new ColumnDefinition());
        statusContent.Children.Add(_hermesStatusDot);Grid.SetColumn(_hermesStatus,1);statusContent.Children.Add(_hermesStatus);

        ConfigureHermesActionButton(_hermesDetect,"重新检测");
        ConfigureHermesActionButton(_hermesTest,"连接测试");
        _hermesTest.Margin=new Thickness(8,0,0,0);
        var actions=new StackPanel{Orientation=Orientation.Horizontal,HorizontalAlignment=HorizontalAlignment.Right};
        actions.Children.Add(_hermesDetect);actions.Children.Add(_hermesTest);
        var statusRow=new Grid{Margin=new Thickness(1,13,0,12)};
        statusRow.ColumnDefinitions.Add(new ColumnDefinition());statusRow.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});
        statusRow.Children.Add(statusContent);Grid.SetColumn(actions,1);statusRow.Children.Add(actions);

        _hermesAgentSelector.DisplayMemberPath=nameof(HermesAgentOption.Label);
        _hermesAgentSelector.MinWidth=240;
        System.Windows.Automation.AutomationProperties.SetName(_hermesAgentSelector,"Hermes Agent / 人格");
        EnsureStoredHermesAgentItem();

        _hermesModelSelector.DisplayMemberPath=nameof(HermesModelOption.DisplayName);
        _hermesModelSelector.MinWidth=240;
        System.Windows.Automation.AutomationProperties.SetName(_hermesModelSelector,"Hermes 模型");
        System.Windows.Automation.AutomationProperties.SetName(_hermesReasoning,"Hermes 思考程度");
        EnsureStoredHermesModelItem();
        PopulateHermesReasoningChoices(_host.Settings.HermesReasoningEffort);
        var agentField=new StackPanel{Margin=new Thickness(0,0,0,10)};
        agentField.Children.Add(HermesFieldLabel("Agent / 人格"));agentField.Children.Add(_hermesAgentSelector);
        var selectors=new Grid{Margin=new Thickness(0,0,0,7)};
        selectors.ColumnDefinitions.Add(new ColumnDefinition{Width=new GridLength(2,GridUnitType.Star)});
        selectors.ColumnDefinitions.Add(new ColumnDefinition{Width=new GridLength(12)});
        selectors.ColumnDefinitions.Add(new ColumnDefinition{Width=new GridLength(1,GridUnitType.Star)});
        var modelField=new StackPanel();modelField.Children.Add(HermesFieldLabel("模型"));modelField.Children.Add(_hermesModelSelector);
        var reasoningField=new StackPanel();reasoningField.Children.Add(HermesFieldLabel("思考程度"));reasoningField.Children.Add(_hermesReasoning);
        selectors.Children.Add(modelField);Grid.SetColumn(reasoningField,2);selectors.Children.Add(reasoningField);

        _hermesAutoReadAloud.Content="回复后自动朗读";
        _hermesAutoReadAloud.IsChecked=_host.Settings.HermesAutoReadAloud;
        _hermesAutoReadAloud.Margin=new Thickness(0,5,0,0);
        System.Windows.Automation.AutomationProperties.SetName(_hermesAutoReadAloud,"Hermes 回复后自动朗读");

        var body=new StackPanel();body.Children.Add(header);body.Children.Add(statusRow);body.Children.Add(agentField);body.Children.Add(selectors);body.Children.Add(_hermesAutoReadAloud);
        var card=new Border
        {
            CornerRadius=new CornerRadius(15),Padding=new Thickness(16),Margin=new Thickness(0,0,0,14),
            Background=new SolidColorBrush(Color.FromRgb(248,251,255)),
            BorderBrush=new SolidColorBrush(Color.FromRgb(217,230,244)),BorderThickness=new Thickness(1),
            Child=body
        };

        _hermesEnabled.Checked+=HermesEnabledChanged;
        _hermesEnabled.Unchecked+=HermesEnabledChanged;
        _hermesAgentSelector.SelectionChanged+=async (_,_)=>
        {
            if(_loadingHermes||!IsLoaded||_hermesEnabled.IsChecked!=true)return;
            _hermesModelSelector.Items.Clear();
            await ConnectHermesAsync(false);
        };
        _hermesModelSelector.SelectionChanged+=(_,_)=>{if(!_loadingHermes)PopulateHermesReasoningChoices(ReadHermesReasoning());};
        _hermesDetect.Click+=async (_,_)=>
        {
            DetectHermes();
            if(_hermesEnabled.IsChecked==true&&_hermesInstallation is not null)await ConnectHermesAsync(true);
        };
        _hermesTest.Click+=async (_,_)=>await ConnectHermesAsync(true);
        _loadingHermes=false;
        DetectHermes();
        UpdateHermesControls();
        return card;
    }

    private static TextBlock HermesFieldLabel(string text)=>new()
    {
        Text=text,FontSize=11,Foreground=SecondaryBrush,Margin=new Thickness(1,0,0,6)
    };

    private static void ConfigureHermesActionButton(Button button,string text)
    {
        button.Content=text;button.Padding=new Thickness(13,7,13,7);button.Cursor=System.Windows.Input.Cursors.Hand;
        button.SetResourceReference(StyleProperty,"SecondaryButton");
        System.Windows.Automation.AutomationProperties.SetName(button,text);
    }

    private async void SettingsWindowLoaded(object sender,RoutedEventArgs e)
    {
        Loaded-=SettingsWindowLoaded;
        DetectHermes();
        if(_hermesEnabled.IsChecked==true&&_hermesInstallation is not null)
            await ConnectHermesAsync(false);
    }

    private void HermesEnabledChanged(object sender,RoutedEventArgs e)
    {
        if(_loadingHermes)return;
        DetectHermes();
        UpdateHermesControls();
        if(IsLoaded&&_hermesEnabled.IsChecked==true&&_hermesInstallation is not null)
            _=ConnectHermesAsync(false);
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
            _hermesReasoning.Items.Add(new ComboBoxItem{Content=choice.Label,Tag=choice.Value});
        _hermesReasoning.SelectedItem=_hermesReasoning.Items.OfType<ComboBoxItem>().FirstOrDefault(item=>string.Equals(item.Tag?.ToString(),normalized,StringComparison.Ordinal))
            ??_hermesReasoning.Items.OfType<ComboBoxItem>().FirstOrDefault();
    }

    private string ReadHermesReasoning()=>
        (_hermesReasoning.SelectedItem as ComboBoxItem)?.Tag?.ToString()??_host.Settings.HermesReasoningEffort;

    private void UpdateHermesControls()
    {
        var enabled=_hermesEnabled.IsChecked==true;
        _hermesDetect.IsEnabled=!_hermesBusy;
        _hermesTest.IsEnabled=!_hermesBusy&&_hermesInstallation is not null;
        _hermesAgentSelector.IsEnabled=enabled&&!_hermesBusy&&_hermesAgentSelector.Items.Count>0;
        _hermesModelSelector.IsEnabled=enabled&&!_hermesBusy&&_hermesModelSelector.Items.Count>0;
        _hermesReasoning.IsEnabled=enabled&&!_hermesBusy&&_hermesReasoning.Items.Count>0;
        _hermesAutoReadAloud.IsEnabled=enabled;
    }

    private void SetHermesStatus(string message,HermesStatusTone tone)
    {
        _hermesStatus.Text=message;_hermesStatus.ToolTip=message;
        var color=tone switch
        {
            HermesStatusTone.Connected=>Color.FromRgb(34,157,105),
            HermesStatusTone.Ready=>Color.FromRgb(44,137,204),
            HermesStatusTone.Working=>Color.FromRgb(221,143,50),
            _=>Color.FromRgb(201,78,91)
        };
        var brush=new SolidColorBrush(color);_hermesStatusDot.Fill=brush;_hermesStatus.Foreground=brush;
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
        panel.Children.Add(Text("媒体默认不永久保存；截图只有明确点击发送后才会上传。", true));
        var clearHistory = ActionButton("清空本地对话历史");
        clearHistory.Margin = new Thickness(0, 8, 0, 0);
        clearHistory.Click += async (_, _) =>
        {
            clearHistory.IsEnabled=false;
            try
            {
                await new ConversationHistoryService().ClearAsync(_windowLifetime.Token);
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
        panel.Children.Add(Text($"应用版本：{typeof(SettingsWindow).Assembly.GetName().Version}\n.NET：{Environment.Version}\nWindows：{Environment.OSVersion.Version}\n捕获：GDI desktop snapshot / PP-OCRv6（Windows OCR 仅故障降级）\n录屏：Media Foundation H.264", true));
        return panel;
    }

    private static AiProviderSettings CloneProvider(AiProviderSettings source) => ProviderHeaderCredentialService.Clone(source);

    private void SelectProvider(AiProviderSettings? provider)
    {
        if (_loadingProvider || provider is null) return;
        _selectedProvider = provider;
        _loadingProvider = true;
        _providerName.Text = provider.Name;
        _providerType.SelectedValue = provider.Type.Equals("MiniMax", StringComparison.OrdinalIgnoreCase) ? "MiniMax" : "OpenAICompatible";
        _baseUrl.Text = provider.BaseUrl;
        _model.Text = provider.Model;
        PopulateModelSuggestions(provider.BaseUrl,provider.Model);
        _customHeaders.Text = _captureProtectionAvailable==false
            ?"屏幕防捕获不可用，Custom Headers 已隐藏。"
            :provider.CustomHeaders.Count == 0 ? "{}" : JsonSerializer.Serialize(provider.CustomHeaders, new JsonSerializerOptions { WriteIndented = true });
        _apiKey.Password = _pendingApiKeys.GetValueOrDefault(provider.Id) ?? string.Empty;
        _defaultProvider.IsChecked = provider.Id == _defaultProviderId;
        _loadingProvider = false;
        UpdateApiKeyStatus();
    }

    private void PopulateModelSuggestions(string baseUrl,string currentModel,IEnumerable<string>? liveModels=null)
    {
        _model.Items.Clear();var models=new List<string>();
        try{if(VolcengineModelPolicy.IsEndpoint(ProviderEndpointPolicy.NormalizeBaseUri(baseUrl)))models.AddRange(liveModels??VolcengineModelPolicy.RecommendedModels);}catch(InvalidOperationException){}
        if(!string.IsNullOrWhiteSpace(currentModel)&&!models.Contains(currentModel,StringComparer.OrdinalIgnoreCase))models.Insert(0,currentModel);
        foreach(var model in models.Distinct(StringComparer.OrdinalIgnoreCase))_model.Items.Add(model);
        _model.Text=currentModel;
    }

    private async Task RefreshVolcengineModelsAsync(Button button)
    {
        if(_selectedProvider is null||_loadingProvider)return;button.IsEnabled=false;
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(_windowLifetime.Token);timeout.CancelAfter(TimeSpan.FromSeconds(30));
        try
        {
            if(_captureProtectionAvailable==false)throw new InvalidOperationException("系统未能启用设置窗口防捕获，不能读取敏感凭据");
            if(!StoreSelectedProvider(true))return;
            var key=!string.IsNullOrWhiteSpace(_apiKey.Password)?_apiKey.Password:_apiKeysMarkedForDeletion.Contains(_selectedProvider.Id)?null:new CredentialService().Read(_selectedProvider.CredentialId);
            var models=await new VolcengineModelCatalogService().GetChatModelsAsync(_baseUrl.Text.TrimEnd('/'),key??string.Empty,ParseHeaders(),timeout.Token);
            if(models.Count==0)throw new InvalidOperationException("当前账户没有返回可用的对话模型");
            PopulateModelSuggestions(_baseUrl.Text,_model.Text,models);button.Content=$"已获取 {models.Count} 个";
        }
        catch(OperationCanceledException)when(_windowLifetime.IsCancellationRequested){}
        catch(OperationCanceledException){if(IsVisible)MessageBox.Show(this,"获取模型列表超时，请检查网络后重试。","火山模型列表");}
        catch(Exception ex)when(ex is InvalidOperationException or JsonException or HttpRequestException or IOException){if(IsVisible)MessageBox.Show(this,ex.Message,"火山模型列表");}
        finally{if(IsVisible)button.IsEnabled=true;}
    }

    private bool StoreSelectedProvider(bool showValidationError=false)
    {
        if (_selectedProvider is null || _loadingProvider) return true;
        Dictionary<string,string> headers;
        try{headers=_captureProtectionAvailable==false?_selectedProvider.CustomHeaders:ParseHeaders();}
        catch(Exception ex)when(ex is JsonException or InvalidOperationException)
        {
            if(showValidationError)MessageBox.Show(this,$"Custom Headers 无效：{ex.Message}","Provider 配置无效");
            return false;
        }
        _selectedProvider.Name = string.IsNullOrWhiteSpace(_providerName.Text) ? "未命名 Provider" : _providerName.Text.Trim();
        _selectedProvider.Type = IsMiniMaxTypeSelected() ? "MiniMax" : "OpenAICompatible";
        _selectedProvider.BaseUrl = _baseUrl.Text.TrimEnd('/');
        _selectedProvider.Model = _model.Text.Trim();
        if (_selectedProvider.Type == "MiniMax" && _selectedProvider.BaseUrl.Contains("api.openai.com", StringComparison.OrdinalIgnoreCase)) _selectedProvider.BaseUrl = "https://api.minimaxi.com/v1";
        if (_selectedProvider.Type == "MiniMax" && _selectedProvider.Model.StartsWith("gpt-", StringComparison.OrdinalIgnoreCase)) _selectedProvider.Model = "MiniMax-M3";
        _selectedProvider.CustomHeaders=headers;
        if (!string.IsNullOrWhiteSpace(_apiKey.Password)){_pendingApiKeys[_selectedProvider.Id]=_apiKey.Password;_apiKeysMarkedForDeletion.Remove(_selectedProvider.Id);}
        else if(!_apiKeysMarkedForDeletion.Contains(_selectedProvider.Id))_pendingApiKeys.Remove(_selectedProvider.Id);
        if (_defaultProvider.IsChecked == true) _defaultProviderId = _selectedProvider.Id;
        _providerSelector.Items.Refresh();
        return true;
    }

    private void ProviderSelectionChanged(object sender,SelectionChangedEventArgs e)
    {
        if(_loadingProvider)return;
        var next=_providerSelector.SelectedItem as AiProviderSettings;
        if(_selectedProvider is not null&&!ReferenceEquals(next,_selectedProvider)&&!StoreSelectedProvider(true))
        {
            _loadingProvider=true;_providerSelector.SelectedItem=_selectedProvider;_loadingProvider=false;return;
        }
        SelectProvider(next);
    }

    private void AddProvider()
    {
        if(!StoreSelectedProvider(true))return;
        var provider = new AiProviderSettings { Name = $"Provider {_providers.Count + 1}" };
        _providers.Add(provider);
        _providerSelector.Items.Add(provider);
        _providerSelector.SelectedItem = provider;
    }

    private void RemoveProvider()
    {
        if (_selectedProvider is null) return;
        if (_providers.Count == 1)
        {
            MessageBox.Show(this,"至少保留一个 Provider 配置。", "喵呜AI");
            return;
        }
        var removed=_selectedProvider;var index = _providers.IndexOf(removed);
        _loadingProvider=true;
        _providers.Remove(removed);
        _providerSelector.Items.Remove(removed);
        _pendingApiKeys.Remove(removed.Id);
        _apiKeysMarkedForDeletion.Remove(removed.Id);
        _unavailableSensitiveHeaders.Remove(removed);
        _hydrationErrors.Remove(removed);
        var next=_providers[Math.Clamp(index,0,_providers.Count-1)];
        if(_defaultProviderId==removed.Id)_defaultProviderId=null;
        _selectedProvider=null;_providerSelector.SelectedItem=next;_loadingProvider=false;
        SelectProvider(next);
        RefreshConfigurationWarnings();
    }

    private async Task TestConnectionAsync(Button button)
    {
        button.IsEnabled = false;
        _connectionTest?.Cancel();
        using var test=new CancellationTokenSource(TimeSpan.FromSeconds(25));
        _connectionTest=test;
        try
        {
            if(_captureProtectionAvailable==false)throw new InvalidOperationException("系统未能启用设置窗口防捕获，敏感凭据已隐藏。请重启应用后再测试连接。");
            if(!StoreSelectedProvider(true))return;
            var existing = _selectedProvider;
            var key = !string.IsNullOrWhiteSpace(_apiKey.Password) ? _apiKey.Password : existing is null||_apiKeysMarkedForDeletion.Contains(existing.Id) ? null : new CredentialService().Read(existing.CredentialId);
            var settings = new AiProviderSettings { Type = IsMiniMaxTypeSelected() ? "MiniMax" : "OpenAICompatible", BaseUrl = _baseUrl.Text.TrimEnd('/'), Model = _model.Text.Trim(), CustomHeaders = ParseHeaders() };
            ValidateProvider(settings);
            if(!string.IsNullOrWhiteSpace(key)&&settings.CustomHeaders.Keys.Any(ProviderHeaderCredentialService.IsAuthentication))throw new InvalidOperationException("API Key 与认证 Custom Header 不能同时发送。请清除已保存 API Key，或移除认证 Header。");
            if(string.IsNullOrWhiteSpace(key)&&!settings.CustomHeaders.Keys.Any(ProviderHeaderCredentialService.IsAuthentication))throw new InvalidOperationException("请先输入 API Key，或在 Custom Headers 中配置认证字段");
            key??=string.Empty;
            IAiProvider provider = settings.Type == "MiniMax" ? new MiniMaxProvider(settings, key) : new OpenAiCompatibleProvider(settings, key);
            var ok = await provider.TestConnectionAsync(test.Token);
            MessageBox.Show(this,ok ? "连接成功" : "服务返回失败状态", "AI 连接测试");
        }
        catch(OperationCanceledException) when(test.IsCancellationRequested) { if(IsVisible)MessageBox.Show(this,"连接测试已取消或超时，请检查网络与 Provider 地址。", "AI 连接测试"); }
        catch (Exception ex) { if(IsVisible)MessageBox.Show(this,ex.Message, "AI 连接测试失败"); }
        finally { if(ReferenceEquals(_connectionTest,test))_connectionTest=null;if(IsVisible)button.IsEnabled = true; }
    }

    private Dictionary<string, string> ParseHeaders()
    {
        var headers=JsonSerializer.Deserialize<Dictionary<string, string>>(string.IsNullOrWhiteSpace(_customHeaders.Text) ? "{}" : _customHeaders.Text) ?? [];
        ProviderHeaderPolicy.EnsureValid(headers);
        return headers;
    }

    private bool IsMiniMaxTypeSelected() =>
        string.Equals(_providerType.SelectedValue as string, "MiniMax", StringComparison.OrdinalIgnoreCase);

    private void Save()
    {
        if(!StoreSelectedProvider(true))return;
        var hermesEnabled=_hermesEnabled.IsChecked==true;
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
        var modifiers = System.Windows.Input.ModifierKeys.None;
        if (_ctrl.IsChecked == true) modifiers |= System.Windows.Input.ModifierKeys.Control;
        if (_shift.IsChecked == true) modifiers |= System.Windows.Input.ModifierKeys.Shift;
        if (_alt.IsChecked == true) modifiers |= System.Windows.Input.ModifierKeys.Alt;
        if (_hotkey.SelectedItem is not string key || !Enum.TryParse<System.Windows.Input.Key>(key, out var parsed)) { MessageBox.Show(this,"请选择有效的快捷键。", "无法保存"); return; }
        if (modifiers == System.Windows.Input.ModifierKeys.None) { MessageBox.Show(this,"快捷键至少需要 Ctrl、Shift 或 Alt 中的一个修饰键。", "无法保存"); return; }
        try{foreach(var provider in _providers){ValidateProvider(provider);ValidateSensitiveHeaderAvailability(provider);}}
        catch(InvalidOperationException ex){MessageBox.Show(this,ex.Message,"Provider 配置无效");return;}
        if(string.IsNullOrWhiteSpace(_defaultProviderId)||_providers.All(provider=>provider.Id!=_defaultProviderId))
        {
            MessageBox.Show(this,"请在 AI 页选择一个 Provider，并明确勾选“设为默认 Provider”。","无法保存");
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
        if(!hermesEnabled)
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
                OverlayOpacity=overlayOpacityPercent/100d,
                CaptureDelaySeconds=_delay.SelectedIndex switch{1=>3,2=>5,_=>0},
                DefaultImageFormat=_imageFormat.SelectedIndex==1?"jpg":"png",
                IncludeCaptureCursor=_captureCursor.IsChecked==true,
                RecordingFps=ReadNumericChoice(_recordingFps,30),
                RecordingQuality=ReadNumericChoice(_recordingQuality,75),
                GifFps=ReadNumericChoice(_gifFps,15),
                IncludeRecordingCursor=_recordCursor.IsChecked==true,
                TempCleanupDays=ReadNumericChoice(_tempCleanup,_host.Settings.TempCleanupDays),
                SaveConversationHistory=_history.IsChecked==true,
                EnableVoiceInput=_voice.IsChecked==true,
                AutomaticallyStartListening=_voice.IsChecked==true&&_autoVoice.IsChecked==true,
                VoiceLanguage=(_voiceLanguage.SelectedItem as ComboBoxItem)?.Tag?.ToString()??"system",
                HermesEnabled=hermesEnabled,
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
        if(string.IsNullOrWhiteSpace(provider.Name))throw new InvalidOperationException("Provider 名称不能为空");
        if(string.IsNullOrWhiteSpace(provider.Model))throw new InvalidOperationException($"{provider.Name} 的 Model 不能为空");
        try{_ = ProviderEndpointPolicy.NormalizeBaseUri(provider.BaseUrl);}
        catch(InvalidOperationException ex){throw new InvalidOperationException($"{provider.Name}：{ex.Message}",ex);}
        ProviderHeaderPolicy.EnsureValid(provider.CustomHeaders);
    }

    private void ToggleApiKeyDeletion()
    {
        if(_selectedProvider is null)return;
        if(_apiKeysMarkedForDeletion.Remove(_selectedProvider.Id)){UpdateApiKeyStatus();return;}
        var hasSaved=!string.IsNullOrWhiteSpace(_selectedProvider.CredentialId);var hasDraft=!string.IsNullOrWhiteSpace(_apiKey.Password)||_pendingApiKeys.ContainsKey(_selectedProvider.Id);
        if(!hasSaved&&!hasDraft){UpdateApiKeyStatus();return;}
        if(MessageBox.Show(this,"保存设置后将删除此 Provider 的 API Key。Custom Headers 中的独立凭据不会受影响。","清除 API Key",MessageBoxButton.YesNo,MessageBoxImage.Warning)!=MessageBoxResult.Yes)return;
        _apiKeysMarkedForDeletion.Add(_selectedProvider.Id);_pendingApiKeys.Remove(_selectedProvider.Id);_loadingProvider=true;_apiKey.Clear();_loadingProvider=false;UpdateApiKeyStatus();
    }

    private void UpdateApiKeyStatus()
    {
        if(_selectedProvider is null){_clearApiKey.IsEnabled=false;_apiKeyStatus.Text="";return;}
        if(_captureProtectionAvailable==false){_clearApiKey.IsEnabled=false;_apiKeyStatus.Text="屏幕防捕获不可用，API Key 与敏感 Header 已隐藏。";_apiKeyStatus.Foreground=new SolidColorBrush(Color.FromRgb(196,76,88));return;}
        var deleting=_apiKeysMarkedForDeletion.Contains(_selectedProvider.Id);var replacement=!string.IsNullOrWhiteSpace(_apiKey.Password);var savedReference=!string.IsNullOrWhiteSpace(_selectedProvider.CredentialId);var saved=savedReference&&!string.IsNullOrWhiteSpace(new CredentialService().Read(_selectedProvider.CredentialId));
        _clearApiKey.Content=deleting?"撤销清除":"清除已保存密钥";_clearApiKey.IsEnabled=deleting||replacement||savedReference;
        _apiKeyStatus.Text=deleting?"保存后将清除现有 API Key；点击“撤销清除”可保留。":replacement?"新 API Key 将在保存后替换现有密钥。":saved?"已有可用的加密 API Key；输入新值可替换，留空会保留。":savedReference?"已保存的 API Key 无法读取，请输入新值后保存。":"尚未保存 API Key，可改用认证 Custom Header。";
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
            messages.Add("请选择一个 Provider，并明确勾选“设为默认 Provider”。");
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
