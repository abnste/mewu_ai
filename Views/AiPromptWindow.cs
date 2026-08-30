using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Media.Effects;
using System.Windows.Media.Imaging;
using System.Windows.Shapes;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Speech;
using Button=System.Windows.Controls.Button;
using TextBox=System.Windows.Controls.TextBox;

namespace mewu_ai_Assistant.Views;

public sealed class AiPromptWindow : Window
{
    private static readonly Brush Cyan=new SolidColorBrush(Color.FromRgb(61,190,255));
    private static readonly Brush Secondary=new SolidColorBrush(Color.FromRgb(145,160,181));
    private readonly AppHost _host;
    private readonly Models.ScreenRect _region;
    private readonly bool _translate;
    private readonly CaptureFrame _desktop;
    private BitmapSource _image;
    private readonly Canvas _root=new(){Background=Brushes.Transparent};
    private readonly Grid _selectionHost=new();
    private readonly Image _selectedImage=new(){Stretch=Stretch.Fill};
    private readonly Canvas _annotationLayer=new();
    private readonly Border _answerCard=new();
    private readonly StackPanel _answerHeader=new(){Orientation=Orientation.Horizontal};
    private readonly ScrollViewer _answerScroll=new();
    private readonly Border _divider=new();
    private readonly TextBlock _answer=new(){TextWrapping=TextWrapping.Wrap,LineHeight=23,Foreground=new SolidColorBrush(Color.FromRgb(235,242,252))};
    private readonly TextBox _prompt=new(){AcceptsReturn=true,TextWrapping=TextWrapping.Wrap,MinHeight=46,MaxHeight=92,VerticalScrollBarVisibility=ScrollBarVisibility.Auto,BorderThickness=new Thickness(0),Background=Brushes.Transparent,Foreground=Brushes.White,CaretBrush=Cyan};
    private readonly TextBlock _status=new(){Foreground=Secondary,FontSize=11};
    private readonly Button _send=new();
    private readonly string _initialPrompt;
    private readonly List<Models.AiMessage> _history=[new("system","分析图片时只返回 JSON：{answer:string,annotations:[{x,y,width,height,text,type}]}。坐标为图片内 0 到 1 的归一化值；标注 3 到 6 个最重要位置，说明简洁明确；不适合标注时 annotations 为空。")];
    private CancellationTokenSource? _request,_speechRequest;
    private double _selectionWidth,_selectionHeight;private bool _answerExpanded,_conversationHidden;

    public AiPromptWindow(AppHost host,BitmapSource image,bool translate=false,Models.ScreenRect? region=null,string initialPrompt="",CaptureFrame? desktop=null)
    {
        _host=host;_image=image;_translate=translate;_region=region??new Models.ScreenRect(0,0,image.PixelWidth,image.PixelHeight);_initialPrompt=initialPrompt;
        _desktop=desktop??new ScreenCaptureService().CaptureDesktop(host.Settings.IncludeCaptureCursor);
        if(translate)_history.Add(new("system","把图片内容翻译成中文，保留必要的原位批注。"));
        Title="喵呜AI 屏幕助手";WindowStyle=WindowStyle.None;ResizeMode=ResizeMode.NoResize;AllowsTransparency=true;Background=Brushes.Transparent;Topmost=true;ShowInTaskbar=NativeMethods.VisualQaCaptureEnabled;Cursor=Cursors.Arrow;
        var area=System.Windows.Forms.SystemInformation.VirtualScreen;Left=area.Left;Top=area.Top;Width=area.Width;Height=area.Height;

        var desktopImage=new Image{Source=_desktop.Image,Stretch=Stretch.Fill,IsHitTestVisible=false};_root.Children.Add(desktopImage);
        var dimmer=new Rectangle{Fill=new SolidColorBrush(Color.FromArgb(92,0,5,14)),IsHitTestVisible=false};_root.Children.Add(dimmer);

        _selectedImage.Source=image;_selectionHost.Children.Add(_selectedImage);_selectionHost.Children.Add(_annotationLayer);
        var glow=new Border{BorderBrush=Cyan,BorderThickness=new Thickness(2),CornerRadius=new CornerRadius(9),Background=Brushes.Transparent,IsHitTestVisible=false,Effect=new DropShadowEffect{Color=Color.FromRgb(39,157,255),BlurRadius=24,ShadowDepth=0,Opacity=.95}};
        _selectionHost.Children.Add(glow);_selectionHost.MouseEnter+=(_,_)=>SetConversationHidden(true);_selectionHost.MouseLeave+=(_,_)=>SetConversationHidden(false);_root.Children.Add(_selectionHost);
        _answerCard.Child=BuildAnswerCard();_answerCard.CornerRadius=new CornerRadius(18);_answerCard.Background=new SolidColorBrush(Color.FromArgb(244,7,16,29));_answerCard.BorderBrush=new SolidColorBrush(Color.FromArgb(150,55,144,220));_answerCard.BorderThickness=new Thickness(1);_answerCard.Effect=new DropShadowEffect{Color=Colors.Black,BlurRadius=28,ShadowDepth=9,Opacity=.78};_root.Children.Add(_answerCard);

        var close=new Button{Content="×",ToolTip="关闭",FontSize=18,Foreground=Brushes.White,Background=new SolidColorBrush(Color.FromArgb(210,16,28,45)),BorderBrush=new SolidColorBrush(Color.FromArgb(100,90,125,170)),BorderThickness=new Thickness(1),Padding=new Thickness(12,6,12,6)};close.Click+=(_,_)=>Close();_root.Children.Add(close);Canvas.SetRight(close,18);Canvas.SetTop(close,16);
        Content=_root;
        SourceInitialized+=(_,_)=>{var hwnd=new System.Windows.Interop.WindowInteropHelper(this).Handle;NativeMethods.SetWindowPos(hwnd,new IntPtr(-1),area.Left,area.Top,area.Width,area.Height,0x0040);NativeMethods.ExcludeFromCapture(hwnd);};
        Loaded+=async(_,_)=>{LayoutOverlay();_prompt.Focus();await SendAsync(auto:true);};
        SizeChanged+=(_,_)=>LayoutOverlay();
        Closed+=(_,_)=>{_request?.Cancel();_speechRequest?.Cancel();};
    }

