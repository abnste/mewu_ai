using Microsoft.Win32;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Models;
using Panel=System.Windows.Controls.Panel;
using Button=System.Windows.Controls.Button;
namespace mewu_ai_Assistant.Views;
public sealed class DrawingWindow : Window
{
    private readonly AppHost? _host;private readonly ScreenRect? _region;private readonly BitmapSource _source; private readonly InkCanvas _ink=new(); private readonly Stack<Stroke> _redo=[];private ShapeTool _tool;private Point _shapeStart;private Stroke? _preview;
    public DrawingWindow(BitmapSource source):this(null,source,null){}
    public DrawingWindow(AppHost? host,BitmapSource source,ScreenRect? region)
    {
        _host=host;_source=source;_region=region;Title="喵呜AI 标注";Width=Math.Min(source.PixelWidth+40,1200);Height=Math.Min(source.PixelHeight+100,850);Background=new SolidColorBrush(Color.FromRgb(11,16,24));Foreground=Brushes.White;WindowStartupLocation=WindowStartupLocation.CenterScreen;
        _ink.Width=source.PixelWidth;_ink.Height=source.PixelHeight;_ink.Background=new ImageBrush(source){Stretch=Stretch.Fill};_ink.DefaultDrawingAttributes=new DrawingAttributes{Color=Colors.Red,Width=4,Height=4,FitToCurve=true};_ink.StrokeCollected+=(_,_)=>_redo.Clear();
        var bar=new WrapPanel{Margin=new Thickness(10)};Btn(bar,"画笔",()=>SetTool(ShapeTool.Freehand));Btn(bar,"矩形",()=>SetTool(ShapeTool.Rectangle));Btn(bar,"箭头",()=>SetTool(ShapeTool.Arrow));Btn(bar,"红",()=>{_ink.DefaultDrawingAttributes.IsHighlighter=false;_ink.DefaultDrawingAttributes.Color=Colors.Red;});Btn(bar,"蓝",()=>{_ink.DefaultDrawingAttributes.IsHighlighter=false;_ink.DefaultDrawingAttributes.Color=Color.FromRgb(49,140,255);});Btn(bar,"高亮",()=>{SetTool(ShapeTool.Freehand);_ink.DefaultDrawingAttributes.IsHighlighter=true;_ink.DefaultDrawingAttributes.Color=Colors.Yellow;_ink.DefaultDrawingAttributes.Width=_ink.DefaultDrawingAttributes.Height=18;});Btn(bar,"橡皮",()=>{_tool=ShapeTool.Freehand;_ink.EditingMode=InkCanvasEditingMode.EraseByStroke;});Btn(bar,"细",()=>_ink.DefaultDrawingAttributes.Width=_ink.DefaultDrawingAttributes.Height=3);Btn(bar,"粗",()=>_ink.DefaultDrawingAttributes.Width=_ink.DefaultDrawingAttributes.Height=9);Btn(bar,"撤销",Undo);Btn(bar,"重做",Redo);Btn(bar,"清空",()=>_ink.Strokes.Clear());Btn(bar,"复制",()=>{Clipboard.SetImage(Render());_host?.Notify("已复制带标注的截图");});Btn(bar,"保存",Save);Btn(bar,"贴图",()=>new PinnedImageWindow(Render()).Show());if(_host is not null){Btn(bar,"AI",()=>new AiPromptWindow(_host,Render(),false,_region).Show());Btn(bar,"翻译",()=>new TranslationWindow(_host,Render()).Show());Btn(bar,"文字",()=>new OcrTextWindow(Render()).Show());if(_region is not null)Btn(bar,"录屏",()=>new RecordingControlWindow(_host,_region.Value).Show());}
        var scroll=new ScrollViewer{Content=_ink,HorizontalScrollBarVisibility=ScrollBarVisibility.Auto,VerticalScrollBarVisibility=ScrollBarVisibility.Auto};var grid=new Grid();grid.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});grid.RowDefinitions.Add(new RowDefinition());Grid.SetRow(scroll,1);grid.Children.Add(bar);grid.Children.Add(scroll);Content=grid;
        SourceInitialized+=(_,_)=>NativeMethods.SetWindowDisplayAffinity(new System.Windows.Interop.WindowInteropHelper(this).Handle,NativeMethods.WdaExcludeFromCapture);
        _ink.PreviewMouseLeftButtonDown+=ShapeDown;_ink.PreviewMouseMove+=ShapeMove;_ink.PreviewMouseLeftButtonUp+=ShapeUp;
        KeyDown+=(_,e)=>{if(e.Key!=Key.Escape)return;if(_tool!=ShapeTool.Freehand||_ink.EditingMode!=InkCanvasEditingMode.Ink){SetTool(ShapeTool.Freehand);e.Handled=true;}else Close();};
    }
    private static void Btn(Panel p,string text,Action action){var b=new Button{Content=text,Margin=new Thickness(3),Padding=new Thickness(10,5,10,5)};b.Click+=(_,_)=>action();p.Children.Add(b);}
    private void Undo(){if(_ink.Strokes.Count==0)return;var s=_ink.Strokes[^1];_ink.Strokes.Remove(s);_redo.Push(s);}
    private void Redo(){if(_redo.TryPop(out var s))_ink.Strokes.Add(s);}
    private void SetTool(ShapeTool tool){_tool=tool;_ink.EditingMode=tool==ShapeTool.Freehand?InkCanvasEditingMode.Ink:InkCanvasEditingMode.None;}
    private void ShapeDown(object sender,MouseButtonEventArgs e){if(_tool==ShapeTool.Freehand)return;_shapeStart=e.GetPosition(_ink);_ink.CaptureMouse();e.Handled=true;}
    private void ShapeMove(object sender,MouseEventArgs e){if(_tool==ShapeTool.Freehand||e.LeftButton!=MouseButtonState.Pressed||!_ink.IsMouseCaptured)return;if(_preview is not null)_ink.Strokes.Remove(_preview);_preview=CreateShape(_shapeStart,e.GetPosition(_ink),_tool);_ink.Strokes.Add(_preview);e.Handled=true;}
    private void ShapeUp(object sender,MouseButtonEventArgs e){if(_tool==ShapeTool.Freehand||!_ink.IsMouseCaptured)return;_ink.ReleaseMouseCapture();_preview=null;_redo.Clear();e.Handled=true;}
    private Stroke CreateShape(Point a,Point b,ShapeTool tool){var points=new StylusPointCollection();if(tool==ShapeTool.Rectangle){points.Add(new StylusPoint(a.X,a.Y));points.Add(new StylusPoint(b.X,a.Y));points.Add(new StylusPoint(b.X,b.Y));points.Add(new StylusPoint(a.X,b.Y));points.Add(new StylusPoint(a.X,a.Y));}else{points.Add(new StylusPoint(a.X,a.Y));points.Add(new StylusPoint(b.X,b.Y));var angle=Math.Atan2(b.Y-a.Y,b.X-a.X);var len=Math.Min(24,Math.Max(10,new Vector(b.X-a.X,b.Y-a.Y).Length*.25));points.Add(new StylusPoint(b.X-len*Math.Cos(angle-Math.PI/6),b.Y-len*Math.Sin(angle-Math.PI/6)));points.Add(new StylusPoint(b.X,b.Y));points.Add(new StylusPoint(b.X-len*Math.Cos(angle+Math.PI/6),b.Y-len*Math.Sin(angle+Math.PI/6)));}return new Stroke(points,_ink.DefaultDrawingAttributes.Clone());}
    private BitmapSource Render(){var size=new Size(_ink.ActualWidth,_ink.ActualHeight);_ink.Measure(size);_ink.Arrange(new Rect(size));var bmp=new RenderTargetBitmap(Math.Max(1,(int)size.Width),Math.Max(1,(int)size.Height),96,96,PixelFormats.Pbgra32);bmp.Render(_ink);bmp.Freeze();return bmp;}
    private void Save(){var d=new SaveFileDialog{Filter="PNG 图片|*.png|JPEG 图片|*.jpg;*.jpeg",DefaultExt=".png"};if(d.ShowDialog(this)==true)ScreenCaptureService.Save(Render(),d.FileName,d.FilterIndex==2);}
    private enum ShapeTool{Freehand,Rectangle,Arrow}
}
