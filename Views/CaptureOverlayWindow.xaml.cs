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
using System.Windows.Interop;
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
    private const int ReasoningDisplayLimit=12_000;
    private const double PromptPreferredWidthTight=540;
    private const double PromptEdgeMargin=6;
    private const double PromptFloatingGap=8;
    private const double PromptHiddenOffset=20;
    private const double CompactChipScrollMaxHeight=36;
    private const double CompactQuickPromptMinHeight=30;
    private const double CompactQuickPromptMaxHeight=52;
    private static readonly Brush Cyan=new SolidColorBrush(Color.FromRgb(67,198,255));
    private readonly AppHost _host;
    private readonly CaptureFrame _frame;
    private readonly List<SelectionItem> _selections=[];
    private readonly HashSet<SelectionItem> _ownedSelections=[];
    private readonly HashSet<SelectionItem> _references=[];
    private readonly UndoRedoHistory<OverlaySnapshot> _overlayHistory=new();
    private readonly System.Text.StringBuilder _reasoningBuffer=new();
    private List<SelectionItem> _lastSentSelections=[];
    private readonly List<AiMessage> _history=[new("system","分析屏幕附件时只能返回一个 JSON 根对象：{answer:string,annotations:[{regionIndex,x,y,width,height,text}]}，禁止添加 Markdown 代码围栏、json 标记或 JSON 之外的说明。regionIndex 是从 0 开始的完整附件顺序；坐标是对应图片内 0 到 1 的归一化值。只能给图片附件返回空间批注，视频附件不得返回批注。当用户要求标出、框选或指出关键部分且图片中存在可定位对象时，必须为相关图片返回 1 至 6 条有效批注；确实没有可定位目标时 annotations 才能为空。")];
    private Point _start,_moveStart;
    private Rect _moveOrigin;
    private int _activeIndex=-1;
    private bool _selecting,_moving,_forceNewSelection,_promptBarHidden,_answerExpanded,_reasoningExpanded,_recordingMode,_drawingMode,_recordingPaused,_recordingStopping,_captureExclusionVerified,_autoVoiceStarted,_closed,_positioningPromptBar,_promptBarLayoutPassQueued,_reasoningRenderScheduled;
    private CancellationTokenSource? _speechRequest,_request,_overlayRequest,_recordingStopWatchdog,_readAloudRequest;
    private CancellationTokenSource? _reasoningRenderRequest;
    private TaskCompletionSource<AiInteractionResponse>? _activeInteraction;
    private AiInteractionResponse? _activeInteractionFallback;
    private CancellationTokenRegistration _interactionCancellation;
    private PasswordBox? _activeSensitiveInput;
    private RecordingSession? _recordingSession;
    private SelectionItem? _recordingItem;
    private bool _recordingItemWasReferenced;
    private HwndSource? _overlaySource;
    private System.Drawing.Rectangle _virtualScreenArea;
    private bool _recordingWindowRegionApplied;
    private bool _recordingHoleUpdateQueued;
    private bool _recordingRegionResetQueued;
    private bool _recordingRegionCloseQueued;
    private int _recordingHoleRetryCount;
    private int _recordingRegionResetRetryCount;
    private (int WindowLeft,int WindowTop,int WindowWidth,int WindowHeight,int HoleLeft,int HoleTop,int HoleRight,int HoleBottom,int BarLeft,int BarTop,int BarRight,int BarBottom)? _recordingWindowRegionKey;
    private readonly DispatcherTimer _recordingTimer=new(){Interval=TimeSpan.FromMilliseconds(150)};
    private DrawTool _drawTool=DrawTool.Freehand;
    private Point _drawStart;
    private Stroke? _drawPreview;
    private OverlaySnapshot? _pointerOperationBefore;
    private OverlaySnapshot? _resizeOperationBefore;
    private OverlaySnapshot? _drawingOperationBefore;
    private string _pointerOperationLabel="";
    private bool _drawingOperationChanged;

    private sealed class SelectionItem
    {
        public Rect Bounds;
        public bool IsImplicit;
        public Grid Host { get; }=new();
        public Image Image { get; }=new(){Stretch=Stretch.Fill,IsHitTestVisible=false};
        // Video is rendered into the same Image surface as the selection.
        // The player behind it is WinRT frame-server based (see
        // VideoPreviewSurface), so the overlay never depends on the legacy
        // WPF MediaElement/WMP renderer.
        public Image Video { get; }=new(){Stretch=Stretch.Fill,Visibility=Visibility.Collapsed,IsHitTestVisible=false};
        public VideoPreviewSurface? VideoPreview;
        public InkCanvas Markup { get; }=new(){Background=Brushes.Transparent,IsHitTestVisible=false};
        public Canvas AiAnnotations { get; }=new(){IsHitTestVisible=false};
        public Canvas TextOverlays { get; }=new(){IsHitTestVisible=false};
        public Canvas TextSelection { get; }=new(){IsHitTestVisible=false,Background=Brushes.Transparent};
        public Border Outline { get; }=new(){Background=Brushes.Transparent,CornerRadius=new CornerRadius(7),IsHitTestVisible=false};
        public Border Badge { get; }=new(){HorizontalAlignment=HorizontalAlignment.Left,VerticalAlignment=VerticalAlignment.Top,Margin=new Thickness(7),CornerRadius=new CornerRadius(10),Background=new SolidColorBrush(Color.FromArgb(230,29,119,224)),Padding=new Thickness(7,2,7,2),IsHitTestVisible=false};
        public TextBlock BadgeText { get; }=new(){Foreground=Brushes.White,FontWeight=FontWeights.SemiBold,FontSize=11};
        public Stack<Stroke> Redo { get; }=[];
        public TextLayerState TextLayer { get; set; }=NoTextLayerState.Instance;
        public List<AiAnnotation> AnnotationNotes { get; }=[];
        public OcrTextSelectionSession? TextSession;
        public TempMediaLease? VideoLease;
        public string? VideoPath;
        public TimeSpan VideoDuration;
        public bool VideoPlaying;
    }

    private abstract record TextLayerState;
    private sealed record NoTextLayerState:TextLayerState
    {
        public static NoTextLayerState Instance { get; }=new();
    }
    private sealed record OcrTextLayerState(BitmapSource Image,OcrDocument Document):TextLayerState;
    private sealed record TranslationTextLayerState(BitmapSource Image,IReadOnlyList<OcrLine> Lines,IReadOnlyList<string> Texts):TextLayerState;
    private sealed record SelectionSnapshot(
        SelectionItem Item,
        Rect Bounds,
        bool Referenced,
        StrokeCollection Markup,
        TextLayerState TextLayer,
        IReadOnlyList<AiAnnotation> AnnotationNotes);
    private sealed record OverlaySnapshot(
        IReadOnlyList<SelectionSnapshot> Selections,
        SelectionItem? Active,
        string AnswerMarkdown,
        bool AnswerExpanded,
        IReadOnlyList<AiMessage> History,
        IReadOnlyList<SelectionItem> LastSentSelections);

    private sealed record SelectableGlyph(Rect Bounds,TextPointer Start,TextPointer End);
    private sealed class OcrTextSelectionSession : IDisposable
    {
        private readonly RichTextBox _box;private readonly Canvas _highlights;private readonly IReadOnlyList<SelectableGlyph> _glyphs;private int _anchor=-1;
        public OcrTextSelectionSession(RichTextBox box,Canvas highlights,IReadOnlyList<SelectableGlyph> glyphs)
        {
            _box=box;_highlights=highlights;_glyphs=glyphs;_box.PreviewMouseLeftButtonDown+=MouseDown;_box.PreviewMouseMove+=MouseMove;_box.PreviewMouseLeftButtonUp+=MouseUp;_box.LostMouseCapture+=LostCapture;
        }
        private void MouseDown(object sender,MouseButtonEventArgs e){var index=Nearest(e.GetPosition(_box));if(index<0)return;_anchor=index;Select(index);_box.Focus();_box.CaptureMouse();e.Handled=true;}
        private void MouseMove(object sender,MouseEventArgs e){if(_anchor<0||!_box.IsMouseCaptured||e.LeftButton!=MouseButtonState.Pressed)return;var index=Nearest(e.GetPosition(_box));if(index>=0)Select(index);e.Handled=true;}
        private void MouseUp(object sender,MouseButtonEventArgs e){if(_anchor<0)return;var index=Nearest(e.GetPosition(_box));if(index>=0)Select(index);if(_box.IsMouseCaptured)_box.ReleaseMouseCapture();_anchor=-1;e.Handled=true;}
        private void LostCapture(object sender,MouseEventArgs e)=>_anchor=-1;
        private int Nearest(Point point)
        {
            var best=-1;var bestDistance=double.PositiveInfinity;
            for(var index=0;index<_glyphs.Count;index++)
            {
                var bounds=_glyphs[index].Bounds;if(bounds.Contains(point))return index;var dx=point.X<bounds.Left?bounds.Left-point.X:point.X>bounds.Right?point.X-bounds.Right:0;var dy=point.Y<bounds.Top?bounds.Top-point.Y:point.Y>bounds.Bottom?point.Y-bounds.Bottom:0;var distance=dx*dx+dy*dy;if(distance>=bestDistance)continue;bestDistance=distance;best=index;
            }
            return best;
        }
        private void Select(int current)
        {
            var first=Math.Min(_anchor,current);var last=Math.Max(_anchor,current);_box.Selection.Select(_glyphs[first].Start,_glyphs[last].End);_highlights.Children.Clear();
            Rect? merged=null;for(var index=first;index<=last;index++){var bounds=_glyphs[index].Bounds;if(merged is { } currentBounds&&Math.Abs(currentBounds.Top-bounds.Top)<2&&Math.Abs(currentBounds.Height-bounds.Height)<2&&bounds.Left<=currentBounds.Right+Math.Max(3,bounds.Height*.35)){merged=Rect.Union(currentBounds,bounds);continue;}if(merged is { } completed)AddHighlight(completed);merged=bounds;}if(merged is { } final)AddHighlight(final);
        }
        private void AddHighlight(Rect bounds){var highlight=new Border{Width=bounds.Width,Height=bounds.Height,CornerRadius=new CornerRadius(2),Background=new SolidColorBrush(Color.FromArgb(92,63,145,245)),IsHitTestVisible=false};Canvas.SetLeft(highlight,bounds.Left);Canvas.SetTop(highlight,bounds.Top);_highlights.Children.Add(highlight);}
        public void Dispose(){if(_box.IsMouseCaptured)_box.ReleaseMouseCapture();_box.PreviewMouseLeftButtonDown-=MouseDown;_box.PreviewMouseMove-=MouseMove;_box.PreviewMouseLeftButtonUp-=MouseUp;_box.LostMouseCapture-=LostCapture;_highlights.Children.Clear();}
    }

    private enum DrawTool{Freehand,Rectangle,Arrow}

    private SelectionItem? Active=>_activeIndex>=0&&_activeIndex<_selections.Count?_selections[_activeIndex]:null;

    public CaptureOverlayWindow(AppHost host)
    {
        _host=host;_frame=new ScreenCaptureService().CaptureDesktop(host.Settings.IncludeCaptureCursor);InitializeComponent();
        // Keep overlay text and icon edges crisp at mixed DPI values.  The
        // capture surface remains in physical-pixel coordinates; these flags
        // only affect WPF rasterisation of the presentation layer.
        UseLayoutRounding=true;SnapsToDevicePixels=true;TextOptions.SetTextFormattingMode(this,TextFormattingMode.Display);Root.SnapsToDevicePixels=true;
        ApplyOverlayVisualTuning();
        ApplyVoiceAvailability();
        if(NativeMethods.VisualQaCaptureEnabled)ShowInTaskbar=true;
        DesktopImage.Source=_frame.Image;Dimmer.Fill=new SolidColorBrush(Color.FromArgb((byte)Math.Round(Math.Clamp(host.Settings.OverlayOpacity,.4,.75)*255),0,0,0));
        var area=System.Windows.Forms.SystemInformation.VirtualScreen;
        _virtualScreenArea=area;
        // Window dimensions are WPF DIPs while the virtual desktop and the
        // captured frame are physical pixels.  On a 175% display assigning the
        // raw pixel size here makes the HWND render 1.75x too large and pushes
        // the composer below the screen.  Read the per-monitor DPI once the
        // HWND exists, express the layout surface in DIPs, then pin the native
        // window to the exact physical virtual-screen bounds.
        Left=area.Left;Top=area.Top;Width=area.Width;Height=area.Height;
        SourceInitialized+=(_,_)=>
        {
            var hwnd=new System.Windows.Interop.WindowInteropHelper(this).Handle;
            _overlaySource=HwndSource.FromHwnd(hwnd);
            _overlaySource?.AddHook(OverlayWindowMessage);
            NativeMethods.SetWindowPos(hwnd,new IntPtr(-1),area.Left,area.Top,area.Width,area.Height,0x0040);
            var excluded=NativeMethods.ExcludeFromCapture(hwnd);var nativeError=excluded?0:System.Runtime.InteropServices.Marshal.GetLastWin32Error();_captureExclusionVerified=excluded&&!NativeMethods.VisualQaCaptureEnabled;if(!excluded)new PrivacyLogger().Error("CaptureProtection",new InvalidOperationException($"无法启用覆盖层防捕获，Win32 错误码 {nativeError}"));
        };
        Loaded+=async (_,_)=>
        {
            ApplyOverlayDpiLayout(area);
            DesktopImage.Width=Dimmer.Width=SelectionLayer.Width=Root.ActualWidth;DesktopImage.Height=Dimmer.Height=SelectionLayer.Height=Root.ActualHeight;
            QuickPrompt.TextChanged+=(_,_)=>UpdateQuickPromptHint();QuickPrompt.GotKeyboardFocus+=(_,_)=>{UpdateQuickPromptHint();PromptInputBorder.BorderBrush=new SolidColorBrush(Color.FromRgb(115,130,235));};QuickPrompt.LostKeyboardFocus+=(_,_)=>{UpdateQuickPromptHint();PromptInputBorder.BorderBrush=new SolidColorBrush(Color.FromRgb(220,228,239));};UpdateQuickPromptHint();PromptBar.SizeChanged+=(_,_)=>{PositionPromptBar();UpdatePromptBarHiddenTransform(false);};PositionPromptBar();QuickPrompt.Focus();
            // WPF can re-apply the initial Width/Height after SourceInitialized;
            // run one render-priority pass so the DIP surface remains in sync
            // with the physical HWND before any pointer coordinates arrive.
            _=Dispatcher.BeginInvoke(DispatcherPriority.Render,new Action(()=>
            {
                if(_closed)return;
                ApplyOverlayDpiLayout(area);
                DesktopImage.Width=Dimmer.Width=SelectionLayer.Width=Root.ActualWidth;DesktopImage.Height=Dimmer.Height=SelectionLayer.Height=Root.ActualHeight;
                PositionPromptBar();
            }));
            _=Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle,new Action(()=>
            {
                if(_closed)return;
                ApplyOverlayDpiLayout(area);
                DesktopImage.Width=Dimmer.Width=SelectionLayer.Width=Root.ActualWidth;DesktopImage.Height=Dimmer.Height=SelectionLayer.Height=Root.ActualHeight;
                PositionPromptBar();
            }));
            if(CaptureOverlayPolicy.ShouldStartAutomaticListening(_host.Settings.EnableVoiceInput,_host.Settings.AutomaticallyStartListening,_autoVoiceStarted,_closed)){_autoVoiceStarted=true;await ToggleVoiceAsync();}
        };
        DpiChanged+=(_,_)=>ApplyOverlayDpiLayout(area);
        SizeChanged+=(_,_)=>{DesktopImage.Width=Dimmer.Width=SelectionLayer.Width=Root.ActualWidth;DesktopImage.Height=Dimmer.Height=SelectionLayer.Height=Root.ActualHeight;UpdateRecordingVisualHole();PositionPromptBar();};
        RecordingBar.SizeChanged+=(_,_)=>{if(_recordingMode)UpdateRecordingVisualHole();};
        _recordingTimer.Tick+=(_,_)=>RecordingTick();
        Closed+=OnClosed;
    }

    private void ApplyOverlayVisualTuning()
    {
        Toolbar.Padding=new Thickness(5);
        DrawingToolbar.Padding=new Thickness(5);
        RecordingBar.Padding=new Thickness(8,6,8,6);
        PromptBar.Padding=new Thickness(6);
        PromptBar.CornerRadius=new CornerRadius(18);
        ReferenceChipScroll.MaxHeight=CompactChipScrollMaxHeight;
        ReferenceChips.Margin=new Thickness(4,2,4,0);
        QuickPrompt.MinHeight=CompactQuickPromptMinHeight;
        QuickPrompt.MaxHeight=CompactQuickPromptMaxHeight;
        QuickPrompt.Padding=new Thickness(8,4,8,4);
        QuickPrompt.FontSize=12.5;
        TextBlock.SetLineHeight(QuickPrompt,17);
        QuickPromptHint.Margin=new Thickness(8,0,0,0);
        AnswerHeader.Margin=new Thickness(8,2,8,0);
        AnswerScroll.Margin=new Thickness(8,6,8,6);
        PromptStatus.Margin=new Thickness(0,3,0,0);
        PromptStatus.FontSize=10;
        ReasoningPanel.Margin=new Thickness(6,5,6,0);
        ReasoningPanel.Padding=new Thickness(10,8,10,8);
    }

    private void UpdateQuickPromptHint()
    {
        var empty=QuickPrompt.Text.Length==0;
        QuickPromptHint.Visibility=empty?Visibility.Visible:Visibility.Collapsed;
        QuickPrompt.CaretBrush=empty?Brushes.Transparent:new SolidColorBrush(Color.FromRgb(91,108,235));
    }

    private void OnClosed(object? sender,EventArgs e)
    {
        _closed=true;
        ClearRecordingVisualHole();
        if(_overlaySource is not null)
        {
            try{_overlaySource.RemoveHook(OverlayWindowMessage);}catch(Exception ex){new PrivacyLogger().Error("OverlayHookRemove",ex);}
            _overlaySource=null;
        }
        ResolveOverlayInteractionWithFallback();StopOverlayReadAloud();TryCancel(_speechRequest);TryCancel(_request);TryCancel(_overlayRequest);CancelRecordingStopWatchdog();_recordingTimer.Stop();
        _reasoningRenderRequest=null;_reasoningRenderScheduled=false;ReasoningPulse.BeginAnimation(OpacityProperty,null);
        var session=_recordingSession;_recordingSession=null;_recordingItem=null;_recordingItemWasReferenced=false;
        if(session is not null)
        {
            try{session.Stop();}catch(Exception ex){new PrivacyLogger().Error("RecordingStopOnClose",ex);}
            try{session.Dispose();}catch(Exception ex){new PrivacyLogger().Error("RecordingDisposeOnClose",ex);}
        }
        foreach(var item in _ownedSelections)
            ReleaseSelectionResources(item);
    }

    private static void TryCancel(CancellationTokenSource? source)
    {
        if(source is null)return;
        try{source.Cancel();}catch(ObjectDisposedException){}catch(Exception ex){new PrivacyLogger().Error("OverlayCancelOnClose",ex);}
    }

    private void ApplyOverlayDpiLayout(System.Drawing.Rectangle area)
    {
        if(!IsInitialized)return;
        var hwnd=new System.Windows.Interop.WindowInteropHelper(this).Handle;
        var dpi=Math.Max(96u,NativeMethods.GetDpiForWindow(hwnd));
        // Window and canvas dimensions are WPF DIPs.  The captured frame and
        // native SetWindowPos bounds are physical pixels, so express the
        // virtual desktop surface in DIPs once using the current monitor DPI.
        // Keeping the canvas untransformed preserves a one-to-one logical
        // coordinate space for pointer hit testing and ScreenCoordinateService.
        var dipSize=ScreenCoordinateService.PixelsToDipSize(area.Width,area.Height,dpi);
        Width=dipSize.Width;
        Height=dipSize.Height;
        Root.Width=dipSize.Width;
        Root.Height=dipSize.Height;
        Root.HorizontalAlignment=HorizontalAlignment.Left;
        Root.VerticalAlignment=VerticalAlignment.Top;
        if(_recordingMode)UpdateRecordingVisualHole();
    }

    /// <summary>
    /// During a recording the selected rectangle is a live pass-through hole.
    /// The desktop frame and dimmer remain visible everywhere else, while the
    /// native hit-test hook sends pointer input in the hole to the window below
    /// (for example, a full-screen browser video).
    /// </summary>
    private bool UpdateRecordingVisualHole(bool requireNativeRegion=false)
    {
        if(!_recordingMode||_recordingItem is not { } item)
        {
            ClearRecordingVisualHole();
            return !requireNativeRegion;
        }

        // Size/DPI changes can briefly report a zero-sized root while WPF is
        // rebuilding the surface.  Do not clear an already working native
        // hole in that interval: clearing it makes the frozen overlay swallow
        // the underlying video until the next layout pass.  A single render
        // retry is enough to pick up the settled size without creating an
        // unbounded dispatcher loop.
        if(Root.ActualWidth<=0||Root.ActualHeight<=0)
        {
            if(!requireNativeRegion&&_recordingHoleRetryCount++<3&&!_recordingHoleUpdateQueued&&!Dispatcher.HasShutdownStarted)
            {
                _recordingHoleUpdateQueued=true;
                _=Dispatcher.BeginInvoke(DispatcherPriority.Render,new Action(() =>
                {
                    _recordingHoleUpdateQueued=false;
                    if(!_closed&&_recordingMode)UpdateRecordingVisualHole();
                }));
            }
            return _recordingWindowRegionApplied;
        }

        _recordingHoleRetryCount=0;

        var full=new RectangleGeometry(new Rect(0,0,Math.Max(0,Root.ActualWidth),Math.Max(0,Root.ActualHeight)));
        var hole=new RectangleGeometry(Normalize(item.Bounds));
        // Use an exclude geometry instead of an opacity mask so the pixels
        // under the selection are genuinely transparent and can receive input.
        DesktopImage.Clip=new CombinedGeometry(GeometryCombineMode.Exclude,full,hole);
        Dimmer.Clip=new CombinedGeometry(GeometryCombineMode.Exclude,full,hole);
        item.Image.Visibility=Visibility.Collapsed;
        return ApplyRecordingWindowRegion(item);
    }

    private void ClearRecordingVisualHole()
    {
        _recordingHoleRetryCount=0;
        DesktopImage.Clip=null;
        Dimmer.Clip=null;
        ResetRecordingWindowRegion();
    }

    private bool ApplyRecordingWindowRegion(SelectionItem item)
    {
        if(!IsInitialized||_virtualScreenArea.Width<=0||_virtualScreenArea.Height<=0)return false;
        var hwnd=new WindowInteropHelper(this).Handle;
        if(hwnd==IntPtr.Zero)return false;
        var pixels=ToPixelRect(item.Bounds);
        // Keep a very small ring for the live border and its shadow.  The
        // interior is removed from the native window region, which makes the
        // hole genuinely click-through even when the browser is another UI
        // thread/process (HTTRANSPARENT alone is same-thread only).
        const int borderReserve=5;
        // SetWindowRgn uses coordinates relative to the actual HWND bounds,
        // not the captured frame.  Normally these are identical, but using
        // the live rectangle avoids DPI/layout rounding offsets (especially
        // on mixed-DPI or negative-coordinate virtual desktops).
        var windowLeft=_virtualScreenArea.Left;var windowTop=_virtualScreenArea.Top;
        var windowWidth=_virtualScreenArea.Width;var windowHeight=_virtualScreenArea.Height;
        if(NativeMethods.GetWindowRect(hwnd,out var windowRect))
        {
            var measuredWidth=windowRect.Right-windowRect.Left;var measuredHeight=windowRect.Bottom-windowRect.Top;
            if(measuredWidth>0&&measuredHeight>0)
            {
                windowLeft=windowRect.Left;windowTop=windowRect.Top;windowWidth=measuredWidth;windowHeight=measuredHeight;
            }
        }
        var windowBounds=new ScreenRect(windowLeft,windowTop,windowWidth,windowHeight);
        var holeRect=ScreenCoordinateService.ToWindowRelativePixelRect(pixels,_frame.OriginX,_frame.OriginY,windowBounds,borderReserve);
        if(holeRect.IsEmpty)
        {
            LogRecordingRegionFailure("录屏选区太小或已超出当前窗口，无法建立实时穿透区域");
            return false;
        }
        var left=holeRect.X;var top=holeRect.Y;var right=holeRect.Right;var bottom=holeRect.Bottom;
        var barRect=CreateRecordingBarRegion(windowLeft,windowTop,windowWidth,windowHeight);
        if(RecordingBar.Visibility==Visibility.Visible&&barRect.IsEmpty)
        {
            LogRecordingRegionFailure("录屏控制条尚未完成布局，无法安全开始录制");
            return false;
        }
        var key=(windowLeft,windowTop,windowWidth,windowHeight,left,top,right,bottom,barRect.Left,barRect.Top,barRect.Right,barRect.Bottom);
        if(_recordingWindowRegionApplied&&_recordingWindowRegionKey==key)return true;
        var full=NativeMethods.CreateRectRgn(0,0,windowWidth,windowHeight);
        var hole=NativeMethods.CreateRectRgn(left,top,right,bottom);
        var result=NativeMethods.CreateRectRgn(0,0,0,0);
        if(full==IntPtr.Zero||hole==IntPtr.Zero||result==IntPtr.Zero||NativeMethods.CombineRgn(result,full,hole,NativeMethods.RgnDiff)==0)
        {
            var error=System.Runtime.InteropServices.Marshal.GetLastPInvokeError();
            if(result!=IntPtr.Zero)NativeMethods.DeleteObject(result);
            if(hole!=IntPtr.Zero)NativeMethods.DeleteObject(hole);
            if(full!=IntPtr.Zero)NativeMethods.DeleteObject(full);
            LogRecordingRegionFailure("无法创建录屏实时穿透区域",error);
            return false;
        }
        NativeMethods.DeleteObject(hole);NativeMethods.DeleteObject(full);
        // A nearly full-screen selection can leave no room above or below for
        // the stop/pause bar.  Keep the bar in the native region so its
        // buttons remain actionable even when it overlaps the live hole.
        if(!barRect.IsEmpty)
        {
            var barRegion=NativeMethods.CreateRectRgn(barRect.Left,barRect.Top,barRect.Right,barRect.Bottom);
            var union=NativeMethods.CreateRectRgn(0,0,windowWidth,windowHeight);
            if(barRegion==IntPtr.Zero||union==IntPtr.Zero||NativeMethods.CombineRgn(union,result,barRegion,NativeMethods.RgnOr)==0)
            {
                var error=System.Runtime.InteropServices.Marshal.GetLastPInvokeError();
                if(union!=IntPtr.Zero)NativeMethods.DeleteObject(union);
                if(barRegion!=IntPtr.Zero)NativeMethods.DeleteObject(barRegion);
                NativeMethods.DeleteObject(result);
                LogRecordingRegionFailure("无法保留录屏控制条的交互区域",error);
                return false;
            }
            NativeMethods.DeleteObject(result);result=union;
            NativeMethods.DeleteObject(barRegion);
        }
        if(NativeMethods.SetWindowRgn(hwnd,result,true)==0)
        {
            var error=System.Runtime.InteropServices.Marshal.GetLastPInvokeError();
            NativeMethods.DeleteObject(result);
            LogRecordingRegionFailure("无法设置录屏实时区域的窗口命中区域",error);
            return false;
        }
        // Ownership of result transfers to the window after SetWindowRgn.
        _recordingWindowRegionApplied=true;
        _recordingWindowRegionKey=key;
        return true;
    }

    private static void LogRecordingRegionFailure(string message,int error=0)
    {
        var detail=error==0?message:$"{message}（Win32 {error}）";
        new PrivacyLogger().Error("RecordingHitRegion",error==0?new InvalidOperationException(detail):new System.ComponentModel.Win32Exception(error,detail));
    }

    private System.Drawing.Rectangle CreateRecordingBarRegion(int windowLeft,int windowTop,int windowWidth,int windowHeight)
    {
        if(RecordingBar.Visibility!=Visibility.Visible)return System.Drawing.Rectangle.Empty;
        var left=Canvas.GetLeft(RecordingBar);var top=Canvas.GetTop(RecordingBar);
        var width=RecordingBar.ActualWidth>0?RecordingBar.ActualWidth:RecordingBar.DesiredSize.Width;
        var height=RecordingBar.ActualHeight>0?RecordingBar.ActualHeight:RecordingBar.DesiredSize.Height;
        if(!double.IsFinite(left)||!double.IsFinite(top)||width<=0||height<=0)return System.Drawing.Rectangle.Empty;
        try
        {
            // Convert through the same physical-pixel mapping used for the
            // selected region.  PointToScreen is monitor-DPI dependent and can
            // otherwise introduce a one-monitor offset on mixed-DPI desktops.
            var pixels=ToPixelRect(new Rect(left,top,width,height));
            var relative=ScreenCoordinateService.ToWindowRelativePixelRect(pixels,_frame.OriginX,_frame.OriginY,new ScreenRect(windowLeft,windowTop,windowWidth,windowHeight));
            return relative.IsEmpty?System.Drawing.Rectangle.Empty:new System.Drawing.Rectangle(relative.X,relative.Y,relative.Width,relative.Height);
        }
        catch(Exception ex){new PrivacyLogger().Error("RecordingBarRegion",ex);return System.Drawing.Rectangle.Empty;}
    }

    private void ResetRecordingWindowRegion()
    {
        if(!_recordingWindowRegionApplied||!IsInitialized)return;
        try
        {
            var hwnd=new WindowInteropHelper(this).Handle;
            if(hwnd!=IntPtr.Zero&&NativeMethods.SetWindowRgn(hwnd,IntPtr.Zero,true)!=0)
            {
                _recordingWindowRegionApplied=false;
                _recordingWindowRegionKey=null;
                _recordingRegionResetRetryCount=0;
                _recordingRegionResetQueued=false;
                _recordingRegionCloseQueued=false;
                return;
            }
            var error=System.Runtime.InteropServices.Marshal.GetLastPInvokeError();
            LogRecordingRegionFailure("无法恢复截图覆盖层的完整窗口区域",error);
        }
        catch(Exception ex){new PrivacyLogger().Error("RecordingHitRegionReset",ex);}
        if(_closed)
        {
            // HWND is being destroyed, so Windows will release the stale
            // region even if this final reset call failed.
            _recordingWindowRegionApplied=false;
            _recordingWindowRegionKey=null;
            return;
        }
        if(_recordingRegionResetRetryCount++<3&&!_recordingRegionResetQueued&&!Dispatcher.HasShutdownStarted)
        {
            _recordingRegionResetQueued=true;
            _=Dispatcher.BeginInvoke(DispatcherPriority.Render,new Action(() =>
            {
                _recordingRegionResetQueued=false;
                if(!_closed)ResetRecordingWindowRegion();
            }));
            return;
        }
        if(!_recordingRegionCloseQueued&&!Dispatcher.HasShutdownStarted)
        {
            _recordingRegionCloseQueued=true;
            PromptStatus.Text="窗口交互区域恢复失败，正在安全关闭覆盖层，请重新截图";
            _=Dispatcher.BeginInvoke(DispatcherPriority.Send,new Action(() =>{if(_recordingRegionCloseQueued&&!_closed)Close();}));
        }
    }

    private IntPtr OverlayWindowMessage(IntPtr hwnd,int message,IntPtr wParam,IntPtr lParam,ref bool handled)
    {
        if(message!=NativeMethods.WmNcHitTest||!_recordingMode||_recordingItem is not { } item)return IntPtr.Zero;
        // WM_NCHITTEST carries the actual point being tested.  Reading the
        // current cursor instead can answer for a different point when the
        // pointer is moving quickly, and breaks negative multi-monitor coords.
        var raw=lParam.ToInt64();
        var screenX=(short)(raw&0xffff);
        var screenY=(short)((raw>>16)&0xffff);
        if(!IsScreenPointInRecordingBar(screenX,screenY)&&IsScreenPointInRecordingHole(item,screenX,screenY))
        {
            handled=true;
            return new IntPtr(NativeMethods.HtTransparent);
        }
        return IntPtr.Zero;
    }

    private bool IsScreenPointInRecordingHole(SelectionItem item,int screenX,int screenY)
    {
        var pixels=ToPixelRect(item.Bounds);
        var left=_frame.OriginX+pixels.X;
        var top=_frame.OriginY+pixels.Y;
        return screenX>=left&&screenY>=top&&screenX<left+pixels.Width&&screenY<top+pixels.Height;
    }

    private bool IsScreenPointInRecordingBar(int screenX,int screenY)
    {
        if(RecordingBar.Visibility!=Visibility.Visible)return false;
        var local=PointFromScreen(new Point(screenX,screenY));
        var left=Canvas.GetLeft(RecordingBar);var top=Canvas.GetTop(RecordingBar);
        if(!double.IsFinite(left)||!double.IsFinite(top))return false;
        var width=RecordingBar.ActualWidth>0?RecordingBar.ActualWidth:RecordingBar.DesiredSize.Width;
        var height=RecordingBar.ActualHeight>0?RecordingBar.ActualHeight:RecordingBar.DesiredSize.Height;
        return width>0&&height>0&&new Rect(left,top,width,height).Contains(local);
    }

    private static void ReleaseSelectionResources(SelectionItem item)
    {
        try{item.VideoPreview?.Dispose();item.VideoPreview=null;}catch(Exception ex){new PrivacyLogger().Error("OverlayVideoClose",ex);}
        try{ClearTextSelection(item);}catch(Exception ex){new PrivacyLogger().Error("OverlayTextSelectionClose",ex);}
        item.VideoLease?.Dispose();item.VideoLease=null;
    }

    private void OnMouseDown(object s,MouseButtonEventArgs e)
    {
        if(_recordingMode||_drawingMode)return;
        if(e.OriginalSource is Thumb||IsInside(e.OriginalSource as DependencyObject,PromptBar)||IsInside(e.OriginalSource as DependencyObject,Toolbar)||IsInside(e.OriginalSource as DependencyObject,DrawingToolbar)||IsInside(e.OriginalSource as DependencyObject,RecordingBar)||_selections.Any(item=>IsInside(e.OriginalSource as DependencyObject,item.TextSelection)))return;
        if(RejectIfOverlayOperationBusy())return;
        _pointerOperationBefore=CaptureOverlaySnapshot();
        var p=e.GetPosition(Root);var addNew=_forceNewSelection||Keyboard.Modifiers.HasFlag(ModifierKeys.Shift);_forceNewSelection=false;
        var hit=addNew?-1:FindSelection(p);
        if(hit>=0){Select(hit);_moving=true;_pointerOperationLabel="移动截图区域";_moveStart=p;_moveOrigin=Active!.Bounds;}
        else{RemoveImplicitSelections();var item=CreateSelection(false);_selections.Add(item);_activeIndex=_selections.Count-1;RefreshSelectionNumbers();_selecting=true;_pointerOperationLabel="新建截图区域";_start=p;item.Bounds=new Rect(p,p);}
        Toolbar.Visibility=Visibility.Collapsed;SetPromptBarHidden(true);Root.CaptureMouse();e.Handled=true;
    }

    private void OnMouseMove(object s,MouseEventArgs e)
    {
        if(_recordingMode||_drawingMode)return;
        var p=e.GetPosition(Root);
        if(_selecting&&Active is { } created){created.Bounds=Normalize(new Rect(_start,p));UpdateSelection(created);}
        else if(_moving&&Active is { } moved){var d=p-_moveStart;var next=ClampSelection(new Rect(_moveOrigin.X+d.X,_moveOrigin.Y+d.Y,_moveOrigin.Width,_moveOrigin.Height));if(CaptureOverlayPolicy.HasContentGeometryChanged(moved.Bounds,next))InvalidateImageDerivedLayers(moved);moved.Bounds=next;UpdateSelection(moved);}
        else
        {
            if(Active is null)PositionPromptBar();
            var preserveToolbarPlacement=PointerInToolbarInteractionZone(p);
            SetPromptBarHidden(preserveToolbarPlacement||PointerOverSelection(p),preserveToolbarPlacement);
            return;
        }
    }

    private void OnMouseUp(object s,MouseButtonEventArgs e)
    {
        if(_recordingMode||_drawingMode)return;
        if(!_selecting&&!_moving)return;_selecting=_moving=false;Root.ReleaseMouseCapture();
        if(Active is not { } item||!CaptureOverlayPolicy.IsUsableSelection(item.Bounds.Width,item.Bounds.Height)){RemoveActiveSelection(false);_pointerOperationBefore=null;_pointerOperationLabel="";if(Active is not null)ShowToolbar();SetPromptBarHidden(false);return;}
        UpdateSelection(item);PositionPromptBar();ShowToolbar();SetPromptBarHidden(PointerOverSelection(e.GetPosition(Root)));PromptStatus.Text=$"已选择 {_selections.Count} 个区域 · 可继续拖动添加";e.Handled=true;
        if(_pointerOperationBefore is { } before)RecordGeometryOperationIfChanged(before,_pointerOperationLabel);_pointerOperationBefore=null;_pointerOperationLabel="";
    }

    private void OnLostMouseCapture(object s,MouseEventArgs e)=>FinishInterruptedPointerInteraction();
    private void OnDeactivated(object? s,EventArgs e)
    {
        FinishInterruptedPointerInteraction();
        if(_drawingMode)FinishInterruptedDrawingMode();
    }
    private void FinishInterruptedPointerInteraction()
    {
        if(!_selecting&&!_moving)return;_selecting=_moving=false;if(Root.IsMouseCaptured)Root.ReleaseMouseCapture();
        if(Active is not { } item||!CaptureOverlayPolicy.IsUsableSelection(item.Bounds.Width,item.Bounds.Height)){RemoveActiveSelection(false);_pointerOperationBefore=null;_pointerOperationLabel="";PromptStatus.Text="框选已中断，请重新拖动选择";}
        else{UpdateSelection(item);PositionPromptBar();ShowToolbar();if(_pointerOperationBefore is { } before)RecordGeometryOperationIfChanged(before,_pointerOperationLabel);_pointerOperationBefore=null;_pointerOperationLabel="";PromptStatus.Text="框选已结束，可继续操作";}
        SetPromptBarHidden(false);
    }

    private SelectionItem CreateSelection(bool implicitFullScreen)
    {
        var item=new SelectionItem{IsImplicit=implicitFullScreen};_ownedSelections.Add(item);item.Badge.Child=item.BadgeText;item.Markup.DefaultDrawingAttributes=RegularDrawingAttributes(Colors.Red);item.Markup.StrokeCollected+=(_,_)=>{item.Redo.Clear();if(_drawingMode)_drawingOperationChanged=true;};item.Markup.PreviewMouseLeftButtonDown+=MarkupDown;item.Markup.PreviewMouseMove+=MarkupMove;item.Markup.PreviewMouseLeftButtonUp+=MarkupUp;item.Markup.LostMouseCapture+=MarkupLostMouseCapture;item.Host.Children.Add(item.Image);item.Host.Children.Add(item.Video);item.Host.Children.Add(item.Markup);item.Host.Children.Add(item.TextOverlays);item.Host.Children.Add(item.AiAnnotations);item.Host.Children.Add(item.TextSelection);item.Host.Children.Add(item.Outline);item.Host.Children.Add(item.Badge);SelectionLayer.Children.Add(item.Host);return item;
    }

    private OverlaySnapshot CaptureOverlaySnapshot()=>new(
        _selections.Select(item=>new SelectionSnapshot(
            item,
            item.Bounds,
            _references.Contains(item),
            new StrokeCollection(item.Markup.Strokes.Select(stroke=>stroke.Clone())),
            item.TextLayer,
            item.AnnotationNotes.ToArray())).ToArray(),
        Active,
        AnswerText.Markdown,
        _answerExpanded,
        _history.ToArray(),
        _lastSentSelections.ToArray());

    private void RecordOverlayOperation(OverlaySnapshot before,string label)=>
        _overlayHistory.Record(before,CaptureOverlaySnapshot(),label);

    private void RecordGeometryOperationIfChanged(OverlaySnapshot before,string label)
    {
        if(before.Selections.Count!=_selections.Count){RecordOverlayOperation(before,label);return;}
        var active=Active;var previous=active is null?null:before.Selections.FirstOrDefault(state=>ReferenceEquals(state.Item,active));
        if(previous is not null&&CaptureOverlayPolicy.HasContentGeometryChanged(previous.Bounds,active!.Bounds))RecordOverlayOperation(before,label);
    }

    private void ApplyOverlaySnapshot(OverlaySnapshot snapshot)
    {
        var targetItems=snapshot.Selections.Select(state=>state.Item).ToHashSet();
        SelectionLayer.Children.Clear();
        _selections.Clear();_references.Clear();
        foreach(var state in snapshot.Selections)
        {
            var item=state.Item;
            SelectionLayer.Children.Add(item.Host);
            item.Bounds=state.Bounds;
            item.Markup.Strokes.Clear();
            foreach(var stroke in state.Markup)item.Markup.Strokes.Add(stroke.Clone());
            item.Redo.Clear();item.TextLayer=state.TextLayer;
            item.AnnotationNotes.Clear();item.AnnotationNotes.AddRange(state.AnnotationNotes);
            ApplyTextLayerState(item);RenderAnnotationsForItem(item);
            if(state.Referenced)_references.Add(item);
            _selections.Add(item);
        }
        _activeIndex=snapshot.Active is null?-1:_selections.IndexOf(snapshot.Active);
        if(_activeIndex<0&&_selections.Count>0)_activeIndex=_selections.Count-1;
        _history.Clear();_history.AddRange(snapshot.History);
        _lastSentSelections=[..snapshot.LastSentSelections.Where(targetItems.Contains)];
        _answerExpanded=false;AnswerText.Markdown=snapshot.AnswerMarkdown;
        AnswerHeader.Visibility=AnswerScroll.Visibility=AnswerDivider.Visibility=Visibility.Collapsed;
        if(snapshot.AnswerExpanded&&snapshot.AnswerMarkdown.Length>0)ShowAnswer();
        _reasoningBuffer.Clear();ReasoningText.Text="";ReasoningToggle.Visibility=ReasoningPanel.Visibility=Visibility.Collapsed;
        ResolveOverlayInteractionWithFallback();AgentActivityItems.Children.Clear();AgentActivityCard.Visibility=AiInteractionCard.Visibility=Visibility.Collapsed;
        RefreshSelectionNumbers();
        foreach(var item in _selections)UpdateSelection(item);
        if(Active is null){HideHandles();SizeText.Visibility=Toolbar.Visibility=Visibility.Collapsed;}else ShowToolbar();
        SetPromptBarHidden(false);PositionPromptBar();
    }

    private void UndoOverlayOperation()
    {
        if(!_overlayHistory.TryUndo(out var snapshot,out var label)){PromptStatus.Text="没有可撤销的截图操作";return;}
        ApplyOverlaySnapshot(snapshot);PromptStatus.Text=$"已撤销：{label}";
    }

    private void RedoOverlayOperation()
    {
        if(!_overlayHistory.TryRedo(out var snapshot,out var label)){PromptStatus.Text="没有可重做的截图操作";return;}
        ApplyOverlaySnapshot(snapshot);PromptStatus.Text=$"已重做：{label}";
    }

    private void UpdateSelection(SelectionItem item)
    {
        var r=Normalize(item.Bounds);item.Bounds=r;Canvas.SetLeft(item.Host,r.Left);Canvas.SetTop(item.Host,r.Top);item.Host.Width=r.Width;item.Host.Height=r.Height;item.Markup.Width=item.TextOverlays.Width=item.AiAnnotations.Width=item.TextSelection.Width=r.Width;item.Markup.Height=item.TextOverlays.Height=item.AiAnnotations.Height=item.TextSelection.Height=r.Height;
        var px=ToPixelRect(r);if(px.Width>0&&px.Height>0&&item.VideoPath is null)item.Image.Source=ScreenCaptureService.Crop(_frame.Image,px);
        var active=ReferenceEquals(item,Active);var referenced=_references.Contains(item);item.Outline.BorderBrush=item.IsImplicit?Brushes.Transparent:active?Cyan:referenced?new SolidColorBrush(Color.FromRgb(102,112,235)):new SolidColorBrush(Color.FromArgb(185,67,168,255));item.Outline.BorderThickness=new Thickness(active?2.5:referenced?2:1.5);item.Outline.Effect=active&&!item.IsImplicit?new DropShadowEffect{Color=Color.FromRgb(39,157,255),BlurRadius=18,ShadowDepth=0,Opacity=.85}:null;item.Badge.Background=new SolidColorBrush(referenced?Color.FromArgb(238,91,101,226):Color.FromArgb(230,29,119,224));item.Badge.Visibility=item.IsImplicit?Visibility.Collapsed:Visibility.Visible;
        if(active&&!item.IsImplicit){SizeTextLabel.Text=item.VideoPath is null?$"{px.Width} × {px.Height}":$"视频 · {item.VideoDuration:mm\\:ss}";SizeText.Visibility=Visibility.Visible;Canvas.SetLeft(SizeText,r.Left);Canvas.SetTop(SizeText,Math.Max(0,r.Top-30));PositionHandles(r);}else if(item.IsImplicit){HideHandles();SizeText.Visibility=Visibility.Collapsed;}
    }

    private void Select(int index){_activeIndex=index;for(var i=0;i<_selections.Count;i++)UpdateSelection(_selections[i]);}
    private int FindSelection(Point p){for(var i=_selections.Count-1;i>=0;i--)if(!_selections[i].IsImplicit&&_selections[i].Bounds.Contains(p))return i;return -1;}
    private bool PointerOverSelection(Point p)
    {
        var promptLeft=Canvas.GetLeft(PromptBarHost);var promptTop=Canvas.GetTop(PromptBarHost);
        var promptWidth=PromptBar.ActualWidth>0?PromptBar.ActualWidth:PromptBar.DesiredSize.Width;
        var promptHeight=PromptBar.ActualHeight>0?PromptBar.ActualHeight:PromptBar.DesiredSize.Height;
        var promptBounds=double.IsFinite(promptLeft)&&double.IsFinite(promptTop)&&promptWidth>0&&promptHeight>0
            ?new Rect(promptLeft,promptTop,promptWidth,promptHeight)
            :Rect.Empty;
        return CaptureOverlayPolicy.ShouldAutoHidePromptBar(p,promptBounds,PromptMonitorBounds(),_selections.Where(item=>!item.IsImplicit).Select(item=>item.Bounds));
    }
    private bool PointerOverPromptBar(Point point)
    {
        var left=Canvas.GetLeft(PromptBarHost);var top=Canvas.GetTop(PromptBarHost);
        var width=PromptBar.ActualWidth>0?PromptBar.ActualWidth:PromptBar.DesiredSize.Width;
        var height=PromptBar.ActualHeight>0?PromptBar.ActualHeight:PromptBar.DesiredSize.Height;
        return double.IsFinite(left)&&double.IsFinite(top)&&width>0&&height>0&&new Rect(left,top,width,height).Contains(point);
    }
    private bool PointerInToolbarInteractionZone(Point point)
    {
        if(Toolbar.Visibility!=Visibility.Visible)return false;
        var left=Canvas.GetLeft(Toolbar);var top=Canvas.GetTop(Toolbar);
        var width=Toolbar.ActualWidth>0?Toolbar.ActualWidth:Toolbar.DesiredSize.Width;
        var height=Toolbar.ActualHeight>0?Toolbar.ActualHeight:Toolbar.DesiredSize.Height;
        var bounds=double.IsFinite(left)&&double.IsFinite(top)&&width>0&&height>0
            ?new Rect(left,top,width,height)
            :Rect.Empty;
        // Include the selection-to-toolbar gap so the prompt cannot reappear
        // during the short pointer transit into a toolbar placed below/above.
        return CaptureOverlayPolicy.IsPointerInFloatingBarInteractionZone(point,bounds,PromptFloatingGap+2);
    }
    private void ToolbarMouseEnter(object sender,MouseEventArgs e)=>SetPromptBarHidden(true,true);
    private static bool IsInside(DependencyObject? source,DependencyObject parent){while(source is not null){if(ReferenceEquals(source,parent))return true;source=VisualTreeHelper.GetParent(source);}return false;}
    private Int32Rect ToPixelRect(Rect r)=>ScreenCoordinateService.ToPixelRect(r,Root.ActualWidth,Root.ActualHeight,_frame.Image.PixelWidth,_frame.Image.PixelHeight);
    private static Rect Normalize(Rect r)=>new(Math.Min(r.Left,r.Right),Math.Min(r.Top,r.Bottom),Math.Abs(r.Width),Math.Abs(r.Height));
    private Rect ClampSelection(Rect value){var width=Math.Min(value.Width,Root.ActualWidth);var height=Math.Min(value.Height,Root.ActualHeight);return new Rect(Math.Clamp(value.X,0,Math.Max(0,Root.ActualWidth-width)),Math.Clamp(value.Y,0,Math.Max(0,Root.ActualHeight-height)),width,height);}
    private Rect MonitorBounds(Rect selection)
    {
        var pixels=ToPixelRect(selection);var center=new System.Drawing.Point(_frame.OriginX+pixels.X+pixels.Width/2,_frame.OriginY+pixels.Y+pixels.Height/2);var bounds=System.Windows.Forms.Screen.FromPoint(center).WorkingArea;
        return ScreenCoordinateService.ToLocalDipRect(new ScreenRect(bounds.X,bounds.Y,bounds.Width,bounds.Height),_frame.OriginX,_frame.OriginY,Root.ActualWidth,Root.ActualHeight,_frame.Image.PixelWidth,_frame.Image.PixelHeight);
    }
    private Rect PromptMonitorBounds()
    {
        if(Active is {IsImplicit:false} item)return MonitorBounds(item.Bounds);
        var bounds=System.Windows.Forms.Screen.FromPoint(System.Windows.Forms.Cursor.Position).WorkingArea;
        return ScreenCoordinateService.ToLocalDipRect(new ScreenRect(bounds.X,bounds.Y,bounds.Width,bounds.Height),_frame.OriginX,_frame.OriginY,Root.ActualWidth,Root.ActualHeight,_frame.Image.PixelWidth,_frame.Image.PixelHeight);
    }

    private void ShowToolbar()
    {
        if(Active is not {IsImplicit:false} item||_recordingMode){Toolbar.Visibility=Visibility.Collapsed;return;}
        var regionNumber=_activeIndex+1;var type=item.VideoPath is null?"区域":"视频";ReferenceButton.ToolTip=_references.Contains(item)?$"{type}{regionNumber} 已引用；可在输入框移除":$"引用当前{type}为 @{type}{regionNumber}";ReferenceButton.Background=new SolidColorBrush(_references.Contains(item)?Color.FromRgb(218,239,231):Color.FromRgb(233,237,255));
        var isVideo=item.VideoPath is not null;DrawButton.Visibility=RecordButton.Visibility=TranslateButton.Visibility=OcrButton.Visibility=isVideo?Visibility.Collapsed:Visibility.Visible;VideoPlayButton.Visibility=isVideo?Visibility.Visible:Visibility.Collapsed;PinButton.ToolTip=isVideo?"贴视频 (P)":"贴图 (P)";CopyButton.ToolTip=isVideo?"复制视频文件 (C)":"复制图片 (C)";SaveButton.ToolTip=isVideo?"保存 MP4 / GIF (S)":"保存图片 (S)";
        Toolbar.Visibility=Visibility.Visible;PositionFloatingBar(Toolbar,item);
    }

    private void PositionFloatingBar(FrameworkElement bar,SelectionItem item)
    {
        var monitor=MonitorBounds(item.Bounds);var availableWidth=Math.Max(1,monitor.Width-PromptEdgeMargin*2);bar.MaxWidth=availableWidth;bar.Measure(new Size(availableWidth,double.PositiveInfinity));var w=CaptureOverlayPolicy.ConstrainFloatingBarWidth(monitor,bar.DesiredSize.Width);var h=bar.DesiredSize.Height;var x=Math.Clamp(item.Bounds.Left,monitor.Left+PromptEdgeMargin,Math.Max(monitor.Left+PromptEdgeMargin,monitor.Right-w-PromptEdgeMargin));var promptTop=Canvas.GetTop(PromptBarHost);var promptLeft=Canvas.GetLeft(PromptBarHost);var promptWidth=Math.Max(PromptBar.ActualWidth,PromptBar.DesiredSize.Width);var promptOverlapsMonitor=double.IsFinite(promptTop)&&double.IsFinite(promptLeft)&&promptLeft<monitor.Right&&promptLeft+promptWidth>monitor.Left;var availableBottom=_promptBarHidden||PromptBarHost.Visibility!=Visibility.Visible||!promptOverlapsMonitor?monitor.Bottom-PromptEdgeMargin:Math.Min(monitor.Bottom-PromptEdgeMargin,promptTop-PromptFloatingGap);var below=item.Bounds.Bottom+PromptFloatingGap;var above=item.Bounds.Top-h-PromptFloatingGap;var y=below+h<=availableBottom?below:above>=monitor.Top+PromptEdgeMargin?above:Math.Clamp(below,monitor.Top+PromptEdgeMargin,Math.Max(monitor.Top+PromptEdgeMargin,availableBottom-h));Canvas.SetLeft(bar,x);Canvas.SetTop(bar,y);
        if(ReferenceEquals(bar,Toolbar)&&SizeText.Visibility==Visibility.Visible){SizeText.Measure(new Size(double.PositiveInfinity,double.PositiveInfinity));var sizeHeight=SizeText.DesiredSize.Height;var preferred=y<item.Bounds.Top?y-sizeHeight-4:item.Bounds.Top-sizeHeight-4;var sizeY=preferred>=monitor.Top+4?preferred:Math.Min(item.Bounds.Bottom-sizeHeight-4,item.Bounds.Top+4);Canvas.SetLeft(SizeText,item.Bounds.Left);Canvas.SetTop(SizeText,sizeY);}
    }

    private void PositionPromptBar()
    {
        if(_positioningPromptBar||Root.ActualWidth<=0||Root.ActualHeight<=0)return;
        var monitor=PromptMonitorBounds();
        if(monitor.IsEmpty)return;
        _positioningPromptBar=true;
        try
        {
            var availableWidth=Math.Max(1,monitor.Width-CaptureOverlayPolicy.PromptSideMargin*2);
            PromptBar.Width=Math.Min(Math.Min(CaptureOverlayPolicy.PromptPreferredWidth,PromptPreferredWidthTight),availableWidth);
            // Remove a stale explicit height before measuring content that may
            // have gained reference chips or an answer since the last pass.
            PromptBar.Height=double.NaN;
            PromptBar.MaxHeight=Math.Max(1,monitor.Height-CaptureOverlayPolicy.PromptTopMargin-CaptureOverlayPolicy.PromptBottomMargin);
            // The previous compact pass intentionally leaves ResponseScroll at
            // MaxHeight=0.  Lift that cap before measuring newly visible answer
            // content, otherwise the zero-height response feeds back into the
            // next desired size and the first answer can never expand.
            ResponseScroll.MaxHeight=CaptureOverlayPolicy.GetPromptResponseMaxHeight(PromptBar.MaxHeight);
            PromptBar.Measure(new Size(PromptBar.Width,PromptBar.MaxHeight));
            var desiredHeight=PromptBar.DesiredSize.Height;
            var bounds=CaptureOverlayPolicy.GetPromptBarBounds(monitor,desiredHeight);
            if(bounds.IsEmpty)return;

            // Keep the response budget tied to the monitor's usable height.
            // Feeding the current desired card height back as the next cap
            // makes every measure pass progressively shrink the answer.
            ResponseScroll.MaxHeight=CaptureOverlayPolicy.GetPromptResponseMaxHeight(PromptBar.MaxHeight);
            PromptBar.MaxHeight=bounds.Height;
            PromptBar.InvalidateMeasure();
            PromptBar.Measure(new Size(bounds.Width,bounds.Height));

            // DesiredSize can still describe the pre-chip layout during the
            // SizeChanged callback.  Fit both values now, then run one bounded
            // render-priority pass using the arranged height below.
            var measuredHeight=Math.Max(bounds.Height,PromptBar.DesiredSize.Height);
            bounds=CaptureOverlayPolicy.RefitPromptBarAfterArrange(monitor,bounds,Math.Min(measuredHeight,bounds.Height));
            PromptBar.Width=bounds.Width;
            PromptBar.MaxHeight=bounds.Height;
            Canvas.SetLeft(PromptBarHost,bounds.Left);
            Canvas.SetTop(PromptBarHost,bounds.Top);
            if(_answerExpanded)AnswerScroll.MaxHeight=CaptureOverlayPolicy.GetAnswerViewportHeight(monitor.Height);
            UpdatePromptBarHiddenTransform(false);
            QueuePromptBarLayoutClamp();
            if(Toolbar.Visibility==Visibility.Visible)ShowToolbar();
            if(DrawingToolbar.Visibility==Visibility.Visible&&Active is { } item)PositionFloatingBar(DrawingToolbar,item);
        }
        finally{_positioningPromptBar=false;}
    }

    private void QueuePromptBarLayoutClamp()
    {
        if(_promptBarLayoutPassQueued||_closed||!IsLoaded)return;
        _promptBarLayoutPassQueued=true;
        _=Dispatcher.BeginInvoke(DispatcherPriority.Render,new Action(() =>
        {
            _promptBarLayoutPassQueued=false;
            if(_closed||!IsLoaded||PromptBarHost.Visibility!=Visibility.Visible)return;
            var monitor=PromptMonitorBounds();
            if(monitor.IsEmpty)return;
            var left=Canvas.GetLeft(PromptBarHost);
            var top=Canvas.GetTop(PromptBarHost);
            var width=PromptBar.ActualWidth>0?PromptBar.ActualWidth:PromptBar.DesiredSize.Width;
            var height=PromptBar.ActualHeight>0?PromptBar.ActualHeight:PromptBar.DesiredSize.Height;
            if(!double.IsFinite(left)||!double.IsFinite(top)||!double.IsFinite(width)||!double.IsFinite(height)||width<=0||height<=0)return;
            var candidate=new Rect(left,top,width,height);
            var fitted=CaptureOverlayPolicy.RefitPromptBarAfterArrange(monitor,candidate,height);
            PromptBar.Width=fitted.Width;
            if(fitted.Height<height-.25)PromptBar.MaxHeight=fitted.Height;
            Canvas.SetLeft(PromptBarHost,fitted.Left);
            Canvas.SetTop(PromptBarHost,fitted.Top);
            UpdatePromptBarHiddenTransform(false);
        }));
    }
    private void SetPromptBarHidden(bool hidden,bool preserveToolbarPlacement=false){var changed=_promptBarHidden!=hidden;_promptBarHidden=hidden;PromptBarHost.IsHitTestVisible=!hidden;UpdatePromptBarHiddenTransform(changed);if(!preserveToolbarPlacement&&Toolbar.Visibility==Visibility.Visible)ShowToolbar();}
    private void UpdatePromptBarHiddenTransform(bool animate)
    {
        if(PromptBarHost.RenderTransform is not TranslateTransform transform){transform=new TranslateTransform();PromptBarHost.RenderTransform=transform;}
        var target=0d;if(_promptBarHidden){var monitor=PromptMonitorBounds();var top=Canvas.GetTop(PromptBarHost);target=double.IsFinite(top)?Math.Max(PromptBar.ActualHeight+PromptHiddenOffset,monitor.Bottom-top+PromptHiddenOffset):PromptBar.ActualHeight+PromptHiddenOffset;}
        if(animate){var ease=new CubicEase{EasingMode=EasingMode.EaseOut};transform.BeginAnimation(TranslateTransform.YProperty,new DoubleAnimation(target,TimeSpan.FromMilliseconds(_promptBarHidden?140:180)){EasingFunction=ease});PromptBarHost.BeginAnimation(OpacityProperty,new DoubleAnimation(_promptBarHidden?0d:.99,TimeSpan.FromMilliseconds(140)));}
        else{transform.BeginAnimation(TranslateTransform.YProperty,null);transform.Y=target;PromptBarHost.BeginAnimation(OpacityProperty,null);PromptBarHost.Opacity=_promptBarHidden?0d:.99;}
    }
    private void ShowAnswer(){if(_answerExpanded)return;_answerExpanded=true;AnswerHeader.Visibility=AnswerScroll.Visibility=AnswerDivider.Visibility=Visibility.Visible;if(ReasoningToggle.Visibility!=Visibility.Visible&&!string.IsNullOrWhiteSpace(_reasoningBuffer.ToString()))RevealReasoningInProgress();_ = Dispatcher.BeginInvoke(PositionPromptBar);}
    private void ToggleReasoning(object s,RoutedEventArgs e){_reasoningExpanded=!_reasoningExpanded;ReasoningPanel.Visibility=_reasoningExpanded?Visibility.Visible:Visibility.Collapsed;ReasoningChevronRotation.Angle=_reasoningExpanded?180:0;_ = Dispatcher.BeginInvoke(PositionPromptBar);}
    private void ShowReasoning(string delta,CancellationTokenSource request)
    {
        AppendReasoning(delta);if(_answerExpanded&&ReasoningToggle.Visibility!=Visibility.Visible)RevealReasoningInProgress();if(_reasoningRenderScheduled&&ReferenceEquals(_reasoningRenderRequest,request))return;_reasoningRenderScheduled=true;_reasoningRenderRequest=request;
        _ = Dispatcher.BeginInvoke(DispatcherPriority.Background,new Action(()=>
        {
            if(!ReferenceEquals(_reasoningRenderRequest,request))return;
            _reasoningRenderScheduled=false;_reasoningRenderRequest=null;
            if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;
            ReasoningText.Text=LimitReasoning(_reasoningBuffer.ToString());_ = Dispatcher.BeginInvoke(PositionPromptBar);
        }));
    }
    private void RevealReasoningInProgress()
    {
        if(string.IsNullOrWhiteSpace(_reasoningBuffer.ToString()))return;ReasoningToggle.Visibility=Visibility.Visible;_reasoningExpanded=true;ReasoningPanel.Visibility=Visibility.Visible;ReasoningChevronRotation.Angle=180;ReasoningLabel.Text="正在思考…";ReasoningPulse.Background=new SolidColorBrush(Color.FromRgb(123,138,244));ReasoningPulse.BeginAnimation(OpacityProperty,new DoubleAnimation(.35,1,TimeSpan.FromMilliseconds(650)){AutoReverse=true,RepeatBehavior=RepeatBehavior.Forever});
    }
    private void FinishReasoning(string reasoning)
    {
        if(!string.IsNullOrWhiteSpace(reasoning)){_reasoningBuffer.Clear();AppendReasoning(reasoning.Trim());ReasoningText.Text=LimitReasoning(_reasoningBuffer.ToString());}CloseReasoning("思考过程 · 已完成",Color.FromRgb(95,181,137));
    }
    private void CloseReasoning(string label,Color color)
    {
        _reasoningRenderRequest=null;_reasoningRenderScheduled=false;ReasoningPulse.BeginAnimation(OpacityProperty,null);ReasoningPulse.Opacity=1;ReasoningPulse.Background=new SolidColorBrush(color);
        if(string.IsNullOrWhiteSpace(ReasoningText.Text)&&_reasoningBuffer.Length>0)ReasoningText.Text=LimitReasoning(_reasoningBuffer.ToString());
        _reasoningExpanded=false;ReasoningPanel.Visibility=Visibility.Collapsed;ReasoningChevronRotation.Angle=0;
        if(string.IsNullOrWhiteSpace(ReasoningText.Text)||!_answerExpanded){ReasoningToggle.Visibility=Visibility.Collapsed;return;}
        ReasoningToggle.Visibility=Visibility.Visible;ReasoningLabel.Text=label;_ = Dispatcher.BeginInvoke(PositionPromptBar);
    }
    private void ResetAnswerForRequest()
    {
        foreach(var item in _selections){item.AnnotationNotes.Clear();item.AiAnnotations.Children.Clear();}
        _lastSentSelections.Clear();
        ResolveOverlayInteractionWithFallback();AgentActivityItems.Children.Clear();AgentActivityCard.Visibility=AiInteractionCard.Visibility=Visibility.Collapsed;_answerExpanded=false;AnswerText.Markdown="";AnswerHeader.Visibility=AnswerScroll.Visibility=AnswerDivider.Visibility=Visibility.Collapsed;_reasoningBuffer.Clear();_reasoningRenderScheduled=false;_reasoningRenderRequest=null;ReasoningText.Text="";ReasoningToggle.Visibility=ReasoningPanel.Visibility=Visibility.Collapsed;ReasoningPulse.BeginAnimation(OpacityProperty,null);ReasoningPulse.Background=new SolidColorBrush(Color.FromRgb(123,138,244));_reasoningExpanded=false;_ = Dispatcher.BeginInvoke(PositionPromptBar);
    }

    private void UpdateOverlayAgentActivity(AiAgentEvent update,CancellationTokenSource request)
    {
        if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;
        ShowAnswer();
        var key=$"{update.Kind}:{update.Title}";
        var line=AgentActivityItems.Children.OfType<TextBlock>().FirstOrDefault(item=>string.Equals(item.Tag as string,key,StringComparison.Ordinal));
        if(line is null)
        {
            line=new TextBlock{Tag=key,FontSize=11,TextWrapping=TextWrapping.Wrap,Margin=new Thickness(0,2,0,2),Foreground=new SolidColorBrush(Color.FromRgb(91,105,128))};
            AgentActivityItems.Children.Add(line);
            while(AgentActivityItems.Children.Count>6)AgentActivityItems.Children.RemoveAt(0);
        }
        var state=update.IsError?"失败":update.Kind==AiAgentEventKind.ToolCompleted?"完成":"进行中";
        var detail=string.IsNullOrWhiteSpace(update.Detail)?string.Empty:$" · {LimitAgentDetail(update.Detail)}";
        line.Text=$"{update.Title} · {state}{detail}";
        line.Foreground=new SolidColorBrush(update.IsError?Color.FromRgb(196,73,83):update.Kind==AiAgentEventKind.ToolCompleted?Color.FromRgb(34,157,105):Color.FromRgb(91,105,128));
        AgentActivityCard.Visibility=Visibility.Visible;
        _=Dispatcher.BeginInvoke(PositionPromptBar);
    }

    private Task<AiInteractionResponse> HandleOverlayInteractionAsync(AiInteractionRequest interaction,CancellationToken cancellationToken)
    {
        if(_closed||cancellationToken.IsCancellationRequested)return Task.FromCanceled<AiInteractionResponse>(cancellationToken);
        return Dispatcher.CheckAccess()?BeginOverlayInteraction(interaction,cancellationToken):Dispatcher.InvokeAsync(()=>BeginOverlayInteraction(interaction,cancellationToken)).Task.Unwrap();
    }

    private Task<AiInteractionResponse> BeginOverlayInteraction(AiInteractionRequest interaction,CancellationToken cancellationToken)
    {
        ResolveOverlayInteractionWithFallback();
        var fallback=interaction.Kind==AiInteractionKind.Approval?new AiInteractionResponse(string.Empty,"deny"):new AiInteractionResponse(string.Empty);
        var completion=new TaskCompletionSource<AiInteractionResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
        _activeInteraction=completion;_activeInteractionFallback=fallback;AiInteractionContent.Children.Clear();
        AiInteractionContent.Children.Add(new TextBlock{Text=interaction.Title,FontWeight=FontWeights.SemiBold,FontSize=13,Foreground=new SolidColorBrush(Color.FromRgb(65,76,95))});
        if(!string.IsNullOrWhiteSpace(interaction.Message))AiInteractionContent.Children.Add(new TextBlock{Text=interaction.Message,TextWrapping=TextWrapping.Wrap,Margin=new Thickness(0,5,0,8),Foreground=new SolidColorBrush(Color.FromRgb(87,99,119))});
        if(interaction.Choices.Count>0)AddOverlayChoiceInteraction(interaction);else AddOverlayTextInteraction(interaction);
        ShowAnswer();AiInteractionCard.Visibility=Visibility.Visible;PromptStatus.Text="Hermes 等待你的确认";_interactionCancellation=cancellationToken.Register(()=>Dispatcher.BeginInvoke(new Action(ResolveOverlayInteractionWithFallback)));_=Dispatcher.BeginInvoke(PositionPromptBar);
        return completion.Task;
    }

    private void AddOverlayChoiceInteraction(AiInteractionRequest interaction)
    {
        if(interaction.MultiSelect)
        {
            var checks=interaction.Choices.Select(choice=>new CheckBox{Content=choice,Tag=choice,Margin=new Thickness(0,2,0,2)}).ToList();
            foreach(var check in checks)AiInteractionContent.Children.Add(check);
            var actions=OverlayInteractionActions();
            actions.Children.Add(OverlayInteractionButton("确认",true,()=>CompleteOverlayInteraction(new AiInteractionResponse(string.Empty,Values:checks.Where(check=>check.IsChecked==true).Select(check=>(string)check.Tag).ToArray()))));
            actions.Children.Add(OverlayInteractionButton("取消",false,ResolveOverlayInteractionWithFallback));AiInteractionContent.Children.Add(actions);return;
        }
        var row=OverlayInteractionActions();
        foreach(var choice in interaction.Choices){var captured=choice;row.Children.Add(OverlayInteractionButton(InteractionChoiceLabel(captured),!captured.Equals("deny",StringComparison.OrdinalIgnoreCase),()=>CompleteOverlayInteraction(new AiInteractionResponse(captured,captured))));}
        AiInteractionContent.Children.Add(row);
    }

    private void AddOverlayTextInteraction(AiInteractionRequest interaction)
    {
        Control input;
        if(interaction.IsSensitive){var password=new PasswordBox{MinHeight=34,Padding=new Thickness(8,5,8,5)};_activeSensitiveInput=password;input=password;}
        else input=new TextBox{MinHeight=34,Padding=new Thickness(8,5,8,5),TextWrapping=TextWrapping.Wrap};
        AiInteractionContent.Children.Add(input);var actions=OverlayInteractionActions();
        actions.Children.Add(OverlayInteractionButton("提交",true,()=>CompleteOverlayInteraction(new AiInteractionResponse(input is PasswordBox password?password.Password:((TextBox)input).Text))));
        actions.Children.Add(OverlayInteractionButton("取消",false,ResolveOverlayInteractionWithFallback));AiInteractionContent.Children.Add(actions);_=Dispatcher.BeginInvoke(new Action(()=>Keyboard.Focus(input)));
    }

    private static StackPanel OverlayInteractionActions()=>new(){Orientation=Orientation.Horizontal,Margin=new Thickness(0,8,0,0)};
    private static Button OverlayInteractionButton(string text,bool primary,Action action){var button=new Button{Content=text,MinWidth=70,MinHeight=30,Margin=new Thickness(0,0,7,0),Padding=new Thickness(12,4,12,4)};if(primary){button.Background=new SolidColorBrush(Color.FromRgb(74,111,222));button.Foreground=Brushes.White;}button.Click+=(_,_)=>action();return button;}
    private static string InteractionChoiceLabel(string choice)=>choice.ToLowerInvariant() switch{"once"=>"允许一次","session"=>"本次会话允许","always"=>"始终允许","deny"=>"拒绝",_=>choice};

    private void CompleteOverlayInteraction(AiInteractionResponse response)
    {
        var completion=_activeInteraction;if(completion is null)return;_activeInteraction=null;_activeInteractionFallback=null;_activeSensitiveInput?.Clear();_activeSensitiveInput=null;_interactionCancellation.Dispose();AiInteractionContent.Children.Clear();AiInteractionCard.Visibility=Visibility.Collapsed;PromptStatus.Text="Hermes 已收到选择，继续处理中…";completion.TrySetResult(response);_=Dispatcher.BeginInvoke(PositionPromptBar);
    }
    private void ResolveOverlayInteractionWithFallback(){if(_activeInteraction is not null)CompleteOverlayInteraction(_activeInteractionFallback??new AiInteractionResponse(string.Empty));}
    private static string LimitAgentDetail(string value){var normalized=value?.Trim()??string.Empty;return normalized.Length<=600?normalized:normalized[..600]+"…";}

    private async Task BeginOverlayReadAloudAsync(string text)
    {
        StopOverlayReadAloud();var request=new CancellationTokenSource();_readAloudRequest=request;
        try{await _host.ReadHermesResponseAloudAsync(text,request.Token);}catch(OperationCanceledException)when(request.IsCancellationRequested){}catch(Exception ex){if(!_closed&&ReferenceEquals(_readAloudRequest,request))PromptStatus.Text=$"自动朗读失败：{ex.Message}";}finally{request.Dispose();if(ReferenceEquals(_readAloudRequest,request))_readAloudRequest=null;}
    }
    private void StopOverlayReadAloud(){var request=_readAloudRequest;_readAloudRequest=null;try{request?.Cancel();}catch(ObjectDisposedException){}_host.StopHermesReadAloud();}
    private void AppendReasoning(string value)
    {
        const int bufferLimit=ReasoningDisplayLimit*2;if(value.Length>=bufferLimit){_reasoningBuffer.Clear();_reasoningBuffer.Append(value.AsSpan(value.Length-bufferLimit));return;}var overflow=_reasoningBuffer.Length+value.Length-bufferLimit;if(overflow>0)_reasoningBuffer.Remove(0,overflow);_reasoningBuffer.Append(value);
    }
    private static string LimitReasoning(string value)=>value.Length<=ReasoningDisplayLimit?value:"…较早思考内容已收纳…\n"+value[^ReasoningDisplayLimit..];

    private BitmapSource CurrentImage(){if(Active is null)throw new InvalidOperationException("请先选择区域");return RenderSelectionImage(Active);}
    private BitmapSource RenderSelectionImage(SelectionItem item)
    {
        var pixels=ToPixelRect(item.Bounds);var source=ScreenCaptureService.Crop(_frame.Image,pixels);if(item.Markup.Strokes.Count==0)return source;var visual=new DrawingVisual();using(var drawing=visual.RenderOpen()){drawing.PushTransform(new ScaleTransform(pixels.Width/Math.Max(1,item.Bounds.Width),pixels.Height/Math.Max(1,item.Bounds.Height)));drawing.DrawImage(source,new Rect(0,0,item.Bounds.Width,item.Bounds.Height));drawing.DrawRectangle(new VisualBrush(item.Markup),null,new Rect(0,0,item.Bounds.Width,item.Bounds.Height));drawing.Pop();}var bitmap=new RenderTargetBitmap(Math.Max(1,pixels.Width),Math.Max(1,pixels.Height),96,96,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();return bitmap;
    }
    private async Task<List<AiAttachment>> BuildAttachmentsAsync(IReadOnlyList<SelectionItem> targets,AiProviderCapabilities capabilities,CancellationToken cancellationToken)
    {
        _lastSentSelections=targets.ToList();
        var prepared=_lastSentSelections.Select(item=>(Item:item,Image:item.VideoPath is null?RenderSelectionImage(item):null)).ToList();var imageCount=prepared.Count(entry=>entry.Image is not null);var rawVideoBytes=prepared.Where(entry=>entry.Item.VideoPath is not null).Sum(entry=>Math.Min(45L*1024*1024,new FileInfo(entry.Item.VideoPath!).Length));var aggregateImageBudget=Math.Max(256L*1024,45L*1024*1024-rawVideoBytes);var perImageBudget=imageCount==0?0:Math.Min(capabilities.MaxImageSize,aggregateImageBudget/imageCount);
        return await Task.Run(()=>
        {
            var attachments=new List<AiAttachment>(prepared.Count);
            try
            {
                foreach(var entry in prepared)
                {
                    cancellationToken.ThrowIfCancellationRequested();
                    if(entry.Item.VideoPath is { } path)attachments.Add(new AiAttachment(AiAttachmentType.Video,"video/mp4",FilePath:path,Duration:entry.Item.VideoDuration));
                    else{var encoded=AiImageEncodingService.Encode(entry.Image!,perImageBudget,capabilities.AcceptedMimeTypes,cancellationToken);attachments.Add(new AiAttachment(AiAttachmentType.Image,encoded.MimeType,encoded.Data));}
                }
                return attachments;
            }
            catch{AiImageEncodingService.ClearAttachmentBuffers(attachments);throw;}
        },cancellationToken);
    }
    private void EnsureScreenSelection(){if(_selections.Count>0)return;var item=CreateSelection(true);item.Bounds=new Rect(0,0,Root.ActualWidth,Root.ActualHeight);_selections.Add(item);_activeIndex=0;RefreshSelectionNumbers();UpdateSelection(item);}

    private void PositionHandles(Rect r){var list=new[]{Nw,N,Ne,W,E,Sw,S,Se};foreach(var t in list){t.Width=t.Height=10;t.Background=Cyan;t.Visibility=Visibility.Visible;}Set(Nw,r.Left,r.Top);Set(N,r.Left+r.Width/2,r.Top);Set(Ne,r.Right,r.Top);Set(W,r.Left,r.Top+r.Height/2);Set(E,r.Right,r.Top+r.Height/2);Set(Sw,r.Left,r.Bottom);Set(S,r.Left+r.Width/2,r.Bottom);Set(Se,r.Right,r.Bottom);static void Set(Thumb t,double x,double y){Canvas.SetLeft(t,x-5);Canvas.SetTop(t,y-5);}}
    private void HideHandles(){foreach(var t in new[]{Nw,N,Ne,W,E,Sw,S,Se})t.Visibility=Visibility.Collapsed;}
    private void ResizeDelta(object sender,DragDeltaEventArgs e){if(RejectIfOverlayOperationBusy()||sender is not Thumb t||Active is not {IsImplicit:false} item)return;_resizeOperationBefore??=CaptureOverlaySnapshot();SetPromptBarHidden(true);var d=t.Tag?.ToString()??"";var l=item.Bounds.Left;var top=item.Bounds.Top;var r=item.Bounds.Right;var b=item.Bounds.Bottom;if(d.Contains('W'))l=Math.Clamp(l+e.HorizontalChange,0,r-12);if(d.Contains('E'))r=Math.Clamp(r+e.HorizontalChange,l+12,Root.ActualWidth);if(d.Contains('N'))top=Math.Clamp(top+e.VerticalChange,0,b-12);if(d.Contains('S'))b=Math.Clamp(b+e.VerticalChange,top+12,Root.ActualHeight);var next=new Rect(new Point(l,top),new Point(r,b));if(CaptureOverlayPolicy.HasContentGeometryChanged(item.Bounds,next))InvalidateImageDerivedLayers(item);item.Bounds=next;UpdateSelection(item);ShowToolbar();e.Handled=true;}
    private void ResizeCompleted(object sender,DragCompletedEventArgs e){if(_resizeOperationBefore is { } before)RecordGeometryOperationIfChanged(before,"调整截图区域");_resizeOperationBefore=null;PositionPromptBar();if(Active is not null)ShowToolbar();SetPromptBarHidden(PointerOverSelection(Mouse.GetPosition(Root)));e.Handled=true;}

    private void AddRegion(object s,RoutedEventArgs e){if(RejectIfOverlayOperationBusy())return;_forceNewSelection=true;Toolbar.Visibility=Visibility.Collapsed;HideHandles();PromptStatus.Text="拖动以添加另一个区域 · 可与现有区域重叠";SetPromptBarHidden(false);}
    private void ReferenceRegion(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy()||Active is not {IsImplicit:false} item)return;var before=CaptureOverlaySnapshot();var added=_references.Add(item);UpdateReferenceChips();UpdateSelection(item);ShowToolbar();SetPromptBarHidden(false);QuickPrompt.Focus();if(added)RecordOverlayOperation(before,"引用截图区域");PromptStatus.Text=$"已加入 @{(item.VideoPath is null?"区域":"视频")}{_activeIndex+1} · 输入问题后发送";
    }
    private void RemoveSelection(object s,RoutedEventArgs e){if(RejectIfOverlayOperationBusy()||Active is null)return;var before=CaptureOverlaySnapshot();RemoveActiveSelection(true);RecordOverlayOperation(before,"删除截图区域");}
    private void RemoveImplicitSelections()
    {
        var active=Active;var removed=false;
        foreach(var item in _selections.Where(item=>item.IsImplicit).ToList())
        {
            _references.Remove(item);SelectionLayer.Children.Remove(item.Host);_selections.Remove(item);removed=true;
        }
        if(!removed)return;
        _activeIndex=active is not null&&!active.IsImplicit?_selections.IndexOf(active):_selections.Count-1;RefreshSelectionNumbers();
    }
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
            var chip=new Border{Background=new SolidColorBrush(Color.FromRgb(241,245,255)),BorderBrush=new SolidColorBrush(Color.FromRgb(214,222,238)),BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(8),Margin=new Thickness(0,0,4,2),Padding=new Thickness(1)};var row=new StackPanel{Orientation=Orientation.Horizontal};var type=item.VideoPath is null?"区域":"视频";var link=new Button{Content=$"@{type}{_selections.IndexOf(item)+1}",ToolTip=$"定位到此{type}"};link.SetResourceReference(StyleProperty,"ReferenceChipButton");link.Click+=(_,_)=>{var index=_selections.IndexOf(item);if(index<0)return;Select(index);ShowToolbar();SetPromptBarHidden(false);QuickPrompt.Focus();};var remove=new Button{Content=CreateCloseIcon(),ToolTip="移除此引用"};System.Windows.Automation.AutomationProperties.SetName(remove,$"移除{type}{_selections.IndexOf(item)+1}引用");remove.SetResourceReference(StyleProperty,"ReferenceChipRemoveButton");remove.Click+=(_,_)=>{var before=CaptureOverlaySnapshot();if(!_references.Remove(item))return;UpdateReferenceChips();UpdateSelection(item);if(ReferenceEquals(item,Active))ShowToolbar();QuickPrompt.Focus();RecordOverlayOperation(before,"移除区域引用");};row.Children.Add(link);row.Children.Add(remove);chip.Child=row;ReferenceChips.Children.Add(chip);
        }
        ReferenceChips.Visibility=ReferenceChips.Children.Count>0?Visibility.Visible:Visibility.Collapsed;QuickPromptHint.Text=ReferenceChips.Children.Count>0?"继续输入关于引用区域的问题…":"询问当前屏幕，或连续圈选多个区域…";_ = Dispatcher.BeginInvoke(PositionPromptBar);
    }
    private static System.Windows.Shapes.Path CreateCloseIcon()=>new(){Width=12,Height=12,Stretch=Stretch.Uniform,Stroke=new SolidColorBrush(Color.FromRgb(126,139,160)),StrokeThickness=1.8,StrokeStartLineCap=PenLineCap.Round,StrokeEndLineCap=PenLineCap.Round,Data=Geometry.Parse("M3,3 L9,9 M9,3 L3,9")};

    private async Task SendAsync(bool useDefaultPrompt)
    {
        if(_closed||RejectIfOverlayOperationBusy()||_request is {IsCancellationRequested:false})return;StopOverlayReadAloud();var usingHermes=_host.Settings.HermesEnabled;var provider=_host.CreateConversationProvider(HermesConversationKind.Screen,out var providerError);if(provider is null){PromptStatus.Text=providerError??"请先配置可用的 AI Provider";ShowSettingsFromOverlay();return;}
        var before=CaptureOverlaySnapshot();EnsureScreenSelection();var targets=CaptureOverlayPolicy.SelectSendTargets(_selections,item=>item.IsImplicit,_references.Contains);var hasVideo=targets.Any(x=>x.VideoPath is not null);var hasImage=targets.Any(x=>x.VideoPath is null);if(hasVideo&&!provider.Capabilities.SupportsVideo){PromptStatus.Text="当前 Provider 未开启视频理解能力";return;}if(hasImage&&!provider.Capabilities.SupportsImage){PromptStatus.Text="当前模型不支持图片理解";return;}
        var targetCount=targets.Count;if(targetCount>OpenAiCompatibleProvider.AttachmentCountLimit){PromptStatus.Text=$"单次最多发送 {OpenAiCompatibleProvider.AttachmentCountLimit} 个附件，请移除部分引用后重试";return;}var sentDraft=QuickPrompt.Text;var prompt=sentDraft.Trim();if(prompt.Length==0&&useDefaultPrompt)prompt=hasVideo?"按时间顺序说明引用视频中发生了什么，包括主体、动作和画面变化。":targetCount>1?"综合理解这些引用区域，说明它们之间的关系并标出关键部分。":"理解当前引用区域，解释内容并标出关键部分。";if(prompt.Length==0){QuickPrompt.Focus();return;}
        var request=CaptureOverlayPolicy.CreateManualAiRequestCancellation();_request=request;SendButton.IsEnabled=false;ResetAnswerForRequest();PromptStatus.Text=$"正在准备 {targetCount} 个引用区域…按 Esc 可取消";var requestStage="provider";var streamOpen=true;var streamedContent=new System.Text.StringBuilder();var lastPreview=string.Empty;var previewScheduled=false;var attachmentLeases=new List<TempMediaLease>();List<AiAttachment>? attachments=null;
        try
        {
            foreach(var video in targets.Select(item=>item.VideoPath).Where(path=>path is not null))attachmentLeases.Add(TempMediaRegistry.Shared.AcquireExistingFile(video!));
            attachments=await BuildAttachmentsAsync(targets,provider.Capabilities,request.Token);if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;PromptStatus.Text=$"正在分析 {targetCount} 个引用区域…按 Esc 可取消";
            var progress=provider.Capabilities.SupportsStreaming?new Progress<AiStreamDelta>(delta=>{if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed,streamOpen))return;if(delta.ReasoningContent.Length>0)ShowReasoning(delta.ReasoningContent,request);if(delta.Content.Length>0){streamedContent.Append(delta.Content);if(previewScheduled)return;previewScheduled=true;_ = Dispatcher.BeginInvoke(DispatcherPriority.Background,new Action(()=>{previewScheduled=false;if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed,streamOpen))return;var preview=StructuredResponseParser.GetStreamingAnswerPreview(streamedContent.ToString());if(preview.Length==0||string.Equals(preview,lastPreview,StringComparison.Ordinal))return;lastPreview=preview;ShowAnswer();AnswerText.Markdown=preview;AnswerScroll.ScrollToEnd();PromptStatus.Text="正在整理回答…";}));}}):null;
            var agentProgress=usingHermes?new Progress<AiAgentEvent>(update=>UpdateOverlayAgentActivity(update,request)):null;var aiRequest=CaptureOverlayPolicy.CreateScreenAiRequest(prompt,ConversationContextPolicy.CreateBoundedHistory(_history),attachments,progress,agentProgress,usingHermes?HandleOverlayInteractionAsync:null);var result=await provider.SendAsync(aiRequest,request.Token);requestStage="render";streamOpen=false;if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;request.Token.ThrowIfCancellationRequested();var emptyAnswer=AiResultValidation.GetEmptyAnswerMessage(result);if(emptyAnswer is not null){ApplyOverlaySnapshot(before);PromptStatus.Text=emptyAnswer;return;}ShowAnswer();FinishReasoning(result.Reasoning);AnswerText.Markdown=result.Answer;if(CaptureOverlayPolicy.ShouldClearDraft(QuickPrompt.Text,sentDraft))QuickPrompt.Clear();var renderedAnnotationCount=RenderAnnotations(result.Annotations);
            var configured=_host.Settings.Providers.FirstOrDefault(x=>x.Id==provider.Id);var historyProvider=usingHermes?$"本机 Hermes · {_host.Settings.HermesProfile}":configured?.Name??provider.Id;var historyModel=usingHermes?_host.Settings.HermesModel:configured?.Model??string.Empty;if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;request.Token.ThrowIfCancellationRequested();if(_host.Settings.SaveConversationHistory)await new ConversationHistoryService().TryAppendAsync(historyProvider,historyModel,prompt,result.Answer,request.Token);if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;request.Token.ThrowIfCancellationRequested();_history.Add(new("user",prompt));_history.Add(new("assistant",result.Answer));ConversationContextPolicy.TrimInPlace(_history);RecordOverlayOperation(before,"AI 识图");PromptStatus.Text=hasVideo?"视频理解完成 · 可继续提问":renderedAnnotationCount>0?$"已在 {_lastSentSelections.Count(item=>item.VideoPath is null)} 个引用区域中标出重点 · 可继续提问":"完成 · 可继续提问";if(usingHermes&&_host.Settings.HermesAutoReadAloud)_=BeginOverlayReadAloudAsync(result.Answer);
        }
        catch(OperationCanceledException){if(!_closed&&ReferenceEquals(_request,request)){ApplyOverlaySnapshot(before);PromptStatus.Text="已取消";}}
        catch(Exception ex){new PrivacyLogger().Error(requestStage=="render"?"ScreenAiRender":"ScreenAiRequest",ex);if(!_closed&&ReferenceEquals(_request,request)){ApplyOverlaySnapshot(before);PromptStatus.Text=request.IsCancellationRequested?"已取消":$"请求失败：{ex.Message}";}}
        finally{streamOpen=false;if(attachments is not null)AiImageEncodingService.ClearAttachmentBuffers(attachments);foreach(var lease in attachmentLeases)lease.Dispose();var ownsRequest=ReferenceEquals(_request,request);if(CaptureOverlayPolicy.ShouldFinalizeCanceledAiRequest(_request,request,_closed)){CloseReasoning("思考过程 · 已取消",Color.FromRgb(142,153,169));PromptStatus.Text="已取消";}request.Dispose();if(ownsRequest){_request=null;if(!_closed){SendButton.IsEnabled=true;_ = Dispatcher.BeginInvoke(PositionPromptBar);}}}
    }

    private int RenderAnnotations(IReadOnlyList<AiAnnotation> notes)
    {
        foreach(var item in _lastSentSelections.Distinct()){item.AnnotationNotes.Clear();item.AiAnnotations.Children.Clear();}var rendered=0;
        foreach(var target in CaptureOverlayPolicy.SelectSpatialAnnotationTargets(_lastSentSelections,item=>item.VideoPath is not null))
        {
            var mapped=notes.Where(note=>note.RegionIndex==target.RegionIndex).Take(6).ToArray();
            target.Item.AnnotationNotes.AddRange(mapped);rendered+=mapped.Length;RenderAnnotationsForItem(target.Item);
        }
        return rendered;
    }

    private static void RenderAnnotationsForItem(SelectionItem item)
    {
        item.AiAnnotations.Children.Clear();var w=item.Bounds.Width;var h=item.Bounds.Height;var cardWidth=Math.Clamp(w*.3,145,360);var font=Math.Clamp(w/70,11,22);var slots=new List<double>();
        foreach(var n in item.AnnotationNotes.Take(6))
        {
            var x=Math.Clamp(n.X,0,1)*w;var y=Math.Clamp(n.Y,0,1)*h;var rw=Math.Max(14,Math.Clamp(n.Width,0,1)*w);var rh=Math.Max(14,Math.Clamp(n.Height,0,1)*h);var box=new Border{Width=rw,Height=rh,CornerRadius=new CornerRadius(5),BorderBrush=Cyan,BorderThickness=new Thickness(Math.Max(1.5,w/900)),Background=new SolidColorBrush(Color.FromArgb(14,55,170,255)),Effect=new DropShadowEffect{Color=Color.FromRgb(34,169,255),BlurRadius=13,ShadowDepth=0,Opacity=.9}};Canvas.SetLeft(box,x);Canvas.SetTop(box,y);item.AiAnnotations.Children.Add(box);
            var right=x+rw+cardWidth+28<w;var cardX=right?x+rw+24:Math.Max(5,x-cardWidth-24);var cardY=AnnotationLayoutService.FindCardTop(y+rh*.5-font*1.5,5,Math.Max(5,h-font*4),font*3.2,slots);slots.Add(cardY);var startX=right?x+rw:x;var endX=right?cardX:cardX+cardWidth;item.AiAnnotations.Children.Add(new Line{X1=startX,Y1=y+rh*.5,X2=endX,Y2=cardY+font*1.4,Stroke=Cyan,StrokeThickness=Math.Max(1,w/1200)});var dot=new Ellipse{Width=5,Height=5,Fill=Cyan};Canvas.SetLeft(dot,endX-2.5);Canvas.SetTop(dot,cardY+font*1.4-2.5);item.AiAnnotations.Children.Add(dot);var card=new Border{Width=cardWidth,Padding=new Thickness(font*.65,font*.5,font*.65,font*.5),CornerRadius=new CornerRadius(8),Background=new SolidColorBrush(Color.FromArgb(248,255,255,255)),BorderBrush=new SolidColorBrush(Color.FromArgb(145,61,174,242)),BorderThickness=new Thickness(1),Child=new TextBlock{Text=n.Text,Foreground=new SolidColorBrush(Color.FromRgb(35,48,70)),FontSize=font,TextWrapping=TextWrapping.Wrap,LineHeight=font*1.3},Effect=new DropShadowEffect{Color=Color.FromRgb(51,71,98),BlurRadius=16,ShadowDepth=4,Opacity=.28}};Canvas.SetLeft(card,cardX);Canvas.SetTop(card,cardY);item.AiAnnotations.Children.Add(card);
        }
    }

    private async void Copy(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy()||Active is not { } item)return;
        if(item.VideoPath is not { } video)
        {
            var copied=ClipboardService.TrySetImage(RenderSelectionImage(item),out var copyError);
            PromptStatus.Text=copied?"图片已复制":copyError;SetPromptBarHidden(false);return;
        }

        var operation=BeginOverlayOperation("正在准备视频副本…按 Esc 可取消");
        try
        {
            var result=await ClipboardService.TrySetFileDropListAsync(video,operation.Token);
            if(IsOverlayOperationActive(operation,item))PromptStatus.Text=result.Success?"视频文件已复制":result.Error;
        }
        catch(OperationCanceledException){if(IsOverlayOperationActive(operation,item))PromptStatus.Text="已取消复制视频";}
        finally{EndOverlayOperation(operation);SetPromptBarHidden(false);}
    }
    private async void Save(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy()||Active is not { } item)return;
        if(item.VideoPath is { } video)
        {
            var dialog=new SaveFileDialog{Filter="MP4 视频|*.mp4|GIF 动图|*.gif",DefaultExt=".mp4",FilterIndex=1,AddExtension=true};
            if(dialog.ShowDialog(this)!=true)return;
            var exportGif=dialog.FilterIndex==2;
            var destination=System.IO.Path.ChangeExtension(dialog.FileName,exportGif?".gif":".mp4");
            var operation=BeginOverlayOperation(exportGif?"正在导出 GIF…按 Esc 可取消":"正在保存 MP4…按 Esc 可取消");TempMediaLease? exportLease=null;
            try
            {
                exportLease=TempMediaRegistry.Shared.AcquireExistingFile(video);
                if(exportGif)
                {
                    var fps=_host.Settings.GifFps;
                    var result=await GifExportService.ExportFromVideoAsync(video,destination,fps,operation.Token);
                    if(IsOverlayOperationActive(operation,item))PromptStatus.Text=$"GIF 已保存 · {result.FrameCount} 帧 / {result.EffectiveFps:0.#} FPS";
                }
                else
                {
                    await Task.Run(()=>AtomicFileService.Copy(video,destination),operation.Token);
                    if(IsOverlayOperationActive(operation,item))PromptStatus.Text="MP4 已保存";
                }
            }
            catch(OperationCanceledException){if(IsOverlayOperationActive(operation,item))PromptStatus.Text="已取消保存视频";}
            catch(Exception ex){new PrivacyLogger().Error("SaveVideo",ex);if(IsOverlayOperationActive(operation,item))PromptStatus.Text=$"保存失败：{ex.Message}";}
            finally{exportLease?.Dispose();EndOverlayOperation(operation);}
            return;
        }

        var jpeg=_host.Settings.DefaultImageFormat.Equals("jpg",StringComparison.OrdinalIgnoreCase)||_host.Settings.DefaultImageFormat.Equals("jpeg",StringComparison.OrdinalIgnoreCase);var imageDialog=new SaveFileDialog{Filter="PNG 图片|*.png|JPEG 图片|*.jpg;*.jpeg",DefaultExt=jpeg?".jpg":".png",FilterIndex=jpeg?2:1,AddExtension=true};if(imageDialog.ShowDialog(this)!=true)return;
        var image=RenderSelectionImage(item);var imageOperation=BeginOverlayOperation("正在保存图片…按 Esc 可取消");
        try{await Task.Run(()=>ScreenCaptureService.Save(image,imageDialog.FileName,imageDialog.FilterIndex==2),imageOperation.Token);if(IsOverlayOperationActive(imageOperation,item))PromptStatus.Text="图片已保存";}
        catch(OperationCanceledException){if(IsOverlayOperationActive(imageOperation,item))PromptStatus.Text="已取消保存图片";}
        catch(Exception ex){new PrivacyLogger().Error("SaveImage",ex);if(IsOverlayOperationActive(imageOperation,item))PromptStatus.Text=$"图片保存失败：{ex.Message}";}
        finally{EndOverlayOperation(imageOperation);}
    }
    private void Pin(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy()||Active is not { } item)return;var pixels=ToPixelRect(item.Bounds);var region=ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY);
        try
        {
            if(item.VideoPath is { } video){var window=new PinnedVideoWindow(video,region);try{window.Show();}catch{window.Close();throw;}}
            else new PinnedImageWindow(RenderSelectionImage(item),region).Show();
            PromptStatus.Text=item.VideoPath is null?"已在原位贴图":"已在原位贴视频";SetPromptBarHidden(false);
        }
        catch(Exception ex){new PrivacyLogger().Error("PinMedia",ex);PromptStatus.Text=$"贴图失败：{ex.Message}";}
    }
    private void Draw(object s,RoutedEventArgs e)=>EnterDrawingMode();
    private void EnterDrawingMode()
    {
        if(RejectIfOverlayOperationBusy()||Active is not {IsImplicit:false,VideoPath:null} item)return;_drawingOperationBefore=CaptureOverlaySnapshot();_drawingOperationChanged=false;_drawingMode=true;Toolbar.Visibility=Visibility.Collapsed;HideHandles();SizeText.Visibility=Visibility.Collapsed;item.Markup.IsHitTestVisible=true;SetDrawTool(DrawTool.Freehand);DrawingToolbar.Visibility=Visibility.Visible;PositionFloatingBar(DrawingToolbar,item);SetPromptBarHidden(true);PromptStatus.Text="原位标注中 · Esc 或点击完成按钮结束";
    }
    private void ExitDrawingMode()
    {
        if(Active is { } item)item.Markup.IsHitTestVisible=false;_drawingMode=false;DrawingToolbar.Visibility=Visibility.Collapsed;Cursor=Cursors.Cross;SetPromptBarHidden(false);if(Active is not null){UpdateSelection(Active);ShowToolbar();}if(_drawingOperationChanged&&_drawingOperationBefore is { } before)RecordOverlayOperation(before,"原位标注");_drawingOperationBefore=null;_drawingOperationChanged=false;PromptStatus.Text="标注已保留在当前区域";
    }
    private void FinishInterruptedDrawingMode()
    {
        var canvas=Active?.Markup;_drawPreview=null;if(canvas?.IsMouseCaptured==true)canvas.ReleaseMouseCapture();ExitDrawingMode();
    }
    private void SetDrawTool(DrawTool tool)
    {
        if(Active is not { } item)return;_drawTool=tool;item.Markup.EditingMode=tool==DrawTool.Freehand?InkCanvasEditingMode.Ink:InkCanvasEditingMode.None;Cursor=tool==DrawTool.Freehand?Cursors.Pen:Cursors.Cross;
    }
    private static DrawingAttributes RegularDrawingAttributes(Color color)=>new(){Color=color,Width=4,Height=4,IsHighlighter=false,FitToCurve=true};
    private static DrawingAttributes HighlightDrawingAttributes()=>new(){Color=Colors.Yellow,Width=18,Height=18,IsHighlighter=true,FitToCurve=true};
    private void DrawPen(object s,RoutedEventArgs e){if(Active is { } item){item.Markup.DefaultDrawingAttributes=RegularDrawingAttributes(Colors.Red);SetDrawTool(DrawTool.Freehand);}}
    private void SetShapeTool(DrawTool tool){if(Active is not { } item)return;var current=item.Markup.DefaultDrawingAttributes;item.Markup.DefaultDrawingAttributes=RegularDrawingAttributes(current.IsHighlighter?Colors.Red:current.Color);SetDrawTool(tool);}
    private void DrawRectangleTool(object s,RoutedEventArgs e)=>SetShapeTool(DrawTool.Rectangle);
    private void DrawArrowTool(object s,RoutedEventArgs e)=>SetShapeTool(DrawTool.Arrow);
    private void DrawRed(object s,RoutedEventArgs e){if(Active is { } item){item.Markup.DefaultDrawingAttributes=RegularDrawingAttributes(Colors.Red);SetDrawTool(DrawTool.Freehand);}}
    private void DrawBlue(object s,RoutedEventArgs e){if(Active is { } item){item.Markup.DefaultDrawingAttributes=RegularDrawingAttributes(Color.FromRgb(49,140,255));SetDrawTool(DrawTool.Freehand);}}
    private void DrawHighlight(object s,RoutedEventArgs e){if(Active is { } item){item.Markup.DefaultDrawingAttributes=HighlightDrawingAttributes();SetDrawTool(DrawTool.Freehand);}}
    private void DrawEraser(object s,RoutedEventArgs e){if(Active is { } item){_drawTool=DrawTool.Freehand;item.Markup.EditingMode=InkCanvasEditingMode.EraseByStroke;Cursor=Cursors.Cross;}}
    private void DrawUndo(object s,RoutedEventArgs e){if(Active is not { } item||item.Markup.Strokes.Count==0)return;var stroke=item.Markup.Strokes[^1];item.Markup.Strokes.Remove(stroke);item.Redo.Push(stroke);_drawingOperationChanged=true;}
    private void DrawRedo(object s,RoutedEventArgs e){if(Active is { } item&&item.Redo.TryPop(out var stroke)){item.Markup.Strokes.Add(stroke);_drawingOperationChanged=true;}}
    private void DrawClear(object s,RoutedEventArgs e){if(Active is { } item&&item.Markup.Strokes.Count>0){item.Markup.Strokes.Clear();item.Redo.Clear();_drawingOperationChanged=true;}}
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
        if(sender is not InkCanvas canvas||_drawTool==DrawTool.Freehand||!canvas.IsMouseCaptured)return;_drawPreview=null;canvas.ReleaseMouseCapture();if(Active is { } item)item.Redo.Clear();_drawingOperationChanged=true;e.Handled=true;
    }
    private void MarkupLostMouseCapture(object sender,MouseEventArgs e)
    {
        if(!_drawingMode||sender is not InkCanvas canvas||_drawTool==DrawTool.Freehand||_drawPreview is null)return;_drawPreview=null;if(ReferenceEquals(canvas,Active?.Markup)&&Active is { } item)item.Redo.Clear();PromptStatus.Text="标注笔划已保留，可继续编辑";
    }
    private static Stroke CreateShapeStroke(InkCanvas canvas,Point a,Point b,DrawTool tool)
    {
        var points=new StylusPointCollection();if(tool==DrawTool.Rectangle){points.Add(new StylusPoint(a.X,a.Y));points.Add(new StylusPoint(b.X,a.Y));points.Add(new StylusPoint(b.X,b.Y));points.Add(new StylusPoint(a.X,b.Y));points.Add(new StylusPoint(a.X,a.Y));}else{points.Add(new StylusPoint(a.X,a.Y));points.Add(new StylusPoint(b.X,b.Y));var angle=Math.Atan2(b.Y-a.Y,b.X-a.X);var length=Math.Min(24,Math.Max(10,new Vector(b.X-a.X,b.Y-a.Y).Length*.25));points.Add(new StylusPoint(b.X-length*Math.Cos(angle-Math.PI/6),b.Y-length*Math.Sin(angle-Math.PI/6)));points.Add(new StylusPoint(b.X,b.Y));points.Add(new StylusPoint(b.X-length*Math.Cos(angle+Math.PI/6),b.Y-length*Math.Sin(angle+Math.PI/6)));}var attributes=canvas.DefaultDrawingAttributes.Clone();attributes.FitToCurve=false;return new Stroke(points,attributes);
    }
    private async void QuickSend(object s,RoutedEventArgs e)=>await SendAsync(true);
    private async void QuickPromptKeyDown(object s,KeyEventArgs e){if(e.Key==Key.Enter&&!Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)){e.Handled=true;await SendAsync(true);}}
    private async void QuickVoice(object s,RoutedEventArgs e)=>await ToggleVoiceAsync();
    private async Task ToggleVoiceAsync()
    {
        if(_closed||RejectIfOverlayOperationBusy())return;
        if(!_host.Settings.EnableVoiceInput){ApplyVoiceAvailability();PromptStatus.Text="语音输入已在设置中关闭";return;}
        if(_speechRequest is not null){PromptStatus.Text="正在停止聆听…";_speechRequest.Cancel();return;}
        var microphone=VoiceIcon.Data;var speechRequest=new CancellationTokenSource();_speechRequest=speechRequest;VoiceIcon.Data=Geometry.Parse("M7,7 L17,7 L17,17 L7,17 Z");
        try
        {
            PromptStatus.Text="正在聆听…";var text=await new WindowsSpeechToTextService().RecognizeOnceAsync(_host.Settings.VoiceLanguage,speechRequest.Token);
            if(_closed||!ReferenceEquals(_speechRequest,speechRequest))return;
            if(!string.IsNullOrWhiteSpace(text))QuickPrompt.Text=string.IsNullOrWhiteSpace(QuickPrompt.Text)?text:QuickPrompt.Text+" "+text;PromptStatus.Text="语音已写入";
        }
        catch(OperationCanceledException){if(!_closed&&ReferenceEquals(_speechRequest,speechRequest))PromptStatus.Text="已停止聆听";}
        catch(SpeechRecognitionUnavailableException ex){if(!_closed&&ReferenceEquals(_speechRequest,speechRequest))PromptStatus.Text=ex.Message;}
        catch(Exception){if(!_closed&&ReferenceEquals(_speechRequest,speechRequest))PromptStatus.Text="语音输入暂时不可用";}
        finally{speechRequest.Dispose();if(ReferenceEquals(_speechRequest,speechRequest))_speechRequest=null;if(!_closed)VoiceIcon.Data=microphone;}
    }
    private void ApplyVoiceAvailability()=>VoiceButton.Visibility=_host.Settings.EnableVoiceInput?Visibility.Visible:Visibility.Collapsed;
    private async void Translate(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy())return;
        if(Active is not { } item||!CaptureOverlayPolicy.CanRunImageOnlyCommand(item.IsImplicit,item.VideoPath)){if(Active?.VideoPath is not null)PromptStatus.Text="视频区域不支持 OCR/翻译，请先选择截图区域";else PromptStatus.Text="请先框选截图区域";return;}var before=CaptureOverlaySnapshot();var image=CurrentImage();var operation=BeginOverlayOperation("正在识别文字…按 Esc 可取消");
        try
        {
            var document=await new WindowsOcrService().RecognizeAsync(image,operation.Token);
            if(!IsOverlayOperationActive(operation,item))return;
            if(document.Lines.Count==0){PromptStatus.Text=$"{document.Engine} 未识别到文字";return;}
            var provider=new AiProviderFactory().Create(_host.Settings,out var providerError);if(provider is null){PromptStatus.Text=providerError??"翻译需要先配置可用的 AI Provider";ShowSettingsFromOverlay();return;}
            var batches=CaptureOverlayPolicy.CreateTranslationBatches(document.Lines.Select(line=>line.Text).ToArray());
            var translations=new string[document.Lines.Count];
            for(var batchIndex=0;batchIndex<batches.Count;batchIndex++)
            {
                if(!IsOverlayOperationActive(operation,item))return;
                var batch=batches[batchIndex];var batchNumber=batchIndex+1;
                PromptStatus.Text=$"{document.Engine} 已识别 {document.Lines.Count} 行 · 正在翻译 {batchNumber}/{batches.Count}…按 Esc 可取消";
                var prompt="将 translationsSource 中的每一项翻译成简体中文。保持数组长度和顺序完全一致，只返回 JSON：{\"translations\":[\"译文1\",\"译文2\"]}。translationsSource="+System.Text.Json.JsonSerializer.Serialize(batch.Lines);
                using var networkTimeout=CancellationTokenSource.CreateLinkedTokenSource(operation.Token);networkTimeout.CancelAfter(TimeSpan.FromSeconds(75));var received=false;var batchOpen=true;
                var progress=new Progress<AiStreamDelta>(delta=>{if(!batchOpen||!IsOverlayOperationActive(operation,item))return;if(!received&&(delta.Content.Length>0||delta.ReasoningContent.Length>0)){received=true;PromptStatus.Text=$"{document.Engine} · 正在接收译文 {batchNumber}/{batches.Count}…按 Esc 可取消";}});
                AiResult result;
                try
                {
                    result=await provider.SendAsync(new AiRequest{Prompt=prompt,StreamingProgress=progress,StreamingCompletionPredicate=value=>TranslationResponseParser.TryParse(value,batch.Lines.Count,out _),DisableReasoning=true,MaxOutputTokens=batch.MaxOutputTokens},networkTimeout.Token);
                }
                catch(OperationCanceledException) when(!operation.IsCancellationRequested)
                {
                    throw new TimeoutException($"翻译第 {batchNumber}/{batches.Count} 批超时，请检查 Provider 后重试");
                }
                finally{batchOpen=false;}
                if(!IsOverlayOperationActive(operation,item))return;
                if(!TranslationResponseParser.TryParse(result.Answer,batch.Lines.Count,out var translated)){PromptStatus.Text=$"翻译第 {batchNumber}/{batches.Count} 批结果格式异常，请重试";return;}
                for(var offset=0;offset<translated.Count;offset++)translations[batch.StartIndex+offset]=translated[offset];
            }
            if(!IsOverlayOperationActive(operation,item))return;
            RenderTextOverlays(item,image,document.Lines,translations,true);RecordOverlayOperation(before,"原位翻译");PromptStatus.Text=$"{document.Engine} · 已在原位翻译 {translations.Length} 行";
        }
        catch(OperationCanceledException){if(!_closed&&ReferenceEquals(_overlayRequest,operation))PromptStatus.Text="已取消翻译";}catch(TimeoutException ex){new PrivacyLogger().Error("OverlayTranslate",ex);if(!_closed&&ReferenceEquals(_overlayRequest,operation))PromptStatus.Text=ex.Message;}catch(Exception ex){new PrivacyLogger().Error("OverlayTranslate",ex);if(!_closed&&ReferenceEquals(_overlayRequest,operation))PromptStatus.Text=$"翻译失败：{ex.Message}";}finally{EndOverlayOperation(operation);}
    }

    private async void Ocr(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy())return;
        if(Active is not { } item||!CaptureOverlayPolicy.CanRunImageOnlyCommand(item.IsImplicit,item.VideoPath)){if(Active?.VideoPath is not null)PromptStatus.Text="视频区域不支持 OCR，请先选择截图区域";else PromptStatus.Text="请先框选截图区域";return;}var before=CaptureOverlaySnapshot();var image=CurrentImage();var operation=BeginOverlayOperation("正在本地识别当前区域…");
        try
        {
            var document=await new WindowsOcrService().RecognizeAsync(image,operation.Token);if(!IsOverlayOperationActive(operation,item))return;if(document.Lines.Count==0){item.TextLayer=NoTextLayerState.Instance;item.TextOverlays.Children.Clear();ClearTextSelection(item);}else RenderSelectableText(item,image,document);RecordOverlayOperation(before,"原位文字识别");PromptStatus.Text=document.Lines.Count==0?$"{document.Engine} 未识别到文字":$"{document.Engine} 已识别 {document.Lines.Count} 行 · 可直接拖选并按 Ctrl+C";
        }
        catch(OperationCanceledException){if(!_closed&&ReferenceEquals(_overlayRequest,operation))PromptStatus.Text="已取消文字识别";}catch(Exception ex){new PrivacyLogger().Error("OverlayOcr",ex);if(!_closed&&ReferenceEquals(_overlayRequest,operation))PromptStatus.Text=$"OCR 失败：{ex.Message}";}finally{EndOverlayOperation(operation);}
    }

    private bool RejectIfOverlayOperationBusy()
    {
        if(_overlayRequest is not null){PromptStatus.Text=_overlayRequest.IsCancellationRequested?"正在取消当前操作…":"当前操作尚未完成 · 按 Esc 可取消";return true;}
        if(_request is not null){PromptStatus.Text=_request.IsCancellationRequested?"正在取消 AI 分析…":"AI 正在分析 · 按 Esc 可取消后再修改区域";return true;}
        return false;
    }
    private CancellationTokenSource BeginOverlayOperation(string status){var operation=new CancellationTokenSource();_overlayRequest=operation;PromptStatus.Text=status;Toolbar.IsEnabled=false;return operation;}
    private void EndOverlayOperation(CancellationTokenSource operation){operation.Dispose();if(!ReferenceEquals(_overlayRequest,operation))return;_overlayRequest=null;if(_closed)return;Toolbar.IsEnabled=true;if(Active is not null)ShowToolbar();}
    private bool IsOverlayOperationActive(CancellationTokenSource operation,SelectionItem item)=>!_closed&&ReferenceEquals(_overlayRequest,operation)&&!operation.IsCancellationRequested&&_selections.Contains(item);
    private void ShowSettingsFromOverlay()
    {
        Topmost=false;
        try
        {
            _host.ShowSettings();var settings=Application.Current.Windows.OfType<SettingsWindow>().FirstOrDefault(window=>window.IsVisible);
            if(settings is null){Topmost=true;Activate();return;}
            settings.Topmost=true;
            EventHandler? restore=null;restore=(_,_)=>{settings.Closed-=restore;if(IsVisible){ApplyVoiceAvailability();PositionPromptBar();Topmost=true;Activate();QuickPrompt.Focus();}};settings.Closed+=restore;settings.Activate();
        }
        catch(Exception ex){Topmost=true;Activate();new PrivacyLogger().Error("OverlaySettings",ex);PromptStatus.Text="无法打开设置，请从托盘重试";}
    }
    private static void RenderTextOverlays(SelectionItem item,BitmapSource image,IReadOnlyList<OcrLine> lines,IReadOnlyList<string> texts,bool translated)
    {
        item.TextLayer=new TranslationTextLayerState(image,lines.ToArray(),texts.ToArray());
        RenderTextOverlaysCore(item,image,lines,texts,translated);
    }
    private static void RenderTextOverlaysCore(SelectionItem item,BitmapSource image,IReadOnlyList<OcrLine> lines,IReadOnlyList<string> texts,bool translated)
    {
        ClearTextSelection(item);item.TextOverlays.Children.Clear();var scaleX=item.Bounds.Width/image.PixelWidth;var scaleY=item.Bounds.Height/image.PixelHeight;
        for(var index=0;index<lines.Count&&index<texts.Count;index++)
        {
            var line=lines[index];var text=new TextBlock{Text=texts[index],Foreground=new SolidColorBrush(Color.FromRgb(35,47,67)),FontSize=Math.Clamp(line.Height*scaleY*.72,10,26),FontWeight=translated?FontWeights.SemiBold:FontWeights.Normal,TextWrapping=TextWrapping.Wrap,LineHeight=Math.Clamp(line.Height*scaleY*.9,14,32)};
            var box=new Border{Child=text,Background=new SolidColorBrush(translated?Color.FromArgb(242,239,242,255):Color.FromArgb(228,248,251,255)),BorderBrush=new SolidColorBrush(translated?Color.FromRgb(111,124,245):Color.FromRgb(78,164,224)),BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(5),Padding=new Thickness(3,1,3,1),Width=Math.Max(36,line.Width*scaleX),MinHeight=Math.Max(18,line.Height*scaleY),ToolTip=translated?"原位译文":"本地 OCR 文字"};
            Canvas.SetLeft(box,line.X*scaleX);Canvas.SetTop(box,line.Y*scaleY);item.TextOverlays.Children.Add(box);
        }
    }
    private void RenderSelectableText(SelectionItem item,BitmapSource image,OcrDocument document)
    {
        item.TextLayer=new OcrTextLayerState(image,document);
        RenderSelectableTextCore(item,image,document);
    }
    private void RenderSelectableTextCore(SelectionItem item,BitmapSource image,OcrDocument document)
    {
        item.TextOverlays.Children.Clear();ClearTextSelection(item);if(document.Lines.Count==0)return;var scaleX=item.Bounds.Width/image.PixelWidth;var scaleY=item.Bounds.Height/image.PixelHeight;var layout=OcrSelectionLayout.Build(document.Lines,scaleX,scaleY);if(layout.Count==0)return;
        var flow=new FlowDocument{PagePadding=new Thickness(0),ColumnGap=0,FontFamily=new FontFamily("Segoe UI"),FontSize=1,LineHeight=1,Foreground=Brushes.Transparent};var pending=new List<(OcrSelectionGlyph Glyph,Run Run)>();
        foreach(var line in layout)
        {
            var paragraph=new Paragraph{Margin=new Thickness(0),Padding=new Thickness(0),FontSize=1,LineHeight=1,Foreground=Brushes.Transparent};
            foreach(var token in line.Tokens){if(token.Prefix.Length>0)paragraph.Inlines.Add(new Run(token.Prefix));var run=new Run(token.Text);paragraph.Inlines.Add(run);foreach(var glyph in token.Glyphs)pending.Add((glyph,run));}
            flow.Blocks.Add(paragraph);
        }
        var box=new RichTextBox{Document=flow,IsReadOnly=true,IsReadOnlyCaretVisible=false,Background=Brushes.Transparent,BorderThickness=new Thickness(0),Padding=new Thickness(0),SelectionBrush=Brushes.Transparent,SelectionTextBrush=Brushes.Transparent,Cursor=Cursors.IBeam,Width=item.Bounds.Width,Height=item.Bounds.Height,VerticalScrollBarVisibility=ScrollBarVisibility.Disabled,HorizontalScrollBarVisibility=ScrollBarVisibility.Disabled,ToolTip="可跨行拖动选择文字，Ctrl+C 复制"};
        var highlights=new Canvas{Width=item.Bounds.Width,Height=item.Bounds.Height,IsHitTestVisible=false};var selectable=new List<SelectableGlyph>(pending.Count);
        foreach(var entry in pending){var start=entry.Run.ContentStart.GetPositionAtOffset(entry.Glyph.Utf16Start,LogicalDirection.Forward);var end=entry.Run.ContentStart.GetPositionAtOffset(entry.Glyph.Utf16Start+entry.Glyph.Utf16Length,LogicalDirection.Forward);if(start is not null&&end is not null)selectable.Add(new SelectableGlyph(entry.Glyph.Bounds,start,end));}
        if(selectable.Count==0)return;box.SelectionChanged+=(_,_)=>{var count=new TextRange(box.Selection.Start,box.Selection.End).Text.Length;if(count>0)PromptStatus.Text=$"已选择 {count} 个字符 · Ctrl+C 复制或右键";};box.ContextMenu=CreateTextContextMenu(box,document.Text);item.TextSelection.Children.Add(highlights);item.TextSelection.Children.Add(box);item.TextSession=new OcrTextSelectionSession(box,highlights,selectable);item.TextSelection.IsHitTestVisible=true;
    }
    private void ApplyTextLayerState(SelectionItem item)
    {
        switch(item.TextLayer)
        {
            case OcrTextLayerState ocr:RenderSelectableTextCore(item,ocr.Image,ocr.Document);break;
            case TranslationTextLayerState translation:RenderTextOverlaysCore(item,translation.Image,translation.Lines,translation.Texts,true);break;
            default:item.TextOverlays.Children.Clear();ClearTextSelection(item);break;
        }
    }
    private ContextMenu CreateTextContextMenu(RichTextBox box,string allText)
    {
        var menu=new ContextMenu();menu.SetResourceReference(StyleProperty,"TextSelectionContextMenu");var copy=new MenuItem{Header="复制所选文字"};var copyAll=new MenuItem{Header="复制全部识别文字"};foreach(var entry in new[]{copy,copyAll})entry.SetResourceReference(StyleProperty,"TextSelectionMenuItem");copy.Click+=(_,_)=>CopyTextToClipboard(new TextRange(box.Selection.Start,box.Selection.End).Text.TrimEnd('\r','\n'));copyAll.Click+=(_,_)=>CopyTextToClipboard(allText);var separator=new Separator();separator.SetResourceReference(StyleProperty,"TextSelectionSeparator");menu.Items.Add(copy);menu.Items.Add(separator);menu.Items.Add(copyAll);menu.Opened+=(_,_)=>copy.IsEnabled=!box.Selection.IsEmpty;return menu;
    }
    private void CopyTextToClipboard(string text){if(text.Length==0)return;PromptStatus.Text=ClipboardService.TrySetText(text,out var error)?"文字已复制":error;}
    private static void ClearTextSelection(SelectionItem item){item.TextSession?.Dispose();item.TextSession=null;item.TextSelection.Children.Clear();item.TextSelection.IsHitTestVisible=false;}
    private void Record(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy())return;
        if(!_captureExclusionVerified){SetPromptBarHidden(false);PromptStatus.Text=NativeMethods.VisualQaCaptureEnabled?"视觉验收模式未启用防捕获，已阻止录屏以免录入覆盖控件":"系统未能启用窗口防捕获，为避免录入遮罩和控件，已阻止录屏；请重启软件后重试";return;}
        if(_recordingSession is not null||Active is not {IsImplicit:false,VideoPath:null} item)return;
        try
        {
            var pixels=ToPixelRect(item.Bounds);var region=ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY);var session=new RecordingSession(_host.Settings,region);_recordingSession=session;_recordingItem=item;_recordingItemWasReferenced=_references.Contains(item);session.Completed+=path=>{if(!Dispatcher.HasShutdownStarted)Dispatcher.BeginInvoke(new Action(()=>CompleteRecording(session,item,path)));};session.Failed+=error=>{if(!Dispatcher.HasShutdownStarted)Dispatcher.BeginInvoke(new Action(()=>FailRecording(session,item,error)));};EnterRecordingMode(item);session.Start();_recordingTimer.Start();PromptStatus.Text="正在录制当前区域";
        }
        catch(Exception ex)
        {
            if(_recordingSession is { } failedSession&&ReferenceEquals(_recordingItem,item))FailRecording(failedSession,item,ex.Message);else{new PrivacyLogger().Error("RecordingStart",ex);SetPromptBarHidden(false);PromptStatus.Text=$"录屏失败：{ex.Message}";}
        }
    }

    private void EnterRecordingMode(SelectionItem selected)
    {
        _recordingMode=true;_recordingPaused=_recordingStopping=false;Cursor=Cursors.Arrow;
        if(Root.IsMouseCaptured)Root.ReleaseMouseCapture();
        Toolbar.Visibility=DrawingToolbar.Visibility=PromptBarHost.Visibility=SizeText.Visibility=Visibility.Collapsed;HideHandles();
        foreach(var item in _selections){item.Host.Visibility=ReferenceEquals(item,selected)?Visibility.Visible:Visibility.Collapsed;item.Badge.Visibility=Visibility.Collapsed;item.Markup.Visibility=item.TextOverlays.Visibility=item.AiAnnotations.Visibility=item.TextSelection.Visibility=Visibility.Collapsed;item.Markup.IsHitTestVisible=false;item.Image.Visibility=ReferenceEquals(item,selected)?Visibility.Collapsed:Visibility.Visible;}
        selected.Video.Visibility=Visibility.Collapsed;
        selected.Outline.BorderBrush=new SolidColorBrush(Color.FromRgb(50,151,242));selected.Outline.BorderThickness=new Thickness(2);selected.Outline.Effect=new DropShadowEffect{Color=Color.FromRgb(48,151,242),BlurRadius=18,ShadowDepth=0,Opacity=.9};
        RecordingTime.Text="00:00";SetRecordingPauseVisual(false);RecordingPauseButton.ToolTip="暂停";RecordingBar.Visibility=Visibility.Visible;PositionFloatingBar(RecordingBar,selected);
        if(!UpdateRecordingVisualHole(requireNativeRegion:true))throw new InvalidOperationException("无法建立录屏区域的鼠标穿透，请调整选区后重试");
    }
    private void RecordingTick()
    {
        if(!_recordingMode||_recordingSession is not { } session)return;RecordingTime.Text=session.Elapsed.ToString(@"mm\:ss");
    }
    private void PauseRecording(object s,RoutedEventArgs e)
    {
        if(_recordingSession is not { } session||_recordingItem is not { } item||_recordingStopping)return;try{if(!_recordingPaused){session.Pause();_recordingPaused=true;SetRecordingPauseVisual(true);RecordingPauseButton.ToolTip="继续";}else{session.Resume();_recordingPaused=false;SetRecordingPauseVisual(false);RecordingPauseButton.ToolTip="暂停";}}catch(Exception ex){FailRecording(session,item,ex.Message);}
    }
    private void SetRecordingPauseVisual(bool paused)
    {
        RecordingPauseBars.Visibility=paused?Visibility.Collapsed:Visibility.Visible;
        RecordingResumeIcon.Visibility=paused?Visibility.Visible:Visibility.Collapsed;
    }
    private void StopRecording(object s,RoutedEventArgs e)
    {
        if(_recordingSession is not { } session||_recordingItem is not { } item||_recordingStopping)return;_recordingStopping=true;_recordingTimer.Stop();RecordingTime.Text="处理中…";try{session.Stop();StartRecordingStopWatchdog(session,item);}catch(Exception ex){FailRecording(session,item,ex.Message);}
    }
    private void StartRecordingStopWatchdog(RecordingSession session,SelectionItem item)
    {
        CancelRecordingStopWatchdog();var watchdog=new CancellationTokenSource();_recordingStopWatchdog=watchdog;_ = WatchRecordingStopAsync(session,item,watchdog);
    }
    private async Task WatchRecordingStopAsync(RecordingSession session,SelectionItem item,CancellationTokenSource watchdog)
    {
        try
        {
            await Task.Delay(CaptureOverlayPolicy.RecordingStopTimeout,watchdog.Token);
            if(!_closed&&ReferenceEquals(_recordingStopWatchdog,watchdog)&&IsCurrentRecording(session,item))FailRecording(session,item,"停止录屏超时，已安全结束本次录制，请重试");
        }
        catch(OperationCanceledException)when(watchdog.IsCancellationRequested){}
    }
    private void CancelRecordingStopWatchdog()
    {
        var watchdog=Interlocked.Exchange(ref _recordingStopWatchdog,null);if(watchdog is null)return;try{watchdog.Cancel();}catch(ObjectDisposedException){}watchdog.Dispose();
    }
    private async void CompleteRecording(RecordingSession session,SelectionItem item,string path)
    {
        if(!IsCurrentRecording(session,item))return;
        try
        {
            CancelRecordingStopWatchdog();_recordingTimer.Stop();if(!File.Exists(path)||new FileInfo(path).Length==0)throw new InvalidDataException("录屏文件为空");item.VideoLease=session.RetainCompletedVideo();if(new FileInfo(item.VideoLease.Path).Length==0)throw new InvalidDataException("录屏文件为空");item.VideoPath=item.VideoLease.Path;item.VideoDuration=session.Elapsed;item.VideoPlaying=false;RecordingTime.Text="处理中…";try{ClearImageOnlyLayers(item);}catch(Exception clearError){new PrivacyLogger().Error("RecordingLayerCleanup",clearError);}
            // Dispose the recorder before opening the file in WinRT.  This
            // awaits Media Foundation's final handle release; loading the
            // player first is a race that can leave the selection blank even
            // though the MP4 exists.
            try{await session.DisposeAsync();}catch(Exception ex){new PrivacyLogger().Error("RecordingDispose",ex);}
            if(!IsCurrentRecording(session,item)||_closed||!_selections.Contains(item))return;
            _references.Add(item);ExitRecordingMode(item);StartVideoPreview(item);_recordingSession=null;_recordingItem=null;_recordingItemWasReferenced=false;
            if(item.VideoPath is not null)PromptStatus.Text=$"录屏完成 {item.VideoDuration:mm\\:ss} · 已引用为 @视频{_selections.IndexOf(item)+1}";
        }
        catch(Exception ex){FailRecording(session,item,ex.Message);}
    }
    private void StartVideoPreview(SelectionItem item)
    {
        item.Image.Visibility=Visibility.Collapsed;
        item.Video.Visibility=Visibility.Visible;
        if(item.VideoPath is not { } path)return;
        try
        {
            EnsureVideoPreview(item).Load(path,autoplay:true);
            item.VideoPlaying=true;
        }
        catch(Exception ex)
        {
            item.VideoPlaying=false;
            new PrivacyLogger().Error("RecordingPreviewPlay",ex);
            PromptStatus.Text="录屏已完成，但预览启动失败；仍可保存或复制视频";
        }
    }

    private VideoPreviewSurface EnsureVideoPreview(SelectionItem item)
    {
        if(item.VideoPreview is not null)return item.VideoPreview;
        var preview=new VideoPreviewSurface(item.Video,Dispatcher);
        preview.Failed+=error=>
        {
            if(_closed||!_selections.Contains(item))return;
            item.VideoPlaying=false;
            new PrivacyLogger().Error("RecordingPreviewDecode",error);
            PromptStatus.Text="录屏已完成，但当前视频无法解码；仍可保存或复制视频";
        };
        item.VideoPreview=preview;
        return preview;
    }
    private void FailRecording(RecordingSession session,SelectionItem item,string error)
    {
        if(!IsCurrentRecording(session,item))return;CancelRecordingStopWatchdog();_recordingTimer.Stop();var wasReferenced=_recordingItemWasReferenced;_recordingSession=null;_recordingItem=null;_recordingItemWasReferenced=false;try{session.Dispose();}catch(Exception ex){new PrivacyLogger().Error("RecordingDispose",ex);}var stillPresent=_selections.Contains(item);if(stillPresent)CaptureOverlayPolicy.RestoreRecordingReference(_references,item,wasReferenced);else _references.Remove(item);ResetFailedVideoPreview(item);if(stillPresent)ExitRecordingMode(item);PromptStatus.Text=$"录屏失败：{error}";
    }
    private static void ResetFailedVideoPreview(SelectionItem item)
    {
        try{item.VideoPreview?.CloseSource();}catch(Exception ex){new PrivacyLogger().Error("RecordingPreviewReset",ex);}
        item.Video.Visibility=Visibility.Collapsed;item.Image.Visibility=Visibility.Visible;item.VideoLease?.Dispose();item.VideoLease=null;item.VideoPath=null;item.VideoDuration=TimeSpan.Zero;item.VideoPlaying=false;
    }
    private static void ClearImageOnlyLayers(SelectionItem item){item.Markup.Strokes.Clear();item.Redo.Clear();item.TextLayer=NoTextLayerState.Instance;item.AnnotationNotes.Clear();item.TextOverlays.Children.Clear();item.AiAnnotations.Children.Clear();ClearTextSelection(item);}
    private static void InvalidateImageDerivedLayers(SelectionItem item){if(item.VideoPath is not null)return;ClearImageOnlyLayers(item);}
    private bool IsCurrentRecording(RecordingSession session,SelectionItem item)=>ReferenceEquals(_recordingSession,session)&&ReferenceEquals(_recordingItem,item);
    private void ExitRecordingMode(SelectionItem selected)
    {
        _recordingMode=_recordingPaused=_recordingStopping=false;ClearRecordingVisualHole();RecordingBar.Visibility=Visibility.Collapsed;PromptBarHost.Visibility=Visibility.Visible;Cursor=Cursors.Cross;foreach(var item in _selections){item.Host.Visibility=Visibility.Visible;var isImageOnly=item.VideoPath is null;var imageOnly=isImageOnly?Visibility.Visible:Visibility.Collapsed;item.Image.Visibility=imageOnly;item.Video.Visibility=isImageOnly?Visibility.Collapsed:Visibility.Visible;item.Markup.Visibility=item.TextOverlays.Visibility=item.AiAnnotations.Visibility=item.TextSelection.Visibility=imageOnly;}var index=_selections.IndexOf(selected);if(index>=0)Select(index);RefreshSelectionNumbers();UpdateReferenceChips();ShowToolbar();PositionPromptBar();SetPromptBarHidden(false);
    }
    private void ToggleVideoPlayback(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy()||Active is not {VideoPath:not null} item)return;
        try
        {
            var preview=EnsureVideoPreview(item);
            if(item.VideoPlaying){preview.Pause();item.VideoPlaying=false;PromptStatus.Text="视频已暂停";}
            else{preview.Play();item.VideoPlaying=true;PromptStatus.Text="视频正在原位播放";}
        }
        catch(Exception ex){new PrivacyLogger().Error("RecordingPreviewToggle",ex);item.VideoPlaying=false;PromptStatus.Text="视频预览暂不可用；仍可保存或复制视频";}
    }

    private async void OnPreviewKeyDown(object s,KeyEventArgs e)
    {
        if(e.Key==Key.Escape)
        {
            if(_recordingMode)StopRecording(s,new());
            else if(_drawingMode)ExitDrawingMode();
            else if(_activeInteraction is not null){ResolveOverlayInteractionWithFallback();PromptStatus.Text="已取消本次 Hermes 交互";}
            else if(_overlayRequest is not null){if(!_overlayRequest.IsCancellationRequested)_overlayRequest.Cancel();PromptStatus.Text="正在取消当前操作…";}
            else if(_request is not null){if(!_request.IsCancellationRequested)_request.Cancel();PromptStatus.Text="正在取消 AI 分析…";}
            else
            {
                var focused=Keyboard.FocusedElement as DependencyObject;
                var textSelection=_selections.FirstOrDefault(item=>item.TextSelection.IsHitTestVisible&&focused is not null&&IsInside(focused,item.TextSelection))
                    ??(Active is { } active&&active.TextSelection.IsHitTestVisible?active:null)
                    ??_selections.FirstOrDefault(item=>item.TextSelection.IsHitTestVisible);
                if(textSelection is not null){ClearTextSelection(textSelection);PromptStatus.Text="已退出文字选择";}else Close();
            }
            e.Handled=true;return;
        }
        var modifiers=Keyboard.Modifiers;
        var undo=modifiers==ModifierKeys.Control&&e.Key==Key.Z;
        var redo=(modifiers==(ModifierKeys.Control|ModifierKeys.Shift)&&e.Key==Key.Z)||(modifiers==ModifierKeys.Control&&e.Key==Key.Y);
        if(undo||redo)
        {
            var pointer=Mouse.GetPosition(Root);
            var editablePromptFocused=Keyboard.FocusedElement is TextBoxBase focusedText&&IsInside(focusedText,PromptBar);
            var target=CaptureOverlayPolicy.ResolveUndoTarget(
                editablePromptFocused,
                PointerOverPromptBar(pointer),
                _selections.Any(item=>!item.IsImplicit&&item.Bounds.Contains(pointer)));
            if(target==OverlayUndoTarget.Text)return;
            if(_recordingMode||_overlayRequest is not null||_request is not null){PromptStatus.Text="当前操作完成后才能撤销或重做";e.Handled=true;return;}
            if(_drawingMode){if(undo)DrawUndo(s,new());else DrawRedo(s,new());}
            else if(undo)UndoOverlayOperation();else RedoOverlayOperation();
            e.Handled=true;return;
        }
        if(Keyboard.FocusedElement is RichTextBox richTextBox&&_selections.Any(item=>IsInside(richTextBox,item.TextSelection)))
        {
            if(e.Key==Key.C&&Keyboard.Modifiers.HasFlag(ModifierKeys.Control)&&!richTextBox.Selection.IsEmpty){CopyTextToClipboard(new TextRange(richTextBox.Selection.Start,richTextBox.Selection.End).Text.TrimEnd('\r','\n'));e.Handled=true;}return;
        }
        if(Keyboard.FocusedElement is TextBox or ButtonBase)return;
        if(_recordingMode||_drawingMode)return;
        if(_overlayRequest is not null||_request is not null){e.Handled=true;return;}
        if(e.Key==Key.Delete&&Active is not null){var before=CaptureOverlaySnapshot();RemoveActiveSelection(true);RecordOverlayOperation(before,"删除截图区域");e.Handled=true;return;}
        if(Active is not {IsImplicit:false} item)return;var step=Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)?10:1;
        if(e.Key is Key.Left or Key.Right or Key.Up or Key.Down){var before=CaptureOverlaySnapshot();var next=ClampSelection(new Rect(item.Bounds.X+(e.Key==Key.Left?-step:e.Key==Key.Right?step:0),item.Bounds.Y+(e.Key==Key.Up?-step:e.Key==Key.Down?step:0),item.Bounds.Width,item.Bounds.Height));if(CaptureOverlayPolicy.HasContentGeometryChanged(item.Bounds,next))InvalidateImageDerivedLayers(item);item.Bounds=next;UpdateSelection(item);RecordGeometryOperationIfChanged(before,"移动截图区域");PositionPromptBar();ShowToolbar();e.Handled=true;return;}
        if(Keyboard.Modifiers!=ModifierKeys.None)return;
        if(item.VideoPath is not null&&(e.Key==Key.T||e.Key==Key.O)){PromptStatus.Text="视频区域不支持 OCR/翻译，请先选择截图区域";e.Handled=true;return;}
        if(e.Key==Key.C)Copy(s,new());else if(e.Key==Key.S)Save(s,new());else if(e.Key==Key.P)Pin(s,new());else if(e.Key==Key.D)Draw(s,new());else if(e.Key==Key.T)Translate(s,new());else if(e.Key==Key.O)Ocr(s,new());else if(e.Key==Key.R)Record(s,new());else if(e.Key==Key.Enter){e.Handled=true;await SendAsync(true);return;}else return;e.Handled=true;
    }
}
