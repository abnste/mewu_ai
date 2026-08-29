using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Services;
using ComboBox=System.Windows.Controls.ComboBox;
using TextBox=System.Windows.Controls.TextBox;
using CheckBox=System.Windows.Controls.CheckBox;
using Button=System.Windows.Controls.Button;
namespace mewu_ai_Assistant.Views;
public sealed class SettingsWindow : Window
{
    private readonly AppHost _host; private readonly ComboBox _delay=new(); private readonly TextBox _baseUrl=new(),_model=new(); private readonly CheckBox _history=new(),_voice=new(),_autoVoice=new();
    public SettingsWindow(AppHost host)
    {
        _host=host;Title="喵呜AI 设置";Width=720;Height=600;WindowStartupLocation=WindowStartupLocation.CenterScreen;Background=new SolidColorBrush(Color.FromRgb(11,16,24));Foreground=Brushes.White;
        var tabs=new TabControl{Margin=new Thickness(20)};tabs.Items.Add(Tab("常规",General()));tabs.Items.Add(Tab("捕获",Capture()));tabs.Items.Add(Tab("录屏",Text("录屏帧率与鼠标指针设置将在录屏后端启用。")));tabs.Items.Add(Tab("AI",Ai()));tabs.Items.Add(Tab("语音",Voice()));tabs.Items.Add(Tab("隐私",Privacy()));
        var save=new Button{Content="保存",Padding=new Thickness(24,9,24,9),Margin=new Thickness(0,0,20,20),HorizontalAlignment=HorizontalAlignment.Right};save.Click+=(_,_)=>Save();var grid=new Grid();grid.RowDefinitions.Add(new RowDefinition());grid.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});Grid.SetRow(save,1);grid.Children.Add(tabs);grid.Children.Add(save);Content=grid;
    }
    private static TabItem Tab(string h,UIElement c)=>new(){Header=h,Content=c};private static TextBlock Text(string t)=>new(){Text=t,Margin=new Thickness(20),TextWrapping=TextWrapping.Wrap};
    private UIElement General()=>Text("应用默认常驻托盘。关闭主窗口不会退出；请使用托盘菜单退出。\n\n全局快捷键：Ctrl + Shift + A");
    private UIElement Capture(){var p=Panel();p.Children.Add(Text("延时截图"));foreach(var i in new[]{0,3,5})_delay.Items.Add($"{i} 秒");_delay.SelectedIndex=_host.Settings.CaptureDelaySeconds switch{3=>1,5=>2,_=>0};p.Children.Add(_delay);return p;}
    private UIElement Ai(){var p=Panel();p.Children.Add(Text("OpenAI-Compatible Provider（API Key 将由安全凭据存储管理，当前先配置端点与模型）"));p.Children.Add(Label("Base URL",_baseUrl));p.Children.Add(Label("Model",_model));var provider=_host.Settings.Providers.FirstOrDefault();_baseUrl.Text=provider?.BaseUrl??"https://api.openai.com/v1";_model.Text=provider?.Model??"gpt-4.1-mini";return p;}
    private UIElement Voice(){var p=Panel();_voice.Content="启用语音输入";_voice.IsChecked=_host.Settings.EnableVoiceInput;_autoVoice.Content="Prompt 出现时自动监听";_autoVoice.IsChecked=_host.Settings.AutomaticallyStartListening;p.Children.Add(_voice);p.Children.Add(_autoVoice);return p;}
    private UIElement Privacy(){var p=Panel();_history.Content="在本地保存 AI 对话历史";_history.IsChecked=_host.Settings.SaveConversationHistory;p.Children.Add(_history);p.Children.Add(Text("媒体默认不永久保存；截图只有明确发送 AI 时才会上传。"));return p;}
    private static StackPanel Panel()=>new(){Margin=new Thickness(20)};private static FrameworkElement Label(string name,TextBox box){var p=Panel();p.Children.Add(Text(name));box.Margin=new Thickness(0,4,0,12);box.Padding=new Thickness(8);p.Children.Add(box);return p;}
    private void Save(){_host.Settings.CaptureDelaySeconds=_delay.SelectedIndex switch{1=>3,2=>5,_=>0};_host.Settings.EnableVoiceInput=_voice.IsChecked==true;_host.Settings.AutomaticallyStartListening=_autoVoice.IsChecked==true;_host.Settings.SaveConversationHistory=_history.IsChecked==true;if(!string.IsNullOrWhiteSpace(_baseUrl.Text)){var p=_host.Settings.Providers.FirstOrDefault();if(p is null){p=new();_host.Settings.Providers.Add(p);_host.Settings.DefaultProviderId=p.Id;}p.BaseUrl=_baseUrl.Text.TrimEnd('/');p.Model=_model.Text;}_host.SaveSettings();Close();}
}
