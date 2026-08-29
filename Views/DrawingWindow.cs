using Microsoft.Win32;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Ink;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Services;
using Panel=System.Windows.Controls.Panel;
using Button=System.Windows.Controls.Button;
namespace mewu_ai_Assistant.Views;
public sealed class DrawingWindow : Window
{
    private readonly BitmapSource _source; private readonly InkCanvas _ink=new(); private readonly Stack<Stroke> _redo=[];
    public DrawingWindow(BitmapSource source)
    {
        _source=source;Title="喵呜AI 标注";Width=Math.Min(source.PixelWidth+40,1200);Height=Math.Min(source.PixelHeight+100,850);Background=new SolidColorBrush(Color.FromRgb(11,16,24));Foreground=Brushes.White;WindowStartupLocation=WindowStartupLocation.CenterScreen;
        _ink.Width=source.PixelWidth;_ink.Height=source.PixelHeight;_ink.Background=new ImageBrush(source){Stretch=Stretch.Fill};_ink.DefaultDrawingAttributes=new DrawingAttributes{Color=Colors.Red,Width=4,Height=4,FitToCurve=true};_ink.StrokeCollected+=(_,_)=>_redo.Clear();
        var bar=new StackPanel{Orientation=Orientation.Horizontal,Margin=new Thickness(10)};Btn(bar,"红",()=>{_ink.EditingMode=InkCanvasEditingMode.Ink;_ink.DefaultDrawingAttributes.IsHighlighter=false;_ink.DefaultDrawingAttributes.Color=Colors.Red;});Btn(bar,"蓝",()=>{_ink.EditingMode=InkCanvasEditingMode.Ink;_ink.DefaultDrawingAttributes.IsHighlighter=false;_ink.DefaultDrawingAttributes.Color=Color.FromRgb(49,140,255);});Btn(bar,"高亮",()=>{_ink.EditingMode=InkCanvasEditingMode.Ink;_ink.DefaultDrawingAttributes.IsHighlighter=true;_ink.DefaultDrawingAttributes.Color=Colors.Yellow;_ink.DefaultDrawingAttributes.Width=_ink.DefaultDrawingAttributes.Height=18;});Btn(bar,"橡皮",()=>_ink.EditingMode=InkCanvasEditingMode.EraseByStroke);Btn(bar,"细",()=>_ink.DefaultDrawingAttributes.Width=_ink.DefaultDrawingAttributes.Height=3);Btn(bar,"粗",()=>_ink.DefaultDrawingAttributes.Width=_ink.DefaultDrawingAttributes.Height=9);Btn(bar,"撤销",Undo);Btn(bar,"重做",Redo);Btn(bar,"清空",()=>_ink.Strokes.Clear());Btn(bar,"复制",()=>Clipboard.SetImage(Render()));Btn(bar,"保存",Save);Btn(bar,"贴图",()=>new PinnedImageWindow(Render()).Show());
        var scroll=new ScrollViewer{Content=_ink,HorizontalScrollBarVisibility=ScrollBarVisibility.Auto,VerticalScrollBarVisibility=ScrollBarVisibility.Auto};var grid=new Grid();grid.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});grid.RowDefinitions.Add(new RowDefinition());Grid.SetRow(scroll,1);grid.Children.Add(bar);grid.Children.Add(scroll);Content=grid;
        SourceInitialized+=(_,_)=>NativeMethods.SetWindowDisplayAffinity(new System.Windows.Interop.WindowInteropHelper(this).Handle,NativeMethods.WdaExcludeFromCapture);
    }
    private static void Btn(Panel p,string text,Action action){var b=new Button{Content=text,Margin=new Thickness(3),Padding=new Thickness(10,5,10,5)};b.Click+=(_,_)=>action();p.Children.Add(b);}
    private void Undo(){if(_ink.Strokes.Count==0)return;var s=_ink.Strokes[^1];_ink.Strokes.Remove(s);_redo.Push(s);}
    private void Redo(){if(_redo.TryPop(out var s))_ink.Strokes.Add(s);}
    private BitmapSource Render(){var size=new Size(_ink.ActualWidth,_ink.ActualHeight);_ink.Measure(size);_ink.Arrange(new Rect(size));var bmp=new RenderTargetBitmap(Math.Max(1,(int)size.Width),Math.Max(1,(int)size.Height),96,96,PixelFormats.Pbgra32);bmp.Render(_ink);bmp.Freeze();return bmp;}
    private void Save(){var d=new SaveFileDialog{Filter="PNG 图片|*.png|JPEG 图片|*.jpg;*.jpeg",DefaultExt=".png"};if(d.ShowDialog(this)==true)ScreenCaptureService.Save(Render(),d.FileName,d.FilterIndex==2);}
}