    private UIElement BuildAnswerCard()
    {
        var grid=new Grid{Margin=new Thickness(22,16,22,14)};grid.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});grid.RowDefinitions.Add(new RowDefinition());grid.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});grid.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});
        _answerHeader.Children.Add(new TextBlock{Text="✦",Foreground=Cyan,FontSize=17});_answerHeader.Children.Add(new TextBlock{Text="AI 回答",Foreground=Brushes.White,FontSize=14,FontWeight=FontWeights.SemiBold,Margin=new Thickness(8,1,0,0)});_answerHeader.Visibility=Visibility.Collapsed;grid.Children.Add(_answerHeader);
        _answer.Text="";_answerScroll.Content=_answer;_answerScroll.VerticalScrollBarVisibility=ScrollBarVisibility.Auto;_answerScroll.Margin=new Thickness(0,10,0,12);_answerScroll.Visibility=Visibility.Collapsed;Grid.SetRow(_answerScroll,1);grid.Children.Add(_answerScroll);
        _divider.Height=1;_divider.Background=new SolidColorBrush(Color.FromArgb(65,114,142,177));_divider.Margin=new Thickness(0,0,0,11);_divider.Visibility=Visibility.Collapsed;Grid.SetRow(_divider,2);grid.Children.Add(_divider);
        var composer=new Grid();composer.ColumnDefinitions.Add(new ColumnDefinition());composer.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});composer.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});
        var inputBorder=new Border{CornerRadius=new CornerRadius(11),BorderBrush=new SolidColorBrush(Color.FromArgb(105,91,122,160)),BorderThickness=new Thickness(1),Background=new SolidColorBrush(Color.FromArgb(195,13,25,42)),Padding=new Thickness(3),Child=_prompt};composer.Children.Add(inputBorder);
        var hint=new TextBlock{Text="继续询问当前屏幕内容…",Foreground=Secondary,Margin=new Thickness(14,13,0,0),IsHitTestVisible=false};composer.Children.Add(hint);_prompt.TextChanged+=(_,_)=>hint.Visibility=string.IsNullOrWhiteSpace(_prompt.Text)?Visibility.Visible:Visibility.Collapsed;
        _prompt.PreviewKeyDown+=async(_,e)=>{if(e.Key==Key.Enter&&!Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)){e.Handled=true;await SendAsync();}};
        var mic=DarkButton("⌁","语音输入");mic.Margin=new Thickness(8,0,0,0);mic.Click+=async(_,_)=>await ListenAsync(mic);Grid.SetColumn(mic,1);composer.Children.Add(mic);
        _send.Content="➤";_send.ToolTip="发送";_send.SetResourceReference(StyleProperty,"PrimaryButton");_send.Padding=new Thickness(17,12,17,12);_send.Margin=new Thickness(8,0,0,0);_send.Click+=async(_,_)=>await SendAsync();Grid.SetColumn(_send,2);composer.Children.Add(_send);
        var composerArea=new Grid();composerArea.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});composerArea.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});composerArea.Children.Add(composer);
        _status.Text="Enter 发送 · Shift+Enter 换行 · Esc 取消";_status.HorizontalAlignment=HorizontalAlignment.Center;_status.Margin=new Thickness(0,7,0,0);Grid.SetRow(_status,1);composerArea.Children.Add(_status);
        Grid.SetRow(composerArea,3);grid.Children.Add(composerArea);return grid;
    }

    private static Button DarkButton(string content,string tip)=>new(){Content=content,ToolTip=tip,Foreground=Brushes.White,Background=new SolidColorBrush(Color.FromArgb(210,18,32,52)),BorderBrush=new SolidColorBrush(Color.FromArgb(85,100,132,170)),BorderThickness=new Thickness(1),Padding=new Thickness(13,10,13,10)};

    private void LayoutOverlay()
    {
        if(_root.ActualWidth<=0||_root.ActualHeight<=0)return;
        foreach(var child in _root.Children.OfType<FrameworkElement>().Take(2)){child.Width=_root.ActualWidth;child.Height=_root.ActualHeight;}
        var sx=_root.ActualWidth/_desktop.Image.PixelWidth;var sy=_root.ActualHeight/_desktop.Image.PixelHeight;
        var x=(_region.X-_desktop.OriginX)*sx;var y=(_region.Y-_desktop.OriginY)*sy;_selectionWidth=Math.Max(12,_region.Width*sx);_selectionHeight=Math.Max(12,_region.Height*sy);
        _selectionHost.Width=_selectionWidth;_selectionHost.Height=_selectionHeight;_annotationLayer.Width=_selectionWidth;_annotationLayer.Height=_selectionHeight;Canvas.SetLeft(_selectionHost,x);Canvas.SetTop(_selectionHost,y);
        var cardWidth=Math.Min(Math.Max(_selectionWidth,620),Math.Max(360,_root.ActualWidth-32));var cardHeight=_answerExpanded?Math.Min(285,Math.Max(220,_root.ActualHeight*.34)):104;_answerCard.Width=cardWidth;_answerCard.Height=cardHeight;
        var cardX=Math.Clamp(x+(_selectionWidth-cardWidth)/2,16,Math.Max(16,_root.ActualWidth-cardWidth-16));var cardY=y+_selectionHeight+12;if(cardY+cardHeight>_root.ActualHeight-16)cardY=y-cardHeight-12;if(cardY<16)cardY=Math.Max(16,_root.ActualHeight-cardHeight-16);Canvas.SetLeft(_answerCard,cardX);Canvas.SetTop(_answerCard,cardY);
    }

    private async Task SendAsync(bool auto=false)
    {
        var provider=new AiProviderFactory().Create(_host.Settings);if(provider is null){_status.Text="请先配置可用的 AI Provider";_host.ShowSettings();return;}if(!provider.Capabilities.SupportsImage){_status.Text="当前模型不支持图片理解";return;}
        var typed=_prompt.Text;var prompt=auto?(!string.IsNullOrWhiteSpace(_initialPrompt)?_initialPrompt:_translate?"翻译图片中的内容并在原位置标出重点。":"理解当前屏幕选区，解释内容并在原位置标出关键部分。"):typed;if(string.IsNullOrWhiteSpace(prompt))return;
        _request?.Cancel();_request=new CancellationTokenSource(TimeSpan.FromMinutes(2));_send.IsEnabled=false;if(!auto)_answer.Text="";_status.Text="正在分析…按 Esc 可取消";
        try
        {
            var progress=provider.Capabilities.SupportsStreaming?new Progress<Models.AiStreamDelta>(delta=>{if(delta.ReasoningContent.Length>0)_status.Text="正在思考…";else if(delta.Content.Length>0)_status.Text="正在整理回答…";}):null;
            var result=await provider.SendAsync(new Models.AiRequest{Prompt=prompt,History=[.._history],Attachments=[new Models.AiAttachment(Models.AiAttachmentType.Image,"image/png",ScreenCaptureService.EncodePng(_image))],StreamingProgress=progress},_request.Token);
            ShowAnswer();_answer.Text=result.Answer;_history.Add(new("user",prompt));_history.Add(new("assistant",result.Answer));if(!auto)_prompt.Clear();RenderAnnotations(result.Annotations);
            var configured=_host.Settings.Providers.FirstOrDefault(x=>x.Id==provider.Id);if(_host.Settings.SaveConversationHistory)await new ConversationHistoryService().AppendAsync(configured?.Name??provider.Id,configured?.Model??"",prompt,result.Answer,_request.Token);
            _status.Text=result.Annotations.Count>0?$"已标出 {result.Annotations.Count} 个重点 · 可继续提问":"完成 · 可继续提问";
        }
        catch(OperationCanceledException){_status.Text="已取消";}
        catch(Exception ex){ShowAnswer();_answer.Text="请求失败";_status.Text=ex.Message;}
        finally{_send.IsEnabled=true;}
    }

    private void ShowAnswer(){if(_answerExpanded)return;_answerExpanded=true;_answerHeader.Visibility=_answerScroll.Visibility=_divider.Visibility=Visibility.Visible;LayoutOverlay();}
    private void SetConversationHidden(bool hidden)
    {
        if(_conversationHidden==hidden)return;_conversationHidden=hidden;_answerCard.IsHitTestVisible=!hidden;if(_answerCard.RenderTransform is not TranslateTransform transform){transform=new TranslateTransform();_answerCard.RenderTransform=transform;}var ease=new CubicEase{EasingMode=EasingMode.EaseOut};transform.BeginAnimation(TranslateTransform.YProperty,new DoubleAnimation(hidden?_answerCard.ActualHeight+24:0,TimeSpan.FromMilliseconds(hidden?150:190)){EasingFunction=ease});_answerCard.BeginAnimation(OpacityProperty,new DoubleAnimation(hidden?.06:.98,TimeSpan.FromMilliseconds(150)));
    }

    private async Task ListenAsync(Button button)
    {
        if(_speechRequest is not null){_speechRequest.Cancel();return;}_speechRequest=new();button.Content="■";_status.Text="正在聆听…";
        try{var text=await new WindowsSpeechToTextService().RecognizeOnceAsync(_host.Settings.VoiceLanguage,_speechRequest.Token);if(!string.IsNullOrWhiteSpace(text))_prompt.Text=string.IsNullOrWhiteSpace(_prompt.Text)?text:_prompt.Text+" "+text;_status.Text="语音已写入";}
        catch(OperationCanceledException){_status.Text="已停止聆听";}catch(Exception ex){_status.Text=$"语音不可用：{ex.Message}";}finally{_speechRequest.Dispose();_speechRequest=null;button.Content="⌁";}
    }

    private void RenderAnnotations(IReadOnlyList<Models.AiAnnotation> notes)
    {
        _annotationLayer.Children.Clear();var w=_selectionWidth;var h=_selectionHeight;var cardWidth=Math.Clamp(w*.3,145,420);var font=Math.Clamp(w/70,11,25);var slots=new List<double>();
        foreach(var n in notes.Take(6))
        {
            var x=Math.Clamp(n.X,0,1)*w;var y=Math.Clamp(n.Y,0,1)*h;var rw=Math.Max(14,Math.Clamp(n.Width,0,1)*w);var rh=Math.Max(14,Math.Clamp(n.Height,0,1)*h);
            var box=new Border{Width=rw,Height=rh,CornerRadius=new CornerRadius(5),BorderBrush=Cyan,BorderThickness=new Thickness(Math.Max(1.5,w/900)),Background=new SolidColorBrush(Color.FromArgb(14,55,170,255)),Effect=new DropShadowEffect{Color=Color.FromRgb(34,169,255),BlurRadius=13,ShadowDepth=0,Opacity=.9}};Canvas.SetLeft(box,x);Canvas.SetTop(box,y);_annotationLayer.Children.Add(box);
            var right=x+rw+cardWidth+28<w;var cardX=right?x+rw+24:Math.Max(5,x-cardWidth-24);var cardY=Math.Clamp(y+rh*.5-font*1.5,5,Math.Max(5,h-font*4));while(slots.Any(v=>Math.Abs(v-cardY)<font*3.2))cardY=Math.Min(Math.Max(5,h-font*4),cardY+font*3.4);slots.Add(cardY);
            var startX=right?x+rw:x;var endX=right?cardX:cardX+cardWidth;var line=new Line{X1=startX,Y1=y+rh*.5,X2=endX,Y2=cardY+font*1.4,Stroke=Cyan,StrokeThickness=Math.Max(1,w/1200)};_annotationLayer.Children.Add(line);
            var dot=new Ellipse{Width=5,Height=5,Fill=Cyan};Canvas.SetLeft(dot,endX-2.5);Canvas.SetTop(dot,cardY+font*1.4-2.5);_annotationLayer.Children.Add(dot);
            var text=new TextBlock{Text=n.Text,Foreground=Brushes.White,FontSize=font,TextWrapping=TextWrapping.Wrap,LineHeight=font*1.3};
            var card=new Border{Width=cardWidth,Padding=new Thickness(font*.65,font*.5,font*.65,font*.5),CornerRadius=new CornerRadius(8),Background=new SolidColorBrush(Color.FromArgb(238,8,18,31)),BorderBrush=new SolidColorBrush(Color.FromArgb(125,61,190,255)),BorderThickness=new Thickness(1),Child=text,Effect=new DropShadowEffect{Color=Colors.Black,BlurRadius=15,ShadowDepth=4,Opacity=.7}};Canvas.SetLeft(card,cardX);Canvas.SetTop(card,cardY);_annotationLayer.Children.Add(card);
        }
    }

    protected override void OnKeyDown(KeyEventArgs e){if(e.Key==Key.Escape){if(_request is {IsCancellationRequested:false})_request.Cancel();else Close();e.Handled=true;return;}base.OnKeyDown(e);}
}
