using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Effects;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Interop;

namespace mewu_ai_Assistant.Views;

public sealed class SettingsWindow : Window
{
    private static readonly Brush PanelBrush = Brushes.White;
    private static readonly Brush InputBrush = Brushes.White;
    private static readonly Brush ControlBorderBrush = new SolidColorBrush(Color.FromRgb(224, 230, 240));
    private static readonly Brush SecondaryBrush = new SolidColorBrush(Color.FromRgb(99, 112, 137));
    private readonly AppHost _host;
    private readonly ComboBox _delay = new(), _imageFormat = new(), _overlayOpacity = new(), _providerSelector = new(), _providerType = new(), _hotkey = new(), _recordingFps = new(), _recordingQuality = new(), _gifFps = new(), _tempCleanup = new(), _voiceLanguage = new();
    private readonly TextBox _providerName = new(), _baseUrl = new(), _model = new(), _customHeaders = new();
    private readonly PasswordBox _apiKey = new();
    private readonly CheckBox _history = new(), _voice = new(), _autoVoice = new(), _startup = new(), _ctrl = new(), _shift = new(), _alt = new(), _captureCursor = new(), _recordCursor = new(), _defaultProvider = new();
    private readonly List<AiProviderSettings> _providers;
    private readonly Dictionary<string, string> _pendingApiKeys = [];
    private AiProviderSettings? _selectedProvider;
    private string _defaultProviderId;
    private bool _loadingProvider;

