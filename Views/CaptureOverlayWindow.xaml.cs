using Microsoft.Win32;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Media.Effects;
using System.Windows.Media.Imaging;
using System.Windows.Shapes;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.OCR;
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
    private bool _selecting,_moving,_forceNewSelection,_promptBarHidden,_answerExpanded,_reasoningExpanded;
    private CancellationTokenSource? _speechRequest,_request,_overlayRequest;

    private sealed class SelectionItem
    {
        public Rect Bounds;
        public bool IsImplicit;
        public Grid Host { get; }=new();
        public Image Image { get; }=new(){Stretch=Stretch.Fill,IsHitTestVisible=false};
        public Canvas Annotations { get; }=new(){IsHitTestVisible=false};
        public Border Outline { get; }=new(){Background=Brushes.Transparent,CornerRadius=new CornerRadius(7),IsHitTestVisible=false};
        public Border Badge { get; }=new(){HorizontalAlignment=HorizontalAlignment.Left,VerticalAlignment=VerticalAlignment.Top,Margin=new Thickness(7),CornerRadius=new CornerRadius(10),Background=new SolidColorBrush(Color.FromArgb(230,29,119,224)),Padding=new Thickness(7,2,7,2),IsHitTestVisible=false};
        public TextBlock BadgeText { get; }=new(){Foreground=Brushes.White,FontWeight=FontWeights.SemiBold,FontSize=11};
    }

    private SelectionItem? Active=>_activeIndex>=0&&_activeIndex<_selections.Count?_selections[_activeIndex]:null;

    public CaptureOverlayWindow(AppHost host)
    {
        _host=host;_frame=new ScreenCaptureService().CaptureDesktop(host.Settings.IncludeCaptureCursor);InitializeComponent();
        if(NativeMethods.VisualQaCaptureEnabled)ShowInTaskbar=true;
        DesktopImage.Source=_frame.Image;Dimmer.Fill=new SolidColorBrush(Color.FromArgb((byte)Math.Round(Math.Clamp(host.Settings.OverlayOpacity,.4,.75)*255),0,0,0));
        var area=System.Windows.Forms.SystemInformation.VirtualScreen;Left=area.Left;Top=area.Top;Width=area.Width;Height=area.Height;
        SourceInitialized+=(_,_)=>{var hwnd=new System.Windows.Interop.WindowInteropHelper(this).Handle;NativeMethods.SetWindowPos(hwnd,new IntPtr(-1),area.Left,area.Top,area.Width,area.Height,0x0040);NativeMethods.ExcludeFromCapture(hwnd);};
        Loaded+=(_,_)=>{DesktopImage.Width=Dimmer.Width=SelectionLayer.Width=Root.ActualWidth;DesktopImage.Height=Dimmer.Height=SelectionLayer.Height=Root.ActualHeight;QuickPrompt.TextChanged+=(_,_)=>QuickPromptHint.Visibility=string.IsNullOrWhiteSpace(QuickPrompt.Text)?Visibility.Visible:Visibility.Collapsed;PromptBar.SizeChanged+=(_,_)=>PositionPromptBar();PositionPromptBar();QuickPrompt.Focus();};
        SizeChanged+=(_,_)=>{DesktopImage.Width=Dimmer.Width=SelectionLayer.Width=Root.ActualWidth;DesktopImage.Height=Dimmer.Height=SelectionLayer.Height=Root.ActualHeight;PositionPromptBar();};
        Closed+=(_,_)=>{_speechRequest?.Cancel();_request?.Cancel();_overlayRequest?.Cancel();};
    }

    private void OnMouseDown(object s,MouseButtonEventArgs e)
    {
        if(e.OriginalSource is Thumb||IsInside(e.OriginalSource as DependencyObject,PromptBar)||IsInside(e.OriginalSource as DependencyObject,Toolbar))return;
        var p=e.GetPosition(Root);var addNew=_forceNewSelection||Keyboard.Modifiers.HasFlag(ModifierKeys.Shift);_forceNewSelection=false;
        var hit=addNew?-1:FindSelection(p);
        if(hit>=0){Select(hit);_moving=true;_moveStart=p;_moveOrigin=Active!.Bounds;}
        else{var item=CreateSelection(false);_selections.Add(item);_activeIndex=_selections.Count-1;RefreshSelectionNumbers();_selecting=true;_start=p;item.Bounds=new Rect(p,p);}
        Toolbar.Visibility=Visibility.Collapsed;SetPromptBarHidden(true);Root.CaptureMouse();e.Handled=true;
    }

    private void OnMouseMove(object s,MouseEventArgs e)
    {
        var p=e.GetPosition(Root);
        if(_selecting&&Active is { } created){created.Bounds=Normalize(new Rect(_start,p));UpdateSelection(created);}
        else if(_moving&&Active is { } moved){var d=p-_moveStart;moved.Bounds=ClampSelection(new Rect(_moveOrigin.X+d.X,_moveOrigin.Y+d.Y,_moveOrigin.Width,_moveOrigin.Height));UpdateSelection(moved);}
        else{SetPromptBarHidden(PointerOverSelection(p));return;}
    }

    private void OnMouseUp(object s,MouseButtonEventArgs e)
    {
        if(!_selecting&&!_moving)return;_selecting=_moving=false;Root.ReleaseMouseCapture();
        if(Active is not { } item||item.Bounds.Width<8||item.Bounds.Height<8){RemoveActiveSelection(false);if(Active is not null)ShowToolbar();SetPromptBarHidden(false);return;}
        UpdateSelection(item);ShowToolbar();SetPromptBarHidden(PointerOverSelection(e.GetPosition(Root)));PromptStatus.Text=$"已选择 {_selections.Count} 个区域 · 可继续拖动添加";e.Handled=true;
    }

    private SelectionItem CreateSelection(bool implicitFullScreen)
    {
        var item=new SelectionItem{IsImplicit=implicitFullScreen};item.Badge.Child=item.BadgeText;item.Host.Children.Add(item.Image);item.Host.Children.Add(item.Annotations);item.Host.Children.Add(item.Outline);item.Host.Children.Add(item.Badge);SelectionLayer.Children.Add(item.Host);return item;
    }

    private void UpdateSelection(SelectionItem item)
    {
        var r=Normalize(item.Bounds);item.Bounds=r;Canvas.SetLeft(item.Host,r.Left);Canvas.SetTop(item.Host,r.Top);item.Host.Width=r.Width;item.Host.Height=r.Height;item.Annotations.Width=r.Width;item.Annotations.Height=r.Height;
        var px=ToPixelRect(r);if(px.Width>0&&px.Height>0)item.Image.Source=ScreenCaptureService.Crop(_frame.Image,px);
        var active=ReferenceEquals(item,Active);var referenced=_references.Contains(item);item.Outline.BorderBrush=item.IsImplicit?Brushes.Transparent:active?Cyan:referenced?new SolidColorBrush(Color.FromRgb(102,112,235)):new SolidColorBrush(Color.FromArgb(185,67,168,255));item.Outline.BorderThickness=new Thickness(active?2.5:referenced?2:1.5);item.Outline.Effect=active&&!item.IsImplicit?new DropShadowEffect{Color=Color.FromRgb(39,157,255),BlurRadius=18,ShadowDepth=0,Opacity=.85}:null;item.Badge.Background=new SolidColorBrush(referenced?Color.FromArgb(238,91,101,226):Color.FromArgb(230,29,119,224));item.Badge.Visibility=item.IsImplicit?Visibility.Collapsed:Visibility.Visible;
        if(active&&!item.IsImplicit){SizeText.Text=$"{px.Width} × {px.Height}";SizeText.Visibility=Visibility.Visible;Canvas.SetLeft(SizeText,r.Left);Canvas.SetTop(SizeText,Math.Max(0,r.Top-30));PositionHandles(r);}else if(item.IsImplicit){HideHandles();SizeText.Visibility=Visibility.Collapsed;}
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
        if(Active is not {IsImplicit:false} item){Toolbar.Visibility=Visibility.Collapsed;return;}var regionNumber=_activeIndex+1;ReferenceButton.ToolTip=_references.Contains(item)?$"区域{regionNumber} 已引用；可在输入框移除":$"引用当前区域为 @区域{regionNumber}";ReferenceButton.Background=new SolidColorBrush(_references.Contains(item)?Color.FromRgb(218,239,231):Color.FromRgb(233,237,255));Toolbar.Visibility=Visibility.Visible;Toolbar.Measure(new Size(double.PositiveInfinity,double.PositiveInfinity));var w=Toolbar.DesiredSize.Width;var h=Toolbar.DesiredSize.Height;var monitor=MonitorBounds(item.Bounds);var x=Math.Clamp(item.Bounds.Left,monitor.Left+8,Math.Max(monitor.Left+8,monitor.Right-w-8));var promptTop=Canvas.GetTop(PromptBar);var availableBottom=_promptBarHidden?monitor.Bottom-8:Math.Min(monitor.Bottom-8,promptTop-8);var below=item.Bounds.Bottom+10;var above=item.Bounds.Top-h-10;var y=below+h<=availableBottom?below:above>=monitor.Top+8?above:Math.Clamp(below,monitor.Top+8,Math.Max(monitor.Top+8,availableBottom-h));Canvas.SetLeft(Toolbar,x);Canvas.SetTop(Toolbar,y);
    }

    private void PositionPromptBar(){if(Root.ActualWidth<=0||Root.ActualHeight<=0)return;PromptBar.Measure(new Size(double.PositiveInfinity,double.PositiveInfinity));var w=Math.Min(PromptBar.DesiredSize.Width,Math.Max(320,Root.ActualWidth-32));PromptBar.Width=w;Canvas.SetLeft(PromptBar,(Root.ActualWidth-w)/2);Canvas.SetTop(PromptBar,Math.Max(16,Root.ActualHeight-PromptBar.ActualHeight-24));if(Toolbar.Visibility==Visibility.Visible)ShowToolbar();}
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
        if(string.IsNullOrWhiteSpace(reasoning)&&ReasoningText.Text.Length==0)return;if(ReasoningText.Text.Length==0)ReasoningText.Text=reasoning.Trim();ReasoningToggle.Visibility=Visibility.Visible;ReasoningLabel.Text="思考过程 · 已完成";ReasoningPulse.BeginAnimation(OpacityProperty,null);ReasoningPulse.Opacity=1;ReasoningPulse.Background=new SolidColorBrush(Color.FromRgb(95,181,137));_reasoningExpanded=false;ReasoningPanel.Visibility=Visibility.Collapsed;ReasoningChevron.Text="⌄";
    }

    private BitmapSource CurrentImage(){if(Active is null)throw new InvalidOperationException("请先选择区域");return ScreenCaptureService.Crop(_frame.Image,ToPixelRect(Active.Bounds));}
    private List<AiAttachment> BuildAttachments(){EnsureScreenSelection();_lastSentSelections=(_references.Count>0?_selections.Where(_references.Contains):_selections).ToList();return _lastSentSelections.Select(x=>new AiAttachment(AiAttachmentType.Image,"image/png",ScreenCaptureService.EncodePng(ScreenCaptureService.Crop(_frame.Image,ToPixelRect(x.Bounds))))).ToList();}
    private void EnsureScreenSelection(){if(_selections.Count>0)return;var item=CreateSelection(true);item.Bounds=new Rect(0,0,Root.ActualWidth,Root.ActualHeight);_selections.Add(item);_activeIndex=0;RefreshSelectionNumbers();UpdateSelection(item);}

    private void PositionHandles(Rect r){var list=new[]{Nw,N,Ne,W,E,Sw,S,Se};foreach(var t in list){t.Width=t.Height=10;t.Background=Cyan;t.Visibility=Visibility.Visible;}Set(Nw,r.Left,r.Top);Set(N,r.Left+r.Width/2,r.Top);Set(Ne,r.Right,r.Top);Set(W,r.Left,r.Top+r.Height/2);Set(E,r.Right,r.Top+r.Height/2);Set(Sw,r.Left,r.Bottom);Set(S,r.Left+r.Width/2,r.Bottom);Set(Se,r.Right,r.Bottom);static void Set(Thumb t,double x,double y){Canvas.SetLeft(t,x-5);Canvas.SetTop(t,y-5);}}
    private void HideHandles(){foreach(var t in new[]{Nw,N,Ne,W,E,Sw,S,Se})t.Visibility=Visibility.Collapsed;}
    private void ResizeDelta(object sender,DragDeltaEventArgs e){if(sender is not Thumb t||Active is not {IsImplicit:false} item)return;SetPromptBarHidden(true);var d=t.Tag?.ToString()??"";var l=item.Bounds.Left;var top=item.Bounds.Top;var r=item.Bounds.Right;var b=item.Bounds.Bottom;if(d.Contains('W'))l=Math.Clamp(l+e.HorizontalChange,0,r-12);if(d.Contains('E'))r=Math.Clamp(r+e.HorizontalChange,l+12,Root.ActualWidth);if(d.Contains('N'))top=Math.Clamp(top+e.VerticalChange,0,b-12);if(d.Contains('S'))b=Math.Clamp(b+e.VerticalChange,top+12,Root.ActualHeight);item.Bounds=new Rect(new Point(l,top),new Point(r,b));item.Annotations.Children.Clear();UpdateSelection(item);ShowToolbar();e.Handled=true;}

    private void AddRegion(object s,RoutedEventArgs e){_forceNewSelection=true;Toolbar.Visibility=Visibility.Collapsed;HideHandles();PromptStatus.Text="拖动以添加另一个区域 · 可与现有区域重叠";SetPromptBarHidden(false);}
    private void ReferenceRegion(object s,RoutedEventArgs e)
    {
        if(Active is not {IsImplicit:false} item)return;_references.Add(item);UpdateReferenceChips();UpdateSelection(item);ShowToolbar();SetPromptBarHidden(false);QuickPrompt.Focus();PromptStatus.Text=$"已加入 @区域{_activeIndex+1} · 输入问题后发送";
    }
    private void RemoveSelection(object s,RoutedEventArgs e)=>RemoveActiveSelection(true);
    private void RemoveActiveSelection(bool updateUi)
    {
        if(Active is not { } item)return;_references.Remove(item);SelectionLayer.Children.Remove(item.Host);_selections.RemoveAt(_activeIndex);_activeIndex=_selections.Count-1;RefreshSelectionNumbers();if(Active is { } next)UpdateSelection(next);else{HideHandles();SizeText.Visibility=Toolbar.Visibility=Visibility.Collapsed;}if(updateUi){PromptStatus.Text=_selections.Count==0?"拖动可连续框选多个区域":$"剩余 {_selections.Count} 个区域";if(Active is not null)ShowToolbar();}
    }
    private void RefreshSelectionNumbers(){for(var i=0;i<_selections.Count;i++)_selections[i].BadgeText.Text=(i+1).ToString();UpdateReferenceChips();}
    private void UpdateReferenceChips()
    {
        ReferenceChips.Children.Clear();
        foreach(var item in _selections.Where(_references.Contains))
        {
            var chip=new Border{Background=new SolidColorBrush(Color.FromRgb(231,236,255)),BorderBrush=new SolidColorBrush(Color.FromRgb(205,214,252)),BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(9),Margin=new Thickness(0,0,5,3)};var row=new StackPanel{Orientation=Orientation.Horizontal};var link=new Button{Content=$"@区域{_selections.IndexOf(item)+1}",ToolTip="定位到此区域"};link.SetResourceReference(StyleProperty,"ReferenceChipButton");link.Click+=(_,_)=>{var index=_selections.IndexOf(item);if(index<0)return;Select(index);ShowToolbar();SetPromptBarHidden(false);QuickPrompt.Focus();};var remove=new Button{Content="×",ToolTip="移除此引用"};remove.SetResourceReference(StyleProperty,"ReferenceChipRemoveButton");remove.Click+=(_,_)=>{_references.Remove(item);UpdateReferenceChips();UpdateSelection(item);if(ReferenceEquals(item,Active))ShowToolbar();QuickPrompt.Focus();};row.Children.Add(link);row.Children.Add(remove);chip.Child=row;ReferenceChips.Children.Add(chip);
        }
        ReferenceChips.Visibility=ReferenceChips.Children.Count>0?Visibility.Visible:Visibility.Collapsed;QuickPromptHint.Text=ReferenceChips.Children.Count>0?"继续输入关于引用区域的问题…":"询问当前屏幕，或连续圈选多个区域…";_ = Dispatcher.BeginInvoke(PositionPromptBar);
    }

    private async Task SendAsync(bool useDefaultPrompt)
    {
        if(_request is {IsCancellationRequested:false})return;var provider=new AiProviderFactory().Create(_host.Settings);if(provider is null){PromptStatus.Text="请先配置可用的 AI Provider";_host.ShowSettings();return;}if(!provider.Capabilities.SupportsImage){PromptStatus.Text="当前模型不支持图片理解";return;}
        var targetCount=_references.Count>0?_references.Count:Math.Max(1,_selections.Count);var prompt=QuickPrompt.Text.Trim();if(prompt.Length==0&&useDefaultPrompt)prompt=targetCount>1?"综合理解这些引用区域，说明它们之间的关系并标出关键部分。":"理解当前引用区域，解释内容并标出关键部分。";if(prompt.Length==0){QuickPrompt.Focus();return;}
        _request=new CancellationTokenSource(TimeSpan.FromMinutes(2));SendButton.IsEnabled=false;AnswerText.Text="";ReasoningText.Text="";ReasoningToggle.Visibility=ReasoningPanel.Visibility=Visibility.Collapsed;_reasoningExpanded=false;PromptStatus.Text=$"正在分析 {targetCount} 个引用区域…按 Esc 可取消";
        try
        {
            var progress=provider.Capabilities.SupportsStreaming?new Progress<AiStreamDelta>(delta=>{if(delta.ReasoningContent.Length>0)ShowReasoning(delta.ReasoningContent);if(delta.Content.Length>0)PromptStatus.Text="正在整理回答…";}):null;
            var result=await provider.SendAsync(new AiRequest{Prompt=prompt,History=[.._history],Attachments=BuildAttachments(),StreamingProgress=progress},_request.Token);FinishReasoning(result.Reasoning);ShowAnswer();AnswerText.Text=result.Answer;_history.Add(new("user",prompt));_history.Add(new("assistant",result.Answer));QuickPrompt.Clear();RenderAnnotations(result.Annotations);
            var configured=_host.Settings.Providers.FirstOrDefault(x=>x.Id==provider.Id);if(_host.Settings.SaveConversationHistory)await new ConversationHistoryService().AppendAsync(configured?.Name??provider.Id,configured?.Model??"",prompt,result.Answer,_request.Token);PromptStatus.Text=result.Annotations.Count>0?$"已在 {_lastSentSelections.Count} 个引用区域中标出重点 · 可继续提问":"完成 · 可继续提问";
        }
        catch(OperationCanceledException){PromptStatus.Text="已取消";}
        catch(Exception ex){ShowAnswer();AnswerText.Text="请求失败";PromptStatus.Text=ex.Message;}
        finally{_request.Dispose();_request=null;SendButton.IsEnabled=true;_ = Dispatcher.BeginInvoke(PositionPromptBar);}
    }

    private void RenderAnnotations(IReadOnlyList<AiAnnotation> notes)
    {
        foreach(var item in _selections)item.Annotations.Children.Clear();
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

    private void Copy(object s,RoutedEventArgs e){Clipboard.SetImage(CurrentImage());_host.Notify("已复制到剪贴板");Close();}
    private void Save(object s,RoutedEventArgs e){var jpeg=_host.Settings.DefaultImageFormat.Equals("jpg",StringComparison.OrdinalIgnoreCase)||_host.Settings.DefaultImageFormat.Equals("jpeg",StringComparison.OrdinalIgnoreCase);var d=new SaveFileDialog{Filter="PNG 图片|*.png|JPEG 图片|*.jpg;*.jpeg",DefaultExt=jpeg?".jpg":".png",FilterIndex=jpeg?2:1,AddExtension=true};if(d.ShowDialog(this)==true)ScreenCaptureService.Save(CurrentImage(),d.FileName,d.FilterIndex==2);}
    private void Pin(object s,RoutedEventArgs e){new PinnedImageWindow(CurrentImage()).Show();Close();}
    private void Draw(object s,RoutedEventArgs e){if(Active is not { } item)return;var pixels=ToPixelRect(item.Bounds);new DrawingWindow(_host,CurrentImage(),ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY)).Show();Close();}
    private async void QuickSend(object s,RoutedEventArgs e)=>await SendAsync(true);
    private async void QuickPromptKeyDown(object s,KeyEventArgs e){if(e.Key==Key.Enter&&!Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)){e.Handled=true;await SendAsync(true);}}
    private async void QuickVoice(object s,RoutedEventArgs e)
    {
        if(_speechRequest is not null){_speechRequest.Cancel();return;}var microphone=VoiceIcon.Data;_speechRequest=new();VoiceIcon.Data=Geometry.Parse("M7,7 L17,7 L17,17 L7,17 Z");
        try{PromptStatus.Text="正在聆听…";var text=await new WindowsSpeechToTextService().RecognizeOnceAsync(_host.Settings.VoiceLanguage,_speechRequest.Token);if(!string.IsNullOrWhiteSpace(text))QuickPrompt.Text=string.IsNullOrWhiteSpace(QuickPrompt.Text)?text:QuickPrompt.Text+" "+text;PromptStatus.Text="语音已写入";}
        catch(OperationCanceledException){PromptStatus.Text="已停止聆听";}catch(Exception ex){PromptStatus.Text=$"语音不可用：{ex.Message}";}finally{_speechRequest.Dispose();_speechRequest=null;VoiceIcon.Data=microphone;}
    }
    private async void Translate(object s,RoutedEventArgs e)
    {
        if(Active is not {IsImplicit:false} item)return;var image=CurrentImage();var operation=BeginOverlayOperation("正在识别并翻译当前区域…");
        try
        {
            var document=await new WindowsOcrService().RecognizeAsync(image,operation.Token);if(!_selections.Contains(item))return;if(document.Lines.Count==0){PromptStatus.Text="当前区域未识别到文字";return;}
            var provider=new AiProviderFactory().Create(_host.Settings);if(provider is null){PromptStatus.Text="翻译需要先配置可用的 AI Provider";_host.ShowSettings();return;}
            var prompt="将 translationsSource 中的每一项翻译成简体中文。保持数组长度和顺序完全一致，只返回 JSON：{\"translations\":[\"译文1\",\"译文2\"]}。translationsSource="+System.Text.Json.JsonSerializer.Serialize(document.Lines.Select(line=>line.Text).ToArray());
            var result=await provider.SendAsync(new AiRequest{Prompt=prompt},operation.Token);if(!_selections.Contains(item))return;if(!TranslationResponseParser.TryParse(result.Answer,document.Lines.Count,out var translated)){PromptStatus.Text="翻译结果格式异常，请重试";return;}
            RenderTextOverlays(item,image,document.Lines,translated,true);PromptStatus.Text=$"已在原位翻译 {translated.Count} 行";
        }
        catch(OperationCanceledException){}catch(Exception ex){PromptStatus.Text=$"翻译失败：{ex.Message}";}finally{EndOverlayOperation(operation);}
    }

    private async void Ocr(object s,RoutedEventArgs e)
    {
        if(Active is not {IsImplicit:false} item)return;var image=CurrentImage();var operation=BeginOverlayOperation("正在本地识别当前区域…");
        try
        {
            var document=await new WindowsOcrService().RecognizeAsync(image,operation.Token);if(!_selections.Contains(item))return;RenderTextOverlays(item,image,document.Lines,document.Lines.Select(line=>line.Text).ToList(),false);PromptStatus.Text=document.Lines.Count==0?"当前区域未识别到文字":$"已在原位贴出 {document.Lines.Count} 行文字";
        }
        catch(OperationCanceledException){}catch(Exception ex){PromptStatus.Text=$"OCR 失败：{ex.Message}";}finally{EndOverlayOperation(operation);}
    }

    private CancellationTokenSource BeginOverlayOperation(string status){_overlayRequest?.Cancel();var operation=new CancellationTokenSource(TimeSpan.FromMinutes(2));_overlayRequest=operation;PromptStatus.Text=status;Toolbar.IsEnabled=false;return operation;}
    private void EndOverlayOperation(CancellationTokenSource operation){operation.Dispose();if(!ReferenceEquals(_overlayRequest,operation))return;_overlayRequest=null;Toolbar.IsEnabled=true;if(Active is not null)ShowToolbar();}
    private static void RenderTextOverlays(SelectionItem item,BitmapSource image,IReadOnlyList<OcrLine> lines,IReadOnlyList<string> texts,bool translated)
    {
        item.Annotations.Children.Clear();var scaleX=item.Bounds.Width/image.PixelWidth;var scaleY=item.Bounds.Height/image.PixelHeight;
        for(var index=0;index<lines.Count&&index<texts.Count;index++)
        {
            var line=lines[index];var text=new TextBlock{Text=texts[index],Foreground=new SolidColorBrush(Color.FromRgb(35,47,67)),FontSize=Math.Clamp(line.Height*scaleY*.72,10,26),FontWeight=translated?FontWeights.SemiBold:FontWeights.Normal,TextWrapping=TextWrapping.Wrap,LineHeight=Math.Clamp(line.Height*scaleY*.9,14,32)};
            var box=new Border{Child=text,Background=new SolidColorBrush(translated?Color.FromArgb(242,239,242,255):Color.FromArgb(228,248,251,255)),BorderBrush=new SolidColorBrush(translated?Color.FromRgb(111,124,245):Color.FromRgb(78,164,224)),BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(5),Padding=new Thickness(3,1,3,1),Width=Math.Max(36,line.Width*scaleX),MinHeight=Math.Max(18,line.Height*scaleY),ToolTip=translated?"原位译文":"本地 OCR 文字"};
            Canvas.SetLeft(box,line.X*scaleX);Canvas.SetTop(box,line.Y*scaleY);item.Annotations.Children.Add(box);
        }
    }
    private void Record(object s,RoutedEventArgs e){if(Active is not { } item)return;var px=ToPixelRect(item.Bounds);new RecordingControlWindow(_host,ScreenCoordinateService.ToScreenRect(px,_frame.OriginX,_frame.OriginY)).Show();Close();}

    private async void OnKeyDown(object s,KeyEventArgs e)
    {
        if(e.Key==Key.Escape){if(_request is {IsCancellationRequested:false}){_request.Cancel();e.Handled=true;}else Close();return;}
        if(e.Key==Key.Delete&&Active is not null){RemoveActiveSelection(true);e.Handled=true;return;}
        if(Active is not {IsImplicit:false} item)return;var step=Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)?10:1;
        if(e.Key is Key.Left or Key.Right or Key.Up or Key.Down){item.Bounds=ClampSelection(new Rect(item.Bounds.X+(e.Key==Key.Left?-step:e.Key==Key.Right?step:0),item.Bounds.Y+(e.Key==Key.Up?-step:e.Key==Key.Down?step:0),item.Bounds.Width,item.Bounds.Height));item.Annotations.Children.Clear();UpdateSelection(item);ShowToolbar();e.Handled=true;return;}
        if(e.Key==Key.C)Copy(s,new());else if(e.Key==Key.S)Save(s,new());else if(e.Key==Key.P)Pin(s,new());else if(e.Key==Key.D)Draw(s,new());else if(e.Key==Key.T)Translate(s,new());else if(e.Key==Key.O)Ocr(s,new());else if(e.Key==Key.Enter)await SendAsync(true);else if(e.Key==Key.R)Record(s,new());
    }
}
