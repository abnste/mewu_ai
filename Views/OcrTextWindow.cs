using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.OCR;
namespace mewu_ai_Assistant.Views;
public sealed class OcrTextWindow : Window
{
    private readonly BitmapSource _image;private readonly Canvas _canvas=new();private readonly TextBlock _status=new();private CancellationTokenSource? _recognition;
    public OcrTextWindow(BitmapSource image)
    {
        _image=image;Title="喵呜AI 文字识别";Width=Math.Min(image.PixelWidth+40,1200);Height=Math.Min(image.PixelHeight+100,850);WindowStartupLocation=WindowStartupLocation.CenterScreen;Background=new SolidColorBrush(Color.FromRgb(11,16,24));
        _canvas.Background=new ImageBrush(image){Stretch=Stretch.Fill};var copy=new Button{Content="全部复制",Padding=new Thickness(12,6,12,6),Margin=new Thickness(8)};copy.Click+=(_,_)=>Clipboard.SetText(string.Join(Environment.NewLine,_canvas.Children.OfType<TextBox>().Select(x=>x.Text)));var retry=new Button{Content="重新识别",Padding=new Thickness(12,6,12,6),Margin=new Thickness(0,8,8,8)};retry.Click+=async(_,_)=>await RecognizeAsync();var bar=new DockPanel{LastChildFill=true};bar.Children.Add(copy);bar.Children.Add(retry);_status.Text="正在使用 Windows 本地 OCR…";_status.Foreground=Brushes.White;_status.VerticalAlignment=VerticalAlignment.Center;bar.Children.Add(_status);var grid=new Grid();grid.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});grid.RowDefinitions.Add(new RowDefinition());Grid.SetRow(_canvas,1);grid.Children.Add(bar);grid.Children.Add(_canvas);Content=grid;Loaded+=async(_,_)=>await RecognizeAsync();Closed+=(_,_)=>_recognition?.Cancel();KeyDown+=(_,e)=>{if(e.Key==System.Windows.Input.Key.Escape)Close();};SourceInitialized+=(_,_)=>NativeMethods.ExcludeFromCapture(new System.Windows.Interop.WindowInteropHelper(this).Handle);
    }
    private async Task RecognizeAsync(){_recognition?.Cancel();_recognition=new();_canvas.Children.Clear();_status.Text="正在使用 Windows 本地 OCR…";try{var doc=await new WindowsOcrService().RecognizeAsync(_image,_recognition.Token);var sx=_canvas.ActualWidth/_image.PixelWidth;var sy=_canvas.ActualHeight/_image.PixelHeight;foreach(var line in doc.Lines){var box=new TextBox{Text=line.Text,Foreground=Brushes.White,Background=new SolidColorBrush(Color.FromArgb(170,8,15,25)),BorderBrush=new SolidColorBrush(Color.FromRgb(49,140,255)),BorderThickness=new Thickness(1),FontSize=Math.Max(11,line.Height*sy*.72),Padding=new Thickness(2),ToolTip="可按字符选择并复制；双击可选词"};Canvas.SetLeft(box,line.X*sx);Canvas.SetTop(box,line.Y*sy);box.Width=Math.Max(35,line.Width*sx);box.Height=Math.Max(22,line.Height*sy+6);_canvas.Children.Add(box);}_status.Text=$"已识别 {doc.Lines.Count} 行，可按字符或单词选择文字";}catch(OperationCanceledException){}catch(Exception ex){_status.Text=$"OCR 失败：{ex.Message}";}}
}
