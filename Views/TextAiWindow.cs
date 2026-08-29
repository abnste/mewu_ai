using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Speech;

namespace mewu_ai_Assistant.Views;

public sealed class TextAiWindow : Window
{
    private readonly AppHost _host;
    private readonly TextBox _prompt=new(),_answer=new();
    private readonly TextBlock _status=new();
    private readonly List<AiMessage> _history=[];
    private readonly Button _microphone=new(){Content="🎤",Padding=new Thickness(12,8,12,8),Margin=new Thickness(0,8,8,0)};
    private CancellationTokenSource? _request,_speechRequest;

    public TextAiWindow(AppHost host,string initial="")
    {
        _host=host;Title="喵呜AI 文字问答";Width=680;Height=500;WindowStartupLocation=WindowStartupLocation.CenterScreen;Background=new SolidColorBrush(Color.FromRgb(11,16,24));Foreground=Brushes.White;
        var grid=new Grid{Margin=new Thickness(20)};grid.RowDefinitions.Add(new RowDefinition());grid.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});grid.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});grid.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});
        _answer.IsReadOnly=true;_answer.TextWrapping=TextWrapping.Wrap;_answer.VerticalScrollBarVisibility=ScrollBarVisibility.Auto;_answer.Background=new SolidColorBrush(Color.FromRgb(18,26,38));_answer.Foreground=Brushes.White;_answer.Padding=new Thickness(12);
        _prompt.Text=initial;_prompt.MinHeight=56;_prompt.Padding=new Thickness(12,10,12,10);_prompt.AcceptsReturn=true;_prompt.TextWrapping=TextWrapping.Wrap;_prompt.VerticalScrollBarVisibility=ScrollBarVisibility.Auto;_prompt.Background=new SolidColorBrush(Color.FromRgb(18,26,38));_prompt.Foreground=Brushes.White;_prompt.CaretBrush=Brushes.White;_prompt.BorderBrush=new SolidColorBrush(Color.FromRgb(38,50,70));_prompt.BorderThickness=new Thickness(1);
        var hint=new TextBlock{Text="问喵呜AI任何问题…",Foreground=new SolidColorBrush(Color.FromRgb(145,160,181)),Margin=new Thickness(13,11,0,0),VerticalAlignment=VerticalAlignment.Top,IsHitTestVisible=false};hint.Visibility=string.IsNullOrEmpty(initial)?Visibility.Visible:Visibility.Collapsed;_prompt.TextChanged+=(_,_)=>hint.Visibility=string.IsNullOrEmpty(_prompt.Text)?Visibility.Visible:Visibility.Collapsed;var promptHost=new Grid();promptHost.Children.Add(_prompt);promptHost.Children.Add(hint);
        _status.Foreground=new SolidColorBrush(Color.FromRgb(145,160,181));_status.Margin=new Thickness(2,6,0,0);
        var actions=new StackPanel{Orientation=Orientation.Horizontal,HorizontalAlignment=HorizontalAlignment.Right};_microphone.Background=new SolidColorBrush(Color.FromRgb(32,43,58));_microphone.Foreground=Brushes.White;_microphone.BorderThickness=new Thickness(0);_microphone.Click+=async(_,_)=>await ToggleListeningAsync();var send=new Button{Content="发送",Padding=new Thickness(20,8,20,8),Margin=new Thickness(0,8,0,0),Background=new SolidColorBrush(Color.FromRgb(49,140,255)),Foreground=Brushes.White,BorderThickness=new Thickness(0)};send.Click+=async(_,_)=>await SendAsync();actions.Children.Add(_microphone);actions.Children.Add(send);
        Grid.SetRow(promptHost,1);Grid.SetRow(_status,2);Grid.SetRow(actions,3);grid.Children.Add(_answer);grid.Children.Add(promptHost);grid.Children.Add(_status);grid.Children.Add(actions);Content=grid;
        Loaded+=async(_,_)=>{_prompt.Focus();if(host.Settings.EnableVoiceInput&&host.Settings.AutomaticallyStartListening)await ToggleListeningAsync();};Closed+=(_,_)=>{_request?.Cancel();_speechRequest?.Cancel();};KeyDown+=(_,e)=>{if(e.Key==System.Windows.Input.Key.Escape){_request?.Cancel();Close();}};
    }

    protected override void OnSourceInitialized(EventArgs e){base.OnSourceInitialized(e);NativeMethods.ExcludeFromCapture(new System.Windows.Interop.WindowInteropHelper(this).Handle);}

    private async Task ToggleListeningAsync()
    {
        if(_speechRequest is not null){_speechRequest.Cancel();return;}_speechRequest=new();_microphone.Content="■";_status.Text="正在聆听…再次点击可停止";
        try{var text=await new WindowsSpeechToTextService().RecognizeOnceAsync(_host.Settings.VoiceLanguage,_speechRequest.Token);if(!string.IsNullOrWhiteSpace(text))_prompt.Text+=text;_status.Text="语音已写入，可编辑后发送";}
        catch(OperationCanceledException){_status.Text="已停止聆听";}catch(Exception ex){_status.Text=$"语音不可用，仍可键盘输入：{ex.Message}";}finally{_speechRequest.Dispose();_speechRequest=null;_microphone.Content="🎤";}
    }

    private async Task SendAsync()
    {
        if(string.IsNullOrWhiteSpace(_prompt.Text))return;var provider=new AiProviderFactory().Create(_host.Settings);if(provider is null){_status.Text="尚未配置 AI 模型或 API Key";_host.ShowSettings();return;}_request?.Cancel();_request=new();var prompt=_prompt.Text;_answer.Text="";_status.Text="生成中…";
        try
        {
            var progress=provider.Capabilities.SupportsStreaming?new Progress<string>(delta=>_answer.Text+=delta):null;var result=await provider.SendAsync(new AiRequest{Prompt=prompt,History=[.._history],StreamingProgress=progress},_request.Token);_answer.Text=result.Answer;_history.Add(new("user",prompt));_history.Add(new("assistant",result.Answer));var configured=_host.Settings.Providers.FirstOrDefault(x=>x.Id==provider.Id);if(_host.Settings.SaveConversationHistory)await new ConversationHistoryService().AppendAsync(configured?.Name??provider.Id,configured?.Model??"",prompt,result.Answer,_request.Token);_prompt.Clear();_status.Text="完成，可继续追问";
        }
        catch(OperationCanceledException){_status.Text="已取消";}catch(Exception ex){_status.Text=ex.Message;}
    }
}
