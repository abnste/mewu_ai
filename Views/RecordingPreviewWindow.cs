using Microsoft.Win32;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;
public sealed class RecordingPreviewWindow : Window
{
    private readonly AppHost _host;private readonly string _video,_frames;private readonly MediaElement _player=new(){LoadedBehavior=MediaState.Manual,UnloadedBehavior=MediaState.Close,Stretch=Stretch.Uniform};
    public RecordingPreviewWindow(AppHost host,string video,string frames){_host=host;_video=video;_frames=frames;Title="喵呜AI 录屏预览";Width=900;Height=650;Background=new SolidColorBrush(Color.FromRgb(11,16,24));WindowStartupLocation=WindowStartupLocation.CenterScreen;_player.Source=new Uri(video);var bar=new StackPanel{Orientation=Orientation.Horizontal,Margin=new Thickness(10)};Add(bar,"播放",()=>_player.Play());Add(bar,"暂停",()=>_player.Pause());Add(bar,"保存 MP4",SaveMp4);Add(bar,"导出 GIF",SaveGif);Add(bar,"关闭",Close);var g=new Grid();g.RowDefinitions.Add(new RowDefinition());g.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});Grid.SetRow(bar,1);g.Children.Add(_player);g.Children.Add(bar);Content=g;Loaded+=(_,_)=>_player.Play();}
    private static void Add(Panel p,string text,Action a){var b=new Button{Content=text,Margin=new Thickness(4),Padding=new Thickness(12,6,12,6)};b.Click+=(_,_)=>a();p.Children.Add(b);}
    private void SaveMp4(){var d=new SaveFileDialog{Filter="MP4 视频|*.mp4",DefaultExt=".mp4"};if(d.ShowDialog(this)==true)File.Copy(_video,d.FileName,true);}
    private void SaveGif(){var d=new SaveFileDialog{Filter="GIF 动图|*.gif",DefaultExt=".gif"};if(d.ShowDialog(this)==true){try{GifExportService.Export(_frames,d.FileName,_host.Settings.GifFps);}catch(Exception ex){MessageBox.Show(ex.Message,"GIF 导出失败");}}}
}
