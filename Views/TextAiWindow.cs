using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Speech;

namespace mewu_ai_Assistant.Views;

public sealed class TextAiWindow : Window
{
    private readonly AppHost _host;
    private readonly TextBox _prompt = new(), _answer = new();
    private readonly TextBlock _status = new(), _reasoning = new();
    private readonly Border _reasoningPanel = new();
    private readonly Button _reasoningToggle = new(), _microphone = new();
    private readonly List<AiMessage> _history = [];
    private CancellationTokenSource? _request, _speechRequest;

    public TextAiWindow(AppHost host, string initial = "")
    {
        _host = host;
        var body = BuildBody(initial);
        ProductWindowShell.Configure(this, "文字问答", 680, 460, 560, 400, body);
        Loaded += async (_, _) => { _prompt.Focus(); if (host.Settings.EnableVoiceInput && host.Settings.AutomaticallyStartListening) await ToggleListeningAsync(); };
        Closed += (_, _) => { _request?.Cancel(); _speechRequest?.Cancel(); };
        PreviewKeyDown += (_, e) => { if (e.Key == Key.Escape) { _request?.Cancel(); Close(); e.Handled = true; } };
    }

    protected override void OnSourceInitialized(EventArgs e) { base.OnSourceInitialized(e); NativeMethods.ExcludeFromCapture(new System.Windows.Interop.WindowInteropHelper(this).Handle); }

    private UIElement BuildBody(string initial)
    {
        var grid = new Grid { Margin = new Thickness(16, 0, 16, 16) };
        grid.RowDefinitions.Add(new RowDefinition());
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        _answer.IsReadOnly = true;
        _answer.TextWrapping = TextWrapping.Wrap;
        _answer.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
        _answer.Background = Brushes.Transparent;
        _answer.BorderThickness = new Thickness(0);
        _answer.Padding = new Thickness(14, 12, 14, 12);
        var answerCard = new Border { Background = Brushes.White, BorderBrush = new SolidColorBrush(Color.FromRgb(220, 228, 239)), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(14), Child = _answer };
        grid.Children.Add(answerCard);

        _reasoning.TextWrapping = TextWrapping.Wrap;
        _reasoning.Foreground = new SolidColorBrush(Color.FromRgb(101, 116, 138));
        _reasoning.FontSize = 12.5;
        _reasoning.LineHeight = 19;
        _reasoningPanel.Background = new SolidColorBrush(Color.FromRgb(243, 246, 250));
        _reasoningPanel.BorderBrush = new SolidColorBrush(Color.FromRgb(223, 231, 241));
        _reasoningPanel.BorderThickness = new Thickness(1);
        _reasoningPanel.CornerRadius = new CornerRadius(11);
        _reasoningPanel.Padding = new Thickness(12, 9, 12, 9);
        _reasoningPanel.Margin = new Thickness(0, 6, 0, 0);
        _reasoningPanel.Visibility = Visibility.Collapsed;
        _reasoningPanel.Child = new ScrollViewer { MaxHeight = 84, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = _reasoning };
        _reasoningToggle.Content = "思考过程";
        _reasoningToggle.HorizontalContentAlignment = HorizontalAlignment.Left;
        _reasoningToggle.Padding = new Thickness(12, 7, 12, 7);
        _reasoningToggle.Margin = new Thickness(0, 8, 0, 0);
        _reasoningToggle.Visibility = Visibility.Collapsed;
        _reasoningToggle.Click += (_, _) => _reasoningPanel.Visibility = _reasoningPanel.Visibility == Visibility.Visible ? Visibility.Collapsed : Visibility.Visible;
        var reasoningHost = new StackPanel();
        reasoningHost.Children.Add(_reasoningToggle);
        reasoningHost.Children.Add(_reasoningPanel);

        _prompt.Text = initial;
        _prompt.MinHeight = 40;
        _prompt.MaxHeight = 76;
        _prompt.Padding = new Thickness(9, 5, 9, 5);
        _prompt.FontSize = 13;
        TextBlock.SetLineHeight(_prompt, 19);
        _prompt.VerticalContentAlignment = VerticalAlignment.Center;
        _prompt.AcceptsReturn = true;
        _prompt.TextWrapping = TextWrapping.Wrap;
        _prompt.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
        _prompt.BorderThickness = new Thickness(0);
        _prompt.Background = Brushes.Transparent;
        _prompt.PreviewKeyDown += async (_, e) => { if (e.Key == Key.Enter && !Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)) { e.Handled = true; await SendAsync(); } };
        var hint = new TextBlock { Text = "输入问题…", Foreground = new SolidColorBrush(Color.FromRgb(139, 153, 173)), Margin = new Thickness(9, 0, 0, 0), FontSize = 13, LineHeight = 19, VerticalAlignment = VerticalAlignment.Center, IsHitTestVisible = false };
        hint.Visibility = initial.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
        _prompt.TextChanged += (_, _) => hint.Visibility = _prompt.Text.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
        var promptHost = new Grid(); promptHost.Children.Add(_prompt); promptHost.Children.Add(hint);
        var inputBorder = new Border { Background = new SolidColorBrush(Color.FromRgb(244, 247, 251)), BorderBrush = new SolidColorBrush(Color.FromRgb(220, 228, 239)), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(14), Padding = new Thickness(3), Child = promptHost };

