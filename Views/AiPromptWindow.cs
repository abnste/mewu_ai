using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Services;
using TextBox=System.Windows.Controls.TextBox;
using Button=System.Windows.Controls.Button;
namespace mewu_ai_Assistant.Views;
public sealed class AiPromptWindow : Window
{
    private readonly AppHost _host;private readonly TextBox _prompt=new();private readonly TextBlock _status=new();
    public AiPromptWindow(AppHost host,BitmapSource image,bool translate=false)
    {
        _host=host;Title=translate?"喵呜AI 翻译":"喵呜AI";Width=680;Height=520;WindowStartupLocation=WindowStartupLocation.CenterScreen;Background=new SolidColorBrush(Color.FromRgb(11,16,24));Foreground=Brushes.White;
        var grid=new Grid{Margin=new Thickness(20)};grid.RowDefinitions.Add(new RowDefinition());grid.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});grid.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});
        var preview=new Image{Source=image,Stretch=Stretch.Uniform,Margin=new Thickness(0,0,0,14)};_prompt.Text=translate?"翻译成中文":"";_prompt.MinHeight=46;_prompt.Padding=new Thickness(10);_prompt.ToolTip="询问图中内容…";var send=new Button{Content="发送",Padding=new Thickness(20,9,20,9),HorizontalAlignment=HorizontalAlignment.Right,Margin=new Thickness(0,10,0,0)};send.Click+=(_,_)=>Send();_status.Foreground=new SolidColorBrush(Color.FromRgb(145,160,181));_status.Text="将发送：图片 + 文字（只有点击发送后才上传）";Grid.SetRow(_prompt,1);Grid.SetRow(send,2);grid.Children.Add(preview);grid.Children.Add(_prompt);grid.Children.Add(send);Content=grid;
        SourceInitialized+=(_,_)=>NativeMethods.SetWindowDisplayAffinity(new System.Windows.Interop.WindowInteropHelper(this).Handle,NativeMethods.WdaExcludeFromCapture);
    }
    private void Send(){if(_host.Settings.Providers.Count==0){_status.Text="尚未配置 AI 模型。请先打开设置 → AI。";_host.ShowSettings();return;}_status.Text="AI Provider 请求管线正在准备；当前不会静默上传。";}
}