    public SettingsWindow(AppHost host)
    {
        _host = host;
        _providers = host.Settings.Providers.Select(CloneProvider).ToList();
        if (_providers.Count == 0) _providers.Add(new AiProviderSettings());
        _defaultProviderId = host.Settings.DefaultProviderId ?? _providers[0].Id;
        Title = "喵呜AI 设置";
        Width = 820;
        Height = 650;
        MinWidth = 640;
        MinHeight = 500;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        WindowStyle = WindowStyle.None;
        AllowsTransparency = true;
        Background = Brushes.Transparent;
        Foreground = new SolidColorBrush(Color.FromRgb(23,32,51));

        var tabs = new TabControl
        {
            Margin = new Thickness(20, 12, 20, 18),
            TabStripPlacement = Dock.Left
        };
        tabs.Items.Add(Tab("常规", General()));
        tabs.Items.Add(Tab("捕获", Capture()));
        tabs.Items.Add(Tab("录屏", Recording()));
        tabs.Items.Add(Tab("AI", Ai()));
        tabs.Items.Add(Tab("语音", Voice()));
        tabs.Items.Add(Tab("隐私", Privacy()));

        var save = ActionButton("保存", true);
        save.Margin = new Thickness(0, 0, 20, 18);
        save.HorizontalAlignment = HorizontalAlignment.Right;
        save.Click += (_, _) => Save();
        var grid = new Grid { Background = new SolidColorBrush(Color.FromRgb(245,247,252)) };
        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(54) });
        grid.RowDefinitions.Add(new RowDefinition());
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        var header = new Grid { Margin = new Thickness(22,0,14,0) };
        header.MouseLeftButtonDown += (_,e) => { if(e.ButtonState==System.Windows.Input.MouseButtonState.Pressed&&e.OriginalSource is not Button) DragMove(); };
        header.Children.Add(new TextBlock { Text="喵呜AI 设置", FontSize=17, FontWeight=FontWeights.SemiBold, VerticalAlignment=VerticalAlignment.Center });
        var close=ActionButton("×");close.Padding=new Thickness(13,7,13,7);close.HorizontalAlignment=HorizontalAlignment.Right;close.VerticalAlignment=VerticalAlignment.Center;close.Click+=(_,_)=>Close();header.Children.Add(close);
        Grid.SetRow(tabs,1);Grid.SetRow(save,2);
        grid.Children.Add(header);grid.Children.Add(tabs);
        grid.Children.Add(save);
        Content = new Border { CornerRadius=new CornerRadius(18), BorderBrush=ControlBorderBrush, BorderThickness=new Thickness(1), Background=new SolidColorBrush(Color.FromRgb(245,247,252)), Child=grid, Effect=new DropShadowEffect{Color=Color.FromRgb(102,117,140),BlurRadius=30,ShadowDepth=9,Opacity=.25} };
        SourceInitialized += (_, _) => NativeMethods.ExcludeFromCapture(new System.Windows.Interop.WindowInteropHelper(this).Handle);
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
            Padding = new Thickness(4)
        }
    };

    private static TextBlock Text(string text, bool secondary = false) => new()
    {
        Text = text,
        Margin = new Thickness(0, 5, 0, 9),
        TextWrapping = TextWrapping.Wrap,
        Foreground = secondary ? SecondaryBrush : new SolidColorBrush(Color.FromRgb(23,32,51))
    };

    private static StackPanel Panel() => new() { Margin = new Thickness(24) };

    private static FrameworkElement Labeled(string name, Control control)
    {
        var panel = new StackPanel();
        panel.Children.Add(Text(name, true));
        control.Margin = new Thickness(0, 0, 0, 12);
        panel.Children.Add(control);
        return panel;
    }

    private static Button ActionButton(string text, bool primary = false)
    {
        var button=new Button{Content=text,Padding=new Thickness(18,9,18,9),Cursor=System.Windows.Input.Cursors.Hand};
        button.SetResourceReference(StyleProperty,primary?"PrimaryButton":"SecondaryButton");
        return button;
    }

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
        panel.Children.Add(_delay);
        panel.Children.Add(Text("默认图片格式", true));
        _imageFormat.Items.Add("PNG");
        _imageFormat.Items.Add("JPEG");
        _imageFormat.SelectedIndex = _host.Settings.DefaultImageFormat.Equals("jpg", StringComparison.OrdinalIgnoreCase) || _host.Settings.DefaultImageFormat.Equals("jpeg", StringComparison.OrdinalIgnoreCase) ? 1 : 0;
        panel.Children.Add(_imageFormat);
        panel.Children.Add(Text("选区外暗化程度", true));
        foreach (var opacity in new[] { 55, 60, 65 }) _overlayOpacity.Items.Add($"{opacity}%");
        _overlayOpacity.SelectedItem = $"{Math.Round(Math.Clamp(_host.Settings.OverlayOpacity,.55,.65)*100):0}%";
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
        foreach (var fps in new[] { 15, 24, 30, 60 }) _recordingFps.Items.Add(fps);
        _recordingFps.SelectedItem = _host.Settings.RecordingFps;
        panel.Children.Add(_recordingFps);
        panel.Children.Add(Text("MP4 质量", true));
        foreach (var quality in new[] { 50, 75, 90 }) _recordingQuality.Items.Add(quality);
        _recordingQuality.SelectedItem = _host.Settings.RecordingQuality;
        panel.Children.Add(_recordingQuality);
        panel.Children.Add(Text("GIF 帧率", true));
        foreach (var fps in new[] { 5, 10, 15 }) _gifFps.Items.Add(fps);
        _gifFps.SelectedItem = _host.Settings.GifFps;
        panel.Children.Add(_gifFps);
        _recordCursor.Content = "录屏包含系统鼠标指针";
        _recordCursor.IsChecked = _host.Settings.IncludeRecordingCursor;
        panel.Children.Add(_recordCursor);
        panel.Children.Add(Text("自动清理临时媒体", true));
        foreach (var days in new[] { 1, 3, 7, 14, 30 }) _tempCleanup.Items.Add($"{days} 天");
        _tempCleanup.SelectedItem = $"{Math.Clamp(_host.Settings.TempCleanupDays,1,30)} 天";
        panel.Children.Add(_tempCleanup);
        panel.Children.Add(Text("未保存的录制暂存在本机，并由应用自动清理。", true));
        return panel;
    }

    private UIElement Ai()
    {
        var panel = Panel();
        panel.Children.Add(Text("API Key 使用 Windows DPAPI 加密，仅保存在本机当前用户目录。", true));
        panel.Children.Add(Text("Provider 配置", true));
        var providerRow = new Grid();
        providerRow.ColumnDefinitions.Add(new ColumnDefinition());
        providerRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        providerRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        _providerSelector.DisplayMemberPath = nameof(AiProviderSettings.Name);
        _providerSelector.Margin = new Thickness(0, 0, 8, 10);
        foreach (var configured in _providers) _providerSelector.Items.Add(configured);
        var add = ActionButton("新增");
        add.Margin = new Thickness(0, 0, 8, 10);
        add.Click += (_, _) => AddProvider();
        var remove = ActionButton("删除");
        remove.Margin = new Thickness(0, 0, 0, 10);
        remove.Click += (_, _) => RemoveProvider();
        Grid.SetColumn(add, 1);
        Grid.SetColumn(remove, 2);
        providerRow.Children.Add(_providerSelector);
        providerRow.Children.Add(add);
        providerRow.Children.Add(remove);
        panel.Children.Add(providerRow);
        panel.Children.Add(Labeled("Provider 名称", _providerName));
        _defaultProvider.Content = "设为默认 Provider";
        panel.Children.Add(_defaultProvider);
        _providerType.Items.Add("OpenAICompatible");
        _providerType.Items.Add("MiniMax");
        panel.Children.Add(Labeled("Provider 类型", _providerType));
        panel.Children.Add(Labeled("Base URL", _baseUrl));
        panel.Children.Add(Labeled("Model", _model));
        panel.Children.Add(Labeled("API Key（留空则保留现有密钥）", _apiKey));
        _customHeaders.AcceptsReturn = true;
        _customHeaders.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
        _customHeaders.MinHeight = 88;
        _customHeaders.FontFamily = new FontFamily("Cascadia Mono, Consolas");
        panel.Children.Add(Labeled("Custom Headers JSON（高级）", _customHeaders));
        var test = ActionButton("测试连接");
        test.Click += async (_, _) => await TestConnectionAsync(test);
        panel.Children.Add(test);
        _providerSelector.SelectionChanged += (_, _) => SelectProvider(_providerSelector.SelectedItem as AiProviderSettings);
        var initial = _providers.FirstOrDefault(x => x.Id == _defaultProviderId) ?? _providers[0];
        _providerSelector.SelectedItem = initial;
        return panel;
    }

    private UIElement Voice()
    {
        var panel = Panel();
        _voice.Content = "启用语音输入";
        _voice.IsChecked = _host.Settings.EnableVoiceInput;
        _autoVoice.Content = "Prompt 出现时自动监听";
        _autoVoice.IsChecked = _host.Settings.AutomaticallyStartListening;
        panel.Children.Add(_voice);
        panel.Children.Add(_autoVoice);
        panel.Children.Add(Text("识别语言", true));
        foreach (var item in new[] { ("跟随 Windows", "system"), ("简体中文", "zh-CN"), ("英语", "en-US") })
            _voiceLanguage.Items.Add(new ComboBoxItem { Content = item.Item1, Tag = item.Item2 });
        _voiceLanguage.SelectedIndex = _host.Settings.VoiceLanguage switch { "zh-CN" => 1, "en-US" => 2, _ => 0 };
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
        clearHistory.Margin = new Thickness(0, 10, 0, 0);
        clearHistory.Click += (_, _) => { new ConversationHistoryService().Clear(); MessageBox.Show("本地对话历史已清空", "喵呜AI"); };
        panel.Children.Add(clearHistory);
        var clear = ActionButton("清理临时媒体");
        clear.Margin = new Thickness(0, 8, 0, 0);
        clear.Click += (_, _) => { new TempFileService().Cleanup(TimeSpan.Zero); MessageBox.Show("临时媒体已清理", "喵呜AI"); };
        panel.Children.Add(clear);
        var open = ActionButton("打开数据目录");
        open.Margin = new Thickness(0, 8, 0, 0);
        open.Click += (_, _) => System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "MewuAI")) { UseShellExecute = true });
        panel.Children.Add(open);
        panel.Children.Add(Text($"应用版本：{typeof(SettingsWindow).Assembly.GetName().Version}\n.NET：{Environment.Version}\nWindows：{Environment.OSVersion.Version}\n捕获：GDI desktop snapshot / Windows OCR\n录屏：Media Foundation H.264", true));
        return panel;
    }

    private static AiProviderSettings CloneProvider(AiProviderSettings source) => new()
    {
        Id = source.Id,
        Name = source.Name,
        Type = source.Type,
        BaseUrl = source.BaseUrl,
        Model = source.Model,
        CredentialId = source.CredentialId,
        CustomHeaders = new Dictionary<string, string>(source.CustomHeaders)
    };

    private void SelectProvider(AiProviderSettings? provider)
    {
        if (_loadingProvider || provider is null) return;
        StoreSelectedProvider();
        _selectedProvider = provider;
        _loadingProvider = true;
        _providerName.Text = provider.Name;
        _providerType.SelectedIndex = provider.Type.Equals("MiniMax", StringComparison.OrdinalIgnoreCase) ? 1 : 0;
        _baseUrl.Text = provider.BaseUrl;
        _model.Text = provider.Model;
        _customHeaders.Text = provider.CustomHeaders.Count == 0 ? "{}" : JsonSerializer.Serialize(provider.CustomHeaders, new JsonSerializerOptions { WriteIndented = true });
        _apiKey.Password = _pendingApiKeys.GetValueOrDefault(provider.Id) ?? string.Empty;
        _defaultProvider.IsChecked = provider.Id == _defaultProviderId;
        _loadingProvider = false;
    }

    private void StoreSelectedProvider()
    {
        if (_selectedProvider is null || _loadingProvider) return;
        _selectedProvider.Name = string.IsNullOrWhiteSpace(_providerName.Text) ? "未命名 Provider" : _providerName.Text.Trim();
        _selectedProvider.Type = _providerType.SelectedIndex == 1 ? "MiniMax" : "OpenAICompatible";
        _selectedProvider.BaseUrl = _baseUrl.Text.TrimEnd('/');
        _selectedProvider.Model = _model.Text.Trim();
        if (_selectedProvider.Type == "MiniMax" && _selectedProvider.BaseUrl.Contains("api.openai.com", StringComparison.OrdinalIgnoreCase)) _selectedProvider.BaseUrl = "https://api.minimaxi.com/v1";
        if (_selectedProvider.Type == "MiniMax" && _selectedProvider.Model.StartsWith("gpt-", StringComparison.OrdinalIgnoreCase)) _selectedProvider.Model = "MiniMax-M3";
        try { _selectedProvider.CustomHeaders = ParseHeaders(); } catch (JsonException) { }
        if (!string.IsNullOrWhiteSpace(_apiKey.Password)) _pendingApiKeys[_selectedProvider.Id] = _apiKey.Password;
        if (_defaultProvider.IsChecked == true) _defaultProviderId = _selectedProvider.Id;
        _providerSelector.Items.Refresh();
    }

    private void AddProvider()
    {
        StoreSelectedProvider();
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
            MessageBox.Show("至少保留一个 Provider 配置。", "喵呜AI");
            return;
        }
        var index = _providers.IndexOf(_selectedProvider);
        _providers.Remove(_selectedProvider);
        _providerSelector.Items.Remove(_selectedProvider);
        _pendingApiKeys.Remove(_selectedProvider.Id);
        _selectedProvider = null;
        _providerSelector.SelectedItem = _providers[Math.Clamp(index, 0, _providers.Count - 1)];
    }

    private async Task TestConnectionAsync(Button button)
    {
        button.IsEnabled = false;
        try
        {
            StoreSelectedProvider();
            var existing = _selectedProvider;
            var key = !string.IsNullOrWhiteSpace(_apiKey.Password) ? _apiKey.Password : existing is null ? null : new CredentialService().Read(existing.CredentialId);
            if (string.IsNullOrWhiteSpace(key)) throw new InvalidOperationException("请先输入 API Key");
            var settings = new AiProviderSettings { Type = _providerType.SelectedIndex == 1 ? "MiniMax" : "OpenAICompatible", BaseUrl = _baseUrl.Text.TrimEnd('/'), Model = _model.Text, CustomHeaders = ParseHeaders() };
            IAiProvider provider = settings.Type == "MiniMax" ? new MiniMaxProvider(settings, key) : new OpenAiCompatibleProvider(settings, key);
            var ok = await provider.TestConnectionAsync(CancellationToken.None);
            MessageBox.Show(ok ? "连接成功" : "服务返回失败状态", "AI 连接测试");
        }
        catch (Exception ex) { MessageBox.Show(ex.Message, "AI 连接测试失败"); }
        finally { button.IsEnabled = true; }
    }

    private Dictionary<string, string> ParseHeaders() => JsonSerializer.Deserialize<Dictionary<string, string>>(string.IsNullOrWhiteSpace(_customHeaders.Text) ? "{}" : _customHeaders.Text) ?? [];

    private void Save()
    {
        Dictionary<string, string> headers;
        try { headers = ParseHeaders(); }
        catch (JsonException ex) { MessageBox.Show($"Custom Headers JSON 无效：{ex.Message}", "无法保存"); return; }
        StoreSelectedProvider();
        if (_selectedProvider is not null) _selectedProvider.CustomHeaders = headers;
        _host.Settings.LaunchAtStartup = _startup.IsChecked == true;
        StartupService.SetEnabled(_host.Settings.LaunchAtStartup);
        var modifiers = System.Windows.Input.ModifierKeys.None;
        if (_ctrl.IsChecked == true) modifiers |= System.Windows.Input.ModifierKeys.Control;
        if (_shift.IsChecked == true) modifiers |= System.Windows.Input.ModifierKeys.Shift;
        if (_alt.IsChecked == true) modifiers |= System.Windows.Input.ModifierKeys.Alt;
        if (_hotkey.SelectedItem is not string key || !Enum.TryParse<System.Windows.Input.Key>(key, out var parsed)) { MessageBox.Show("请选择有效的快捷键。", "无法保存"); return; }
        if (modifiers == System.Windows.Input.ModifierKeys.None) { MessageBox.Show("快捷键至少需要 Ctrl、Shift 或 Alt 中的一个修饰键。", "无法保存"); return; }
        if (!_host.TrySetCaptureHotkey(new HotkeySetting { Key = parsed, Modifiers = modifiers })) { MessageBox.Show("该快捷键可能已被其他应用占用。旧快捷键仍然有效，请换一个组合。", "快捷键冲突"); return; }
        _host.Settings.CaptureDelaySeconds = _delay.SelectedIndex switch { 1 => 3, 2 => 5, _ => 0 };
        _host.Settings.DefaultImageFormat = _imageFormat.SelectedIndex == 1 ? "jpg" : "png";
        _host.Settings.OverlayOpacity = _overlayOpacity.SelectedIndex switch { 0 => .55, 2 => .65, _ => .60 };
        _host.Settings.IncludeCaptureCursor = _captureCursor.IsChecked == true;
        _host.Settings.RecordingFps = _recordingFps.SelectedItem is int recordingFps ? recordingFps : 30;
        _host.Settings.RecordingQuality = _recordingQuality.SelectedItem is int recordingQuality ? recordingQuality : 75;
        _host.Settings.GifFps = _gifFps.SelectedItem is int gifFps ? gifFps : 15;
        _host.Settings.IncludeRecordingCursor = _recordCursor.IsChecked == true;
        _host.Settings.TempCleanupDays = _tempCleanup.SelectedIndex switch { 0 => 1, 2 => 7, 3 => 14, 4 => 30, _ => 3 };
        _host.Settings.EnableVoiceInput = _voice.IsChecked == true;
        _host.Settings.AutomaticallyStartListening = _autoVoice.IsChecked == true;
        _host.Settings.VoiceLanguage = (_voiceLanguage.SelectedItem as ComboBoxItem)?.Tag?.ToString() ?? "system";
        _host.Settings.SaveConversationHistory = _history.IsChecked == true;
        var credentials = new CredentialService();
        foreach (var removed in _host.Settings.Providers.Where(x => _providers.All(p => p.Id != x.Id)))
            if (!string.IsNullOrWhiteSpace(removed.CredentialId)) credentials.Delete(removed.CredentialId);
        foreach (var provider in _providers)
        {
            if (_pendingApiKeys.TryGetValue(provider.Id, out var keyValue) && !string.IsNullOrWhiteSpace(keyValue))
            {
                if (string.IsNullOrWhiteSpace(provider.CredentialId)) provider.CredentialId = Guid.NewGuid().ToString("N");
                credentials.Save(provider.CredentialId, keyValue);
            }
        }
        _host.Settings.Providers = _providers.Select(CloneProvider).ToList();
        _host.Settings.DefaultProviderId = _providers.Any(x => x.Id == _defaultProviderId) ? _defaultProviderId : _providers[0].Id;
        _host.SaveSettings();
        Close();
    }
}
