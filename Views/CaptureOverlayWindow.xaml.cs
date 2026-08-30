using Microsoft.Win32;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Documents;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Media.Effects;
using System.Windows.Media.Imaging;
using System.Windows.Shapes;
using System.Windows.Threading;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.OCR;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Speech;
using Button=System.Windows.Controls.Button;
using KeyEventArgs=System.Windows.Input.KeyEventArgs;
using MouseEventArgs=System.Windows.Input.MouseEventArgs;
using Point=System.Windows.Point;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow : Window
{
    private static readonly Brush Cyan=new SolidColorBrush(Color.FromRgb(67,198,255));
    private readonly AppHost _host;
    private readonly CaptureFrame _frame;
    private readonly List<SelectionItem> _selections=[];
    private readonly HashSet<SelectionItem> _references=[];
    private List<SelectionItem> _lastSentSelections=[];
    private readonly List<AiMessage> _history=[new("system","分析多张区域图片时只返回 JSON：{answer:string,annotations:[{regionIndex,x,y,width,height,text,type}]}。regionIndex 是从 0 开始的图片序号；坐标是对应图片内 0 到 1 的归一化值。每个重要区域给出简洁批注，不适合批注时 annotations 为空。")];
    private Point _start,_moveStart;
    private Rect _moveOrigin;
    private int _activeIndex=-1;
    private bool _selecting,_moving,_forceNewSelection,_promptBarHidden,_answerExpanded,_reasoningExpanded,_recordingMode,_drawingMode,_recordingPaused,_recordingStopping;
    private CancellationTokenSource? _speechRequest,_request,_overlayRequest;
    private RecordingSession? _recordingSession;
    private SelectionItem? _recordingItem;
    private DateTime _recordingStarted;
    private readonly DispatcherTimer _recordingTimer=new(){Interval=TimeSpan.FromMilliseconds(150)};
    private DrawTool _drawTool=DrawTool.Freehand;
    private Point _drawStart;
    private Stroke? _drawPreview;

    private sealed class SelectionItem
    {
        public Rect Bounds;
        public bool IsImplicit;
        public Grid Host { get; }=new();
        public Image Image { get; }=new(){Stretch=Stretch.Fill,IsHitTestVisible=false};
        public MediaElement Video { get; }=new(){Stretch=Stretch.Fill,LoadedBehavior=MediaState.Manual,UnloadedBehavior=MediaState.Close,Visibility=Visibility.Collapsed,IsHitTestVisible=false};
        public InkCanvas Markup { get; }=new(){Background=Brushes.Transparent,IsHitTestVisible=false};
        public Canvas Annotations { get; }=new(){IsHitTestVisible=false};
        public Canvas TextSelection { get; }=new(){IsHitTestVisible=false,Background=Brushes.Transparent};
        public Border Outline { get; }=new(){Background=Brushes.Transparent,CornerRadius=new CornerRadius(7),IsHitTestVisible=false};
        public Border Badge { get; }=new(){HorizontalAlignment=HorizontalAlignment.Left,VerticalAlignment=VerticalAlignment.Top,Margin=new Thickness(7),CornerRadius=new CornerRadius(10),Background=new SolidColorBrush(Color.FromArgb(230,29,119,224)),Padding=new Thickness(7,2,7,2),IsHitTestVisible=false};
        public TextBlock BadgeText { get; }=new(){Foreground=Brushes.White,FontWeight=FontWeights.SemiBold,FontSize=11};
        public Stack<Stroke> Redo { get; }=[];
        public string? VideoPath;
        public string? FramesDirectory;
        public TimeSpan VideoDuration;
        public bool VideoPlaying;
    }

    private enum DrawTool{Freehand,Rectangle,Arrow}

    private SelectionItem? Active=>_activeIndex>=0&&_activeIndex<_selections.Count?_selections[_activeIndex]:null;

    public CaptureOverlayWindow(AppHost host)
    {
        _host=host;_frame=new ScreenCaptureService().CaptureDesktop(host.Settings.IncludeCaptureCursor);InitializeComponent();
        if(NativeMethods.VisualQaCaptureEnabled)ShowInTaskbar=true;
        DesktopImage.Source=_frame.Image;Dimmer.Fill=new SolidColorBrush(Color.FromArgb((byte)Math.Round(Math.Clamp(host.Settings.OverlayOpacity,.4,.75)*255),0,0,0));
        var area=System.Windows.Forms.SystemInformation.VirtualScreen;Left=area.Left;Top=area.Top;Width=area.Width;Height=area.Height;
        SourceInitialized+=(_,_)=>{var hwnd=new System.Windows.Interop.WindowInteropHelper(this).Handle;NativeMethods.SetWindowPos(hwnd,new IntPtr(-1),area.Left,area.Top,area.Width,area.Height,0x0040);NativeMethods.ExcludeFromCapture(hwnd);};
        Loaded+=(_,_)=>{DesktopImage.Width=Dimmer.Width=SelectionLayer.Width=Root.ActualWidth;DesktopImage.Height=Dimmer.Height=SelectionLayer.Height=Root.ActualHeight;QuickPrompt.TextChanged+=(_,_)=>QuickPromptHint.Visibility=QuickPrompt.Text.Length==0?Visibility.Visible:Visibility.Collapsed;PromptBar.SizeChanged+=(_,_)=>PositionPromptBar();PositionPromptBar();QuickPrompt.Focus();};
        SizeChanged+=(_,_)=>{DesktopImage.Width=Dimmer.Width=SelectionLayer.Width=Root.ActualWidth;DesktopImage.Height=Dimmer.Height=SelectionLayer.Height=Root.ActualHeight;PositionPromptBar();};
        _recordingTimer.Tick+=(_,_)=>RecordingTick();
        Closed+=(_,_)=>{_speechRequest?.Cancel();_request?.Cancel();_overlayRequest?.Cancel();_recordingTimer.Stop();if(_recordingSession is not null){_recordingSession.Stop();_recordingSession.Dispose();}_recordingSession=null;foreach(var item in _selections)item.Video.Close();};
    }

    private void OnMouseDown(object s,MouseButtonEventArgs e)
    {
        if(_recordingMode||_drawingMode)return;
        if(e.OriginalSource is Thumb||IsInside(e.OriginalSource as DependencyObject,PromptBar)||IsInside(e.OriginalSource as DependencyObject,Toolbar)||IsInside(e.OriginalSource as DependencyObject,DrawingToolbar)||IsInside(e.OriginalSource as DependencyObject,RecordingBar)||_selections.Any(item=>IsInside(e.OriginalSource as DependencyObject,item.TextSelection)))return;
        var p=e.GetPosition(Root);var addNew=_forceNewSelection||Keyboard.Modifiers.HasFlag(ModifierKeys.Shift);_forceNewSelection=false;
        var hit=addNew?-1:FindSelection(p);
        if(hit>=0){Select(hit);_moving=true;_moveStart=p;_moveOrigin=Active!.Bounds;}
        else{var item=CreateSelection(false);_selections.Add(item);_activeIndex=_selections.Count-1;RefreshSelectionNumbers();_selecting=true;_start=p;item.Bounds=new Rect(p,p);}
        Toolbar.Visibility=Visibility.Collapsed;SetPromptBarHidden(true);Root.CaptureMouse();e.Handled=true;
    }

    private void OnMouseMove(object s,MouseEventArgs e)
    {
        if(_recordingMode||_drawingMode)return;
        var p=e.GetPosition(Root);
        if(_selecting&&Active is { } created){created.Bounds=Normalize(new Rect(_start,p));UpdateSelection(created);}
        else if(_moving&&Active is { } moved){var d=p-_moveStart;moved.Bounds=ClampSelection(new Rect(_moveOrigin.X+d.X,_moveOrigin.Y+d.Y,_moveOrigin.Width,_moveOrigin.Height));UpdateSelection(moved);}
        else{SetPromptBarHidden(PointerOverSelection(p));return;}
    }

    private void OnMouseUp(object s,MouseButtonEventArgs e)
    {
        if(_recordingMode||_drawingMode)return;
        if(!_selecting&&!_moving)return;_selecting=_moving=false;Root.ReleaseMouseCapture();
        if(Active is not { } item||item.Bounds.Width<8||item.Bounds.Height<8){RemoveActiveSelection(false);if(Active is not null)ShowToolbar();SetPromptBarHidden(false);return;}
        UpdateSelection(item);ShowToolbar();SetPromptBarHidden(PointerOverSelection(e.GetPosition(Root)));PromptStatus.Text=$"已选择 {_selections.Count} 个区域 · 可继续拖动添加";e.Handled=true;
    }

    private SelectionItem CreateSelection(bool implicitFullScreen)
    {
        var item=new SelectionItem{IsImplicit=implicitFullScreen};item.Badge.Child=item.BadgeText;item.Markup.DefaultDrawingAttributes=new DrawingAttributes{Color=Colors.Red,Width=4,Height=4,FitToCurve=true};item.Markup.StrokeCollected+=(_,_)=>item.Redo.Clear();item.Markup.PreviewMouseLeftButtonDown+=MarkupDown;item.Markup.PreviewMouseMove+=MarkupMove;item.Markup.PreviewMouseLeftButtonUp+=MarkupUp;item.Video.MediaEnded+=(_,_)=>{item.Video.Position=TimeSpan.Zero;if(item.VideoPlaying)item.Video.Play();};item.Host.Children.Add(item.Image);item.Host.Children.Add(item.Video);item.Host.Children.Add(item.Markup);item.Host.Children.Add(item.Annotations);item.Host.Children.Add(item.TextSelection);item.Host.Children.Add(item.Outline);item.Host.Children.Add(item.Badge);SelectionLayer.Children.Add(item.Host);return item;
    }

    private void UpdateSelection(SelectionItem item)
    {
        var r=Normalize(item.Bounds);item.Bounds=r;Canvas.SetLeft(item.Host,r.Left);Canvas.SetTop(item.Host,r.Top);item.Host.Width=r.Width;item.Host.Height=r.Height;item.Markup.Width=item.Annotations.Width=item.TextSelection.Width=r.Width;item.Markup.Height=item.Annotations.Height=item.TextSelection.Height=r.Height;
        var px=ToPixelRect(r);if(px.Width>0&&px.Height>0&&item.VideoPath is null)item.Image.Source=ScreenCaptureService.Crop(_frame.Image,px);
        var active=ReferenceEquals(item,Active);var referenced=_references.Contains(item);item.Outline.BorderBrush=item.IsImplicit?Brushes.Transparent:active?Cyan:referenced?new SolidColorBrush(Color.FromRgb(102,112,235)):new SolidColorBrush(Color.FromArgb(185,67,168,255));item.Outline.BorderThickness=new Thickness(active?2.5:referenced?2:1.5);item.Outline.Effect=active&&!item.IsImplicit?new DropShadowEffect{Color=Color.FromRgb(39,157,255),BlurRadius=18,ShadowDepth=0,Opacity=.85}:null;item.Badge.Background=new SolidColorBrush(referenced?Color.FromArgb(238,91,101,226):Color.FromArgb(230,29,119,224));item.Badge.Visibility=item.IsImplicit?Visibility.Collapsed:Visibility.Visible;
        if(active&&!item.IsImplicit){SizeText.Text=item.VideoPath is null?$"{px.Width} × {px.Height}":$"视频 · {item.VideoDuration:mm\\:ss}";SizeText.Visibility=Visibility.Visible;Canvas.SetLeft(SizeText,r.Left);Canvas.SetTop(SizeText,Math.Max(0,r.Top-30));PositionHandles(r);}else if(item.IsImplicit){HideHandles();SizeText.Visibility=Visibility.Collapsed;}
    }

    private void Select(int index){_activeIndex=index;for(var i=0;i<_selections.Count;i++)UpdateSelection(_selections[i]);}
    private int FindSelection(Point p){for(var i=_selections.Count-1;i>=0;i--)if(!_selections[i].IsImplicit&&_selections[i].Bounds.Contains(p))return i;return -1;}
    private bool PointerOverSelection(Point p)=>_selections.Any(x=>!x.IsImplicit&&x.Bounds.Contains(p));
    private static bool IsInside(DependencyObject? source,DependencyObject parent){while(source is not null){if(ReferenceEquals(source,parent))return true;source=VisualTreeHelper.GetParent(source);}return false;}
    private Int32Rect ToPixelRect(Rect r)=>ScreenCoordinateService.ToPixelRect(r,Root.ActualWidth,Root.ActualHeight,_frame.Image.PixelWidth,_frame.Image.PixelHeight);
    private static Rect Normalize(Rect r)=>new(Math.Min(r.Left,r.Right),Math.Min(r.Top,r.Bottom),Math.Abs(r.Width),Math.Abs(r.Height));
    private Rect ClampSelection(Rect value){var width=Math.Min(value.Width,Root.ActualWidth);var height=Math.Min(value.Height,Root.ActualHeight);return new Rect(Math.Clamp(value.X,0,Math.Max(0,Root.ActualWidth-width)),Math.Clamp(value.Y,0,Math.Max(0,Root.ActualHeight-height)),width,height);}
    private Rect MonitorBounds(Rect selection){var pixels=ToPixelRect(selection);var center=new System.Drawing.Point(_frame.OriginX+pixels.X+pixels.Width/2,_frame.OriginY+pixels.Y+pixels.Height/2);var bounds=System.Windows.Forms.Screen.FromPoint(center).Bounds;var sx=Root.ActualWidth/_frame.Image.PixelWidth;var sy=Root.ActualHeight/_frame.Image.PixelHeight;return new Rect((bounds.Left-_frame.OriginX)*sx,(bounds.Top-_frame.OriginY)*sy,bounds.Width*sx,bounds.Height*sy);}

    private void ShowToolbar()
    {
        if(Active is not {IsImplicit:false} item||_recordingMode){Toolbar.Visibility=Visibility.Collapsed;return;}
        var regionNumber=_activeIndex+1;var type=item.VideoPath is null?"区域":"视频";ReferenceButton.ToolTip=_references.Contains(item)?$"{type}{regionNumber} 已引用；可在输入框移除":$"引用当前{type}为 @{type}{regionNumber}";ReferenceButton.Background=new SolidColorBrush(_references.Contains(item)?Color.FromRgb(218,239,231):Color.FromRgb(233,237,255));
        DrawButton.Visibility=item.VideoPath is null?Visibility.Visible:Visibility.Collapsed;RecordButton.Visibility=item.VideoPath is null?Visibility.Visible:Visibility.Collapsed;VideoPlayButton.Visibility=item.VideoPath is null?Visibility.Collapsed:Visibility.Visible;PinButton.ToolTip=item.VideoPath is null?"贴图 (P)":"贴视频 (P)";CopyButton.ToolTip=item.VideoPath is null?"复制图片 (C)":"复制视频文件 (C)";SaveButton.ToolTip=item.VideoPath is null?"保存图片 (S)":"保存 MP4 (S)";
        Toolbar.Visibility=Visibility.Visible;PositionFloatingBar(Toolbar,item);
    }

    private void PositionFloatingBar(FrameworkElement bar,SelectionItem item)
    {
        bar.Measure(new Size(double.PositiveInfinity,double.PositiveInfinity));var w=bar.DesiredSize.Width;var h=bar.DesiredSize.Height;var monitor=MonitorBounds(item.Bounds);var x=Math.Clamp(item.Bounds.Left,monitor.Left+8,Math.Max(monitor.Left+8,monitor.Right-w-8));var promptTop=Canvas.GetTop(PromptBar);var availableBottom=_promptBarHidden||PromptBar.Visibility!=Visibility.Visible?monitor.Bottom-8:Math.Min(monitor.Bottom-8,promptTop-8);var below=item.Bounds.Bottom+10;var above=item.Bounds.Top-h-10;var y=below+h<=availableBottom?below:above>=monitor.Top+8?above:Math.Clamp(below,monitor.Top+8,Math.Max(monitor.Top+8,availableBottom-h));Canvas.SetLeft(bar,x);Canvas.SetTop(bar,y);
        if(ReferenceEquals(bar,Toolbar)&&SizeText.Visibility==Visibility.Visible){SizeText.Measure(new Size(double.PositiveInfinity,double.PositiveInfinity));var sizeHeight=SizeText.DesiredSize.Height;var preferred=y<item.Bounds.Top?y-sizeHeight-4:item.Bounds.Top-sizeHeight-4;var sizeY=preferred>=monitor.Top+4?preferred:Math.Min(item.Bounds.Bottom-sizeHeight-4,item.Bounds.Top+4);Canvas.SetLeft(SizeText,item.Bounds.Left);Canvas.SetTop(SizeText,sizeY);}
    }

    private void PositionPromptBar(){if(Root.ActualWidth<=0||Root.ActualHeight<=0)return;PromptBar.Measure(new Size(double.PositiveInfinity,double.PositiveInfinity));var w=Math.Min(PromptBar.DesiredSize.Width,Math.Max(320,Root.ActualWidth-32));PromptBar.Width=w;Canvas.SetLeft(PromptBar,(Root.ActualWidth-w)/2);Canvas.SetTop(PromptBar,Math.Max(16,Root.ActualHeight-PromptBar.ActualHeight-24));if(Toolbar.Visibility==Visibility.Visible)ShowToolbar();if(DrawingToolbar.Visibility==Visibility.Visible&&Active is { } item)PositionFloatingBar(DrawingToolbar,item);}
    private void SetPromptBarHidden(bool hidden){if(_promptBarHidden==hidden)return;_promptBarHidden=hidden;PromptBar.IsHitTestVisible=!hidden;if(PromptBar.RenderTransform is not TranslateTransform transform){transform=new TranslateTransform();PromptBar.RenderTransform=transform;}var ease=new CubicEase{EasingMode=EasingMode.EaseOut};transform.BeginAnimation(TranslateTransform.YProperty,new DoubleAnimation(hidden?PromptBar.ActualHeight+32:0,TimeSpan.FromMilliseconds(hidden?150:190)){EasingFunction=ease});PromptBar.BeginAnimation(OpacityProperty,new DoubleAnimation(hidden?.08:.98,TimeSpan.FromMilliseconds(150)));if(Toolbar.Visibility==Visibility.Visible)ShowToolbar();}
    private void ShowAnswer(){if(_answerExpanded)return;_answerExpanded=true;AnswerHeader.Visibility=AnswerScroll.Visibility=AnswerDivider.Visibility=Visibility.Visible;_ = Dispatcher.BeginInvoke(PositionPromptBar);}
    private void ToggleReasoning(object s,RoutedEventArgs e){_reasoningExpanded=!_reasoningExpanded;ReasoningPanel.Visibility=_reasoningExpanded?Visibility.Visible:Visibility.Collapsed;ReasoningChevron.Text=_reasoningExpanded?"⌃":"⌄";_ = Dispatcher.BeginInvoke(PositionPromptBar);}
    private void ShowReasoning(string delta)
    {
        if(ReasoningToggle.Visibility!=Visibility.Visible){ReasoningToggle.Visibility=Visibility.Visible;_reasoningExpanded=true;ReasoningPanel.Visibility=Visibility.Visible;ReasoningChevron.Text="⌃";ReasoningLabel.Text="正在思考…";ReasoningPulse.BeginAnimation(OpacityProperty,new DoubleAnimation(.35,1,TimeSpan.FromMilliseconds(650)){AutoReverse=true,RepeatBehavior=RepeatBehavior.Forever});}
        ReasoningText.Text+=delta;ReasoningText.GetBindingExpression(TextBlock.TextProperty)?.UpdateTarget();_ = Dispatcher.BeginInvoke(PositionPromptBar);
    }
    private void FinishReasoning(string reasoning)
    {
        if(!string.IsNullOrWhiteSpace(reasoning))ReasoningText.Text=reasoning.Trim();CloseReasoning("思考过程 · 已完成",Color.FromRgb(95,181,137));
    }
    private void CloseReasoning(string label,Color color)
    {
        if(ReasoningText.Text.Length==0)return;ReasoningToggle.Visibility=Visibility.Visible;ReasoningLabel.Text=label;ReasoningPulse.BeginAnimation(OpacityProperty,null);ReasoningPulse.Opacity=1;ReasoningPulse.Background=new SolidColorBrush(color);_reasoningExpanded=false;ReasoningPanel.Visibility=Visibility.Collapsed;ReasoningChevron.Text="⌄";_ = Dispatcher.BeginInvoke(PositionPromptBar);
    }

    private BitmapSource CurrentImage(){if(Active is null)throw new InvalidOperationException("请先选择区域");return RenderSelectionImage(Active);}
    private BitmapSource RenderSelectionImage(SelectionItem item)
    {
        var pixels=ToPixelRect(item.Bounds);var source=ScreenCaptureService.Crop(_frame.Image,pixels);if(item.Markup.Strokes.Count==0)return source;var visual=new DrawingVisual();using(var drawing=visual.RenderOpen()){drawing.PushTransform(new ScaleTransform(pixels.Width/Math.Max(1,item.Bounds.Width),pixels.Height/Math.Max(1,item.Bounds.Height)));drawing.DrawImage(source,new Rect(0,0,item.Bounds.Width,item.Bounds.Height));drawing.DrawRectangle(new VisualBrush(item.Markup),null,new Rect(0,0,item.Bounds.Width,item.Bounds.Height));drawing.Pop();}var bitmap=new RenderTargetBitmap(Math.Max(1,pixels.Width),Math.Max(1,pixels.Height),96,96,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();return bitmap;
    }
    private List<AiAttachment> BuildAttachments()
    {
        EnsureScreenSelection();_lastSentSelections=(_references.Count>0?_selections.Where(_references.Contains):_selections).ToList();return _lastSentSelections.Select(item=>item.VideoPath is { } path?new AiAttachment(AiAttachmentType.Video,"video/mp4",FilePath:path,Duration:item.VideoDuration):new AiAttachment(AiAttachmentType.Image,"image/png",ScreenCaptureService.EncodePng(RenderSelectionImage(item)))).ToList();
    }
    private void EnsureScreenSelection(){if(_selections.Count>0)return;var item=CreateSelection(true);item.Bounds=new Rect(0,0,Root.ActualWidth,Root.ActualHeight);_selections.Add(item);_activeIndex=0;RefreshSelectionNumbers();UpdateSelection(item);}

    private void PositionHandles(Rect r){var list=new[]{Nw,N,Ne,W,E,Sw,S,Se};foreach(var t in list){t.Width=t.Height=10;t.Background=Cyan;t.Visibility=Visibility.Visible;}Set(Nw,r.Left,r.Top);Set(N,r.Left+r.Width/2,r.Top);Set(Ne,r.Right,r.Top);Set(W,r.Left,r.Top+r.Height/2);Set(E,r.Right,r.Top+r.Height/2);Set(Sw,r.Left,r.Bottom);Set(S,r.Left+r.Width/2,r.Bottom);Set(Se,r.Right,r.Bottom);static void Set(Thumb t,double x,double y){Canvas.SetLeft(t,x-5);Canvas.SetTop(t,y-5);}}
    private void HideHandles(){foreach(var t in new[]{Nw,N,Ne,W,E,Sw,S,Se})t.Visibility=Visibility.Collapsed;}
    private void ResizeDelta(object sender,DragDeltaEventArgs e){if(sender is not Thumb t||Active is not {IsImplicit:false} item)return;SetPromptBarHidden(true);var d=t.Tag?.ToString()??"";var l=item.Bounds.Left;var top=item.Bounds.Top;var r=item.Bounds.Right;var b=item.Bounds.Bottom;if(d.Contains('W'))l=Math.Clamp(l+e.HorizontalChange,0,r-12);if(d.Contains('E'))r=Math.Clamp(r+e.HorizontalChange,l+12,Root.ActualWidth);if(d.Contains('N'))top=Math.Clamp(top+e.VerticalChange,0,b-12);if(d.Contains('S'))b=Math.Clamp(b+e.VerticalChange,top+12,Root.ActualHeight);item.Bounds=new Rect(new Point(l,top),new Point(r,b));item.Annotations.Children.Clear();ClearTextSelection(item);UpdateSelection(item);ShowToolbar();e.Handled=true;}

    private void AddRegion(object s,RoutedEventArgs e){_forceNewSelection=true;Toolbar.Visibility=Visibility.Collapsed;HideHandles();PromptStatus.Text="拖动以添加另一个区域 · 可与现有区域重叠";SetPromptBarHidden(false);}
    private void ReferenceRegion(object s,RoutedEventArgs e)
    {
        if(Active is not {IsImplicit:false} item)return;_references.Add(item);UpdateReferenceChips();UpdateSelection(item);ShowToolbar();SetPromptBarHidden(false);QuickPrompt.Focus();PromptStatus.Text=$"已加入 @{(item.VideoPath is null?"区域":"视频")}{_activeIndex+1} · 输入问题后发送";
    }
    private void RemoveSelection(object s,RoutedEventArgs e)=>RemoveActiveSelection(true);
    private void RemoveActiveSelection(bool updateUi)
    {
        if(Active is not { } item)return;item.Video.Close();_references.Remove(item);SelectionLayer.Children.Remove(item.Host);_selections.RemoveAt(_activeIndex);_activeIndex=_selections.Count-1;RefreshSelectionNumbers();if(Active is { } next)UpdateSelection(next);else{HideHandles();SizeText.Visibility=Toolbar.Visibility=Visibility.Collapsed;}if(updateUi){PromptStatus.Text=_selections.Count==0?"拖动可连续框选多个区域":$"剩余 {_selections.Count} 个区域";if(Active is not null)ShowToolbar();}
    }
    private void RefreshSelectionNumbers(){for(var i=0;i<_selections.Count;i++)_selections[i].BadgeText.Text=(i+1).ToString();UpdateReferenceChips();}
    private void UpdateReferenceChips()
    {
        ReferenceChips.Children.Clear();
        foreach(var item in _selections.Where(_references.Contains))
        {
            var chip=new Border{Background=new SolidColorBrush(Color.FromRgb(231,236,255)),BorderBrush=new SolidColorBrush(Color.FromRgb(205,214,252)),BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(9),Margin=new Thickness(0,0,5,3)};var row=new StackPanel{Orientation=Orientation.Horizontal};var type=item.VideoPath is null?"区域":"视频";var link=new Button{Content=$"@{type}{_selections.IndexOf(item)+1}",ToolTip=$"定位到此{type}"};link.SetResourceReference(StyleProperty,"ReferenceChipButton");link.Click+=(_,_)=>{var index=_selections.IndexOf(item);if(index<0)return;Select(index);ShowToolbar();SetPromptBarHidden(false);QuickPrompt.Focus();};var remove=new Button{Content="×",ToolTip="移除此引用"};remove.SetResourceReference(StyleProperty,"ReferenceChipRemoveButton");remove.Click+=(_,_)=>{_references.Remove(item);UpdateReferenceChips();UpdateSelection(item);if(ReferenceEquals(item,Active))ShowToolbar();QuickPrompt.Focus();};row.Children.Add(link);row.Children.Add(remove);chip.Child=row;ReferenceChips.Children.Add(chip);
        }
        ReferenceChips.Visibility=ReferenceChips.Children.Count>0?Visibility.Visible:Visibility.Collapsed;QuickPromptHint.Text=ReferenceChips.Children.Count>0?"继续输入关于引用区域的问题…":"询问当前屏幕，或连续圈选多个区域…";_ = Dispatcher.BeginInvoke(PositionPromptBar);
    }

    private async Task SendAsync(bool useDefaultPrompt)
    {
        if(_request is {IsCancellationRequested:false})return;var provider=new AiProviderFactory().Create(_host.Settings);if(provider is null){PromptStatus.Text="请先配置可用的 AI Provider";_host.ShowSettings();return;}
        EnsureScreenSelection();var targets=(_references.Count>0?_selections.Where(_references.Contains):_selections).ToList();var hasVideo=targets.Any(x=>x.VideoPath is not null);var hasImage=targets.Any(x=>x.VideoPath is null);if(hasVideo&&!provider.Capabilities.SupportsVideo){PromptStatus.Text="当前 Provider 未开启视频理解能力";return;}if(hasImage&&!provider.Capabilities.SupportsImage){PromptStatus.Text="当前模型不支持图片理解";return;}
        var targetCount=targets.Count;var prompt=QuickPrompt.Text.Trim();if(prompt.Length==0&&useDefaultPrompt)prompt=hasVideo?"按时间顺序说明引用视频中发生了什么，包括主体、动作和画面变化。":targetCount>1?"综合理解这些引用区域，说明它们之间的关系并标出关键部分。":"理解当前引用区域，解释内容并标出关键部分。";if(prompt.Length==0){QuickPrompt.Focus();return;}
        var request=new CancellationTokenSource(TimeSpan.FromMinutes(2));_request=request;SendButton.IsEnabled=false;AnswerText.Text="";ReasoningText.Text="";ReasoningToggle.Visibility=ReasoningPanel.Visibility=Visibility.Collapsed;ReasoningPulse.Background=new SolidColorBrush(Color.FromRgb(123,138,244));_reasoningExpanded=false;PromptStatus.Text=$"正在分析 {targetCount} 个引用区域…按 Esc 可取消";var streamOpen=true;
        try
        {
            var progress=provider.Capabilities.SupportsStreaming?new Progress<AiStreamDelta>(delta=>{if(!streamOpen||!ReferenceEquals(_request,request))return;if(delta.ReasoningContent.Length>0)ShowReasoning(delta.ReasoningContent);if(delta.Content.Length>0)PromptStatus.Text="正在整理回答…";}):null;
            var result=await provider.SendAsync(new AiRequest{Prompt=prompt,History=[.._history],Attachments=BuildAttachments(),StreamingProgress=progress},request.Token);streamOpen=false;if(!ReferenceEquals(_request,request))return;FinishReasoning(result.Reasoning);var emptyAnswer=AiResultValidation.GetEmptyAnswerMessage(result);if(emptyAnswer is not null){ShowAnswer();AnswerText.Text=emptyAnswer;PromptStatus.Text=emptyAnswer;return;}ShowAnswer();AnswerText.Text=result.Answer;_history.Add(new("user",prompt));_history.Add(new("assistant",result.Answer));QuickPrompt.Clear();RenderAnnotations(result.Annotations);
            var configured=_host.Settings.Providers.FirstOrDefault(x=>x.Id==provider.Id);if(_host.Settings.SaveConversationHistory)await new ConversationHistoryService().AppendAsync(configured?.Name??provider.Id,configured?.Model??"",prompt,result.Answer,request.Token);PromptStatus.Text=hasVideo?"视频理解完成 · 可继续提问":result.Annotations.Count>0?$"已在 {_lastSentSelections.Count} 个引用区域中标出重点 · 可继续提问":"完成 · 可继续提问";
        }
        catch(OperationCanceledException){if(ReferenceEquals(_request,request)){CloseReasoning("思考过程 · 已取消",Color.FromRgb(142,153,169));PromptStatus.Text="已取消";}}
        catch(Exception ex){if(ReferenceEquals(_request,request)){CloseReasoning("思考过程 · 已中止",Color.FromRgb(213,91,104));ShowAnswer();AnswerText.Text="请求失败";PromptStatus.Text=ex.Message;}}
        finally{streamOpen=false;request.Dispose();if(ReferenceEquals(_request,request)){_request=null;SendButton.IsEnabled=true;_ = Dispatcher.BeginInvoke(PositionPromptBar);}}
    }

    private void RenderAnnotations(IReadOnlyList<AiAnnotation> notes)
    {
        foreach(var item in _selections){item.Annotations.Children.Clear();ClearTextSelection(item);}
        for(var regionIndex=0;regionIndex<_lastSentSelections.Count;regionIndex++)
        {
            var item=_lastSentSelections[regionIndex];var w=item.Bounds.Width;var h=item.Bounds.Height;var cardWidth=Math.Clamp(w*.3,145,360);var font=Math.Clamp(w/70,11,22);var slots=new List<double>();
            foreach(var n in notes.Where(x=>x.RegionIndex==regionIndex).Take(6))
            {
                var x=Math.Clamp(n.X,0,1)*w;var y=Math.Clamp(n.Y,0,1)*h;var rw=Math.Max(14,Math.Clamp(n.Width,0,1)*w);var rh=Math.Max(14,Math.Clamp(n.Height,0,1)*h);var box=new Border{Width=rw,Height=rh,CornerRadius=new CornerRadius(5),BorderBrush=Cyan,BorderThickness=new Thickness(Math.Max(1.5,w/900)),Background=new SolidColorBrush(Color.FromArgb(14,55,170,255)),Effect=new DropShadowEffect{Color=Color.FromRgb(34,169,255),BlurRadius=13,ShadowDepth=0,Opacity=.9}};Canvas.SetLeft(box,x);Canvas.SetTop(box,y);item.Annotations.Children.Add(box);
                var right=x+rw+cardWidth+28<w;var cardX=right?x+rw+24:Math.Max(5,x-cardWidth-24);var cardY=AnnotationLayoutService.FindCardTop(y+rh*.5-font*1.5,5,Math.Max(5,h-font*4),font*3.2,slots);slots.Add(cardY);var startX=right?x+rw:x;var endX=right?cardX:cardX+cardWidth;item.Annotations.Children.Add(new Line{X1=startX,Y1=y+rh*.5,X2=endX,Y2=cardY+font*1.4,Stroke=Cyan,StrokeThickness=Math.Max(1,w/1200)});var dot=new Ellipse{Width=5,Height=5,Fill=Cyan};Canvas.SetLeft(dot,endX-2.5);Canvas.SetTop(dot,cardY+font*1.4-2.5);item.Annotations.Children.Add(dot);var card=new Border{Width=cardWidth,Padding=new Thickness(font*.65,font*.5,font*.65,font*.5),CornerRadius=new CornerRadius(8),Background=new SolidColorBrush(Color.FromArgb(238,8,18,31)),BorderBrush=new SolidColorBrush(Color.FromArgb(125,61,190,255)),BorderThickness=new Thickness(1),Child=new TextBlock{Text=n.Text,Foreground=Brushes.White,FontSize=font,TextWrapping=TextWrapping.Wrap,LineHeight=font*1.3},Effect=new DropShadowEffect{Color=Colors.Black,BlurRadius=15,ShadowDepth=4,Opacity=.7}};Canvas.SetLeft(card,cardX);Canvas.SetTop(card,cardY);item.Annotations.Children.Add(card);
            }
        }
    }

    private void Copy(object s,RoutedEventArgs e)
    {
        if(Active is not { } item)return;if(item.VideoPath is { } video){var files=new System.Collections.Specialized.StringCollection{video};var data=new DataObject();data.SetFileDropList(files);Clipboard.SetDataObject(data,true);PromptStatus.Text="视频文件已复制";}else{Clipboard.SetImage(RenderSelectionImage(item));PromptStatus.Text="图片已复制";}SetPromptBarHidden(false);
    }
    private void Save(object s,RoutedEventArgs e)
    {
        if(Active is not { } item)return;if(item.VideoPath is { } video){var dialog=new SaveFileDialog{Filter="MP4 视频|*.mp4",DefaultExt=".mp4"};if(dialog.ShowDialog(this)==true){File.Copy(video,dialog.FileName,true);PromptStatus.Text="MP4 已保存";}return;}var jpeg=_host.Settings.DefaultImageFormat.Equals("jpg",StringComparison.OrdinalIgnoreCase)||_host.Settings.DefaultImageFormat.Equals("jpeg",StringComparison.OrdinalIgnoreCase);var imageDialog=new SaveFileDialog{Filter="PNG 图片|*.png|JPEG 图片|*.jpg;*.jpeg",DefaultExt=jpeg?".jpg":".png",FilterIndex=jpeg?2:1,AddExtension=true};if(imageDialog.ShowDialog(this)==true){ScreenCaptureService.Save(RenderSelectionImage(item),imageDialog.FileName,imageDialog.FilterIndex==2);PromptStatus.Text="图片已保存";}
    }
    private void Pin(object s,RoutedEventArgs e)
    {
        if(Active is not { } item)return;var pixels=ToPixelRect(item.Bounds);var region=ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY);if(item.VideoPath is { } video)new PinnedVideoWindow(video,region).Show();else new PinnedImageWindow(RenderSelectionImage(item),region).Show();PromptStatus.Text=item.VideoPath is null?"已在原位贴图":"已在原位贴视频";SetPromptBarHidden(false);
    }
    private void Draw(object s,RoutedEventArgs e)=>EnterDrawingMode();
    private void EnterDrawingMode()
    {
        if(Active is not {IsImplicit:false,VideoPath:null} item)return;_drawingMode=true;Toolbar.Visibility=Visibility.Collapsed;HideHandles();SizeText.Visibility=Visibility.Collapsed;item.Markup.IsHitTestVisible=true;SetDrawTool(DrawTool.Freehand);DrawingToolbar.Visibility=Visibility.Visible;PositionFloatingBar(DrawingToolbar,item);SetPromptBarHidden(true);PromptStatus.Text="原位标注中 · Esc 或 ✓ 完成";
    }
    private void ExitDrawingMode()
    {
        if(Active is { } item)item.Markup.IsHitTestVisible=false;_drawingMode=false;DrawingToolbar.Visibility=Visibility.Collapsed;Cursor=Cursors.Cross;SetPromptBarHidden(false);if(Active is not null){UpdateSelection(Active);ShowToolbar();}PromptStatus.Text="标注已保留在当前区域";
    }
    private void SetDrawTool(DrawTool tool)
    {
        if(Active is not { } item)return;_drawTool=tool;item.Markup.EditingMode=tool==DrawTool.Freehand?InkCanvasEditingMode.Ink:InkCanvasEditingMode.None;Cursor=tool==DrawTool.Freehand?Cursors.Pen:Cursors.Cross;
    }
    private void DrawPen(object s,RoutedEventArgs e){if(Active is { } item){item.Markup.DefaultDrawingAttributes.IsHighlighter=false;SetDrawTool(DrawTool.Freehand);}}
    private void DrawRectangleTool(object s,RoutedEventArgs e)=>SetDrawTool(DrawTool.Rectangle);
    private void DrawArrowTool(object s,RoutedEventArgs e)=>SetDrawTool(DrawTool.Arrow);
    private void DrawRed(object s,RoutedEventArgs e){if(Active is { } item){item.Markup.DefaultDrawingAttributes.IsHighlighter=false;item.Markup.DefaultDrawingAttributes.Color=Colors.Red;SetDrawTool(DrawTool.Freehand);}}
    private void DrawBlue(object s,RoutedEventArgs e){if(Active is { } item){item.Markup.DefaultDrawingAttributes.IsHighlighter=false;item.Markup.DefaultDrawingAttributes.Color=Color.FromRgb(49,140,255);SetDrawTool(DrawTool.Freehand);}}
    private void DrawHighlight(object s,RoutedEventArgs e){if(Active is { } item){item.Markup.DefaultDrawingAttributes.IsHighlighter=true;item.Markup.DefaultDrawingAttributes.Color=Colors.Yellow;item.Markup.DefaultDrawingAttributes.Width=item.Markup.DefaultDrawingAttributes.Height=18;SetDrawTool(DrawTool.Freehand);}}
    private void DrawEraser(object s,RoutedEventArgs e){if(Active is { } item){_drawTool=DrawTool.Freehand;item.Markup.EditingMode=InkCanvasEditingMode.EraseByStroke;Cursor=Cursors.Cross;}}
    private void DrawUndo(object s,RoutedEventArgs e){if(Active is not { } item||item.Markup.Strokes.Count==0)return;var stroke=item.Markup.Strokes[^1];item.Markup.Strokes.Remove(stroke);item.Redo.Push(stroke);}
    private void DrawRedo(object s,RoutedEventArgs e){if(Active is { } item&&item.Redo.TryPop(out var stroke))item.Markup.Strokes.Add(stroke);}
    private void DrawClear(object s,RoutedEventArgs e){if(Active is { } item){item.Markup.Strokes.Clear();item.Redo.Clear();}}
    private void DrawDone(object s,RoutedEventArgs e)=>ExitDrawingMode();
    private void MarkupDown(object sender,MouseButtonEventArgs e)
    {
        if(!_drawingMode||sender is not InkCanvas canvas||!ReferenceEquals(canvas,Active?.Markup)||_drawTool==DrawTool.Freehand)return;_drawStart=e.GetPosition(canvas);canvas.CaptureMouse();e.Handled=true;
    }
    private void MarkupMove(object sender,MouseEventArgs e)
    {
        if(!_drawingMode||sender is not InkCanvas canvas||!ReferenceEquals(canvas,Active?.Markup)||_drawTool==DrawTool.Freehand||e.LeftButton!=MouseButtonState.Pressed||!canvas.IsMouseCaptured)return;if(_drawPreview is not null)canvas.Strokes.Remove(_drawPreview);_drawPreview=CreateShapeStroke(canvas,_drawStart,e.GetPosition(canvas),_drawTool);canvas.Strokes.Add(_drawPreview);e.Handled=true;
    }
    private void MarkupUp(object sender,MouseButtonEventArgs e)
    {
        if(sender is not InkCanvas canvas||_drawTool==DrawTool.Freehand||!canvas.IsMouseCaptured)return;canvas.ReleaseMouseCapture();_drawPreview=null;if(Active is { } item)item.Redo.Clear();e.Handled=true;
    }
    private static Stroke CreateShapeStroke(InkCanvas canvas,Point a,Point b,DrawTool tool)
    {
        var points=new StylusPointCollection();if(tool==DrawTool.Rectangle){points.Add(new StylusPoint(a.X,a.Y));points.Add(new StylusPoint(b.X,a.Y));points.Add(new StylusPoint(b.X,b.Y));points.Add(new StylusPoint(a.X,b.Y));points.Add(new StylusPoint(a.X,a.Y));}else{points.Add(new StylusPoint(a.X,a.Y));points.Add(new StylusPoint(b.X,b.Y));var angle=Math.Atan2(b.Y-a.Y,b.X-a.X);var length=Math.Min(24,Math.Max(10,new Vector(b.X-a.X,b.Y-a.Y).Length*.25));points.Add(new StylusPoint(b.X-length*Math.Cos(angle-Math.PI/6),b.Y-length*Math.Sin(angle-Math.PI/6)));points.Add(new StylusPoint(b.X,b.Y));points.Add(new StylusPoint(b.X-length*Math.Cos(angle+Math.PI/6),b.Y-length*Math.Sin(angle+Math.PI/6)));}var attributes=canvas.DefaultDrawingAttributes.Clone();attributes.FitToCurve=false;return new Stroke(points,attributes);
    }
    private async void QuickSend(object s,RoutedEventArgs e)=>await SendAsync(true);
    private async void QuickPromptKeyDown(object s,KeyEventArgs e){if(e.Key==Key.Enter&&!Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)){e.Handled=true;await SendAsync(true);}}
    private async void QuickVoice(object s,RoutedEventArgs e)
    {
        if(_speechRequest is not null){PromptStatus.Text="正在停止聆听…";_speechRequest.Cancel();return;}var microphone=VoiceIcon.Data;_speechRequest=new();VoiceIcon.Data=Geometry.Parse("M7,7 L17,7 L17,17 L7,17 Z");
        try{PromptStatus.Text="正在聆听…";var text=await new WindowsSpeechToTextService().RecognizeOnceAsync(_host.Settings.VoiceLanguage,_speechRequest.Token);if(!string.IsNullOrWhiteSpace(text))QuickPrompt.Text=string.IsNullOrWhiteSpace(QuickPrompt.Text)?text:QuickPrompt.Text+" "+text;PromptStatus.Text="语音已写入";}
        catch(OperationCanceledException){PromptStatus.Text="已停止聆听";}catch(SpeechRecognitionUnavailableException ex){PromptStatus.Text=ex.Message;}catch(Exception){PromptStatus.Text="语音输入暂时不可用";}finally{_speechRequest.Dispose();_speechRequest=null;VoiceIcon.Data=microphone;}
    }
    private async void Translate(object s,RoutedEventArgs e)
    {
        if(Active is not {IsImplicit:false} item)return;var image=CurrentImage();var operation=BeginOverlayOperation("正在识别文字…按 Esc 可取消");
        try
        {
            var document=await new WindowsOcrService().RecognizeAsync(image,operation.Token);if(!_selections.Contains(item))return;if(document.Lines.Count==0){PromptStatus.Text="当前区域未识别到文字";return;}
            var provider=new AiProviderFactory().Create(_host.Settings);if(provider is null){PromptStatus.Text="翻译需要先配置可用的 AI Provider";_host.ShowSettings();return;}
            PromptStatus.Text=$"已识别 {document.Lines.Count} 行，正在翻译…按 Esc 可取消";
            var prompt="将 translationsSource 中的每一项翻译成简体中文。保持数组长度和顺序完全一致，只返回 JSON：{\"translations\":[\"译文1\",\"译文2\"]}。translationsSource="+System.Text.Json.JsonSerializer.Serialize(document.Lines.Select(line=>line.Text).ToArray());
            using var networkTimeout=CancellationTokenSource.CreateLinkedTokenSource(operation.Token);networkTimeout.CancelAfter(TimeSpan.FromSeconds(75));var received=false;var progress=new Progress<AiStreamDelta>(delta=>{if(!received&&(delta.Content.Length>0||delta.ReasoningContent.Length>0)){received=true;PromptStatus.Text="正在接收译文…按 Esc 可取消";}});
            var result=await provider.SendAsync(new AiRequest{Prompt=prompt,StreamingProgress=progress,StreamingCompletionPredicate=value=>TranslationResponseParser.TryParse(value,document.Lines.Count,out _),DisableReasoning=true,MaxOutputTokens=2048},networkTimeout.Token);if(!_selections.Contains(item))return;if(!TranslationResponseParser.TryParse(result.Answer,document.Lines.Count,out var translated)){PromptStatus.Text="翻译结果格式异常，请重试";return;}
            RenderTextOverlays(item,image,document.Lines,translated,true);PromptStatus.Text=$"已在原位翻译 {translated.Count} 行";
        }
        catch(OperationCanceledException){PromptStatus.Text=operation.IsCancellationRequested?"已取消翻译":"翻译超时，请检查 Provider 后重试";}catch(Exception ex){PromptStatus.Text=$"翻译失败：{ex.Message}";}finally{EndOverlayOperation(operation);}
    }

    private async void Ocr(object s,RoutedEventArgs e)
    {
        if(Active is not {IsImplicit:false} item)return;var image=CurrentImage();var operation=BeginOverlayOperation("正在本地识别当前区域…");
        try
        {
            var document=await new WindowsOcrService().RecognizeAsync(image,operation.Token);if(!_selections.Contains(item))return;RenderSelectableText(item,image,document);PromptStatus.Text=document.Lines.Count==0?"当前区域未识别到文字":$"{document.Engine} 已识别 {document.Lines.Count} 行 · 可直接拖选并按 Ctrl+C";
        }
        catch(OperationCanceledException){}catch(Exception ex){PromptStatus.Text=$"OCR 失败：{ex.Message}";}finally{EndOverlayOperation(operation);}
    }

    private CancellationTokenSource BeginOverlayOperation(string status){_overlayRequest?.Cancel();var operation=new CancellationTokenSource(TimeSpan.FromMinutes(2));_overlayRequest=operation;PromptStatus.Text=status;Toolbar.IsEnabled=false;return operation;}
    private void EndOverlayOperation(CancellationTokenSource operation){operation.Dispose();if(!ReferenceEquals(_overlayRequest,operation))return;_overlayRequest=null;Toolbar.IsEnabled=true;if(Active is not null)ShowToolbar();}
    private static void RenderTextOverlays(SelectionItem item,BitmapSource image,IReadOnlyList<OcrLine> lines,IReadOnlyList<string> texts,bool translated)
    {
        ClearTextSelection(item);item.Annotations.Children.Clear();var scaleX=item.Bounds.Width/image.PixelWidth;var scaleY=item.Bounds.Height/image.PixelHeight;
        for(var index=0;index<lines.Count&&index<texts.Count;index++)
        {
            var line=lines[index];var text=new TextBlock{Text=texts[index],Foreground=new SolidColorBrush(Color.FromRgb(35,47,67)),FontSize=Math.Clamp(line.Height*scaleY*.72,10,26),FontWeight=translated?FontWeights.SemiBold:FontWeights.Normal,TextWrapping=TextWrapping.Wrap,LineHeight=Math.Clamp(line.Height*scaleY*.9,14,32)};
            var box=new Border{Child=text,Background=new SolidColorBrush(translated?Color.FromArgb(242,239,242,255):Color.FromArgb(228,248,251,255)),BorderBrush=new SolidColorBrush(translated?Color.FromRgb(111,124,245):Color.FromRgb(78,164,224)),BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(5),Padding=new Thickness(3,1,3,1),Width=Math.Max(36,line.Width*scaleX),MinHeight=Math.Max(18,line.Height*scaleY),ToolTip=translated?"原位译文":"本地 OCR 文字"};
            Canvas.SetLeft(box,line.X*scaleX);Canvas.SetTop(box,line.Y*scaleY);item.Annotations.Children.Add(box);
        }
    }
    private void RenderSelectableText(SelectionItem item,BitmapSource image,OcrDocument document)
    {
        item.Annotations.Children.Clear();ClearTextSelection(item);var scaleX=item.Bounds.Width/image.PixelWidth;var scaleY=item.Bounds.Height/image.PixelHeight;item.TextSelection.IsHitTestVisible=true;
        var flow=new FlowDocument{PagePadding=new Thickness(0),ColumnGap=0,FontFamily=new FontFamily("Segoe UI")};double previousBottom=0;
        foreach(var line in document.Lines.OrderBy(x=>x.Y))
        {
            var top=line.Y*scaleY;var height=Math.Max(14,line.Height*scaleY);var paragraph=new Paragraph(new Run(line.Text)){Margin=new Thickness(Math.Max(0,line.X*scaleX),Math.Max(0,top-previousBottom),0,0),Padding=new Thickness(0),FontSize=Math.Clamp(height*.82,10,30),LineHeight=height,Foreground=new SolidColorBrush(Color.FromArgb(1,0,0,0)),TextAlignment=TextAlignment.Left};flow.Blocks.Add(paragraph);previousBottom=top+height;
        }
        var box=new RichTextBox{Document=flow,IsReadOnly=true,IsReadOnlyCaretVisible=false,Background=Brushes.Transparent,BorderThickness=new Thickness(0),Padding=new Thickness(0),SelectionBrush=new SolidColorBrush(Color.FromArgb(105,63,145,245)),SelectionTextBrush=new SolidColorBrush(Color.FromArgb(1,0,0,0)),Cursor=Cursors.IBeam,Width=item.Bounds.Width,Height=item.Bounds.Height,VerticalScrollBarVisibility=ScrollBarVisibility.Disabled,HorizontalScrollBarVisibility=ScrollBarVisibility.Disabled,ToolTip="可跨行拖动选择文字，Ctrl+C 复制"};
        box.SelectionChanged+=(_,_)=>{var count=new TextRange(box.Selection.Start,box.Selection.End).Text.Length;if(count>0)PromptStatus.Text=$"已选择 {count} 个字符 · Ctrl+C 复制或右键";};box.ContextMenu=CreateTextContextMenu(box,document.Text);item.TextSelection.Children.Add(box);
    }
    private static ContextMenu CreateTextContextMenu(RichTextBox box,string allText)
    {
        var menu=new ContextMenu();menu.SetResourceReference(StyleProperty,"TextSelectionContextMenu");var copy=new MenuItem{Header="复制所选文字"};var copyAll=new MenuItem{Header="复制全部识别文字"};foreach(var entry in new[]{copy,copyAll})entry.SetResourceReference(StyleProperty,"TextSelectionMenuItem");copy.Click+=(_,_)=>{var text=new TextRange(box.Selection.Start,box.Selection.End).Text.TrimEnd('\r','\n');if(text.Length>0)Clipboard.SetText(text);};copyAll.Click+=(_,_)=>Clipboard.SetText(allText);var separator=new Separator();separator.SetResourceReference(StyleProperty,"TextSelectionSeparator");menu.Items.Add(copy);menu.Items.Add(separator);menu.Items.Add(copyAll);menu.Opened+=(_,_)=>copy.IsEnabled=!box.Selection.IsEmpty;return menu;
    }
    private static void ClearTextSelection(SelectionItem item){item.TextSelection.Children.Clear();item.TextSelection.IsHitTestVisible=false;}
    private void Record(object s,RoutedEventArgs e)
    {
        if(_recordingSession is not null||Active is not {IsImplicit:false,VideoPath:null} item)return;var pixels=ToPixelRect(item.Bounds);var region=ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY);var session=new RecordingSession(_host.Settings,region);_recordingSession=session;_recordingItem=item;session.Completed+=path=>Dispatcher.InvokeAsync(async()=>await CompleteRecordingAsync(path));session.Failed+=error=>Dispatcher.Invoke(()=>FailRecording(error));
        try{EnterRecordingMode(item);session.Start();_recordingStarted=DateTime.UtcNow;_recordingTimer.Start();PromptStatus.Text="正在录制当前区域";}catch(Exception ex){FailRecording(ex.Message);}
    }

    private void EnterRecordingMode(SelectionItem selected)
    {
        _recordingMode=true;_recordingPaused=_recordingStopping=false;Cursor=Cursors.Arrow;Toolbar.Visibility=DrawingToolbar.Visibility=PromptBar.Visibility=SizeText.Visibility=Visibility.Collapsed;HideHandles();
        foreach(var item in _selections){item.Host.Visibility=ReferenceEquals(item,selected)?Visibility.Visible:Visibility.Collapsed;item.Badge.Visibility=Visibility.Collapsed;item.Annotations.Visibility=Visibility.Collapsed;item.TextSelection.Visibility=Visibility.Collapsed;item.Markup.IsHitTestVisible=false;}
        selected.Outline.BorderBrush=new SolidColorBrush(Color.FromRgb(50,151,242));selected.Outline.BorderThickness=new Thickness(2);selected.Outline.Effect=new DropShadowEffect{Color=Color.FromRgb(48,151,242),BlurRadius=18,ShadowDepth=0,Opacity=.9};RecordingTime.Text="00:00";RecordingPauseButton.Content=new TextBlock{Text="Ⅱ",FontSize=14};RecordingPauseButton.ToolTip="暂停";RecordingBar.Visibility=Visibility.Visible;PositionFloatingBar(RecordingBar,selected);
    }
    private void RecordingTick()
    {
        if(!_recordingMode||_recordingItem is not { } item)return;RecordingTime.Text=(DateTime.UtcNow-_recordingStarted).ToString(@"mm\:ss");if(_recordingPaused||NativeMethods.VisualQaCaptureEnabled)return;try{var pixels=ToPixelRect(item.Bounds);var region=ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY);item.Image.Source=new ScreenCaptureService().CaptureRegion(region,_host.Settings.IncludeRecordingCursor);}catch{}
    }
    private void PauseRecording(object s,RoutedEventArgs e)
    {
        if(_recordingSession is null||_recordingStopping)return;_recordingPaused=!_recordingPaused;if(_recordingPaused){_recordingSession.Pause();RecordingPauseButton.Content=new TextBlock{Text="▶",FontSize=14};RecordingPauseButton.ToolTip="继续";}else{_recordingSession.Resume();RecordingPauseButton.Content=new TextBlock{Text="Ⅱ",FontSize=14};RecordingPauseButton.ToolTip="暂停";}
    }
    private async void StopRecording(object s,RoutedEventArgs e)
    {
        if(_recordingSession is null||_recordingStopping)return;_recordingStopping=true;_recordingTimer.Stop();RecordingTime.Text="处理中…";_recordingSession.Stop();await _recordingSession.WaitFramesAsync();
    }
    private async Task CompleteRecordingAsync(string path)
    {
        var session=_recordingSession;var item=_recordingItem;if(session is null||item is null)return;_recordingTimer.Stop();await session.WaitFramesAsync();item.VideoPath=path;item.FramesDirectory=session.FramesDirectory;item.VideoDuration=DateTime.UtcNow-_recordingStarted;session.Dispose();_recordingSession=null;_recordingItem=null;item.Video.Source=new Uri(path);item.Video.Visibility=Visibility.Visible;item.Image.Visibility=Visibility.Collapsed;item.Video.Position=TimeSpan.Zero;item.Video.Play();item.VideoPlaying=true;_references.Add(item);ExitRecordingMode(item);PromptStatus.Text=$"录屏完成 {item.VideoDuration:mm\\:ss} · 已引用为 @视频{_selections.IndexOf(item)+1}";
    }
    private void FailRecording(string error)
    {
        _recordingTimer.Stop();_recordingSession?.Dispose();_recordingSession=null;var item=_recordingItem;_recordingItem=null;if(item is not null)ExitRecordingMode(item);PromptStatus.Text=$"录屏失败：{error}";
    }
    private void ExitRecordingMode(SelectionItem selected)
    {
        _recordingMode=_recordingPaused=_recordingStopping=false;RecordingBar.Visibility=Visibility.Collapsed;PromptBar.Visibility=Visibility.Visible;Cursor=Cursors.Cross;foreach(var item in _selections){item.Host.Visibility=Visibility.Visible;item.Annotations.Visibility=Visibility.Visible;item.TextSelection.Visibility=Visibility.Visible;}var index=_selections.IndexOf(selected);if(index>=0)Select(index);RefreshSelectionNumbers();UpdateReferenceChips();ShowToolbar();SetPromptBarHidden(false);
    }
    private void ToggleVideoPlayback(object s,RoutedEventArgs e)
    {
        if(Active is not {VideoPath:not null} item)return;if(item.VideoPlaying){item.Video.Pause();item.VideoPlaying=false;PromptStatus.Text="视频已暂停";}else{item.Video.Play();item.VideoPlaying=true;PromptStatus.Text="视频正在原位播放";}
    }

    private async void OnKeyDown(object s,KeyEventArgs e)
    {
        if(e.Key==Key.Escape){if(_recordingMode){StopRecording(s,new());e.Handled=true;}else if(_drawingMode){ExitDrawingMode();e.Handled=true;}else if(_overlayRequest is {IsCancellationRequested:false}){_overlayRequest.Cancel();PromptStatus.Text="正在取消…";e.Handled=true;}else if(_request is {IsCancellationRequested:false}){_request.Cancel();e.Handled=true;}else if(_selections.FirstOrDefault(x=>x.TextSelection.IsHitTestVisible) is { } textSelection){ClearTextSelection(textSelection);PromptStatus.Text="已退出文字选择";e.Handled=true;}else Close();return;}
        if(Keyboard.FocusedElement is RichTextBox richTextBox&&_selections.Any(item=>IsInside(richTextBox,item.TextSelection))){if(e.Key==Key.C&&Keyboard.Modifiers.HasFlag(ModifierKeys.Control)&&!richTextBox.Selection.IsEmpty){var text=new TextRange(richTextBox.Selection.Start,richTextBox.Selection.End).Text.TrimEnd('\r','\n');if(text.Length>0){Clipboard.SetText(text);PromptStatus.Text="文字已复制";}}e.Handled=true;return;}
        if(e.Key==Key.Delete&&Active is not null){RemoveActiveSelection(true);e.Handled=true;return;}
        if(Active is not {IsImplicit:false} item)return;var step=Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)?10:1;
        if(e.Key is Key.Left or Key.Right or Key.Up or Key.Down){item.Bounds=ClampSelection(new Rect(item.Bounds.X+(e.Key==Key.Left?-step:e.Key==Key.Right?step:0),item.Bounds.Y+(e.Key==Key.Up?-step:e.Key==Key.Down?step:0),item.Bounds.Width,item.Bounds.Height));item.Annotations.Children.Clear();ClearTextSelection(item);UpdateSelection(item);ShowToolbar();e.Handled=true;return;}
        if(e.Key==Key.C)Copy(s,new());else if(e.Key==Key.S)Save(s,new());else if(e.Key==Key.P)Pin(s,new());else if(e.Key==Key.D)Draw(s,new());else if(e.Key==Key.T)Translate(s,new());else if(e.Key==Key.O)Ocr(s,new());else if(e.Key==Key.Enter)await SendAsync(true);else if(e.Key==Key.R)Record(s,new());
    }
}
