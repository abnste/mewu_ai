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
    private const int ReasoningDisplayLimit=12_000;
    // Keep the composer light at rest; grow only when an answer actually
    // arrives so the quick-question surface does not feel like an empty
    // document window.
    private const double CompactWindowHeight=176;
    private const double ExpandedWindowHeight=360;
    private readonly AppHost _host;
    private readonly TextBox _prompt = new(), _answer = new();
    private readonly Border _answerCard = new();
    private readonly TextBlock _status = new(), _reasoning = new();
    private readonly Border _reasoningPanel = new();
    private readonly Button _reasoningToggle = new(), _microphone = new();
    private readonly List<AiMessage> _history = [];
    private CancellationTokenSource? _request, _speechRequest;
    private bool _answerWindowAutoExpanded;
    private bool _closed;

    public TextAiWindow(AppHost host, string initial = "")
    {
        _host = host;
        var body = BuildBody(initial);
        ProductWindowShell.Configure(this, "文字问答", 660, CompactWindowHeight, 540, 164, body);
        Loaded += async (_, _) => { _ = Dispatcher.BeginInvoke(System.Windows.Threading.DispatcherPriority.Input,new Action(()=>Keyboard.Focus(_prompt))); if (host.Settings.EnableVoiceInput && host.Settings.AutomaticallyStartListening) await ToggleListeningAsync(); };
        Closed += (_, _) => { _closed=true;_request?.Cancel(); _speechRequest?.Cancel(); };
        PreviewKeyDown += (_, e) => { if (e.Key == Key.Escape) { _request?.Cancel(); Close(); e.Handled = true; } };
    }

    protected override void OnSourceInitialized(EventArgs e) { base.OnSourceInitialized(e); NativeMethods.ExcludeFromCapture(new System.Windows.Interop.WindowInteropHelper(this).Handle); }

    private UIElement BuildBody(string initial)
    {
        var grid = new Grid { Margin = new Thickness(14, 0, 14, 14) };
        // The answer row is collapsed until the first response chunk arrives.
        // Keep it Auto-sized so the compact composer does not reserve a
        // blank star row above the input while the window is idle.
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        _answer.IsReadOnly = true;
        _answer.IsTabStop = false;
        _answer.TextWrapping = TextWrapping.Wrap;
        _answer.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
        _answer.Background = Brushes.Transparent;
        _answer.BorderThickness = new Thickness(0);
        _answer.Padding = new Thickness(12, 10, 12, 10);
        _answer.MaxHeight = 214;
        _answer.FontSize = 13;
        TextBlock.SetLineHeight(_answer, 20);
        System.Windows.Automation.AutomationProperties.SetName(_answer, "AI 回答");
        _answerCard.Background = Brushes.White;
        _answerCard.BorderBrush = new SolidColorBrush(Color.FromRgb(220, 228, 239));
        _answerCard.BorderThickness = new Thickness(1);
        _answerCard.CornerRadius = new CornerRadius(13);
        _answerCard.Margin = new Thickness(0, 0, 0, 6);
        _answerCard.VerticalAlignment = VerticalAlignment.Top;
        _answerCard.Child = _answer;
        // Keep the text-only entry state focused on the composer.  The answer
        // surface is revealed as soon as the first streamed character arrives,
        // so an empty query window does not present a large blank card.
        _answerCard.Visibility = Visibility.Collapsed;
        grid.Children.Add(_answerCard);

        _reasoning.TextWrapping = TextWrapping.Wrap;
        _reasoning.Foreground = new SolidColorBrush(Color.FromRgb(101, 116, 138));
        _reasoning.FontSize = 12.5;
        _reasoning.LineHeight = 19;
        _reasoningPanel.Background = new SolidColorBrush(Color.FromRgb(243, 246, 250));
        _reasoningPanel.BorderBrush = new SolidColorBrush(Color.FromRgb(223, 231, 241));
        _reasoningPanel.BorderThickness = new Thickness(1);
        _reasoningPanel.CornerRadius = new CornerRadius(10);
        _reasoningPanel.Padding = new Thickness(10, 8, 10, 8);
        _reasoningPanel.Margin = new Thickness(0, 4, 0, 0);
        _reasoningPanel.Visibility = Visibility.Collapsed;
        _reasoningPanel.Child = new ScrollViewer { MaxHeight = 72, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = _reasoning };
        _reasoningToggle.Content = "思考过程";
        _reasoningToggle.HorizontalContentAlignment = HorizontalAlignment.Left;
        _reasoningToggle.Padding = new Thickness(10, 6, 10, 6);
        _reasoningToggle.Margin = new Thickness(0, 6, 0, 0);
        _reasoningToggle.FontSize = 12.5;
        _reasoningToggle.FocusVisualStyle = null;
        _reasoningToggle.Cursor = Cursors.Hand;
        _reasoningToggle.ToolTip = "展开或收起思考过程";
        System.Windows.Automation.AutomationProperties.SetName(_reasoningToggle, "展开或收起思考过程");
        _reasoningToggle.Visibility = Visibility.Collapsed;
        _reasoningToggle.Click += (_, _) => _reasoningPanel.Visibility = _reasoningPanel.Visibility == Visibility.Visible ? Visibility.Collapsed : Visibility.Visible;
        var reasoningHost = new StackPanel();
        reasoningHost.Children.Add(_reasoningToggle);
        reasoningHost.Children.Add(_reasoningPanel);

        _prompt.Text = initial;
        _prompt.MinHeight = 38;
        _prompt.MaxHeight = 68;
        _prompt.Padding = new Thickness(10, 6, 10, 6);
        _prompt.FontSize = 12.5;
        TextBlock.SetLineHeight(_prompt, 18);
        _prompt.VerticalContentAlignment = VerticalAlignment.Center;
        _prompt.AcceptsReturn = true;
        _prompt.TextWrapping = TextWrapping.Wrap;
        _prompt.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
        _prompt.BorderThickness = new Thickness(0);
        _prompt.Background = Brushes.Transparent;
        System.Windows.Automation.AutomationProperties.SetName(_prompt, "输入问题");
        _prompt.PreviewKeyDown += async (_, e) => { if (e.Key == Key.Enter && !Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)) { e.Handled = true; await SendAsync(); } };
        var hint = new TextBlock { Text = "输入问题…", Foreground = new SolidColorBrush(Color.FromRgb(139, 153, 173)), Margin = new Thickness(10, 0, 0, 0), FontSize = 12.5, LineHeight = 18, VerticalAlignment = VerticalAlignment.Center, IsHitTestVisible = false };
        // A watermark layered above a TextBox otherwise sits underneath the
        // caret when the empty field receives focus.  Hide the caret while
        // the watermark is visible, then restore it as soon as the user types.
        void UpdatePromptHint()
        {
            var empty = _prompt.Text.Length == 0;
            hint.Visibility = empty ? Visibility.Visible : Visibility.Collapsed;
            _prompt.CaretBrush = empty ? Brushes.Transparent : new SolidColorBrush(Color.FromRgb(91, 108, 235));
        }
        UpdatePromptHint();
        _prompt.TextChanged += (_, _) => UpdatePromptHint();
        _prompt.GotKeyboardFocus += (_, _) => UpdatePromptHint();
        _prompt.LostKeyboardFocus += (_, _) => UpdatePromptHint();
        var promptHost = new Grid(); promptHost.Children.Add(_prompt); promptHost.Children.Add(hint);
        var inputBorder = new Border { Background = new SolidColorBrush(Color.FromRgb(244, 247, 251)), BorderBrush = new SolidColorBrush(Color.FromRgb(220, 228, 239)), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(13), Padding = new Thickness(2), Child = promptHost };
        var restingInputBorder = inputBorder.BorderBrush;
        var focusedInputBorder = new SolidColorBrush(Color.FromRgb(115, 130, 235));
        _prompt.GotKeyboardFocus += (_, _) => inputBorder.BorderBrush = focusedInputBorder;
        _prompt.LostKeyboardFocus += (_, _) => inputBorder.BorderBrush = restingInputBorder;

        _microphone.Content = MicrophoneIcon();
        _microphone.ToolTip = "语音输入";
        _microphone.Width = 42;
        _microphone.Height = 42;
        _microphone.MinWidth = 42;
        _microphone.MinHeight = 42;
        _microphone.Padding = new Thickness(0);
        _microphone.Margin = new Thickness(8, 0, 0, 0);
        _microphone.Visibility = _host.Settings.EnableVoiceInput ? Visibility.Visible : Visibility.Collapsed;
        System.Windows.Automation.AutomationProperties.SetName(_microphone, "语音输入");
        _microphone.SetResourceReference(StyleProperty, "RoundIconButton");
        _microphone.Click += async (_, _) => await ToggleListeningAsync();
        var send = new Button { Content = SendIcon(), Width = 42, Height = 42, MinWidth = 42, MinHeight = 42, Padding = new Thickness(0), Margin = new Thickness(8, 0, 0, 0), ToolTip = "发送" };
        send.SetResourceReference(StyleProperty, "AccentIconButton");
        System.Windows.Automation.AutomationProperties.SetName(send, "发送");
        send.Click += async (_, _) => await SendAsync();
        var composer = new Grid();
        composer.ColumnDefinitions.Add(new ColumnDefinition());
        composer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        composer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        composer.Children.Add(inputBorder); Grid.SetColumn(_microphone, 1); composer.Children.Add(_microphone); Grid.SetColumn(send, 2); composer.Children.Add(send);

        _status.Foreground = new SolidColorBrush(Color.FromRgb(139, 153, 173));
        _status.FontSize = 10.5;
        _status.Text = "Enter 发送 · Shift+Enter 换行";
        _status.HorizontalAlignment = HorizontalAlignment.Center;
        _status.TextAlignment = TextAlignment.Center;
        _status.TextWrapping = TextWrapping.Wrap;
        _status.MaxWidth = 600;
        _status.Margin = new Thickness(0, 6, 0, 0);
        var composerArea = new StackPanel { Margin = new Thickness(0, 8, 0, 0) };
        composerArea.Children.Add(composer);
        composerArea.Children.Add(_status);

        Grid.SetRow(reasoningHost, 1); grid.Children.Add(reasoningHost);
        Grid.SetRow(composerArea, 2); grid.Children.Add(composerArea);
        return grid;
    }

    private async Task ToggleListeningAsync()
    {
        if(_closed)return;
        if (!_host.Settings.EnableVoiceInput) { _microphone.Visibility = Visibility.Collapsed; _status.Text = "语音输入已在设置中关闭"; return; }
        if (_speechRequest is not null) { _status.Text = "正在停止聆听…"; _speechRequest.Cancel(); return; }
        var speechRequest=new CancellationTokenSource();
        _speechRequest=speechRequest;
        _microphone.Content = new System.Windows.Shapes.Rectangle{Width=11,Height=11,RadiusX=2,RadiusY=2,Fill=new SolidColorBrush(Color.FromRgb(210,72,86))}; _status.Text = "正在聆听…再次点击可停止";
        try
        {
            var text=await new WindowsSpeechToTextService().RecognizeOnceAsync(_host.Settings.VoiceLanguage,speechRequest.Token);
            if(_closed||!ReferenceEquals(_speechRequest,speechRequest))return;
            if(!string.IsNullOrWhiteSpace(text))_prompt.Text+=text;
            _status.Text="语音已写入，可编辑后发送";
        }
        catch(OperationCanceledException) { if(!_closed&&ReferenceEquals(_speechRequest,speechRequest))_status.Text="已停止聆听"; }
        catch(SpeechRecognitionUnavailableException ex) { if(!_closed&&ReferenceEquals(_speechRequest,speechRequest))_status.Text=ex.Message; }
        catch(Exception) { if(!_closed&&ReferenceEquals(_speechRequest,speechRequest))_status.Text="语音输入暂时不可用"; }
        finally
        {
            speechRequest.Dispose();
            if(ReferenceEquals(_speechRequest,speechRequest))
            {
                _speechRequest=null;
                if(!_closed)_microphone.Content=MicrophoneIcon();
            }
        }
    }

    private async Task SendAsync()
    {
        if (_closed||string.IsNullOrWhiteSpace(_prompt.Text)) return;
        var provider = new AiProviderFactory().Create(_host.Settings, out var providerError);
        if (provider is null) { _status.Text = providerError ?? "尚未配置 AI 模型或 API Key"; _host.ShowSettings(); return; }
        _request?.Cancel(); var request = new CancellationTokenSource(); _request = request; var prompt = _prompt.Text; ResetAnswerSurface(); _reasoning.Text = ""; _reasoningToggle.Visibility = _reasoningPanel.Visibility = Visibility.Collapsed; _status.Text = "生成中…"; var streamOpen = true;var streamedContent=new System.Text.StringBuilder();var reasoningBuffer=new System.Text.StringBuilder();var lastPreview=string.Empty;var previewScheduled=false;var reasoningScheduled=false;
        try
        {
            var progress = provider.Capabilities.SupportsStreaming ? new Progress<AiStreamDelta>(delta =>
            {
                if (_closed||!streamOpen || !ReferenceEquals(_request, request)) return;
                if(delta.ReasoningContent.Length>0)
                {
                    var first=reasoningBuffer.Length==0;reasoningBuffer.Append(delta.ReasoningContent);if(reasoningBuffer.Length>ReasoningDisplayLimit*2)reasoningBuffer.Remove(0,reasoningBuffer.Length-ReasoningDisplayLimit*2);
                    _reasoningToggle.Visibility=Visibility.Visible;_reasoningToggle.Content="正在思考…";_reasoningPanel.Visibility=Visibility.Visible;if(first)_reasoning.Text=LimitReasoning(reasoningBuffer.ToString());
                    if(!reasoningScheduled){reasoningScheduled=true;_ = Dispatcher.BeginInvoke(System.Windows.Threading.DispatcherPriority.Background,new Action(()=>{reasoningScheduled=false;if(_closed||!streamOpen||!ReferenceEquals(_request,request))return;_reasoning.Text=LimitReasoning(reasoningBuffer.ToString());}));}
                }
                if(delta.Content.Length>0){streamedContent.Append(delta.Content);if(previewScheduled)return;previewScheduled=true;_ = Dispatcher.BeginInvoke(System.Windows.Threading.DispatcherPriority.Background,new Action(()=>{previewScheduled=false;if(_closed||!streamOpen||!ReferenceEquals(_request,request))return;var preview=StructuredResponseParser.GetStreamingTextPreview(streamedContent.ToString());if(preview.Length==0||string.Equals(preview,lastPreview,StringComparison.Ordinal))return;lastPreview=preview;RevealAnswerCard();_answer.Text=preview;}));}
            }) : null;
            var result = await provider.SendAsync(new AiRequest { Prompt = prompt, History = [.. ConversationContextPolicy.CreateBoundedHistory(_history)], StreamingProgress = progress,MaxOutputTokens=4096 }, request.Token);
            streamOpen = false;
            // The provider can finish concurrently with Esc, window close, or
            // a replacement request.  Do not render or commit that late result
            // unless this request is still the live, uncancelled one.
            if (!CanAcceptRequest(request)) return;
            request.Token.ThrowIfCancellationRequested();
            if (!string.IsNullOrWhiteSpace(result.Reasoning)) _reasoning.Text = LimitReasoning(result.Reasoning.Trim());
            var emptyAnswer = AiResultValidation.GetEmptyAnswerMessage(result);
            if (emptyAnswer is not null) { CloseReasoning("思考过程"); RevealAnswerCard(); _answer.Text = emptyAnswer; _status.Text = emptyAnswer; return; }
            CloseReasoning("思考过程");
            RevealAnswerCard();
            _answer.Text = result.Answer;
            var configured = _host.Settings.Providers.FirstOrDefault(x => x.Id == provider.Id);
            if (_host.Settings.SaveConversationHistory)
            {
                try
                {
                    if (!CanAcceptRequest(request)) return;
                    request.Token.ThrowIfCancellationRequested();
                    await new ConversationHistoryService().TryAppendAsync(configured?.Name ?? provider.Id, configured?.Model ?? "", prompt, result.Answer, request.Token);
                    // TryAppendAsync intentionally converts cancellation into
                    // a false result for fire-and-forget callers.  This request
                    // is interactive, so surface cancellation to the outer
                    // handler to collapse reasoning and clear the busy state.
                    request.Token.ThrowIfCancellationRequested();
                }
                catch (OperationCanceledException) when (request.Token.IsCancellationRequested) { throw; }
                catch(Exception ex){try{new PrivacyLogger().Error("ConversationHistory",ex);}catch{}}
            }
            // Keep the in-memory conversation in lockstep with the persisted
            // one, and re-check identity after the asynchronous write so a
            // replaced/cancelled request cannot add a stale answer to context.
            if(!CanAcceptRequest(request))return;
            request.Token.ThrowIfCancellationRequested();
            _history.Add(new("user", prompt)); _history.Add(new("assistant", result.Answer));ConversationContextPolicy.TrimInPlace(_history);
            if(!CanAcceptRequest(request))return;
            if(_prompt.Text==prompt)_prompt.Clear(); _status.Text = "完成 · 可继续追问";
        }
        catch (OperationCanceledException) { if (!_closed&&ReferenceEquals(_request, request)) { CloseReasoning("思考过程"); _status.Text = "已取消"; } }
        catch (Exception ex) { if (!_closed&&ReferenceEquals(_request, request)) { CloseReasoning("思考过程"); RevealAnswerCard(); _answer.Text = "请求失败"; _status.Text = ex.Message; } }
        finally { streamOpen = false; request.Dispose(); if (ReferenceEquals(_request, request)) _request = null; }
    }

    private void CloseReasoning(string label)
    {
        _reasoningPanel.Visibility = Visibility.Collapsed;
        if (_reasoning.Text.Length == 0) { _reasoningToggle.Visibility = Visibility.Collapsed; return; }
        _reasoningToggle.Visibility = Visibility.Visible;
        _reasoningToggle.Content = label;
    }

    private void RevealAnswerCard()
    {
        _answerCard.Visibility = Visibility.Visible;
        // Only grow the window when it is still at the compact default size.
        // If the user resized it deliberately, keep that choice intact.
        if (WindowState == WindowState.Normal && Height <= 211 && Height < ExpandedWindowHeight)
        {
            Height = ExpandedWindowHeight;
            _answerWindowAutoExpanded = true;
        }
    }

    private void ResetAnswerSurface()
    {
        _answerCard.Visibility = Visibility.Collapsed;
        // A completed answer expands the compact question window for reading.
        // Collapse that automatic expansion on the next request, but never
        // overwrite a size the user selected manually afterwards.
        if (_answerWindowAutoExpanded && WindowState == WindowState.Normal && Math.Abs(Height - ExpandedWindowHeight) <= 3)
            Height = CompactWindowHeight;
        _answerWindowAutoExpanded = false;
    }

    private bool CanAcceptRequest(CancellationTokenSource request)=>
        CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed);

    private static string LimitReasoning(string value)=>value.Length<=ReasoningDisplayLimit?value:"…较早思考内容已收纳…\n"+value[^ReasoningDisplayLimit..];

    private static System.Windows.Shapes.Path MicrophoneIcon()=>new()
    {
        Width=16,
        Height=18,
        Stretch=Stretch.Uniform,
        Fill=new SolidColorBrush(Color.FromRgb(82,99,122)),
        Data=Geometry.Parse("M12,14 C13.66,14 15,12.66 15,11 L15,5 C15,3.34 13.66,2 12,2 C10.34,2 9,3.34 9,5 L9,11 C9,12.66 10.34,14 12,14 M17.3,11 C17.3,14 14.76,16.1 12,16.1 C9.24,16.1 6.7,14 6.7,11 L5,11 C5,14.41 7.72,17.23 11,17.72 L11,21 L13,21 L13,17.72 C16.28,17.23 19,14.41 19,11 Z")
    };

    private static System.Windows.Shapes.Path SendIcon()=>new()
    {
        Width=16,
        Height=16,
        Stretch=Stretch.Uniform,
        Fill=Brushes.White,
        Data=Geometry.Parse("M3,13 L15,8 L3,3 L6,8 Z")
    };
}
