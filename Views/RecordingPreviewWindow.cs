using Microsoft.Win32;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Views;
public sealed class RecordingPreviewWindow : Window
{
    private readonly AppHost _host;private readonly ScreenRect _region;private readonly string _video,_frames;private readonly MediaElement _player=new(){LoadedBehavior=MediaState.Manual,UnloadedBehavior=MediaState.Close,Stretch=Stretch.Uniform};
    public RecordingPreviewWindow(AppHost host,ScreenRect region,string video,string frames){_host=host;_region=region;_video=video;_frames=frames;Title="喵呜AI 录屏预览";Width=900;Height=650;Background=new SolidColorBrush(Color.FromRgb(11,16,24));Foreground=Brushes.White;WindowStartupLocation=WindowStartupLocation.CenterScreen;_player.Source=new Uri(video);var bar=new WrapPanel{Margin=new Thickness(10)};Add(bar,"播放",()=>_player.Play());Add(bar,"暂停",()=>_player.Pause());Add(bar,"重新录制",ReRecord);Add(bar,"保存 MP4",SaveMp4);Add(bar,"导出 GIF",SaveGif);Add(bar,"关键帧问 AI",AskKeyframes);Add(bar,"关闭",Close);var g=new Grid();g.RowDefinitions.Add(new RowDefinition());g.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});Grid.SetRow(bar,1);g.Children.Add(_player);g.Children.Add(bar);Content=g;Loaded+=(_,_)=>_player.Play();Closed+=(_,_)=>Cleanup();SourceInitialized+=(_,_)=>NativeMethods.ExcludeFromCapture(new System.Windows.Interop.WindowInteropHelper(this).Handle);}
    private static void Add(Panel p,string text,Action a){var b=new Button{Content=text,Margin=new Thickness(4),Padding=new Thickness(12,6,12,6),Background=new SolidColorBrush(Color.FromRgb(32,43,58)),Foreground=Brushes.White,BorderBrush=new SolidColorBrush(Color.FromRgb(49,67,91)),BorderThickness=new Thickness(1)};b.Click+=(_,_)=>a();p.Children.Add(b);}
    private void ReRecord(){new RecordingControlWindow(_host,_region).Show();Close();}
    private void SaveMp4(){var d=new SaveFileDialog{Filter="MP4 视频|*.mp4",DefaultExt=".mp4"};if(d.ShowDialog(this)==true){File.Copy(_video,d.FileName,true);_host.Notify("MP4 已保存");}}
    private void SaveGif(){var d=new SaveFileDialog{Filter="GIF 动图|*.gif",DefaultExt=".gif"};if(d.ShowDialog(this)==true){try{GifExportService.Export(_frames,d.FileName,_host.Settings.GifFps);_host.Notify("GIF 已保存");}catch(Exception ex){MessageBox.Show(ex.Message,"GIF 导出失败");}}}
    private void AskKeyframes(){try{var files=Directory.EnumerateFiles(_frames,"*.png").OrderBy(x=>x).ToList();if(files.Count==0)throw new InvalidOperationException("没有可分析的关键帧");var selected=new[]{files[0],files[files.Count/2],files[^1]}.Distinct().ToList();var frames=selected.Select(f=>{using var s=File.OpenRead(f);return (System.Windows.Media.Imaging.BitmapSource)new System.Windows.Media.Imaging.PngBitmapDecoder(s,System.Windows.Media.Imaging.BitmapCreateOptions.PreservePixelFormat,System.Windows.Media.Imaging.BitmapCacheOption.OnLoad).Frames[0];}).ToList();var width=frames.Max(x=>x.PixelWidth);var height=frames.Sum(x=>x.PixelHeight);var visual=new DrawingVisual();using(var dc=visual.RenderOpen()){var y=0d;foreach(var frame in frames){dc.DrawImage(frame,new Rect(0,y,width,frame.PixelHeight));y+=frame.PixelHeight;}}var sheet=new System.Windows.Media.Imaging.RenderTargetBitmap(width,height,96,96,PixelFormats.Pbgra32);sheet.Render(visual);MessageBox.Show("当前 Provider 未声明直接视频输入能力，将明确使用开始/中间/结束关键帧分析，不会假装理解完整视频。","关键帧分析",MessageBoxButton.OK,MessageBoxImage.Information);new AiPromptWindow(_host,sheet).Show();}catch(Exception ex){MessageBox.Show(ex.Message,"无法分析录像");}}
    private void Cleanup(){_player.Stop();_player.Close();try{if(File.Exists(_video))File.Delete(_video);}catch{}try{if(Directory.Exists(_frames))Directory.Delete(_frames,true);}catch{}}
}
