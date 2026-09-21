// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Data;
using System.Windows.Media;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public sealed partial class SettingsWindow
{
    private UIElement Api()
    {
        var panel = new StackPanel();
        TextOptions.SetTextFormattingMode(panel, TextFormattingMode.Display);
        System.Windows.Documents.TextElement.SetFontWeight(panel, FontWeights.Normal);
        _apiKey.PasswordChar = '\u25CF';
        AiSettingsForm.PrepareEditor(_apiKey);
        _apiKey.VerticalContentAlignment = VerticalAlignment.Center;
        AutomationProperties.SetName(_apiKey, "API Key");
        panel.Children.Add(AiSettingsForm.Field("API Key", _apiKey));
        _apiKeyStatus.Foreground = SecondaryBrush;
        _apiKeyStatus.FontSize = 11;
        _apiKeyStatus.Margin = new Thickness(0, -8, 0, 12);
        _apiKeyStatus.TextWrapping = TextWrapping.Wrap;
        panel.Children.Add(_apiKeyStatus);

        _model.IsEditable = true;
        _model.IsTextSearchEnabled = false;
        _model.StaysOpenOnEdit = true;
        AiSettingsForm.PrepareEditor(_model);
        AutomationProperties.SetName(_model, LocalizationService.T("模型", "Model"));
        var refreshModels = CreateModelRefreshButton();
        refreshModels.ToolTip = LocalizationService.T("刷新模型", "Refresh models");
        AutomationProperties.SetName(refreshModels, LocalizationService.T("刷新模型", "Refresh models"));
        refreshModels.Click += async (_, _) => await RefreshModelsAsync();
        var modelRow = new Grid();
        modelRow.ColumnDefinitions.Add(new ColumnDefinition());
        modelRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        modelRow.Children.Add(_model);
        Grid.SetColumn(refreshModels, 1);
        modelRow.Children.Add(refreshModels);
        panel.Children.Add(AiSettingsForm.Field(LocalizationService.T("模型", "Model"), modelRow));
        _modelStatus.Foreground = SecondaryBrush;
        _modelStatus.Margin = new Thickness(0, -7, 0, 12);
        _modelStatus.FontSize = 11;
        _modelStatus.TextWrapping = TextWrapping.Wrap;
        panel.Children.Add(_modelStatus);

        _testApiConnection.Content = LocalizationService.T("测试连接", "Test connection");
        _testApiConnection.MinHeight = 38;
        _testApiConnection.Padding = new Thickness(12, 6, 12, 6);
        _testApiConnection.SetResourceReference(StyleProperty, "SecondaryButton");
        _testApiConnection.Click += async (_, _) => await TestConnectionAsync(_testApiConnection);
        var testRow = new Grid { Margin = new Thickness(0, 0, 0, 12) };
        testRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        testRow.ColumnDefinitions.Add(new ColumnDefinition());
        testRow.Children.Add(_testApiConnection);
        _connectionStatus.Margin = new Thickness(12, 0, 0, 0);
        Grid.SetColumn(_connectionStatus, 1);
        testRow.Children.Add(_connectionStatus);
        panel.Children.Add(testRow);

        _customHeaders.AcceptsReturn = true;
        _customHeaders.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
        _customHeaders.HorizontalScrollBarVisibility = ScrollBarVisibility.Auto;
        _customHeaders.TextWrapping = TextWrapping.NoWrap;
        _customHeaders.MinHeight = 80;
        _customHeaders.FontFamily = new FontFamily("Cascadia Mono, Consolas");
        _requestParameters.AcceptsReturn = true;
        _requestParameters.MinHeight = 84;
        _requestParameters.MaxLength = 16384;
        _requestParameters.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
        _requestParameters.FontFamily = new FontFamily("Cascadia Mono, Consolas");
        var advancedContent = new StackPanel { Margin = new Thickness(0, 10, 0, 0) };
        advancedContent.Children.Add(AiSettingsForm.Field(LocalizationService.T("API 地址", "API endpoint"), _baseUrl));
        _apiFormat.IsEditable=true;_apiFormat.ItemsSource=new[]{"auto","chat","responses","anthropic"};
        _authMode.IsEditable=true;_authMode.ItemsSource=new[]{"auto","bearer","api_key","none"};
        AiSettingsForm.PrepareEditor(_apiFormat); AiSettingsForm.PrepareEditor(_authMode);
        AiSettingsForm.PrepareEditor(_requestPath); AiSettingsForm.PrepareEditor(_region); AiSettingsForm.PrepareEditor(_plan);
        var protocolRow=new Grid();protocolRow.ColumnDefinitions.Add(new ColumnDefinition());protocolRow.ColumnDefinitions.Add(new ColumnDefinition());
        var formatField=AiSettingsForm.Field("API 格式",_apiFormat); Grid.SetColumn(formatField,0); protocolRow.Children.Add(formatField);
        var authField=AiSettingsForm.Field("认证模式",_authMode); Grid.SetColumn(authField,1); protocolRow.Children.Add(authField); advancedContent.Children.Add(protocolRow);
        var routingRow=new Grid();routingRow.ColumnDefinitions.Add(new ColumnDefinition());routingRow.ColumnDefinitions.Add(new ColumnDefinition());
        var pathField=AiSettingsForm.Field("请求路径（可选）",_requestPath); Grid.SetColumn(pathField,0); routingRow.Children.Add(pathField);
        var regionField=AiSettingsForm.Field("地区（可选）",_region); Grid.SetColumn(regionField,1); routingRow.Children.Add(regionField); advancedContent.Children.Add(routingRow);
        advancedContent.Children.Add(AiSettingsForm.Field("Plan / 套餐标识（可选）",_plan));
        var parameterHeader = new Grid();
        parameterHeader.ColumnDefinitions.Add(new ColumnDefinition());
        parameterHeader.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        parameterHeader.Children.Add(Text(LocalizationService.T("请求参数 JSON", "Request parameters JSON"), true));
        var parameterHelp = new Button { Content = "?", Width = 22, Height = 22, MinWidth = 22, MinHeight = 22, Padding = new Thickness(0) };
        parameterHelp.SetResourceReference(StyleProperty, "RoundIconButton");
        AutomationProperties.SetName(parameterHelp, LocalizationService.T("请求参数帮助", "Request parameter help"));
        parameterHelp.ToolTip = new TextBlock
        {
            MaxWidth = 350, TextWrapping = TextWrapping.Wrap,
            Text = LocalizationService.T("例如：{\"service_tier\":\"priority\"}。MiniMax M3 优先服务按标准价格的 1.5 倍计费；{} 使用默认服务。也支持 temperature、top_p。",
                "Example: {\"service_tier\":\"priority\"}. MiniMax M3 priority costs 1.5× the standard rate; {} uses the default tier. Also supports temperature and top_p.")
        };
        Grid.SetColumn(parameterHelp, 1);
        parameterHeader.Children.Add(parameterHelp);
        advancedContent.Children.Add(parameterHeader);
        advancedContent.Children.Add(_requestParameters);
        advancedContent.Children.Add(Labeled(LocalizationService.T("Custom Headers JSON（敏感值保存时自动加密）", "Custom Headers JSON (sensitive values are encrypted on save)"), _customHeaders));
        _clearApiKey.Content = LocalizationService.T("清除已保存密钥", "Clear saved key");
        _clearApiKey.MinHeight = 34;
        _clearApiKey.Padding = new Thickness(12, 6, 12, 6);
        _clearApiKey.HorizontalAlignment = HorizontalAlignment.Left;
        _clearApiKey.SetResourceReference(StyleProperty, "SecondaryButton");
        _clearApiKey.Click += (_, _) => ToggleApiKeyDeletion();
        advancedContent.Children.Add(_clearApiKey);
        advancedContent.Children.Add(Text(LocalizationService.T("密钥和敏感 Header 仅加密保存在本机。", "Keys and sensitive headers are encrypted and stored on this computer only."), true));
        _apiAdvanced = new Expander
        {
            Header = LocalizationService.T("高级设置", "Advanced settings"),
            IsExpanded = false, Content = advancedContent, Margin = new Thickness(0, 2, 0, 0)
        };
        panel.Children.Add(_apiAdvanced);

        _modelLoadDebounce.Tick += async (_, _) => { _modelLoadDebounce.Stop(); await RefreshModelsAsync(); };
        _baseUrl.TextChanged += (_, _) => { InvalidateConnectionTest(); ScheduleModelLoad(); };
        _customHeaders.TextChanged += (_, _) => { InvalidateConnectionTest(); ScheduleModelLoad(); };
        _requestParameters.TextChanged += (_, _) => InvalidateConnectionTest();
        _apiFormat.SelectionChanged += (_, _) => { InvalidateConnectionTest(); ScheduleModelLoad(); };
        _authMode.SelectionChanged += (_, _) => { InvalidateConnectionTest(); ScheduleModelLoad(); };
        _apiFormat.AddHandler(TextBox.TextChangedEvent, new TextChangedEventHandler((_, _) => { InvalidateConnectionTest(); ScheduleModelLoad(); }));
        _authMode.AddHandler(TextBox.TextChangedEvent, new TextChangedEventHandler((_, _) => { InvalidateConnectionTest(); ScheduleModelLoad(); }));
        _requestPath.TextChanged += (_, _) => { InvalidateConnectionTest(); ScheduleModelLoad(); };
        _region.TextChanged += (_, _) => InvalidateConnectionTest();_plan.TextChanged += (_, _) => InvalidateConnectionTest();
        _model.AddHandler(TextBox.TextChangedEvent, new TextChangedEventHandler((_, _) => InvalidateConnectionTest()));
        _model.Loaded += ApiModelLoaded;
        _apiKey.PasswordChanged += (_, _) =>
        {
            if (_loadingProvider || _captureProtectionAvailable != true || _selectedProvider is null) return;
            ProviderApiKeyEditorPolicy.RecordEdit(_selectedProvider.Id, _apiKey.Password, _pendingApiKeys, _apiKeysMarkedForDeletion);
            UpdateApiKeyStatus();
            InvalidateConnectionTest();
            ScheduleModelLoad();
        };

        _apiConnections = new ApiConnectionsView(panel, TrySelectApiConnection, AddApiProvider, SetDefaultApiProvider, RemoveApiProvider, RenameApiProvider);
        _aiConfigurationWarning.Foreground = new SolidColorBrush(Color.FromRgb(185, 93, 32));
        _aiConfigurationWarning.Background = new SolidColorBrush(Color.FromRgb(255, 247, 235));
        _aiConfigurationWarning.Padding = new Thickness(12, 9, 12, 9);
        _aiConfigurationWarning.Margin = new Thickness(0, 0, 0, 12);
        _aiConfigurationWarning.TextWrapping = TextWrapping.Wrap;
        var page = new StackPanel { Margin = new Thickness(6, 4, 6, 8) };
        page.Children.Add(_aiConfigurationWarning);
        page.Children.Add(_apiConnections);
        var initial = _providers.FirstOrDefault(p => p.Id == _defaultProviderId) ?? _providers[0];
        SelectProvider(initial);
        RefreshProviderList();
        return page;
    }

    private Expander _apiAdvanced = null!;

    private static Button CreateModelRefreshButton()
    {
        var button = new Button
        {
            Width = 38, Height = 38, MinWidth = 38, MinHeight = 38,
            Padding = new Thickness(8), Margin = new Thickness(8, 0, 0, 0),
            VerticalAlignment = VerticalAlignment.Center,
            Content = new System.Windows.Shapes.Path
            {
                Data = Geometry.Parse("M15,9 A6,6 0 1 1 13.243,4.757 M9.843,4.757 H13.243 V1.357"),
                Stroke = SecondaryBrush, StrokeThickness = 1.6,
                StrokeStartLineCap = PenLineCap.Round, StrokeEndLineCap = PenLineCap.Round,
                StrokeLineJoin = PenLineJoin.Round,
                Width = 18, Height = 18, Stretch = Stretch.Uniform
            }
        };
        button.SetResourceReference(StyleProperty, "IconButton");

        // Keep the chrome's dimensions stable; keyboard focus is drawn separately
        // so mouse activation cannot shrink or clip the circular arrow.
        var chrome = new FrameworkElementFactory(typeof(Border), "RefreshChrome");
        chrome.SetValue(Border.CornerRadiusProperty, new CornerRadius(10));
        foreach (var property in new[] { Border.BackgroundProperty, Border.BorderBrushProperty, Border.BorderThicknessProperty, Border.PaddingProperty })
            chrome.SetBinding(property, new Binding(property.Name) { RelativeSource = new RelativeSource(RelativeSourceMode.TemplatedParent) });
        var content = new FrameworkElementFactory(typeof(ContentPresenter));
        content.SetValue(HorizontalAlignmentProperty, HorizontalAlignment.Center);
        content.SetValue(VerticalAlignmentProperty, VerticalAlignment.Center);
        chrome.AppendChild(content);
        var template = new ControlTemplate(typeof(Button)) { VisualTree = chrome };
        var hover = new Trigger { Property = IsMouseOverProperty, Value = true };
        hover.Setters.Add(new Setter(Border.BackgroundProperty, new SolidColorBrush(Color.FromRgb(234, 241, 250)), "RefreshChrome"));
        template.Triggers.Add(hover);
        var pressed = new Trigger { Property = System.Windows.Controls.Primitives.ButtonBase.IsPressedProperty, Value = true };
        pressed.Setters.Add(new Setter(OpacityProperty, 0.72, "RefreshChrome"));
        template.Triggers.Add(pressed);
        var disabled = new Trigger { Property = IsEnabledProperty, Value = false };
        disabled.Setters.Add(new Setter(OpacityProperty, 0.4, "RefreshChrome"));
        template.Triggers.Add(disabled);
        button.Template = template;

        var focusBorder = new FrameworkElementFactory(typeof(Border));
        focusBorder.SetValue(Border.CornerRadiusProperty, new CornerRadius(8));
        focusBorder.SetValue(Border.BorderThicknessProperty, new Thickness(1.5));
        focusBorder.SetValue(MarginProperty, new Thickness(2));
        focusBorder.SetResourceReference(Border.BorderBrushProperty, "Accent");
        var focusStyle = new Style(typeof(Control));
        focusStyle.Setters.Add(new Setter(Control.TemplateProperty, new ControlTemplate(typeof(Control)) { VisualTree = focusBorder }));
        button.FocusVisualStyle = focusStyle;
        return button;
    }

    private void CaptureApiDraft()
    {
        if (_loadingProvider || _selectedProvider is null) return;
        _providerDrafts[_selectedProvider] = new ApiConnectionDraft(
            _baseUrl.Text, _model.Text,
            _captureProtectionAvailable == false ? ApiConnectionDraft.FromProvider(_selectedProvider).HeadersJson : _customHeaders.Text,
            _requestParameters.Text,_apiFormat.Text,_authMode.Text,_requestPath.Text,_region.Text,_plan.Text);
        // Titles may summarize unsaved model/endpoint changes, while raw JSON
        // stays in the draft until an explicit save or connection test.
        _selectedProvider.BaseUrl = _baseUrl.Text.TrimEnd('/');
        _selectedProvider.Model = _model.Text.Trim();
    }

    private bool TrySelectApiConnection(AiProviderSettings provider)
    {
        if (!_providers.Contains(provider)) return false;
        CaptureApiDraft();
        SelectProvider(provider);
        RefreshProviderList();
        return true;
    }

    private void RefreshProviderList() => _apiConnections.Refresh(
        _providers.OrderByDescending(provider => provider.Id == _defaultProviderId).ToArray(), _defaultProviderId, _selectedProvider);

    private void AddApiProvider(ProviderPreset preset)
    {
        CaptureApiDraft();
        var provider = ProviderPresetPolicy.Create(preset);
        var stem = preset.Id switch
        {
            "MiniMax" => LocalizationService.T("MiniMax 国内", "MiniMax CN"),
            "MiniMaxGlobal" => LocalizationService.T("MiniMax 国际", "MiniMax Global"),
            "Volcengine" => LocalizationService.T("火山引擎", "Volcengine"),
            _ => LocalizationService.T("自定义连接", "Custom connection")
        };
        provider.Name = stem;
        for (var suffix = 2; suffix <= _providers.Count + 2 && _providers.Any(p => p.Name == provider.Name); suffix++)
            provider.Name = $"{stem} {suffix}";
        _providers.Add(provider);
        SelectProvider(provider);
        RefreshProviderList();
        RefreshConfigurationWarnings();
    }

    private void SetDefaultApiProvider(AiProviderSettings provider)
    {
        if (!_providers.Contains(provider)) return;
        CaptureApiDraft();
        _defaultProviderId = provider.Id;
        RefreshProviderList();
        RefreshConfigurationWarnings();
    }

    private bool RenameApiProvider(AiProviderSettings provider, string name)
    {
        name = name.Trim();
        if (!_providers.Contains(provider) || name.Length is 0 or > 80) return false;
        CaptureApiDraft();
        provider.Name = name;
        RefreshProviderList();
        return true;
    }

    private void RemoveApiProvider(AiProviderSettings provider)
    {
        if (!_providers.Contains(provider) || _providers.Count <= 1) return;
        CaptureApiDraft();
        var deletingDefault = string.Equals(_defaultProviderId, provider.Id, StringComparison.Ordinal);
        var explanation = deletingDefault
            ? LocalizationService.T($"删除“{provider.Name}”后，请另选一条默认连接。保存设置后生效。", $"After removing “{provider.Name}”, choose another default connection. Changes apply when you save.")
            : LocalizationService.T($"删除“{provider.Name}”？保存设置后生效。", $"Remove “{provider.Name}”? Changes apply when you save.");
        if (MewuDialogWindow.ShowChoice(this, LocalizationService.T("删除连接", "Remove connection"), explanation,
                LocalizationService.T("删除", "Remove"), string.Empty, LocalizationService.T("取消", "Cancel")) != MewuDialogResult.Primary) return;
        _providers.Remove(provider);
        _providerDrafts.Remove(provider);
        _pendingApiKeys.Remove(provider.Id);
        _apiKeysMarkedForDeletion.Remove(provider.Id);
        _unavailableSensitiveHeaders.Remove(provider);
        if (_hydrationErrors.Remove(provider, out var warning))
            _editorWarnings.RemoveAll(item => item.EndsWith(warning, StringComparison.Ordinal));
        if (deletingDefault) _defaultProviderId = null;
        if (ReferenceEquals(_selectedProvider, provider))
            SelectProvider(_providers.FirstOrDefault(p => p.Id == _defaultProviderId) ?? _providers[0]);
        RefreshProviderList();
        RefreshConfigurationWarnings();
    }

    private void InvalidateConnectionTest()
    {
        if (_loadingProvider) return;
        _connectionTest?.Cancel();
        _connectionTest = null;
        _testApiConnection.IsEnabled = true;
        _connectionStatus.Text = string.Empty;
        _connectionStatus.ToolTip = null;
    }
}