        _microphone.Content = "⌕";
        _microphone.ToolTip = "语音输入";
        _microphone.Width = 44;
        _microphone.Height = 44;
        _microphone.Padding = new Thickness(0);
        _microphone.Margin = new Thickness(8, 0, 0, 0);
        _microphone.SetResourceReference(StyleProperty, "IconButton");
        _microphone.Click += async (_, _) => await ToggleListeningAsync();
        var send = new Button { Content = "发送", Padding = new Thickness(20, 10, 20, 10), Margin = new Thickness(8, 0, 0, 0) };
        send.SetResourceReference(StyleProperty, "PrimaryButton");
        send.Click += async (_, _) => await SendAsync();
        var composer = new Grid();
        composer.ColumnDefinitions.Add(new ColumnDefinition());
        composer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        composer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        composer.Children.Add(inputBorder); Grid.SetColumn(_microphone, 1); composer.Children.Add(_microphone); Grid.SetColumn(send, 2); composer.Children.Add(send);

        _status.Foreground = new SolidColorBrush(Color.FromRgb(139, 153, 173));
        _status.FontSize = 11;
        _status.Text = "Enter 发送 · Shift+Enter 换行";
        _status.HorizontalAlignment = HorizontalAlignment.Center;
        _status.Margin = new Thickness(0, 7, 0, 0);
        var composerArea = new StackPanel { Margin = new Thickness(0, 10, 0, 0) };
        composerArea.Children.Add(composer);
        composerArea.Children.Add(_status);

        Grid.SetRow(reasoningHost, 1); grid.Children.Add(reasoningHost);
        Grid.SetRow(composerArea, 2); grid.Children.Add(composerArea);
        return grid;
    }

    private async Task ToggleListeningAsync()
    {
        if (_speechRequest is not null) { _status.Text = "正在停止聆听…"; _speechRequest.Cancel(); return; }
        _speechRequest = new(); _microphone.Content = "■"; _status.Text = "正在聆听…再次点击可停止";
        try { var text = await new WindowsSpeechToTextService().RecognizeOnceAsync(_host.Settings.VoiceLanguage, _speechRequest.Token); if (!string.IsNullOrWhiteSpace(text)) _prompt.Text += text; _status.Text = "语音已写入，可编辑后发送"; }
        catch (OperationCanceledException) { _status.Text = "已停止聆听"; }
        catch (SpeechRecognitionUnavailableException ex) { _status.Text = ex.Message; }
        catch (Exception) { _status.Text = "语音输入暂时不可用"; }
        finally { _speechRequest.Dispose(); _speechRequest = null; _microphone.Content = "⌕"; }
    }

    private async Task SendAsync()
    {
        if (string.IsNullOrWhiteSpace(_prompt.Text)) return;
        var provider = new AiProviderFactory().Create(_host.Settings);
        if (provider is null) { _status.Text = "尚未配置 AI 模型或 API Key"; _host.ShowSettings(); return; }
        _request?.Cancel(); var request = new CancellationTokenSource(); _request = request; var prompt = _prompt.Text; _answer.Text = ""; _reasoning.Text = ""; _reasoningToggle.Visibility = _reasoningPanel.Visibility = Visibility.Collapsed; _status.Text = "生成中…"; var streamOpen = true;
        try
        {
            var progress = provider.Capabilities.SupportsStreaming ? new Progress<AiStreamDelta>(delta =>
            {
                if (!streamOpen || !ReferenceEquals(_request, request)) return;
                if (delta.ReasoningContent.Length > 0) { _reasoning.Text += delta.ReasoningContent; _reasoningToggle.Visibility = Visibility.Visible; _reasoningToggle.Content = "正在思考…"; _reasoningPanel.Visibility = Visibility.Visible; }
                if (delta.Content.Length > 0) _answer.Text += delta.Content;
            }) : null;
            var result = await provider.SendAsync(new AiRequest { Prompt = prompt, History = [.. _history], StreamingProgress = progress }, request.Token);
            streamOpen = false;
            if (!ReferenceEquals(_request, request)) return;
            if (!string.IsNullOrWhiteSpace(result.Reasoning)) _reasoning.Text = result.Reasoning.Trim();
            CloseReasoning("思考过程 · 已完成");
            var emptyAnswer = AiResultValidation.GetEmptyAnswerMessage(result);
            if (emptyAnswer is not null) { _answer.Text = emptyAnswer; _status.Text = emptyAnswer; return; }
            _answer.Text = result.Answer;
            _history.Add(new("user", prompt)); _history.Add(new("assistant", result.Answer));
            var configured = _host.Settings.Providers.FirstOrDefault(x => x.Id == provider.Id);
            if (_host.Settings.SaveConversationHistory) await new ConversationHistoryService().AppendAsync(configured?.Name ?? provider.Id, configured?.Model ?? "", prompt, result.Answer, request.Token);
            _prompt.Clear(); _status.Text = "完成 · 可继续追问";
        }
        catch (OperationCanceledException) { if (ReferenceEquals(_request, request)) { CloseReasoning("思考过程 · 已取消"); _status.Text = "已取消"; } }
        catch (Exception ex) { if (ReferenceEquals(_request, request)) { CloseReasoning("思考过程 · 已中止"); _status.Text = ex.Message; } }
        finally { streamOpen = false; request.Dispose(); if (ReferenceEquals(_request, request)) _request = null; }
    }

    private void CloseReasoning(string label)
    {
        if (_reasoning.Text.Length == 0) return;
        _reasoningToggle.Visibility = Visibility.Visible;
        _reasoningToggle.Content = label;
        _reasoningPanel.Visibility = Visibility.Collapsed;
    }
}
