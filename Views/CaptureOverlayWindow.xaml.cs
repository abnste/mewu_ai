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
using System.Windows.Markup;
using System.Globalization;
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
    private const string UploadedReferenceDragFormat="MewuAI.UploadedImageReference";
    private static readonly Brush Cyan=new SolidColorBrush(Color.FromRgb(67,198,255));
    private readonly AppHost _host;
    private CaptureFrame _frame;
    private int _desktopFrameVersion;
    private readonly List<SelectionItem> _selections=[];
    private readonly HashSet<SelectionItem> _ownedSelections=[];
    private readonly HashSet<SelectionItem> _references=[];
    private readonly List<SelectionItem> _referencePickerCandidates=[];
    private readonly List<UploadedReference> _uploadedReferences=[];
    private UploadedReference? _uploadedReferenceDragCandidate;
    private Point _uploadedReferenceDragStart;
    private bool _uploadedReferenceDragActive;
    private bool _suppressUploadedReferenceClick;
    private readonly UndoRedoHistory<OverlaySnapshot> _overlayHistory=new();
    private readonly System.Text.StringBuilder _reasoningBuffer=new();
    private List<SelectionItem> _lastSentSelections=[];
    private List<SentAnnotationTarget> _lastSentAnnotationTargets=[];
    private readonly List<AiMessage> _history=[new("system",VisualAnnotationProtocol.SystemInstruction)];
    private string _lastSubmittedPrompt=string.Empty;
    private CancellationTokenSource? _historyLoadRequest;
    private int _historyLoadVersion;
    private Point _start,_moveStart;
    private Rect _moveOrigin;
    private int _activeIndex=-1;
    private bool _selecting,_moving,_forceNewSelection,_promptBarHidden=true,_promptBarVisibilityAnimating,_promptBarEntranceStarted,_answerExpanded,_historyExpanded,_reasoningExpanded,_recordingMode,_recordingCountdownActive,_drawingMode,_drawingModalOpen,_longCaptureMode,_recordingPaused,_recordingStopping,_captureExclusionVerified,_autoVoiceStarted,_closed,_positioningPromptBar,_promptBarLayoutPassQueued,_promptBarInputLayoutQueued,_reasoningRenderScheduled;
    private int _promptBarAnimationVersion;
    private int _systemFileDialogDepth;
    private int _referenceMentionStart=-1;
    private int _nextUploadNumber;
    private CancellationTokenSource? _speechRequest,_request,_overlayRequest,_recordingCountdownRequest,_recordingStopWatchdog,_readAloudRequest,_snapProbeRequest;
    private CancellationTokenSource? _longCaptureSampleRequest;
    private Task? _longCaptureSampleTask;
    private CancellationTokenSource? _reasoningRenderRequest;
    private TaskCompletionSource<AiInteractionResponse>? _activeInteraction;
    private AiInteractionResponse? _activeInteractionFallback;
    private CancellationTokenRegistration _interactionCancellation;
    private PasswordBox? _activeSensitiveInput;
    private RecordingSession? _recordingSession;
    private SelectionItem? _recordingItem;
    private SelectionItem? _longCaptureItem;
    private OverlaySnapshot? _longCaptureBefore;
    private readonly List<BitmapSource> _longCaptureFrames=[];
    private BitmapSource? _longCaptureComposite;
    private IntPtr _longCaptureScrollTarget;
    private int _longCaptureSampleVersion;
    private bool _recordingItemWasReferenced;
    private HwndSource? _overlaySource;
    private System.Drawing.Rectangle _virtualScreenArea;
    private bool _recordingWindowRegionApplied;
    private bool _overlayReady;
    private bool _recordingHoleUpdateQueued;
    private bool _recordingRegionResetQueued;
    private bool _recordingRegionCloseQueued;
    private int _recordingHoleRetryCount;
    private int _recordingRegionResetRetryCount;
    private (int WindowLeft,int WindowTop,int WindowWidth,int WindowHeight,int HoleLeft,int HoleTop,int HoleRight,int HoleBottom,int BarLeft,int BarTop,int BarRight,int BarBottom)? _recordingWindowRegionKey;
    private readonly DispatcherTimer _recordingTimer=new(){Interval=TimeSpan.FromMilliseconds(150)};
    private DrawTool _drawTool=DrawTool.Freehand;
    private Color _drawColor=Colors.Red;
    private bool _drawHighlighter,_drawTextHighlight,_restoringDrawingAction,_drawingFontsLoaded;
    private bool _conversationAiAvailable,_translationAiAvailable;
    private bool _lastSubmittedTurnRecorded;
    private readonly NativeWindowSnapService _windowSnap=new();
    private Rect _snapCandidate=Rect.Empty;
    private Rect _stableSnapCandidate=Rect.Empty;
    private Rect? _pendingAutoSelection;
    private long _lastSnapProbeTicks;
    private Point _latestSnapProbePoint;
    private bool _latestSnapProbePointValid;
    private string _drawFontFamily="Microsoft YaHei UI";
    private double _drawFontSize=24;
    private Point _drawStart;
    private Stroke? _drawPreview;
    private Guid? _selectedDrawingElementId;
    private Stroke? _selectedDrawingStroke;
    private DrawingElementSpec? _drawingMoveOriginalElement;
    private StrokeDrawingState? _drawingMoveOriginalStroke;
    private Point _drawingMovePointerStart;
    private Point? _lastEraserPoint;
    private bool _drawingObjectMoving;
    private Border? _drawingSelectionOutline;
    private OverlaySnapshot? _pointerOperationBefore;
    private OverlaySnapshot? _resizeOperationBefore;
    private OverlaySnapshot? _drawingOperationBefore;
    private string _pointerOperationLabel="";
    private bool _drawingOperationChanged;

    private sealed class SelectionItem
    {
        public string ReferenceHandle { get; }="ref-"+Guid.NewGuid().ToString("N");
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
        public InkCanvas Markup { get; }=new(){Background=Brushes.Transparent,IsHitTestVisible=false,ClipToBounds=true,Focusable=true};
        public Canvas AiAnnotations { get; }=new(){IsHitTestVisible=true};
        public Canvas TextOverlays { get; }=new(){IsHitTestVisible=false};
        public Canvas TextSelection { get; }=new(){IsHitTestVisible=false,Background=Brushes.Transparent};
        public Border Outline { get; }=new(){Background=Brushes.Transparent,CornerRadius=new CornerRadius(7),IsHitTestVisible=false};
        public Border Badge { get; }=new(){HorizontalAlignment=HorizontalAlignment.Left,VerticalAlignment=VerticalAlignment.Top,Margin=new Thickness(7),CornerRadius=new CornerRadius(10),Background=new SolidColorBrush(Color.FromArgb(230,29,119,224)),Padding=new Thickness(7,2,7,2),IsHitTestVisible=false};
        public TextBlock BadgeText { get; }=new(){Foreground=Brushes.White,FontWeight=FontWeights.SemiBold,FontSize=11};
        public List<DrawingElementSpec> DrawingElements { get; }=[];
        public List<DrawingAction> DrawingOrder { get; }=[];
        public Stack<DrawingAction> DrawingRedo { get; }=[];
        public int NextDrawingNumber { get; set; }=1;
        public TextLayerState TextLayer { get; set; }=NoTextLayerState.Instance;
        public List<AiAnnotation> AnnotationNotes { get; }=[];
        public Dictionary<AiAnnotation,Point> AnnotationCardPositions { get; }=[];
        public OcrDocument? AnnotationOcrDocument;
        public int AnnotationOcrFrameVersion=-1;
        public Rect AnnotationOcrBounds;
        public OcrTextSelectionSession? TextSession;
        public TempMediaLease? VideoLease;
        public string? VideoPath;
        public TimeSpan VideoDuration;
        public bool VideoPlaying;
        public CancellationTokenSource? VideoAnnotationPlayback;
        public BitmapSource? CapturedImageOverride;
    }

    private sealed record UploadedReference(string Handle,string Path,AiAttachmentType Type,string MimeType,string Preview)
    {
        public string Label { get; set; }=string.Empty;
    }
    private sealed record SentAnnotationTarget(string ReferenceHandle,AiAttachmentType Type,SelectionItem? Selection);

    private abstract record TextLayerState;
    private sealed record NoTextLayerState:TextLayerState
    {
        public static NoTextLayerState Instance { get; }=new();
    }
    private sealed record OcrTextLayerState(BitmapSource Image,OcrDocument Document):TextLayerState;
    private sealed record TranslationTextLayerState(BitmapSource Image,IReadOnlyList<OcrLine> Lines,IReadOnlyList<string> Texts):TextLayerState;
    private sealed record TranslationVisualRow(string Text,double Width);
    private sealed record TranslationVisualEntry(Rect Bounds,double FontSize,double LineHeight,IReadOnlyList<TranslationVisualRow> Rows,Color BackdropColor);
    private sealed record SelectionSnapshot(
        SelectionItem Item,
        Rect Bounds,
        bool Referenced,
        StrokeCollection Markup,
        IReadOnlyList<DrawingElementSpec> DrawingElements,
        BitmapSource? CapturedImageOverride,
        TextLayerState TextLayer,
        IReadOnlyList<AiAnnotation> AnnotationNotes);
    private sealed record OverlaySnapshot(
        IReadOnlyList<SelectionSnapshot> Selections,
        SelectionItem? Active,
        string AnswerMarkdown,
        bool AnswerExpanded,
        IReadOnlyList<AiMessage> History,
        IReadOnlyList<SelectionItem> LastSentSelections,
        string LastSubmittedPrompt);
    private sealed record AnnotationMappingResult(
        IReadOnlyDictionary<SelectionItem,IReadOnlyList<AiAnnotation>> BySelection,
        int RawCount,
        int TimelineCandidateCount,
        int RegionMismatchCount,
        int TypeMismatchCount,
        int DurationRejectedCount,
        int SingleVideoRemapCount,
        int DurationClampedCount,
        int HandleMismatchCount,
        int HandleRemapCount,
        int QualityRejectedCount,
        int DuplicateRemovedCount,
        int KeyframesRemovedCount)
    {
        public int RenderedCount=>BySelection.Sum(entry=>entry.Value.Count);
    }

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

    private abstract record DrawingElementSpec(Guid Id,double X,double Y);
    private sealed record TextDrawingElement(Guid Id,double X,double Y,double Width,string Text,string FontFamily,double FontSize,Color Color,bool Highlight):DrawingElementSpec(Id,X,Y);
    private sealed record NumberDrawingElement(Guid Id,double X,double Y,double Diameter,int Number,Color Color):DrawingElementSpec(Id,X,Y);
    private sealed record MosaicDrawingElement(Guid Id,double X,double Y,double Width,double Height):DrawingElementSpec(Id,X,Y);
    private sealed record DrawingFontChoice(string Source,string DisplayName)
    {
        public override string ToString()=>DisplayName;
    }
    private abstract record DrawingAction;
    private sealed record StrokeDrawingAction(Stroke Stroke):DrawingAction;
    private sealed record ElementDrawingAction(DrawingElementSpec Element):DrawingAction;
    private sealed record StrokeRemovalDrawingAction(Stroke Stroke):DrawingAction;
    private sealed record ElementRemovalDrawingAction(DrawingElementSpec Element):DrawingAction;
    private sealed record StrokeMoveDrawingAction(Stroke Stroke,StrokeDrawingState Before,StrokeDrawingState After):DrawingAction;
    private sealed record ElementMoveDrawingAction(DrawingElementSpec Before,DrawingElementSpec After):DrawingAction;
    private sealed record StrokeDrawingState(IReadOnlyList<StylusPoint> Points);
    private enum DrawTool{Freehand,Rectangle,Ellipse,Arrow,Mosaic,Text,Number,Select,Eraser}

    private SelectionItem? Active=>_activeIndex>=0&&_activeIndex<_selections.Count?_selections[_activeIndex]:null;

    public CaptureOverlayWindow(AppHost host)
    {
        _host=host;_frame=new ScreenCaptureService().CaptureDesktop(host.Settings.IncludeCaptureCursor);InitializeComponent();LocalizationService.SetExcludeFromLocalization(SelectionLayer,true);LocalizationService.SetExcludeFromLocalization(HistoryItems,true);LocalizationService.SetExcludeFromLocalization(ReferenceChips,true);AnswerText.MarkdownChanged+=(_,_)=>TableCopyButton.Visibility=TableClipboardService.Parse(AnswerText.Markdown).Count>0?Visibility.Visible:Visibility.Collapsed;
        _history[0]=_history[0] with{Text=_history[0].Text+" 本轮附件清单中的 referenceHandle 是不可变主键；每条批注都必须原样返回它。句柄与 regionIndex 冲突时以句柄为准，禁止按 @图片N 或 @视频N 的显示编号猜测附件顺序。"};
        LoadSessionHistory();
        // Keep the composer fully below the viewport until its first arranged
        // frame.  Starting visible here lets WPF paint one terminal frame
        // before Loaded can start the entrance animation.
        PromptBarHost.Opacity=0;
        PromptBarHost.IsHitTestVisible=false;
        // Keep overlay text and icon edges crisp at mixed DPI values.  The
        // capture surface remains in physical-pixel coordinates; these flags
        // only affect WPF rasterisation of the presentation layer.
        UseLayoutRounding=true;SnapsToDevicePixels=true;TextOptions.SetTextFormattingMode(this,TextFormattingMode.Display);Root.SnapsToDevicePixels=true;
        ApplyOverlayVisualTuning();
        RefreshAiFeatureAvailability();
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
             QuickPrompt.TextChanged+=(_,_)=>{UpdateQuickPromptHint();UpdateReferencePicker();QueuePromptBarInputLayout();};QuickPrompt.GotKeyboardFocus+=(_,_)=>{UpdateQuickPromptHint();PromptInputBorder.BorderBrush=new SolidColorBrush(Color.FromRgb(115,130,235));};QuickPrompt.LostKeyboardFocus+=(_,_)=>{UpdateQuickPromptHint();PromptInputBorder.BorderBrush=new SolidColorBrush(Color.FromRgb(220,228,239));};UpdateQuickPromptHint();RefreshHistoryPreview();PromptBar.SizeChanged+=(_,_)=>PositionPromptBar();PositionPromptBar();if(_conversationAiAvailable)QuickPrompt.Focus();
            _=LoadPersistedHistoryAsync();
            // WPF can re-apply the initial Width/Height after SourceInitialized;
            // run one render-priority pass so the DIP surface remains in sync
            // with the physical HWND before any pointer coordinates arrive.
            _=Dispatcher.BeginInvoke(DispatcherPriority.Render,new Action(()=>
            {
                if(_closed)return;
                ApplyOverlayDpiLayout(area);
                DesktopImage.Width=Dimmer.Width=SelectionLayer.Width=Root.ActualWidth;DesktopImage.Height=Dimmer.Height=SelectionLayer.Height=Root.ActualHeight;
                PositionPromptBar();
                if(_conversationAiAvailable&&!_promptBarEntranceStarted)
                {
                    _promptBarEntranceStarted=true;
                    SetPromptBarHidden(false);
                }
            }));
            _=Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle,new Action(()=>
            {
                if(_closed)return;
                ApplyOverlayDpiLayout(area);
                DesktopImage.Width=Dimmer.Width=SelectionLayer.Width=Root.ActualWidth;DesktopImage.Height=Dimmer.Height=SelectionLayer.Height=Root.ActualHeight;
                PositionPromptBar();
            }));
            _overlayReady=true;
            RaisePinnedWindowsAboveOverlay();
            if(_conversationAiAvailable&&CaptureOverlayPolicy.ShouldStartAutomaticListening(_host.Settings.EnableVoiceInput,_host.Settings.AutomaticallyStartListening,_autoVoiceStarted,_closed)){_autoVoiceStarted=true;await ToggleVoiceAsync();}
        };
        DpiChanged+=(_,_)=>ApplyOverlayDpiLayout(area);
        SizeChanged+=(_,_)=>{DesktopImage.Width=Dimmer.Width=SelectionLayer.Width=Root.ActualWidth;DesktopImage.Height=Dimmer.Height=SelectionLayer.Height=Root.ActualHeight;UpdateRecordingVisualHole();if(_longCaptureMode&&_longCaptureItem is { } item){BeginLongCaptureLiveRegion(item);PositionFloatingBar(LongCaptureBar,item);if(_longCaptureComposite is { } composite)UpdateLongCapturePreview(item,composite);}PositionPromptBar();};
        RecordingBar.SizeChanged+=(_,_)=>{if(_recordingMode)UpdateRecordingVisualHole();};
        _recordingTimer.Tick+=(_,_)=>RecordingTick();
        Activated+=OnActivated;
        Closed+=OnClosed;
    }

    private void ApplyOverlayVisualTuning()
    {
        Toolbar.Padding=new Thickness(5);
        DrawingToolbar.Padding=new Thickness(5);
        RecordingBar.Padding=new Thickness(8,6,8,6);
        PromptBar.Padding=new Thickness(6,2,6,6);
        PromptBar.CornerRadius=new CornerRadius(18);
        HistoryPanel.MaxHeight=GetHistoryMaxHeight()+28;
        HistoryScroll.MaxHeight=GetHistoryMaxHeight();
        ReferenceChipScroll.MaxHeight=CompactChipScrollMaxHeight;
        ReferenceChips.Margin=new Thickness(4,2,4,0);
        QuickPrompt.MinHeight=CompactQuickPromptMinHeight;
        QuickPrompt.MaxHeight=CompactQuickPromptMaxHeight;
        QuickPrompt.Padding=new Thickness(8,4,8,4);
        QuickPrompt.FontSize=12.5;
        TextBlock.SetLineHeight(QuickPrompt,17);
        // Keep the empty-state hint just to the right of the visible caret.  The
        // TextBox retains its own left padding so typed text stays aligned while
        // the hint no longer paints underneath the caret on an empty input.
        QuickPromptHint.Margin=new Thickness(20,0,0,0);
        AnswerHeader.Margin=new Thickness(8,2,8,0);
        AnswerScroll.Margin=new Thickness(8,6,8,6);
        PromptStatus.Margin=new Thickness(0,3,0,0);
        PromptStatus.FontSize=10;
        ReasoningPanel.Margin=new Thickness(6,5,6,0);
        ReasoningPanel.Padding=new Thickness(10,8,10,8);
    }

    private void RefreshAiFeatureAvailability()
    {
        _conversationAiAvailable=_host.IsScreenAiAvailable(out _);
        _translationAiAvailable=_host.IsTranslationAvailable(out _);
        PromptBarHost.Visibility=_conversationAiAvailable?Visibility.Visible:Visibility.Collapsed;
        PromptBarHost.IsHitTestVisible=_conversationAiAvailable&&!_promptBarHidden;
        ReferenceButton.Visibility=_conversationAiAvailable?Visibility.Visible:Visibility.Collapsed;
        ApplyVoiceAvailability();
    }

    private void UpdateQuickPromptHint()
    {
        var empty=QuickPrompt.Text.Length==0;
        QuickPromptHint.Visibility=empty?Visibility.Visible:Visibility.Collapsed;
        QuickPrompt.CaretBrush=new SolidColorBrush(Color.FromRgb(91,108,235));
    }

    private (string Provider,string Model) GetHistoryScope()
    {
        if(_host.Settings.HermesEnabled)
            return ($"本机 Hermes · {_host.Settings.HermesProfile}",_host.Settings.HermesModel??string.Empty);
        var configured=_host.Settings.Providers.FirstOrDefault(item=>item.Id==_host.Settings.DefaultProviderId);
        return (configured?.Name??configured?.Id??string.Empty,configured?.Model??string.Empty);
    }

    private void LoadSessionHistory()
    {
        var (provider,model)=GetHistoryScope();
        if(string.IsNullOrWhiteSpace(provider))return;
        var entries=_host.GetSessionConversationHistory(provider,model);
        if(entries.Count==0)return;
        MergeHistoryEntries(entries);
    }

    private async Task LoadPersistedHistoryAsync()
    {
        if(!_host.Settings.SaveConversationHistory||_closed)return;
        var operation=new CancellationTokenSource();
        var version=Interlocked.Increment(ref _historyLoadVersion);
        var previous=Interlocked.Exchange(ref _historyLoadRequest,operation);
        if(previous is not null)TryCancel(previous);
        try
        {
            var entries=await new ConversationHistoryService().ReadRecentAsync(48,operation.Token).ConfigureAwait(false);
            if(operation.IsCancellationRequested||_closed||version!=Volatile.Read(ref _historyLoadVersion))return;
            await Dispatcher.InvokeAsync(() =>
            {
                if(_closed||operation.IsCancellationRequested||version!=Volatile.Read(ref _historyLoadVersion))return;
                var (provider,model)=GetHistoryScope();
                MergeHistoryEntries(entries.Where(entry=>string.Equals(entry.Provider,provider,StringComparison.Ordinal)&&string.Equals(entry.Model,model,StringComparison.Ordinal)));
                RefreshHistoryPreview();
            },DispatcherPriority.Background);
        }
        catch(OperationCanceledException) when(operation.IsCancellationRequested||_closed){}
        catch(Exception ex)
        {
            // History is optional UI state; a read failure must never prevent
            // the overlay or a new AI request from opening.
            try{new PrivacyLogger().Error("ConversationHistoryLoad",ex);}catch{}
        }
        finally
        {
            if(ReferenceEquals(Interlocked.CompareExchange(ref _historyLoadRequest,null,operation),operation))
                operation.Dispose();
            else operation.Dispose();
        }
    }

    private void MergeHistoryEntries(IEnumerable<ConversationHistoryEntry> entries)
    {
        var existingPairs=_history
            .SkipWhile(message=>string.Equals(message.Role,"system",StringComparison.OrdinalIgnoreCase))
            .Chunk(2)
            .Where(pair=>pair.Length==2&&string.Equals(pair[0].Role,"user",StringComparison.OrdinalIgnoreCase)&&string.Equals(pair[1].Role,"assistant",StringComparison.OrdinalIgnoreCase))
            .Select(pair=>(Prompt:pair[0].Text,Answer:pair[1].Text))
            .ToList();
        var existingKeys=existingPairs.Select(pair=>CreateHistoryPairKey(pair.Prompt,pair.Answer)).ToHashSet(StringComparer.Ordinal);
        var incomingKeys=new HashSet<string>(StringComparer.Ordinal);
        var merged=new List<AiMessage>();
        foreach(var entry in entries)
        {
            if(string.IsNullOrWhiteSpace(entry.Prompt)||string.IsNullOrWhiteSpace(entry.Answer))continue;
            var key=CreateHistoryPairKey(entry.Prompt,entry.Answer);
            if(existingKeys.Contains(key)||!incomingKeys.Add(key))continue;
            merged.Add(new AiMessage("user",entry.Prompt));
            merged.Add(new AiMessage("assistant",entry.Answer));
        }
        foreach(var pair in existingPairs)
        {
            var key=CreateHistoryPairKey(pair.Prompt,pair.Answer);
            if(incomingKeys.Contains(key))continue;
            merged.Add(new AiMessage("user",pair.Prompt));
            merged.Add(new AiMessage("assistant",pair.Answer));
        }
        var system=_history.FirstOrDefault(message=>string.Equals(message.Role,"system",StringComparison.OrdinalIgnoreCase))??new AiMessage("system",VisualAnnotationProtocol.SystemInstruction);
        _history.Clear();_history.Add(system);_history.AddRange(merged);
        ConversationContextPolicy.TrimInPlace(_history);
    }

    private static string CreateHistoryPairKey(string prompt,string answer)=>prompt+"\u001f"+answer;

    private void ToggleHistory(object sender,RoutedEventArgs e)
    {
        _historyExpanded=!_historyExpanded;
        RefreshHistoryPreview();
        if(_historyExpanded)
        {
            HistoryScroll.UpdateLayout();
            HistoryScroll.ScrollToEnd();
        }
        PositionPromptBar();
        e.Handled=true;
    }

    private void RefreshHistoryPreview()
    {
        if(!IsInitialized||HistoryItems is null)return;

        var messages=_history
            .Where(message=>message is not null&&(string.Equals(message.Role,"user",StringComparison.OrdinalIgnoreCase)||string.Equals(message.Role,"assistant",StringComparison.OrdinalIgnoreCase)))
            .ToArray();
        var pairs=ConversationHistoryPairing.Pair(messages).TakeLast(6).ToArray();
        var latestPairIndex=pairs.Length-1;
        var currentIsInHistory=_lastSubmittedTurnRecorded&&!string.IsNullOrWhiteSpace(_lastSubmittedPrompt)&&latestPairIndex>=0&&string.Equals(pairs[latestPairIndex].Prompt,_lastSubmittedPrompt,StringComparison.Ordinal);

        HistoryItems.Children.Clear();
        for(var pairIndex=0;pairIndex<pairs.Length;pairIndex++)
        {
            var pair=pairs[pairIndex];
            var isCurrent=currentIsInHistory&&pairIndex==latestPairIndex;
            AddHistoryPair(pair.Prompt,pair.Answer,isCurrent);
        }
        if(!string.IsNullOrWhiteSpace(_lastSubmittedPrompt)&&!currentIsInHistory)
            AddHistoryPair(_lastSubmittedPrompt,_request is null?"未收到 AI 回复 / No AI response":"正在生成回答… / Generating response…",true);
        if(HistoryItems.Children.Count==0)
        {
            HistoryItems.Children.Add(new TextBlock{Text="暂无历史对话 / No conversation yet",Foreground=new SolidColorBrush(Color.FromRgb(127,141,161)),FontSize=12,Margin=new Thickness(2,2,2,2)});
        }

        var conversationCount=pairs.Length+(!currentIsInHistory&&!string.IsNullOrWhiteSpace(_lastSubmittedPrompt)?1:0);
        HistoryToggle.ToolTip=conversationCount>0
            ?$"查看提问与历史 / Prompt & history ({conversationCount})"
            :"查看提问与历史 / Prompt & history";
        HistoryPanel.Visibility=_historyExpanded?Visibility.Visible:Visibility.Collapsed;
        HistoryChevronRotation.Angle=_historyExpanded?0:180;
        HistoryScroll.MaxHeight=GetHistoryMaxHeight();
    }

    private void AddHistoryPair(string prompt,string answer,bool current)
    {
        var card=new Border
        {
            Background=new SolidColorBrush(current?Color.FromRgb(232,237,255):Color.FromRgb(255,255,255)),
            BorderBrush=new SolidColorBrush(current?Color.FromRgb(197,207,250):Color.FromRgb(224,231,240)),
            BorderThickness=new Thickness(1),
            CornerRadius=new CornerRadius(9),
            Padding=new Thickness(8,6,8,6),
            Margin=new Thickness(0,0,0,5)
        };
        var content=new StackPanel();
        var roleColor=new SolidColorBrush(current?Color.FromRgb(79,95,207):Color.FromRgb(96,112,135));
        content.Children.Add(new TextBlock{Text="用户 / You",Foreground=roleColor,FontSize=10.5,FontWeight=FontWeights.SemiBold});
        content.Children.Add(new TextBlock{Text=LimitHistoryText(prompt),Foreground=new SolidColorBrush(Color.FromRgb(47,61,82)),FontSize=12,LineHeight=18,TextWrapping=TextWrapping.Wrap,Margin=new Thickness(0,2,0,5)});
        content.Children.Add(new Border{Height=1,Background=new SolidColorBrush(current?Color.FromRgb(205,214,246):Color.FromRgb(230,235,242)),Margin=new Thickness(0,0,0,5)});
        content.Children.Add(new TextBlock{Text="AI",Foreground=roleColor,FontSize=10.5,FontWeight=FontWeights.SemiBold});
        content.Children.Add(new TextBlock{Text=LimitHistoryText(answer),Foreground=new SolidColorBrush(Color.FromRgb(47,61,82)),FontSize=12,LineHeight=18,TextWrapping=TextWrapping.Wrap,Margin=new Thickness(0,2,0,0)});
        card.Child=content;
        HistoryItems.Children.Add(card);
    }

    private static string LimitHistoryText(string? text)
    {
        var value=(text??string.Empty).Trim();
        const int maxCharacters=900;
        return value.Length<=maxCharacters?value:value[..maxCharacters]+"…";
    }

    private double GetHistoryMaxHeight()
    {
        var monitor=PromptMonitorBounds();
        if(monitor.IsEmpty||!double.IsFinite(monitor.Height))return 104;
        // History is an optional peek panel. Keep it compact so expanding the
        // arrow does not push the composer far away from the screen edge;
        // users can still scroll through all paired turns inside it.
        return Math.Clamp(monitor.Height*.14,84,112);
    }

    private string GetReferenceLabel(SelectionItem item)
    {
        var index=_selections.IndexOf(item);
        return index<0?string.Empty:$"@{(item.VideoPath is null?"图片":"视频")}{index+1}";
    }

    private void UpdateReferencePicker()
    {
        if(!_conversationAiAvailable||QuickPrompt.CaretIndex<1||(_selections.All(item=>item.IsImplicit)&&_uploadedReferences.Count==0)){ReferencePicker.IsOpen=false;return;}
        var caret=Math.Min(QuickPrompt.CaretIndex,QuickPrompt.Text.Length);var start=caret-1;
        while(start>=0&&!char.IsWhiteSpace(QuickPrompt.Text[start])&&QuickPrompt.Text[start] is not ',' and not '，' and not '。' and not '；' and not ';')start--;
        start++;
        if(start>=caret||QuickPrompt.Text[start]!='@'){ReferencePicker.IsOpen=false;return;}
        var prefix=QuickPrompt.Text[(start+1)..caret];
        if(prefix.Length>12||prefix.Any(ch=>char.IsPunctuation(ch)&&ch!='@')){ReferencePicker.IsOpen=false;return;}
        _referenceMentionStart=start;_referencePickerCandidates.Clear();ReferencePickerItems.Children.Clear();
        foreach(var item in _selections.Where(item=>!item.IsImplicit))
        {
            var label=GetReferenceLabel(item);if(prefix.Length>0&&!label.AsSpan(1).StartsWith(prefix,StringComparison.OrdinalIgnoreCase))continue;
            _referencePickerCandidates.Add(item);var pixels=ToPixelRect(item.Bounds);var detail=item.VideoPath is null?$"图片 · {pixels.Width} × {pixels.Height}":$"视频 · {item.VideoDuration:mm\\:ss}";
            var row=new StackPanel{Orientation=Orientation.Horizontal};
            if(item.VideoPath is null){try{row.Children.Add(new Image{Source=new CroppedBitmap(_frame.Image,new Int32Rect(Math.Max(0,pixels.X),Math.Max(0,pixels.Y),Math.Min(pixels.Width,_frame.Image.PixelWidth-Math.Max(0,pixels.X)),Math.Min(pixels.Height,_frame.Image.PixelHeight-Math.Max(0,pixels.Y)))),Width=40,Height=30,Stretch=Stretch.UniformToFill,Margin=new Thickness(0,0,8,0)});}catch{}}
            row.Children.Add(new StackPanel{Children={new TextBlock{Text=label,FontWeight=FontWeights.SemiBold,Foreground=new SolidColorBrush(Color.FromRgb(82,100,223))},new TextBlock{Text=detail,FontSize=10.5,Foreground=new SolidColorBrush(Color.FromRgb(126,139,160))}}});
            var button=new Button{HorizontalContentAlignment=HorizontalAlignment.Left,Margin=new Thickness(1),Padding=new Thickness(10,7,10,7),Content=row};
            button.Click+=(_,_)=>InsertReferenceMention(item);ReferencePickerItems.Children.Add(button);
        }
        foreach(var file in _uploadedReferences)
        {
            if(prefix.Length>0&&!file.Label.AsSpan(1).StartsWith(prefix,StringComparison.OrdinalIgnoreCase))continue;
            var stack=new StackPanel{Orientation=Orientation.Horizontal};
            if(file.Type==AiAttachmentType.Image)
            {
                try{stack.Children.Add(new Image{Source=new BitmapImage(new Uri(file.Path)),Width=34,Height=28,Stretch=Stretch.UniformToFill,Margin=new Thickness(0,0,8,0)});}catch{}
            }
            else stack.Children.Add(new TextBlock{Text=file.Type==AiAttachmentType.Text?"📄":"🎞",FontSize=22,Margin=new Thickness(0,0,8,0)});
            stack.Children.Add(new StackPanel{Children={new TextBlock{Text=file.Label,FontWeight=FontWeights.SemiBold,Foreground=new SolidColorBrush(Color.FromRgb(82,100,223))},new TextBlock{Text=file.Type==AiAttachmentType.Text?file.Preview:file.Path,FontSize=10.5,Foreground=new SolidColorBrush(Color.FromRgb(126,139,160)),TextTrimming=TextTrimming.CharacterEllipsis,MaxWidth=220}}});
            var button=new Button{HorizontalContentAlignment=HorizontalAlignment.Left,Margin=new Thickness(1),Padding=new Thickness(10,7,10,7),Content=stack};
            ConfigureUploadedReferenceDrag(button,file);
            button.Click+=(_,e)=>{if(_suppressUploadedReferenceClick){e.Handled=true;return;}InsertUploadedMention(file);};ReferencePickerItems.Children.Add(button);
        }
        ReferencePicker.IsOpen=ReferencePickerItems.Children.Count>0;
    }

    private void InsertUploadedMention(UploadedReference file)
    {
        var caret=Math.Min(QuickPrompt.CaretIndex,QuickPrompt.Text.Length);var start=Math.Clamp(_referenceMentionStart,0,caret);var label=file.Label+" ";QuickPrompt.Text=QuickPrompt.Text[..start]+label+QuickPrompt.Text[caret..];QuickPrompt.CaretIndex=start+label.Length;ReferencePicker.IsOpen=false;_referenceMentionStart=-1;QuickPrompt.Focus();
    }

    private void ConfigureUploadedReferenceDrag(Button button,UploadedReference file)
    {
        if(file.Type!=AiAttachmentType.Image)return;
        button.ToolTip=$"{file.Preview}\n拖到屏幕可置顶显示";
        button.PreviewMouseLeftButtonDown+=(_,e)=>
        {
            if(e.ButtonState!=MouseButtonState.Pressed)return;
            _uploadedReferenceDragCandidate=file;
            _uploadedReferenceDragStart=button.PointToScreen(e.GetPosition(button));
            _uploadedReferenceDragActive=false;
        };
        button.PreviewMouseMove+=(_,e)=>
        {
            if(!ReferenceEquals(_uploadedReferenceDragCandidate,file)||_uploadedReferenceDragActive||e.LeftButton!=MouseButtonState.Pressed)return;
            var current=button.PointToScreen(e.GetPosition(button));
            var dpi=VisualTreeHelper.GetDpi(button);
            if(!PinnedWindowInteractionPolicy.ShouldBeginDrag(_uploadedReferenceDragStart,current,SystemParameters.MinimumHorizontalDragDistance*dpi.DpiScaleX,SystemParameters.MinimumVerticalDragDistance*dpi.DpiScaleY))return;
            _uploadedReferenceDragActive=true;
            _suppressUploadedReferenceClick=true;
            PromptStatus.Text="松开鼠标即可在当前位置置顶图片";
            var data=new DataObject();data.SetData(UploadedReferenceDragFormat,file);
            try{DragDrop.DoDragDrop(button,data,DragDropEffects.Copy);}
            finally
            {
                _uploadedReferenceDragCandidate=null;
                _uploadedReferenceDragActive=false;
                _=Dispatcher.BeginInvoke(DispatcherPriority.Background,new Action(()=>_suppressUploadedReferenceClick=false));
            }
            e.Handled=true;
        };
        button.PreviewMouseLeftButtonUp+=(_,_)=>
        {
            if(!_uploadedReferenceDragActive)_uploadedReferenceDragCandidate=null;
        };
    }

    private void OnUploadedReferenceDragOver(object sender,DragEventArgs e)
    {
        e.Effects=e.Data.GetDataPresent(UploadedReferenceDragFormat)&&e.Data.GetData(UploadedReferenceDragFormat) is UploadedReference {Type:AiAttachmentType.Image}
            ?DragDropEffects.Copy
            :DragDropEffects.None;
        e.Handled=true;
    }

    private void OnUploadedReferenceDrop(object sender,DragEventArgs e)
    {
        e.Handled=true;
        if(e.Data.GetData(UploadedReferenceDragFormat) is not UploadedReference {Type:AiAttachmentType.Image} file||!_uploadedReferences.Contains(file))return;
        try
        {
            var image=LoadUploadedPinnedImage(file.Path);
            var drop=ScreenCoordinateService.ToScreenPixelPoint(e.GetPosition(Root),Root.ActualWidth,Root.ActualHeight,_frame.Image.PixelWidth,_frame.Image.PixelHeight,_frame.OriginX,_frame.OriginY);
            var workingArea=System.Windows.Forms.Screen.FromPoint(new System.Drawing.Point(drop.X,drop.Y)).WorkingArea;
            var region=PinnedImageDropPlacement.Place(new ScreenRect(workingArea.X,workingArea.Y,workingArea.Width,workingArea.Height),drop.X,drop.Y,image.PixelWidth,image.PixelHeight);
            var window=new PinnedImageWindow(image,region);
            try{window.Show();}catch{window.Close();throw;}
            ReferencePicker.IsOpen=false;
            RefreshDesktopFrameIncludingPinnedWindows();
            RaisePinnedWindowsAboveOverlay();
            PromptStatus.Text=$"{file.Label} 已置顶，可继续框选截图";
            SetPromptBarHidden(false);
            e.Effects=DragDropEffects.Copy;
        }
        catch(Exception ex)
        {
            new PrivacyLogger().Error("UploadedReferencePin",ex);
            PromptStatus.Text=$"附件贴图失败：{ex.Message}";
            e.Effects=DragDropEffects.None;
        }
    }

    private static BitmapSource LoadUploadedPinnedImage(string path)
    {
        using var stream=new FileStream(path,FileMode.Open,FileAccess.Read,FileShare.Read);
        var frame=BitmapFrame.Create(stream,BitmapCreateOptions.PreservePixelFormat,BitmapCacheOption.OnLoad);
        if(frame.PixelWidth<=0||frame.PixelHeight<=0)throw new InvalidDataException("图片尺寸无效");
        if((long)frame.PixelWidth*frame.PixelHeight>100_000_000)throw new InvalidDataException("图片像素过大，无法安全贴图");
        frame.Freeze();
        return frame;
    }

    private void InsertReferenceMention(SelectionItem item)
    {
        var label=GetReferenceLabel(item);if(label.Length==0)return;var before=CaptureOverlaySnapshot();var caret=Math.Min(QuickPrompt.CaretIndex,QuickPrompt.Text.Length);var start=Math.Clamp(_referenceMentionStart,0,caret);var suffix=caret<QuickPrompt.Text.Length?QuickPrompt.Text[caret..]:string.Empty;QuickPrompt.Text=QuickPrompt.Text[..start]+label+" "+suffix;QuickPrompt.CaretIndex=start+label.Length+1;_references.Add(item);ReferencePicker.IsOpen=false;_referenceMentionStart=-1;UpdateReferenceChips();UpdateSelection(item);QuickPrompt.Focus();RecordOverlayOperation(before,"插入附件引用");
    }

    private void OnClosed(object? sender,EventArgs e)
    {
        _closed=true;
        if(IsInitialized)NativeMethods.TrySetWindowMouseTransparent(new WindowInteropHelper(this).Handle,false);
        if(Root.IsMouseCaptured)Root.ReleaseMouseCapture();
        if(Mouse.Captured is not null)Mouse.Capture(null);
        ClearRecordingVisualHole();
        if(_overlaySource is not null)
        {
            try{_overlaySource.RemoveHook(OverlayWindowMessage);}catch(Exception ex){new PrivacyLogger().Error("OverlayHookRemove",ex);}
            _overlaySource=null;
        }
        ResolveOverlayInteractionWithFallback();StopOverlayReadAloud();CancelSnapProbe();CancelLongCaptureSample();ResetLongCaptureState();TryCancel(_speechRequest);TryCancel(_request);TryCancel(_overlayRequest);TryCancel(_historyLoadRequest);CancelRecordingCountdown();CancelRecordingStopWatchdog();_recordingTimer.Stop();
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
        if(message!=NativeMethods.WmNcHitTest)return IntPtr.Zero;
        if(_recordingCountdownActive)
        {
            handled=true;
            return new IntPtr(NativeMethods.HtTransparent);
        }
        if(!_recordingMode||_recordingItem is not { } item)return IntPtr.Zero;
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
        CancelVideoAnnotationPlayback(item);
        try{item.VideoPreview?.Dispose();item.VideoPreview=null;}catch(Exception ex){new PrivacyLogger().Error("OverlayVideoClose",ex);}
        try{ClearTextSelection(item);}catch(Exception ex){new PrivacyLogger().Error("OverlayTextSelectionClose",ex);}
        item.VideoLease?.Dispose();item.VideoLease=null;
    }

    private void OnMouseDown(object s,MouseButtonEventArgs e)
    {
        if(_recordingMode||_drawingMode||_longCaptureMode)return;
        if(e.OriginalSource is Thumb||IsInside(e.OriginalSource as DependencyObject,PromptBar)||IsInside(e.OriginalSource as DependencyObject,Toolbar)||IsInside(e.OriginalSource as DependencyObject,DrawingToolbar)||IsInside(e.OriginalSource as DependencyObject,RecordingBar)||_selections.Any(item=>IsInside(e.OriginalSource as DependencyObject,item.TextSelection)))return;
        if(RejectIfOverlayOperationBusy())return;
        _pointerOperationBefore=CaptureOverlaySnapshot();
        var p=e.GetPosition(Root);var addNew=_forceNewSelection||Keyboard.Modifiers.HasFlag(ModifierKeys.Shift);_forceNewSelection=false;
        var hit=addNew?-1:FindSelection(p);
        if(hit>=0){Select(hit);_moving=true;_pointerOperationLabel="移动截图区域";_moveStart=p;_moveOrigin=Active!.Bounds;}
        else{RemoveImplicitSelections();var item=CreateSelection(false);_selections.Add(item);_references.Add(item);_activeIndex=_selections.Count-1;RefreshSelectionNumbers();_selecting=true;_pointerOperationLabel="新建截图区域";_start=p;var immediate=ProbeSnapRect(p);_pendingAutoSelection=!addNew&&!immediate.IsEmpty&&immediate.Contains(p)?immediate:null;item.Bounds=new Rect(p,p);SnapPreview.Visibility=Visibility.Collapsed;}
        Toolbar.Visibility=Visibility.Collapsed;SetPromptBarHidden(true);Root.CaptureMouse();e.Handled=true;
    }

    private void OnMouseMove(object s,MouseEventArgs e)
    {
        var p=e.GetPosition(Root);
        if(_recordingMode||_drawingMode||_longCaptureMode){PointerInspector.Visibility=Visibility.Collapsed;return;}
        UpdatePointerInspector(p);
        if(_selecting&&Active is { } created){if(_pendingAutoSelection is not null&&(p-_start).Length<SystemParameters.MinimumHorizontalDragDistance)return;_pendingAutoSelection=null;created.Bounds=Normalize(new Rect(_start,p));UpdateSelection(created);}
        else if(_moving&&Active is { } moved){var d=p-_moveStart;var next=ClampSelection(new Rect(_moveOrigin.X+d.X,_moveOrigin.Y+d.Y,_moveOrigin.Width,_moveOrigin.Height));if(CaptureOverlayPolicy.HasContentGeometryChanged(moved.Bounds,next))InvalidateImageDerivedLayers(moved);moved.Bounds=next;UpdateSelection(moved);}
        else
        {
            UpdateSnapPreview(p);
            if(Active is null)PositionPromptBar();
            if(!_forceNewSelection)
            {
                var hovered=FindSelection(p);
                if(hovered>=0&&hovered!=_activeIndex)
                {
                    Select(hovered);
                    PositionPromptBar();
                    ShowToolbar();
                }
            }
            var preserveToolbarPlacement=PointerInToolbarInteractionZone(p);
            SetPromptBarHidden(preserveToolbarPlacement||PointerOverSelection(p),preserveToolbarPlacement);
            return;
        }
    }

    private void OnMouseUp(object s,MouseButtonEventArgs e)
    {
        if(_recordingMode||_drawingMode||_longCaptureMode)return;
        if(!_selecting&&!_moving)return;if(_pendingAutoSelection is { } automatic&&Active is { } automaticItem){automaticItem.Bounds=automatic;_pendingAutoSelection=null;} _selecting=_moving=false;Root.ReleaseMouseCapture();
        if(Active is not { } item||!CaptureOverlayPolicy.IsUsableSelection(item.Bounds.Width,item.Bounds.Height)){RemoveActiveSelection(false);_pointerOperationBefore=null;_pointerOperationLabel="";if(Active is not null)ShowToolbar();SetPromptBarHidden(false);return;}
        UpdateSelection(item);PositionPromptBar();ShowToolbar();SetPromptBarHidden(PointerOverSelection(e.GetPosition(Root)));PromptStatus.Text=$"已选择 {_selections.Count} 个区域 · 可继续拖动添加";e.Handled=true;
        if(_pointerOperationBefore is { } before)RecordGeometryOperationIfChanged(before,_pointerOperationLabel);_pointerOperationBefore=null;_pointerOperationLabel="";
    }

    private void OnLostMouseCapture(object s,MouseEventArgs e)=>FinishInterruptedPointerInteraction();
    private void OnDeactivated(object? s,EventArgs e)
    {
        FinishInterruptedPointerInteraction();
        if(_drawingMode&&!_drawingModalOpen)FinishInterruptedDrawingMode();
    }

    private void OnActivated(object? s,EventArgs e)
    {
        if(_closed||!_overlayReady)return;
        if(_longCaptureMode){RaisePinnedWindowsAboveOverlay();return;}
        // A modal file picker temporarily activates/deactivates its owner.
        // Capturing the desktop during that transition freezes the picker into
        // the screenshot. Keep the original clean frame until the modal has
        // completely unwound on the dispatcher.
        if(_systemFileDialogDepth>0){RaisePinnedWindowsAboveOverlay();return;}
        // A pin stays above the capture UI so it remains directly movable and
        // closable. Refreshing first also removes a pin that was just closed
        // from the frozen frame, or records its latest position before the
        // overlay starts another selection.
        RefreshDesktopFrameIncludingPinnedWindows();
        RaisePinnedWindowsAboveOverlay();
    }

    private static IEnumerable<Window> GetPinnedWindows()
        => Application.Current?.Windows.OfType<Window>().Where(window=>window is PinnedImageWindow or PinnedVideoWindow) ?? [];

    internal static bool TryHandleEscapeFromPinnedWindow()
    {
        var overlay=Application.Current?.Windows.OfType<CaptureOverlayWindow>().FirstOrDefault(window=>window.IsVisible&&!window._closed);
        if(overlay is null)return false;
        overlay.HandleEscape();
        return true;
    }

    private void RestoreOverlayKeyboardFocusAfterPin()
    {
        if(_closed||!IsVisible)return;
        if(!IsActive)Activate();
        Root.Focus();
    }

    private void RaisePinnedWindowsAboveOverlay()
    {
        foreach(var window in GetPinnedWindows())
        {
            if(!window.IsVisible||!window.Topmost)continue;
            SetPinnedWindowZOrder(window,new IntPtr(-1));
        }
    }

    private static void SetPinnedWindowZOrder(Window window,IntPtr insertAfter)
    {
        var handle=new WindowInteropHelper(window).Handle;
        if(handle==IntPtr.Zero)return;
        const uint NoMove=0x0001,NoSize=0x0002,NoActivate=0x0010;
        NativeMethods.SetWindowPos(handle,insertAfter,0,0,0,0,NoMove|NoSize|NoActivate);
    }

    private void RefreshDesktopFrameIncludingPinnedWindows()
    {
        if(_closed||!_overlayReady)return;
        try
        {
            var updated=new ScreenCaptureService().CaptureDesktop(_host.Settings.IncludeCaptureCursor);
            if(updated.OriginX!=_frame.OriginX||updated.OriginY!=_frame.OriginY||updated.Image.PixelWidth!=_frame.Image.PixelWidth||updated.Image.PixelHeight!=_frame.Image.PixelHeight)return;
            _frame=updated;
            _desktopFrameVersion++;
            DesktopImage.Source=updated.Image;
            foreach(var item in _selections)UpdateSelection(item);
            if(ReferencePicker.IsOpen)UpdateReferencePicker();
        }
        catch(Exception ex)
        {
            new PrivacyLogger().Error("OverlayPinnedFrameRefresh",ex);
        }
    }
    private void FinishInterruptedPointerInteraction()
    {
        if(!_selecting&&!_moving)return;_pendingAutoSelection=null;_selecting=_moving=false;if(Root.IsMouseCaptured)Root.ReleaseMouseCapture();
        if(Active is not { } item||!CaptureOverlayPolicy.IsUsableSelection(item.Bounds.Width,item.Bounds.Height)){RemoveActiveSelection(false);_pointerOperationBefore=null;_pointerOperationLabel="";PromptStatus.Text="框选已中断，请重新拖动选择";}
        else{UpdateSelection(item);PositionPromptBar();ShowToolbar();if(_pointerOperationBefore is { } before)RecordGeometryOperationIfChanged(before,_pointerOperationLabel);_pointerOperationBefore=null;_pointerOperationLabel="";PromptStatus.Text="框选已结束，可继续操作";}
        SetPromptBarHidden(false);
    }

    private bool? ShowSystemFileDialog(CommonDialog dialog)
    {
        _systemFileDialogDepth++;
        try{return dialog.ShowDialog(this);}
        finally
        {
            // Owner activation can be raised just before or just after
            // ShowDialog returns. Keep suppression through the next idle pass.
            _=Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle,new Action(()=>_systemFileDialogDepth=Math.Max(0,_systemFileDialogDepth-1)));
        }
    }

    private SelectionItem CreateSelection(bool implicitFullScreen)
    {
        var item=new SelectionItem{IsImplicit=implicitFullScreen};_ownedSelections.Add(item);item.Badge.Child=item.BadgeText;item.Badge.Visibility=Visibility.Collapsed;item.Markup.DefaultDrawingAttributes=RegularDrawingAttributes(_drawColor);item.Markup.StrokeCollected+=(_,args)=>{if(!_drawingMode||_restoringDrawingAction||!ReferenceEquals(item,Active)||_drawTool!=DrawTool.Freehand||ReferenceEquals(args.Stroke,_drawPreview))return;item.DrawingOrder.Add(new StrokeDrawingAction(args.Stroke));item.DrawingRedo.Clear();_drawingOperationChanged=true;};item.Markup.PreviewMouseLeftButtonDown+=MarkupDown;item.Markup.PreviewMouseMove+=MarkupMove;item.Markup.PreviewMouseLeftButtonUp+=MarkupUp;item.Markup.LostMouseCapture+=MarkupLostMouseCapture;item.Host.Children.Add(item.Image);item.Host.Children.Add(item.Video);item.Host.Children.Add(item.Markup);item.Host.Children.Add(item.TextOverlays);item.Host.Children.Add(item.AiAnnotations);item.Host.Children.Add(item.TextSelection);item.Host.Children.Add(item.Outline);SelectionLayer.Children.Add(item.Host);return item;
    }

    private OverlaySnapshot CaptureOverlaySnapshot()=>new(
        _selections.Select(item=>new SelectionSnapshot(
            item,
            item.Bounds,
            _references.Contains(item),
            new StrokeCollection(item.Markup.Strokes.Select(stroke=>stroke.Clone())),
            item.DrawingElements.ToArray(),
            item.CapturedImageOverride,
            item.TextLayer,
            item.AnnotationNotes.ToArray())).ToArray(),
        Active,
        AnswerText.Markdown,
        _answerExpanded,
        _history.ToArray(),
        _lastSentSelections.ToArray(),
        _lastSubmittedPrompt);

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
            CancelVideoAnnotationPlayback(item);
            SelectionLayer.Children.Add(item.Host);
            item.Bounds=state.Bounds;
            item.Markup.Strokes.Clear();
            foreach(var stroke in state.Markup)item.Markup.Strokes.Add(stroke.Clone());
            item.DrawingElements.Clear();item.DrawingElements.AddRange(state.DrawingElements);item.DrawingOrder.Clear();item.DrawingOrder.AddRange(item.Markup.Strokes.Select(stroke=>(DrawingAction)new StrokeDrawingAction(stroke)));item.DrawingOrder.AddRange(item.DrawingElements.Select(element=>(DrawingAction)new ElementDrawingAction(element)));item.DrawingRedo.Clear();item.NextDrawingNumber=Math.Max(1,item.DrawingElements.OfType<NumberDrawingElement>().Select(element=>element.Number).DefaultIfEmpty(0).Max()+1);RebuildDrawingElements(item);item.CapturedImageOverride=state.CapturedImageOverride;item.TextLayer=state.TextLayer;
            item.AnnotationNotes.Clear();item.AnnotationNotes.AddRange(state.AnnotationNotes);
            ApplyTextLayerState(item);RenderAnnotationsForItem(item,item.VideoPath is not null?item.AnnotationNotes.FirstOrDefault(note=>note.IsVideoTimeline)?.StartTime:null);
            if(state.Referenced)_references.Add(item);
            _selections.Add(item);
        }
        _activeIndex=snapshot.Active is null?-1:_selections.IndexOf(snapshot.Active);
        if(_activeIndex<0&&_selections.Count>0)_activeIndex=_selections.Count-1;
        _history.Clear();_history.AddRange(snapshot.History);
        _lastSubmittedPrompt=snapshot.LastSubmittedPrompt;
        _lastSentSelections=[..snapshot.LastSentSelections.Where(targetItems.Contains)];
        _lastSentAnnotationTargets=[.._lastSentSelections.Select(item=>new SentAnnotationTarget(item.ReferenceHandle,item.VideoPath is null?AiAttachmentType.Image:AiAttachmentType.Video,item))];
        _answerExpanded=false;_historyExpanded=false;AnswerText.Markdown=snapshot.AnswerMarkdown;
        ResponseScroll.Visibility=Visibility.Collapsed;AnswerHeader.Visibility=AnswerScroll.Visibility=AnswerDivider.Visibility=Visibility.Collapsed;
        if(snapshot.AnswerExpanded&&snapshot.AnswerMarkdown.Length>0)ShowAnswer();
        _reasoningBuffer.Clear();ReasoningText.Text="";ReasoningToggle.Visibility=ReasoningPanel.Visibility=Visibility.Collapsed;
        ResolveOverlayInteractionWithFallback();AgentActivityItems.Children.Clear();AgentActivityCard.Visibility=AiInteractionCard.Visibility=Visibility.Collapsed;
        RefreshHistoryPreview();
        RefreshSelectionNumbers();
        foreach(var item in _selections)UpdateSelection(item);
        AutoJumpToFirstVideoMarker(_selections);
        ApplyVideoAnswerActions(snapshot.AnswerMarkdown);
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
        var px=ToPixelRect(r);if(px.Width>0&&px.Height>0&&item.VideoPath is null)item.Image.Source=item.CapturedImageOverride??ScreenCaptureService.Crop(_frame.Image,px);
        var active=ReferenceEquals(item,Active);var referenced=_references.Contains(item);item.Outline.BorderBrush=item.IsImplicit?Brushes.Transparent:active?Cyan:referenced?new SolidColorBrush(Color.FromRgb(102,112,235)):new SolidColorBrush(Color.FromArgb(185,67,168,255));item.Outline.BorderThickness=new Thickness(active?2.5:referenced?2:1.5);item.Outline.Effect=active&&!item.IsImplicit?new DropShadowEffect{Color=Color.FromRgb(39,157,255),BlurRadius=18,ShadowDepth=0,Opacity=.85}:null;item.Badge.Background=new SolidColorBrush(referenced?Color.FromArgb(238,91,101,226):Color.FromArgb(230,29,119,224));item.Badge.Visibility=item.IsImplicit?Visibility.Collapsed:Visibility.Visible;
        if(active&&!item.IsImplicit){SizeTextLabel.Text=item.VideoPath is null?$"{px.Width} × {px.Height}":$"视频 · {item.VideoDuration:mm\\:ss}";SizeText.Visibility=Visibility.Visible;Canvas.SetLeft(SizeText,r.Left);Canvas.SetTop(SizeText,Math.Max(0,r.Top-30));PositionHandles(r);}else if(item.IsImplicit){HideHandles();SizeText.Visibility=Visibility.Collapsed;}
    }

    private void Select(int index){_activeIndex=index;for(var i=0;i<_selections.Count;i++)UpdateSelection(_selections[i]);}
    private int FindSelection(Point p)=>CaptureOverlayPolicy.FindTopmostHoveredSelection(p,_selections,item=>item.IsImplicit,item=>item.Bounds);
    private bool PointerOverSelection(Point p)
    {
        var promptLeft=Canvas.GetLeft(PromptBarHost);var promptTop=Canvas.GetTop(PromptBarHost);
        var promptWidth=PromptBar.ActualWidth>0?PromptBar.ActualWidth:PromptBar.DesiredSize.Width;
        var promptHeight=PromptBar.ActualHeight>0?PromptBar.ActualHeight:PromptBar.DesiredSize.Height;
        var promptBounds=double.IsFinite(promptLeft)&&double.IsFinite(promptTop)&&promptWidth>0&&promptHeight>0
            ?new Rect(promptLeft,promptTop,promptWidth,promptHeight)
            :Rect.Empty;
        return CaptureOverlayPolicy.ShouldKeepPromptBarHiddenOverSelection(_promptBarHidden,p,promptBounds,PromptMonitorBounds(),_selections.Where(item=>!item.IsImplicit).Select(item=>item.Bounds));
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

    private void UpdatePointerInspector(Point point)
    {
        if(Root.ActualWidth<=0||Root.ActualHeight<=0||point.X<0||point.Y<0||point.X>=Root.ActualWidth||point.Y>=Root.ActualHeight){PointerInspector.Visibility=Visibility.Collapsed;return;}
        var pixelX=Math.Clamp((int)Math.Floor(point.X*_frame.Image.PixelWidth/Root.ActualWidth),0,_frame.Image.PixelWidth-1);var pixelY=Math.Clamp((int)Math.Floor(point.Y*_frame.Image.PixelHeight/Root.ActualHeight),0,_frame.Image.PixelHeight-1);
        if(!ScreenPixelSampler.TrySample(_frame.Image,pixelX,pixelY,out var color)){PointerInspector.Visibility=Visibility.Collapsed;return;}
        PointerColorSwatch.Fill=new SolidColorBrush(color);PointerColorText.Text=$"#{color.R:X2}{color.G:X2}{color.B:X2}";PointerCoordinateText.Text=$"X {_frame.OriginX+pixelX}  Y {_frame.OriginY+pixelY}";PointerInspector.Visibility=Visibility.Visible;PointerInspector.Measure(new Size(double.PositiveInfinity,double.PositiveInfinity));
        var width=Math.Max(1,PointerInspector.DesiredSize.Width);var height=Math.Max(1,PointerInspector.DesiredSize.Height);const double gap=16;var left=point.X+gap;var top=point.Y+gap;if(left+width>Root.ActualWidth-4)left=point.X-width-gap;if(top+height>Root.ActualHeight-4)top=point.Y-height-gap;Canvas.SetLeft(PointerInspector,Math.Clamp(left,4,Math.Max(4,Root.ActualWidth-width-4)));Canvas.SetTop(PointerInspector,Math.Clamp(top,4,Math.Max(4,Root.ActualHeight-height-4)));
    }
    private static Rect Normalize(Rect r)=>new(Math.Min(r.Left,r.Right),Math.Min(r.Top,r.Bottom),Math.Abs(r.Width),Math.Abs(r.Height));
    private Rect ClampSelection(Rect value){var width=Math.Min(value.Width,Root.ActualWidth);var height=Math.Min(value.Height,Root.ActualHeight);return new Rect(Math.Clamp(value.X,0,Math.Max(0,Root.ActualWidth-width)),Math.Clamp(value.Y,0,Math.Max(0,Root.ActualHeight-height)),width,height);}
    private async void UpdateSnapPreview(Point point)
    {
        _latestSnapProbePoint=point;_latestSnapProbePointValid=true;
        if(PointerOverSelection(point)){_latestSnapProbePointValid=false;CancelSnapProbe();SnapPreview.Visibility=Visibility.Collapsed;_snapCandidate=_stableSnapCandidate=Rect.Empty;return;}
        // UI Automation can take longer than a pointer move.  Keep one probe
        // alive and remember the newest pointer location instead of cancelling
        // the active probe on every move (which previously meant a moving
        // pointer could cancel every request before any snap target arrived).
        if(Root.ActualWidth<=0||Root.ActualHeight<=0)return;
        var scaleX=_frame.Image.PixelWidth/Root.ActualWidth;var scaleY=_frame.Image.PixelHeight/Root.ActualHeight;var screenX=_frame.OriginX+(int)Math.Round(point.X*scaleX);var screenY=_frame.OriginY+(int)Math.Round(point.Y*scaleY);var handle=new WindowInteropHelper(this).Handle;
        // The taskbar is intentionally not a selectable screenshot target.
        // Stop the in-flight semantic refinement too: otherwise its result
        // from the previous app position can repaint a stale full-screen box
        // after the pointer has already entered the taskbar.
        if(_windowSnap.IsTaskbarAt(screenX,screenY)){_latestSnapProbePointValid=false;CancelSnapProbe();_snapCandidate=_stableSnapCandidate=Rect.Empty;SnapPreview.Visibility=Visibility.Collapsed;return;}
        // Native hit testing is intentionally synchronous and cheap. It gives
        // the user a stable preview immediately while the single UIA probe
        // below refines it to a button/menu/image when the provider exposes
        // one.
        var fast=_windowSnap.FindFastTargetAt(screenX,screenY,handle);
        var fastRect=fast is { } fastTarget?ClampSelection(ScreenCoordinateService.ToLocalDipRect(fastTarget.Bounds,_frame.OriginX,_frame.OriginY,Root.ActualWidth,Root.ActualHeight,_frame.Image.PixelWidth,_frame.Image.PixelHeight)):Rect.Empty;
        var immediate=SelectionSnapPolicy.PreferStablePreview(point,_stableSnapCandidate,fastRect);
        if(!immediate.IsEmpty){if(immediate!=_stableSnapCandidate)_stableSnapCandidate=Rect.Empty;_snapCandidate=immediate;ShowSnapPreview(immediate);}else if(_snapProbeRequest is null){_snapCandidate=_stableSnapCandidate=Rect.Empty;SnapPreview.Visibility=Visibility.Collapsed;}
        if(_snapProbeRequest is not null)return;
        var now=System.Diagnostics.Stopwatch.GetTimestamp();if(now-_lastSnapProbeTicks<System.Diagnostics.Stopwatch.Frequency/12)return;_lastSnapProbeTicks=now;
        var probePoint=point;
        var request=new CancellationTokenSource();if(Interlocked.CompareExchange(ref _snapProbeRequest,request,null) is not null){request.Dispose();return;}
        try
        {
            // UI Automation calls can encounter this overlay, so follow the
            // documented UIA threading model and probe from an MTA worker.
            var bounds=await Task.Run(()=>_windowSnap.FindTopmostTargetAt(screenX,screenY,handle),request.Token);
            if(_closed||request.IsCancellationRequested||!ReferenceEquals(_snapProbeRequest,request))return;
            // Do not paint a result for a stale pointer location.  The latest
            // location is scheduled once this bounded probe is released.
            if(!_latestSnapProbePointValid||!_latestSnapProbePoint.Equals(probePoint))return;
            _snapCandidate=bounds is { } value?ClampSelection(ScreenCoordinateService.ToLocalDipRect(value.Bounds,_frame.OriginX,_frame.OriginY,Root.ActualWidth,Root.ActualHeight,_frame.Image.PixelWidth,_frame.Image.PixelHeight)):Rect.Empty;
            _stableSnapCandidate=_snapCandidate;
            if(_snapCandidate.IsEmpty){SnapPreview.Visibility=Visibility.Collapsed;return;}ShowSnapPreview(_snapCandidate);
        }
        catch(OperationCanceledException){}
        catch(Exception ex){new PrivacyLogger().Error("SmartSelectionProbe",ex);if(!_closed&&ReferenceEquals(_snapProbeRequest,request)){_snapCandidate=Rect.Empty;SnapPreview.Visibility=Visibility.Collapsed;}}
        finally
        {
            var ownsRequest=ReferenceEquals(Interlocked.CompareExchange(ref _snapProbeRequest,null,request),request);request.Dispose();
            if(ownsRequest&&!_closed&&_latestSnapProbePointValid&&!_latestSnapProbePoint.Equals(probePoint))
            {
                _lastSnapProbeTicks=0;
                var nextPoint=_latestSnapProbePoint;
                _=Dispatcher.BeginInvoke(DispatcherPriority.Input,new Action(()=>UpdateSnapPreview(nextPoint)));
            }
        }
    }
    private void CancelSnapProbe(){var request=Interlocked.Exchange(ref _snapProbeRequest,null);if(request is null)return;try{request.Cancel();}catch(ObjectDisposedException){}request.Dispose();}
    private void ShowSnapPreview(Rect candidate)
    {
        if(candidate.IsEmpty){SnapPreview.Visibility=Visibility.Collapsed;return;}
        SnapPreview.Width=candidate.Width;SnapPreview.Height=candidate.Height;Canvas.SetLeft(SnapPreview,candidate.Left);Canvas.SetTop(SnapPreview,candidate.Top);SnapPreview.Visibility=Visibility.Visible;
    }
    private Rect ProbeSnapRect(Point point)
    {
        if(Root.ActualWidth<=0||Root.ActualHeight<=0)return Rect.Empty;var scaleX=_frame.Image.PixelWidth/Root.ActualWidth;var scaleY=_frame.Image.PixelHeight/Root.ActualHeight;var screenX=_frame.OriginX+(int)Math.Round(point.X*scaleX);var screenY=_frame.OriginY+(int)Math.Round(point.Y*scaleY);var handle=new WindowInteropHelper(this).Handle;var bounds=_windowSnap.FindTopmostTargetAt(screenX,screenY,handle);var result=bounds is { } value?ClampSelection(ScreenCoordinateService.ToLocalDipRect(value.Bounds,_frame.OriginX,_frame.OriginY,Root.ActualWidth,Root.ActualHeight,_frame.Image.PixelWidth,_frame.Image.PixelHeight)):Rect.Empty;_snapCandidate=_stableSnapCandidate=result;return result;
    }
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
        if(Active is not {IsImplicit:false} item||_recordingMode||_longCaptureMode){Toolbar.Visibility=Visibility.Collapsed;return;}
        var regionNumber=_activeIndex+1;var type=item.VideoPath is null?"区域":"视频";ReferenceButton.ToolTip=_references.Contains(item)?$"{type}{regionNumber} 已引用；可在输入框移除":$"引用当前{type}为 @{type}{regionNumber}";ReferenceButton.Background=new SolidColorBrush(_references.Contains(item)?Color.FromRgb(218,239,231):Color.FromRgb(233,237,255));
        var isVideo=item.VideoPath is not null;var isLongImage=item.CapturedImageOverride is not null;ReferenceButton.Visibility=_conversationAiAvailable?Visibility.Visible:Visibility.Collapsed;DrawButton.Visibility=Visibility.Visible;RecordButton.Visibility=LongCaptureButton.Visibility=!isVideo&&!isLongImage?Visibility.Visible:Visibility.Collapsed;OcrButton.Visibility=isVideo?Visibility.Collapsed:Visibility.Visible;TranslateButton.Visibility=!isVideo&&_translationAiAvailable?Visibility.Visible:Visibility.Collapsed;TableButton.Visibility=!isVideo&&_conversationAiAvailable?Visibility.Visible:Visibility.Collapsed;VideoPlayButton.Visibility=isVideo?Visibility.Visible:Visibility.Collapsed;PinButton.ToolTip=isVideo?"贴视频 (P)":"贴图 (P)";CopyButton.ToolTip=isVideo?"复制视频文件 (C)":"复制图片 (C)";SaveButton.ToolTip=isVideo?"保存 MP4 / GIF (S)":"保存图片 (S)";
        Toolbar.Visibility=Visibility.Visible;PositionFloatingBar(Toolbar,item);
    }

    private void PositionFloatingBar(FrameworkElement bar,SelectionItem item)
    {
        var monitor=MonitorBounds(item.Bounds);var availableWidth=Math.Max(1,monitor.Width-PromptEdgeMargin*2);bar.MaxWidth=availableWidth;bar.Measure(new Size(availableWidth,double.PositiveInfinity));var w=CaptureOverlayPolicy.ConstrainFloatingBarWidth(monitor,bar.DesiredSize.Width);var h=bar.DesiredSize.Height;
        var promptTop=Canvas.GetTop(PromptBarHost);var promptLeft=Canvas.GetLeft(PromptBarHost);var promptWidth=Math.Max(PromptBar.ActualWidth,PromptBar.DesiredSize.Width);var promptHeight=Math.Max(PromptBar.ActualHeight,PromptBar.DesiredSize.Height);var promptBounds=PromptBarHost.Visibility==Visibility.Visible&&double.IsFinite(promptTop)&&double.IsFinite(promptLeft)&&promptWidth>0&&promptHeight>0?new Rect(promptLeft,promptTop,promptWidth,promptHeight):Rect.Empty;
        var placement=CaptureOverlayPolicy.GetFloatingBarPlacement(monitor,item.Bounds,w,h,promptBounds,PromptEdgeMargin,PromptFloatingGap);Canvas.SetLeft(bar,placement.Left);Canvas.SetTop(bar,placement.Top);
        if(ReferenceEquals(bar,Toolbar)&&SizeText.Visibility==Visibility.Visible){SizeText.Measure(new Size(double.PositiveInfinity,double.PositiveInfinity));var sizeHeight=SizeText.DesiredSize.Height;var preferred=placement.Top<item.Bounds.Top?placement.Top-sizeHeight-4:item.Bounds.Top-sizeHeight-4;var sizeY=preferred>=monitor.Top+4?preferred:Math.Min(item.Bounds.Bottom-sizeHeight-4,item.Bounds.Top+4);Canvas.SetLeft(SizeText,item.Bounds.Left);Canvas.SetTop(SizeText,sizeY);}
    }

    private void PositionPromptBar()
    {
        if(!_conversationAiAvailable){PromptBarHost.Visibility=Visibility.Collapsed;return;}
        if(_positioningPromptBar||Root.ActualWidth<=0||Root.ActualHeight<=0)return;
        var monitor=PromptMonitorBounds();
        if(monitor.IsEmpty)return;
        _positioningPromptBar=true;
        try
        {
            var availableWidth=Math.Max(1,monitor.Width-CaptureOverlayPolicy.PromptSideMargin*2);
            PromptBar.Width=Math.Min(Math.Min(CaptureOverlayPolicy.PromptPreferredWidth,PromptPreferredWidthTight),availableWidth);
            var historyMaxHeight=GetHistoryMaxHeight();
            HistoryPanel.MaxHeight=historyMaxHeight+18;
            HistoryScroll.MaxHeight=historyMaxHeight;
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
    private void QueuePromptBarInputLayout()
    {
        if(_promptBarInputLayoutQueued||_closed||!IsLoaded)return;
        _promptBarInputLayoutQueued=true;
        _=Dispatcher.BeginInvoke(DispatcherPriority.Render,new Action(() =>
        {
            _promptBarInputLayoutQueued=false;
            if(_closed||!IsLoaded||PromptBarHost.Visibility!=Visibility.Visible)return;
            // TextBox text changes invalidate the child, but the overlay is
            // hosted by a Canvas and can otherwise keep the previous arranged
            // height for one frame.  Re-measure the composer at render priority
            // so Shift+Enter grows the input row before its second line paints.
            ResizeQuickPromptToContent();
            QuickPrompt.InvalidateMeasure();
            PromptBar.InvalidateMeasure();
            PositionPromptBar();
        }));
    }
    private void ResizeQuickPromptToContent()
    {
        var width=QuickPrompt.ActualWidth;
        if(width<=0)
        {
            var border=PromptInputBorder.BorderThickness;var padding=PromptInputBorder.Padding;
            width=PromptInputBorder.ActualWidth-border.Left-border.Right-padding.Left-padding.Right;
        }
        if(!double.IsFinite(width)||width<=0)return;
        // Measure without the previous arranged height.  With an infinite
        // vertical constraint WPF reports the wrapped content height; the
        // explicit MaxHeight still keeps unusually long prompts compact and
        // lets the TextBox scroll after the bounded expansion.
        QuickPrompt.Height=double.NaN;
        QuickPrompt.Measure(new Size(width,double.PositiveInfinity));
        var desired=QuickPrompt.DesiredSize.Height;
        if(!double.IsFinite(desired)||desired<=0)return;
        QuickPrompt.Height=Math.Clamp(desired,CompactQuickPromptMinHeight,CompactQuickPromptMaxHeight);
    }
    private void SetPromptBarHidden(bool hidden,bool preserveToolbarPlacement=false){if(!_conversationAiAvailable){PromptBarHost.Visibility=Visibility.Collapsed;PromptBarHost.IsHitTestVisible=false;return;}var changed=_promptBarHidden!=hidden;_promptBarHidden=hidden;PromptBarHost.IsHitTestVisible=!hidden;UpdatePromptBarHiddenTransform(changed);if(!preserveToolbarPlacement&&Toolbar.Visibility==Visibility.Visible)ShowToolbar();}
    private void UpdatePromptBarHiddenTransform(bool animate)
    {
        if(PromptBarHost.RenderTransform is not TranslateTransform transform){transform=new TranslateTransform();PromptBarHost.RenderTransform=transform;}
        var target=0d;if(_promptBarHidden){var monitor=PromptMonitorBounds();var top=Canvas.GetTop(PromptBarHost);target=double.IsFinite(top)?Math.Max(PromptBar.ActualHeight+PromptHiddenOffset,monitor.Bottom-top+PromptHiddenOffset):PromptBar.ActualHeight+PromptHiddenOffset;}
        var targetOpacity=_promptBarHidden?0d:.99;
        // Layout passes are frequent while the pointer crosses selections.
        // They may move the host, but must never cancel an in-flight visibility
        // animation and expose its terminal frame for one render.
        if(!animate&&_promptBarVisibilityAnimating)return;
        var currentOffset=transform.Y;
        var currentOpacity=PromptBarHost.Opacity;
        transform.BeginAnimation(TranslateTransform.YProperty,null);
        PromptBarHost.BeginAnimation(OpacityProperty,null);
        transform.Y=target;
        PromptBarHost.Opacity=targetOpacity;
        if(!animate){_promptBarVisibilityAnimating=false;return;}
        var duration=TimeSpan.FromMilliseconds(_promptBarHidden?150:230);
        var version=++_promptBarAnimationVersion;
        _promptBarVisibilityAnimating=true;
        var movement=new DoubleAnimation(currentOffset,target,duration){FillBehavior=FillBehavior.Stop};
        movement.Completed+=(_,_)=>
        {
            if(version!=_promptBarAnimationVersion)return;
            _promptBarVisibilityAnimating=false;
            UpdatePromptBarHiddenTransform(false);
        };
        transform.BeginAnimation(TranslateTransform.YProperty,movement);
        PromptBarHost.BeginAnimation(OpacityProperty,new DoubleAnimation(currentOpacity,targetOpacity,TimeSpan.FromMilliseconds(_promptBarHidden?120:190)){FillBehavior=FillBehavior.Stop});
    }
    private void ShowAnswer(){SetPromptBarHidden(false);ResponseScroll.Visibility=Visibility.Visible;if(_answerExpanded)return;_answerExpanded=true;AnswerHeader.Visibility=AnswerScroll.Visibility=AnswerDivider.Visibility=Visibility.Visible;if(ReasoningToggle.Visibility!=Visibility.Visible&&!string.IsNullOrWhiteSpace(_reasoningBuffer.ToString()))RevealReasoningInProgress();_ = Dispatcher.BeginInvoke(DispatcherPriority.Render,PositionPromptBar);}
    private void ToggleReasoning(object s,RoutedEventArgs e){_reasoningExpanded=!_reasoningExpanded;ReasoningPanel.Visibility=_reasoningExpanded?Visibility.Visible:Visibility.Collapsed;ReasoningChevronRotation.Angle=_reasoningExpanded?180:0;_ = Dispatcher.BeginInvoke(PositionPromptBar);}
    private void ShowReasoning(string delta,CancellationTokenSource request)
    {
        AppendReasoning(delta);if(ReasoningToggle.Visibility!=Visibility.Visible)RevealReasoningInProgress();if(_reasoningRenderScheduled&&ReferenceEquals(_reasoningRenderRequest,request))return;_reasoningRenderScheduled=true;_reasoningRenderRequest=request;
        SetPromptBarHidden(false);
        _ = Dispatcher.BeginInvoke(DispatcherPriority.Render,new Action(()=>
        {
            if(!ReferenceEquals(_reasoningRenderRequest,request))return;
            _reasoningRenderScheduled=false;_reasoningRenderRequest=null;
            if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;
            ReasoningText.Text=LimitReasoning(_reasoningBuffer.ToString());ScrollReasoningToEnd();_ = Dispatcher.BeginInvoke(DispatcherPriority.Render,PositionPromptBar);
        }));
    }
    private void RevealReasoningInProgress()
    {
        if(string.IsNullOrWhiteSpace(_reasoningBuffer.ToString()))return;ResponseScroll.Visibility=Visibility.Visible;ReasoningToggle.Visibility=Visibility.Visible;_reasoningExpanded=true;ReasoningPanel.Visibility=Visibility.Visible;ReasoningChevronRotation.Angle=180;ReasoningLabel.Text="正在思考…";ReasoningPulse.Background=new SolidColorBrush(Color.FromRgb(123,138,244));ReasoningPulse.BeginAnimation(OpacityProperty,new DoubleAnimation(.35,1,TimeSpan.FromMilliseconds(650)){AutoReverse=true,RepeatBehavior=RepeatBehavior.Forever});ScrollReasoningToEnd();
    }
    private void FinishReasoning(string reasoning)
    {
        if(!string.IsNullOrWhiteSpace(reasoning)){_reasoningBuffer.Clear();AppendReasoning(reasoning.Trim());ReasoningText.Text=LimitReasoning(_reasoningBuffer.ToString());ScrollReasoningToEnd();}CloseReasoning("思考过程 · 已完成",Color.FromRgb(95,181,137));
    }
    private void ScrollReasoningToEnd()=>_ = Dispatcher.BeginInvoke(DispatcherPriority.Render,new Action(()=>{ReasoningScroll.UpdateLayout();ReasoningScroll.ScrollToEnd();}));
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
        foreach(var item in _selections)CancelVideoAnnotationPlayback(item);
        _lastSentSelections.Clear();
        _lastSubmittedTurnRecorded=false;
        _lastSentAnnotationTargets.Clear();
        ResolveOverlayInteractionWithFallback();AgentActivityItems.Children.Clear();AgentActivityCard.Visibility=AiInteractionCard.Visibility=Visibility.Collapsed;_answerExpanded=false;_historyExpanded=false;ResponseScroll.Visibility=Visibility.Collapsed;AnswerText.Markdown="";AnswerHeader.Visibility=AnswerScroll.Visibility=AnswerDivider.Visibility=Visibility.Collapsed;_reasoningBuffer.Clear();_reasoningRenderScheduled=false;_reasoningRenderRequest=null;ReasoningText.Text="";ReasoningToggle.Visibility=ReasoningPanel.Visibility=Visibility.Collapsed;ReasoningPulse.BeginAnimation(OpacityProperty,null);ReasoningPulse.Background=new SolidColorBrush(Color.FromRgb(123,138,244));_reasoningExpanded=false;RefreshHistoryPreview();_ = Dispatcher.BeginInvoke(PositionPromptBar);
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
        try{await _host.ReadHermesResponseAloudAsync(text,request.Token);}catch(OperationCanceledException)when(request.IsCancellationRequested){}catch(Exception ex){if(!_closed&&ReferenceEquals(_readAloudRequest,request))PromptStatus.Text=$"Hermes 朗读失败：{ex.Message}。请检查 Hermes 语音服务和默认语音包。";}finally{request.Dispose();if(ReferenceEquals(_readAloudRequest,request))_readAloudRequest=null;}
    }
    private void StopOverlayReadAloud(){var request=_readAloudRequest;_readAloudRequest=null;try{request?.Cancel();}catch(ObjectDisposedException){}_host.StopHermesReadAloud();}
    private void AppendReasoning(string value)
    {
        const int bufferLimit=ReasoningDisplayLimit*2;if(value.Length>=bufferLimit){_reasoningBuffer.Clear();_reasoningBuffer.Append(value.AsSpan(value.Length-bufferLimit));return;}var overflow=_reasoningBuffer.Length+value.Length-bufferLimit;if(overflow>0)_reasoningBuffer.Remove(0,overflow);_reasoningBuffer.Append(value);
    }
    private static string LimitReasoning(string value)=>value.Length<=ReasoningDisplayLimit?value:"…较早思考内容已收纳…\n"+value[^ReasoningDisplayLimit..];

    // Recognition must use the frozen, unmodified capture. Otherwise an OCR
    // or translation pass can feed its own previous overlay back into the next
    // pass and progressively corrupt the result.
    private BitmapSource CurrentImage(){if(Active is null)throw new InvalidOperationException("请先选择区域");return RenderSelectionImage(Active,false,false,false);}
    private BitmapSource RenderSelectionImage(SelectionItem item,bool includeManualAnnotations=true,bool includeAiAnnotations=false,bool includeTranslation=true)
    {
        var pixels=ToPixelRect(item.Bounds);var source=item.CapturedImageOverride??ScreenCaptureService.Crop(_frame.Image,pixels);var width=source.PixelWidth;var height=source.PixelHeight;var hasManual=includeManualAnnotations&&HasManualAnnotations(item);var hasAi=includeAiAnnotations&&item.AnnotationNotes.Any(note=>!note.IsVideoTimeline);var hasTranslation=includeTranslation&&item.TextLayer is TranslationTextLayerState;if(!hasManual&&!hasAi&&!hasTranslation)return source;var manual=hasManual?RenderManualOverlay(item,width,height):null;var translation=hasTranslation?RenderTranslationOverlay(item,width,height):null;var ai=hasAi?AnnotationOverlayRenderer.RenderAiOverlay(width,height,item.AnnotationNotes,null,item.AnnotationCardPositions):null;var background=hasAi?AnnotationOverlayRenderer.ApplyAiMosaics(source,item.AnnotationNotes):source;return AnnotationOverlayRenderer.Composite(background,manual,translation,ai);
    }
    private static bool HasManualAnnotations(SelectionItem item)=>item.Markup.Strokes.Count>0||item.DrawingElements.Count>0;
    private static bool HasAnyAnnotations(SelectionItem item)=>HasManualAnnotations(item)||item.AnnotationNotes.Count>0||item.TextLayer is TranslationTextLayerState;
    private static BitmapSource RenderManualOverlay(SelectionItem item,int pixelWidth,int pixelHeight)
    {
        var visual=new DrawingVisual();using(var drawing=visual.RenderOpen()){drawing.PushTransform(new ScaleTransform(pixelWidth/Math.Max(1,item.Bounds.Width),pixelHeight/Math.Max(1,item.Bounds.Height)));drawing.DrawRectangle(new VisualBrush(item.Markup),null,new Rect(0,0,item.Bounds.Width,item.Bounds.Height));drawing.Pop();}var bitmap=new RenderTargetBitmap(Math.Max(1,pixelWidth),Math.Max(1,pixelHeight),96,96,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();return bitmap;
    }
    private static BitmapSource RenderTranslationOverlay(SelectionItem item,int pixelWidth,int pixelHeight)
    {
        var visual=new DrawingVisual();using(var drawing=visual.RenderOpen()){drawing.PushTransform(new ScaleTransform(pixelWidth/Math.Max(1,item.Bounds.Width),pixelHeight/Math.Max(1,item.Bounds.Height)));drawing.DrawRectangle(new VisualBrush(item.TextOverlays),null,new Rect(0,0,item.Bounds.Width,item.Bounds.Height));drawing.Pop();}var bitmap=new RenderTargetBitmap(Math.Max(1,pixelWidth),Math.Max(1,pixelHeight),96,96,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();return bitmap;
    }
    private bool? AskWhetherToIncludeAnnotations(SelectionItem item)
    {
        if(!HasAnyAnnotations(item))return false;var kind=item.VideoPath is null?"图片":"视频";var result=MewuDialogWindow.ShowChoice(this,"保存标注内容",$"这个{kind}包含翻译、AI 标注或手工标注。请选择导出方式。","保存带标注版本","保存干净原件");return result switch{MewuDialogResult.Primary=>true,MewuDialogResult.Secondary=>false,_=>null};
    }
    private async Task<List<AiAttachment>> BuildAttachmentsAsync(IReadOnlyList<SelectionItem> targets,AiProviderCapabilities capabilities,CancellationToken cancellationToken)
    {
        _lastSentSelections=targets.ToList();
        var prepared=_lastSentSelections.Select(item=>(Item:item,Image:item.VideoPath is null?RenderSelectionImage(item,true,true,true):null)).ToList();var imageCount=prepared.Count(entry=>entry.Image is not null);var rawVideoBytes=prepared.Where(entry=>entry.Item.VideoPath is not null).Sum(entry=>Math.Min(45L*1024*1024,new FileInfo(entry.Item.VideoPath!).Length));var aggregateImageBudget=Math.Max(256L*1024,45L*1024*1024-rawVideoBytes);var perImageBudget=imageCount==0?0:Math.Min(capabilities.MaxImageSize,aggregateImageBudget/imageCount);
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
    private static List<AiAttachment> CloneAttachmentsForFollowUp(IReadOnlyList<AiAttachment> source)
    {
        ArgumentNullException.ThrowIfNull(source);
        var clones=new List<AiAttachment>(source.Count);
        try
        {
            foreach(var attachment in source)
            {
                var data=attachment.Data is { } bytes?bytes.ToArray():null;
                clones.Add(attachment with { Data=data, ProviderOwnsData=data is not null });
            }
            return clones;
        }
        catch
        {
            AiImageEncodingService.ClearAttachmentBuffers(clones);
            throw;
        }
    }
    private void PositionHandles(Rect r){var list=new[]{Nw,N,Ne,W,E,Sw,S,Se};foreach(var t in list){t.Width=t.Height=10;t.Background=Cyan;t.Visibility=Visibility.Visible;}Set(Nw,r.Left,r.Top);Set(N,r.Left+r.Width/2,r.Top);Set(Ne,r.Right,r.Top);Set(W,r.Left,r.Top+r.Height/2);Set(E,r.Right,r.Top+r.Height/2);Set(Sw,r.Left,r.Bottom);Set(S,r.Left+r.Width/2,r.Bottom);Set(Se,r.Right,r.Bottom);static void Set(Thumb t,double x,double y){Canvas.SetLeft(t,x-5);Canvas.SetTop(t,y-5);}}
    private void HideHandles(){foreach(var t in new[]{Nw,N,Ne,W,E,Sw,S,Se})t.Visibility=Visibility.Collapsed;}
    private void ResizeDelta(object sender,DragDeltaEventArgs e){if(RejectIfOverlayOperationBusy()||sender is not Thumb t||Active is not {IsImplicit:false} item)return;_resizeOperationBefore??=CaptureOverlaySnapshot();SetPromptBarHidden(true);var d=t.Tag?.ToString()??"";var l=item.Bounds.Left;var top=item.Bounds.Top;var r=item.Bounds.Right;var b=item.Bounds.Bottom;if(d.Contains('W'))l=Math.Clamp(l+e.HorizontalChange,0,r-12);if(d.Contains('E'))r=Math.Clamp(r+e.HorizontalChange,l+12,Root.ActualWidth);if(d.Contains('N'))top=Math.Clamp(top+e.VerticalChange,0,b-12);if(d.Contains('S'))b=Math.Clamp(b+e.VerticalChange,top+12,Root.ActualHeight);var next=new Rect(new Point(l,top),new Point(r,b));var snapTarget=ProbeSnapRect(Mouse.GetPosition(Root));if(!snapTarget.IsEmpty)next=SelectionSnapPolicy.SnapResize(next,d,snapTarget,9);if(CaptureOverlayPolicy.HasContentGeometryChanged(item.Bounds,next))InvalidateImageDerivedLayers(item);item.Bounds=next;UpdateSelection(item);ShowToolbar();e.Handled=true;}
    private void ResizeCompleted(object sender,DragCompletedEventArgs e){if(_resizeOperationBefore is { } before)RecordGeometryOperationIfChanged(before,"调整截图区域");_resizeOperationBefore=null;PositionPromptBar();if(Active is not null)ShowToolbar();SetPromptBarHidden(PointerOverSelection(Mouse.GetPosition(Root)));e.Handled=true;}

    private void AddRegion(object s,RoutedEventArgs e){if(RejectIfOverlayOperationBusy())return;_forceNewSelection=true;Toolbar.Visibility=Visibility.Collapsed;HideHandles();PromptStatus.Text="拖动以添加另一个区域 · 可与现有区域重叠";SetPromptBarHidden(false);}
    private void ReferenceRegion(object s,RoutedEventArgs e)
    {
        if(!_conversationAiAvailable||RejectIfOverlayOperationBusy()||Active is not {IsImplicit:false} item)return;var before=CaptureOverlaySnapshot();var added=_references.Add(item);UpdateReferenceChips();UpdateSelection(item);ShowToolbar();SetPromptBarHidden(false);QuickPrompt.Focus();if(added)RecordOverlayOperation(before,"引用截图区域");PromptStatus.Text=$"已加入 {GetReferenceLabel(item)} · 输入问题后发送";
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
        if(Active is null){HideHandles();SizeText.Visibility=Toolbar.Visibility=Visibility.Collapsed;}
    }
    private void RemoveActiveSelection(bool updateUi)
    {
        if(Active is not { } item)return;CancelVideoAnnotationPlayback(item);_references.Remove(item);SelectionLayer.Children.Remove(item.Host);_selections.RemoveAt(_activeIndex);_activeIndex=_selections.Count-1;RefreshSelectionNumbers();if(Active is { } next)UpdateSelection(next);else{HideHandles();SizeText.Visibility=Toolbar.Visibility=Visibility.Collapsed;}if(updateUi){PromptStatus.Text=_selections.Count==0?"拖动可连续框选多个区域":$"剩余 {_selections.Count} 个区域";if(Active is not null)ShowToolbar();}
    }
    private void RefreshSelectionNumbers()=>UpdateReferenceChips();
    private void UpdateReferenceChips()
    {
        ReferenceChips.Children.Clear();
        foreach(var item in _selections.Where(_references.Contains))
        {
            var chip=new Border{Background=new SolidColorBrush(Color.FromRgb(241,245,255)),BorderBrush=new SolidColorBrush(Color.FromRgb(214,222,238)),BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(8),Margin=new Thickness(0,0,4,2),Padding=new Thickness(1)};var row=new StackPanel{Orientation=Orientation.Horizontal};var type=item.VideoPath is null?"图片":"视频";var link=new Button{Content=GetReferenceLabel(item),ToolTip=$"定位到此{type}"};link.SetResourceReference(StyleProperty,"ReferenceChipButton");link.Click+=(_,_)=>{var index=_selections.IndexOf(item);if(index<0)return;Select(index);ShowToolbar();SetPromptBarHidden(false);QuickPrompt.Focus();};var remove=new Button{Content=CreateCloseIcon(),ToolTip="移除此引用"};System.Windows.Automation.AutomationProperties.SetName(remove,$"移除{type}{_selections.IndexOf(item)+1}引用");remove.SetResourceReference(StyleProperty,"ReferenceChipRemoveButton");remove.Click+=(_,_)=>{var before=CaptureOverlaySnapshot();if(!_references.Remove(item))return;UpdateReferenceChips();UpdateSelection(item);if(ReferenceEquals(item,Active))ShowToolbar();QuickPrompt.Focus();RecordOverlayOperation(before,"移除区域引用");};row.Children.Add(link);row.Children.Add(remove);chip.Child=row;ReferenceChips.Children.Add(chip);
        }
        foreach(var file in _uploadedReferences)
        {
            var chip=new Border{Background=new SolidColorBrush(Color.FromRgb(241,245,255)),BorderBrush=new SolidColorBrush(Color.FromRgb(214,222,238)),BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(8),Margin=new Thickness(0,0,4,2),Padding=new Thickness(1)};
            var row=new StackPanel{Orientation=Orientation.Horizontal};
            if(file.Type==AiAttachmentType.Image){try{row.Children.Add(new Image{Source=new BitmapImage(new Uri(file.Path)),Width=24,Height=20,Stretch=Stretch.UniformToFill,Margin=new Thickness(2,0,3,0)});}catch{}}
            var link=new Button{Content=file.Label,ToolTip=file.Preview};link.SetResourceReference(StyleProperty,"ReferenceChipButton");ConfigureUploadedReferenceDrag(link,file);var remove=new Button{Content=CreateCloseIcon(),ToolTip="移除附件"};remove.SetResourceReference(StyleProperty,"ReferenceChipRemoveButton");remove.Click+=(_,_)=>{_uploadedReferences.Remove(file);UpdateReferenceChips();UpdateReferencePicker();};row.Children.Add(link);row.Children.Add(remove);chip.Child=row;ReferenceChips.Children.Add(chip);
        }
        ReferenceChips.Visibility=ReferenceChips.Children.Count>0?Visibility.Visible:Visibility.Collapsed;QuickPromptHint.Text=ReferenceChips.Children.Count>0?"继续输入关于引用区域的问题…":"输入文字问题，或先圈选/上传要分析的内容…";_ = Dispatcher.BeginInvoke(PositionPromptBar);
    }
    private static System.Windows.Shapes.Path CreateCloseIcon()=>new(){Width=12,Height=12,Stretch=Stretch.Uniform,Stroke=new SolidColorBrush(Color.FromRgb(126,139,160)),StrokeThickness=1.8,StrokeStartLineCap=PenLineCap.Round,StrokeEndLineCap=PenLineCap.Round,Data=Geometry.Parse("M3,3 L9,9 M9,3 L3,9")};

    private async Task SendAsync(bool useDefaultPrompt,string? explicitPrompt=null,SelectionItem? onlyTarget=null,bool tableRecognition=false)
    {
        if(!_conversationAiAvailable||_closed||RejectIfOverlayOperationBusy()||_request is {IsCancellationRequested:false})return;StopOverlayReadAloud();var usingHermes=_host.Settings.HermesEnabled;var provider=_host.CreateConversationProvider(HermesConversationKind.Screen,out var providerError);if(provider is null){PromptStatus.Text=providerError??"请先配置可用的 AI Provider";RefreshAiFeatureAvailability();return;}
        var uploadedReferences=onlyTarget is null?_uploadedReferences.ToArray():[];
        // A missing explicit attachment is a deliberate text-only turn. Do
        // not manufacture a full-screen screenshot: the user must explicitly
        // select or upload visual content before anything from the desktop is
        // sent to a Provider.
        RemoveImplicitSelections();
        var before=CaptureOverlaySnapshot();
        IReadOnlyList<SelectionItem> targets=onlyTarget is null?CaptureOverlayPolicy.SelectSendTargets(_selections,item=>item.IsImplicit,_references.Contains):[onlyTarget];
        var totalCount=targets.Count+uploadedReferences.Length;
        var hasVideo=targets.Any(x=>x.VideoPath is not null)||uploadedReferences.Any(file=>file.Type==AiAttachmentType.Video);
        var hasImage=targets.Any(x=>x.VideoPath is null)||uploadedReferences.Any(file=>file.Type==AiAttachmentType.Image);
        if(hasVideo&&!provider.Capabilities.SupportsVideo){PromptStatus.Text="当前 Provider 未开启视频理解能力";return;}
        if(hasImage&&!provider.Capabilities.SupportsImage){PromptStatus.Text="当前模型不支持图片理解";return;}
        if(totalCount>OpenAiCompatibleProvider.AttachmentCountLimit){PromptStatus.Text=$"单次最多发送 {OpenAiCompatibleProvider.AttachmentCountLimit} 个附件，请移除部分引用后重试";return;}
        var sentDraft=QuickPrompt.Text;var prompt=explicitPrompt??sentDraft.Trim();
        if(prompt.Length==0&&totalCount==0){QuickPrompt.Focus();return;}
        if(prompt.Length==0&&useDefaultPrompt)
            prompt=hasVideo
                ?LocalizationService.T("按时间顺序说明引用视频中发生了什么，并为关键事件和可定位目标返回时间轴批注；动作目标需要关键帧跟踪。","Describe what happens in the referenced videos in chronological order. Add timeline annotations for key events and identifiable subjects, using keyframes to track moving subjects.")
                :targets.Count>0
                    ?totalCount>1?LocalizationService.T("综合理解这些引用区域和上传附件，说明它们之间的关系并标出关键部分。","Analyze the referenced regions and uploaded attachments together. Explain how they relate and annotate the most important details."):LocalizationService.T("理解当前引用区域，解释内容并标出关键部分。","Analyze the referenced region, explain its content, and annotate the most important details.")
                    :totalCount>1?LocalizationService.T("综合理解这些上传附件，概括它们之间的关系和关键内容。","Analyze these attachments together and summarize their relationships and key details."):LocalizationService.T("理解当前上传附件，概括内容并回答问题。","Analyze the uploaded attachment, summarize it, and answer the question.");
        if(prompt.Length==0){QuickPrompt.Focus();return;}
        var referenceDescriptors=new List<AttachmentReferenceDescriptor>(totalCount);
        foreach(var (item,index) in targets.Select((item,index)=>(item,index)))
        {
            var pixels=ToPixelRect(item.Bounds);var label=item.IsImplicit?"@当前屏幕":GetReferenceLabel(item);
            referenceDescriptors.Add(new AttachmentReferenceDescriptor(index,item.ReferenceHandle,label,item.VideoPath is null?AiAttachmentType.Image:AiAttachmentType.Video,pixels.Width,pixels.Height,item.VideoPath is null?null:item.VideoDuration.TotalSeconds,true,item.AnnotationNotes.Count>0));
        }
        foreach(var (file,index) in uploadedReferences.Select((file,index)=>(file,index)))
        {
            var dimensions=file.Type==AiAttachmentType.Image?GetUploadedImageSize(file.Path):(Width:0,Height:0);
            referenceDescriptors.Add(new AttachmentReferenceDescriptor(targets.Count+index,file.Handle,file.Label,file.Type,dimensions.Width,dimensions.Height,null,false));
        }
        var turnPrompt=tableRecognition?"识别当前区域中的表格":prompt;var hasVisualAttachments=totalCount>0;
        var hadExistingAnnotations=targets.Any(item=>item.AnnotationNotes.Count>0);var providerPrompt=hasVisualAttachments?CaptureOverlayPolicy.CreateReferenceAwarePrompt(prompt,referenceDescriptors):prompt;
            var request=CaptureOverlayPolicy.CreateManualAiRequestCancellation();_lastSubmittedPrompt=turnPrompt;_lastSubmittedTurnRecorded=false;_request=request;SendButton.IsEnabled=false;ResetAnswerForRequest();_lastSentAnnotationTargets=[..targets.Select(item=>new SentAnnotationTarget(item.ReferenceHandle,item.VideoPath is null?AiAttachmentType.Image:AiAttachmentType.Video,item)),..uploadedReferences.Select(file=>new SentAnnotationTarget(file.Handle,file.Type,null))];PromptStatus.Text=tableRecognition?"正在识别表格结构…按 Esc 可取消":hasVisualAttachments?$"正在准备 {totalCount} 个附件…按 Esc 可取消":"正在准备文字请求…按 Esc 可取消";var requestStage="provider";var streamOpen=true;var primaryApplied=false;var streamedContent=new System.Text.StringBuilder();var lastPreview=string.Empty;var previewScheduled=false;var attachmentLeases=new List<TempMediaLease>();List<AiAttachment>? attachments=null;List<AiAttachment>? repairAttachments=null;
            CrashDiagnosticsService.MarkOperation(hasVideo?"屏幕助手：视频理解请求":hasVisualAttachments?"屏幕助手：图片理解请求":"屏幕助手：文字对话请求");
        try
        {
            foreach(var video in targets.Select(item=>item.VideoPath).Where(path=>path is not null))attachmentLeases.Add(TempMediaRegistry.Shared.AcquireExistingFile(video!));
            attachments=await BuildAttachmentsAsync(targets,provider.Capabilities,request.Token);
            foreach(var file in uploadedReferences)
            {
                request.Token.ThrowIfCancellationRequested();
                if(file.Type==AiAttachmentType.Text){var bytes=await File.ReadAllBytesAsync(file.Path,request.Token);attachments.Add(new AiAttachment(AiAttachmentType.Text,file.MimeType,bytes,ProviderOwnsData:true));}
                else attachments.Add(new AiAttachment(file.Type,file.MimeType,FilePath:file.Path,ProviderOwnsData:false));
            }
            // Providers clear owned byte buffers as soon as a request ends.
            // Keep an independent, owned copy for the optional video repair
            // request so it never sends a zeroed image/text attachment.
            repairAttachments=CloneAttachmentsForFollowUp(attachments);
            if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;PromptStatus.Text=hasVisualAttachments?$"正在分析 {totalCount} 个附件…按 Esc 可取消":"正在生成文字回答…按 Esc 可取消";
            var progress=provider.Capabilities.SupportsStreaming?new Progress<AiStreamDelta>(delta=>{if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed,streamOpen))return;if(delta.ReasoningContent.Length>0)ShowReasoning(delta.ReasoningContent,request);if(delta.Content.Length>0){streamedContent.Append(delta.Content);if(previewScheduled)return;previewScheduled=true;_ = Dispatcher.BeginInvoke(DispatcherPriority.Render,new Action(()=>{previewScheduled=false;if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed,streamOpen))return;var preview=StructuredResponseParser.GetStreamingAnswerPreview(streamedContent.ToString());if(preview.Length==0||string.Equals(preview,lastPreview,StringComparison.Ordinal))return;lastPreview=preview;ShowAnswer();AnswerText.Markdown=preview;AnswerScroll.UpdateLayout();AnswerScroll.ScrollToEnd();PromptBar.InvalidateMeasure();PromptBar.UpdateLayout();PositionPromptBar();PromptStatus.Text="正在整理回答…";}));}}):null;
            var agentProgress=usingHermes?new Progress<AiAgentEvent>(update=>UpdateOverlayAgentActivity(update,request)):null;var aiRequest=CaptureOverlayPolicy.CreateScreenAiRequest(providerPrompt,ConversationContextPolicy.CreateBoundedHistory(_history),attachments,progress,agentProgress,usingHermes?HandleOverlayInteractionAsync:null,hasVisualAttachments);var result=await provider.SendAsync(aiRequest,request.Token);requestStage="render";streamOpen=false;if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;request.Token.ThrowIfCancellationRequested();
            // Normalize the protocol before touching the answer card, mapping
            // annotations, or writing history. This prevents a complete JSON
            // envelope from flashing in the UI and makes every downstream
            // operation consume the same validated answer.
            result=NormalizeStructuredResult(result,hasVisualAttachments);
            var emptyAnswer=AiResultValidation.GetEmptyAnswerMessage(result);if(emptyAnswer is not null){FinishReasoning(result.Reasoning);ShowAnswer();AnswerText.Markdown=emptyAnswer;PromptStatus.Text=emptyAnswer;new PrivacyLogger().Info("ScreenAiEmptyAnswer",hasVideo?"视频请求返回空正文，已保留思考与失败状态":"图片请求返回空正文，已保留思考与失败状态");return;}
            ShowAnswer();FinishReasoning(result.Reasoning);AnswerText.Markdown=result.Answer;AnswerScroll.UpdateLayout();AnswerScroll.ScrollToEnd();PromptBar.InvalidateMeasure();PromptBar.UpdateLayout();PositionPromptBar();if(!tableRecognition&&CaptureOverlayPolicy.ShouldClearDraft(QuickPrompt.Text,sentDraft))QuickPrompt.Clear();var primaryMapping=await MapAnnotationsAsync(result.Annotations,request.Token);var primaryReturnedAnnotationCount=primaryMapping.RenderedCount;var renderedAnnotationCount=ApplyAnnotationMapping(primaryMapping,result.AnnotationUpdateMode,true);ApplyVideoAnswerActions(result.Answer);primaryApplied=true;LogAnnotationMapping("初稿",primaryMapping);
            AgentActivityCard.Visibility=Visibility.Collapsed;
            // NormalizeStructuredResult above already handles raw protocol
            // envelopes. Re-parsing the extracted plain answer here discarded
            // its valid annotation list and made diagnostics report zero even
            // though the initial mapping still held annotations.
            var repairReturnedAnnotationCount=-1;var imageRepair=hasVisualAttachments&&!hasVideo&&result.AnnotationUpdateMode!=AiAnnotationUpdateMode.Preserve&&CaptureOverlayPolicy.NeedsImageAnnotationRepair(prompt,result.Answer,primaryReturnedAnnotationCount,primaryMapping.QualityRejectedCount);var videoRepair=CaptureOverlayPolicy.ShouldRunVideoAnnotationRepair(hasVideo,hadExistingAnnotations,result.AnnotationUpdateMode);
            if(videoRepair||imageRepair)
            {
                PromptStatus.Text=hasVideo?(renderedAnnotationCount==0?"模型未返回时间轴标注，正在自动补标…按 Esc 可取消":"正在核对遗漏片段和定位时间…按 Esc 可取消"):"模型没有返回可执行图片标注，正在自动纠正…按 Esc 可取消";requestStage="provider";
                CrashDiagnosticsService.MarkOperation(hasVideo?"屏幕助手：视频批注完整性核验":"屏幕助手：图片批注自动纠正");new PrivacyLogger().Info("ScreenAiAnnotationPhase",$"开始{(hasVideo?"核验":"图片补标")}；初稿有效批注 {renderedAnnotationCount}");
                try
                {
                    var repairMode=CaptureOverlayPolicy.GetRepairAnnotationUpdateMode(hadExistingAnnotations,result.AnnotationUpdateMode);var repairPrompt=hasVideo?CaptureOverlayPolicy.CreateVideoAnnotationRepairPrompt(providerPrompt,result.Answer,repairMode):CaptureOverlayPolicy.CreateImageAnnotationRepairPrompt(providerPrompt,result.Answer,repairMode);var repairRequest=CaptureOverlayPolicy.CreateScreenAiRequest(repairPrompt,ConversationContextPolicy.CreateBoundedHistory(_history),repairAttachments??[],null,agentProgress,usingHermes?HandleOverlayInteractionAsync:null,true);
                    var repaired=await provider.SendAsync(repairRequest,request.Token);requestStage="render";if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;request.Token.ThrowIfCancellationRequested();repaired=NormalizeStructuredResult(repaired,true);repairReturnedAnnotationCount=repaired.Annotations.Count;
                    var repairedMapping=AiResultValidation.GetEmptyAnswerMessage(repaired) is null?await MapAnnotationsAsync(repaired.Annotations,request.Token):await MapAnnotationsAsync([],request.Token);LogAnnotationMapping("核验",repairedMapping);
                    if(repairedMapping.RenderedCount>0&&(!hasVideo||repairedMapping.RenderedCount>=primaryReturnedAnnotationCount)){result=repaired;renderedAnnotationCount=ApplyAnnotationMapping(repairedMapping,repaired.AnnotationUpdateMode,true);ShowAnswer();FinishReasoning(repaired.Reasoning);AnswerText.Markdown=result.Answer;AnswerScroll.UpdateLayout();AnswerScroll.ScrollToEnd();PromptBar.InvalidateMeasure();PromptBar.UpdateLayout();PositionPromptBar();ApplyVideoAnswerActions(result.Answer);}
                }
                catch(OperationCanceledException){throw;}
                catch(Exception ex){new PrivacyLogger().Error("ScreenAiAnnotationRepair",ex);new PrivacyLogger().Info("ScreenAiAnnotationPhase",$"核验失败；保留初稿有效批注 {renderedAnnotationCount}");requestStage="render";}
            }
            new PrivacyLogger().Info("ScreenAiResult",$"附件 {totalCount}，视频 {targets.Count(item=>item.VideoPath is not null)+uploadedReferences.Count(file=>file.Type==AiAttachmentType.Video)}，最终模型批注 {result.Annotations.Count}，补标返回 {repairReturnedAnnotationCount}，有效批注 {renderedAnnotationCount}");
            var configured=_host.Settings.Providers.FirstOrDefault(x=>x.Id==provider.Id);var historyProvider=usingHermes?$"本机 Hermes · {_host.Settings.HermesProfile}":configured?.Name??provider.Id;var historyModel=usingHermes?_host.Settings.HermesModel:configured?.Model??string.Empty;if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;request.Token.ThrowIfCancellationRequested();if(_host.Settings.SaveConversationHistory)await new ConversationHistoryService().TryAppendAsync(historyProvider,historyModel,turnPrompt,result.Answer,request.Token);if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;request.Token.ThrowIfCancellationRequested();_host.RememberConversationHistory(new ConversationHistoryEntry(DateTimeOffset.UtcNow,historyProvider,historyModel,turnPrompt,result.Answer));_history.Add(new("user",turnPrompt));_history.Add(new("assistant",result.Answer));_lastSubmittedTurnRecorded=true;ConversationContextPolicy.TrimInPlace(_history);RefreshHistoryPreview();RecordOverlayOperation(before,tableRecognition?"AI 表格识别":"AI 识图");var tableCount=TableClipboardService.Parse(result.Answer).Count;PromptStatus.Text=tableRecognition?(tableCount>0?$"已识别 {tableCount} 个表格 · 点击回答上方的“复制表格”":"没有识别到完整表格，可调整选区后重试"):hasVideo?CaptureOverlayPolicy.GetVideoCompletionStatus(true,renderedAnnotationCount):renderedAnnotationCount>0?$"已在 {_lastSentSelections.Count(item=>item.VideoPath is null)} 个引用区域中标出重点 · 可继续提问":"完成 · 可继续提问";if(usingHermes&&_host.Settings.HermesAutoReadAloud&&!tableRecognition)_=BeginOverlayReadAloudAsync(result.Answer);
        }
        catch(OperationCanceledException){new PrivacyLogger().Info("ScreenAiAnnotationPhase",primaryApplied?"核验或后续处理已取消；保留已显示的初稿":"初稿请求已取消；恢复发送前状态");if(!_closed&&ReferenceEquals(_request,request)){if(primaryApplied)PromptStatus.Text="已停止核验，保留初稿和已显示标注";else{ApplyOverlaySnapshot(before);PromptStatus.Text="已取消";}}}
        catch(Exception ex){new PrivacyLogger().Error(requestStage=="render"?"ScreenAiRender":"ScreenAiRequest",ex);if(!_closed&&ReferenceEquals(_request,request)){var message=request.IsCancellationRequested?"已取消":$"请求失败：{ex.Message}";if(request.IsCancellationRequested)ApplyOverlaySnapshot(before);else{CloseReasoning("思考过程 · 请求失败",Color.FromRgb(214,120,120));ShowAnswer();AnswerText.Markdown=message;}PromptStatus.Text=message;}}
        finally{streamOpen=false;if(attachments is not null)AiImageEncodingService.ClearAttachmentBuffers(attachments);if(repairAttachments is not null)AiImageEncodingService.ClearAttachmentBuffers(repairAttachments);foreach(var lease in attachmentLeases)lease.Dispose();var ownsRequest=ReferenceEquals(_request,request);if(CaptureOverlayPolicy.ShouldFinalizeCanceledAiRequest(_request,request,_closed)){CloseReasoning(primaryApplied?"思考过程 · 核验已停止":"思考过程 · 已取消",Color.FromRgb(142,153,169));PromptStatus.Text=primaryApplied?"已停止核验，保留初稿和已显示标注":"已取消";}request.Dispose();if(ownsRequest){_request=null;if(!_closed){SendButton.IsEnabled=true;_ = Dispatcher.BeginInvoke(PositionPromptBar);}}if(!_closed)CrashDiagnosticsService.MarkOperation("屏幕助手：等待操作");}
    }

    private static AiResult NormalizeStructuredResult(AiResult result,bool expectStructuredResponse=true)
    {
        // OpenAI-compatible providers already parse the envelope inside their
        // transport layer. Re-parsing a plain answer would discard its valid
        // annotations, so only inspect responses that still look like a raw
        // structured root and have not yielded annotations yet.
        if(!expectStructuredResponse||result.Annotations.Count>0)return result;
        var value=result.Answer?.TrimStart()??string.Empty;
        if(value.StartsWith('{')||value.StartsWith('[')||value.StartsWith("```",StringComparison.OrdinalIgnoreCase)||value.StartsWith("json",StringComparison.OrdinalIgnoreCase))
        {
            var parsed=StructuredResponseParser.Parse(value,result.Reasoning,true);
            if(!string.IsNullOrWhiteSpace(parsed.Answer)||value.Length==0)return parsed;
        }
        return result;
    }

    private async Task<AnnotationMappingResult> MapAnnotationsAsync(IReadOnlyList<AiAnnotation> notes,CancellationToken token)
    {
        var mapped=_lastSentSelections.Distinct().ToDictionary(item=>item,item=>(IReadOnlyList<AiAnnotation>)Array.Empty<AiAnnotation>());var buckets=_lastSentSelections.Distinct().ToDictionary(item=>item,item=>new List<AiAnnotation>());var sentTargets=_lastSentAnnotationTargets.ToArray();var targets=sentTargets.Select(item=>new AnnotationReferenceTarget(item.ReferenceHandle,item.Type==AiAttachmentType.Video)).ToArray();
        var timelineCandidates=0;var regionMismatch=0;var typeMismatch=0;var durationRejected=0;var singleVideoRemaps=0;var durationClamped=0;var handleMismatches=0;var handleRemaps=0;var qualityRejected=0;var duplicatesRemoved=0;var keyframesRemoved=0;var elementAligned=new HashSet<AiAnnotation>();var accessibilityAttempts=0;var overlayHandle=new WindowInteropHelper(this).Handle;
        foreach(var original in notes)
        {
            if(original.IsVideoTimeline)timelineCandidates++;
            var resolution=CaptureOverlayPolicy.ResolveAnnotationTarget(original.RegionIndex,original.ReferenceHandle,original.IsVideoTimeline,targets);
            if(!resolution.Success)
            {
                if(resolution.Failure==AnnotationTargetFailure.HandleMismatch)handleMismatches++;else if(resolution.Failure==AnnotationTargetFailure.RegionMismatch)regionMismatch++;else typeMismatch++;
                continue;
            }
            var targetIndex=resolution.TargetIndex;if(resolution.Remapped){if(string.IsNullOrWhiteSpace(original.ReferenceHandle))singleVideoRemaps++;else handleRemaps++;}
            var sentTarget=sentTargets[targetIndex];
            // Uploaded files are valid model inputs but do not have an in-place
            // selection layer. Never route their annotations to another image.
            if(sentTarget.Selection is not { } target)continue;
            var isVideo=sentTarget.Type==AiAttachmentType.Video;
            var note=original.RegionIndex==targetIndex&&string.Equals(original.ReferenceHandle,sentTarget.ReferenceHandle,StringComparison.Ordinal)?original:original with{RegionIndex=targetIndex,ReferenceHandle=sentTarget.ReferenceHandle};
            if(isVideo)
            {
                if(!VideoAnnotationTimeline.TryFitToDuration(note,target.VideoDuration.TotalSeconds,out note,out var clamped)){durationRejected++;continue;}
                if(clamped)durationClamped++;
            }
            else if(accessibilityAttempts++<12)
            {
                // The model only needs to land near the correct control.  If
                // the frozen screenshot corresponds to a native/UIA element,
                // use its actual physical bounds instead of trusting a visual
                // estimate. This is the same accessibility-first principle
                // used by computer-use for reliable button/edit targeting.
                var pixels=ToPixelRect(target.Bounds);var selectionScreen=ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY);var centerX=selectionScreen.X+(int)Math.Round((note.X+note.Width/2)*selectionScreen.Width);var centerY=selectionScreen.Y+(int)Math.Round((note.Y+note.Height/2)*selectionScreen.Height);var control=_windowSnap.FindTopmostTargetAt(centerX,centerY,overlayHandle);
                if(control is {IsActionableSemanticControl:true}&&AccessibilityAnnotationRefinementService.TryRefine(note,selectionScreen,control.Bounds,out var exact)){note=exact;elementAligned.Add(note);}
            }
            buckets[target].Add(note);
        }
        foreach(var entry in buckets)
        {
            token.ThrowIfCancellationRequested();
            var notesForItem=entry.Value.OrderBy(note=>note.IsVideoTimeline?note.Keyframes![0].Time:0).Take(48).ToArray();
            if(entry.Key.VideoPath is null&&notesForItem.Length>0)
            {
                var cleanImage=RenderSelectionImage(entry.Key,false,false,false);
                var beforeOcr=notesForItem;
                OcrDocument? document=entry.Key.AnnotationOcrFrameVersion==_desktopFrameVersion&&entry.Key.AnnotationOcrBounds==entry.Key.Bounds?entry.Key.AnnotationOcrDocument:null;
                if(document is null&&notesForItem.Any(note=>!elementAligned.Contains(note)&&note.Kind is (AiAnnotationKind.Callout or AiAnnotationKind.Rectangle or AiAnnotationKind.Ellipse)&&!string.IsNullOrWhiteSpace(note.Text)))
                {
                    try{document=await new WindowsOcrService().RecognizeAsync(cleanImage,token);entry.Key.AnnotationOcrDocument=document;entry.Key.AnnotationOcrFrameVersion=_desktopFrameVersion;entry.Key.AnnotationOcrBounds=entry.Key.Bounds;}
                    catch(OperationCanceledException){throw;}
                    catch(Exception ex){new PrivacyLogger().Error("AiAnnotationOcrRefinement",ex);}
                }
                if(document is not null)
                {
                    notesForItem=OcrAnnotationRefinementService.RefineAll(document,cleanImage.PixelWidth,cleanImage.PixelHeight,notesForItem,out var ocrRefined).ToArray();
                    if(ocrRefined>0)new PrivacyLogger().Info("AiAnnotationOcrRefinement",$"OCR 语义锚定 {ocrRefined} 个图片批注框");
                }
                notesForItem=notesForItem.Select((note,index)=>elementAligned.Contains(beforeOcr[index])||!Equals(note,beforeOcr[index])?note:AnnotationBoxRefinementService.Refine(cleanImage,note)).ToArray();
            }
            notesForItem=AnnotationPostProcessor.Process(notesForItem,entry.Key.VideoPath is not null,out var postProcess).ToArray();qualityRejected+=postProcess.QualityRejected;duplicatesRemoved+=postProcess.DuplicatesRemoved;keyframesRemoved+=postProcess.KeyframesRemoved;
            mapped[entry.Key]=notesForItem;
        }
        return new AnnotationMappingResult(mapped,notes.Count,timelineCandidates,regionMismatch,typeMismatch,durationRejected,singleVideoRemaps,durationClamped,handleMismatches,handleRemaps,qualityRejected,duplicatesRemoved,keyframesRemoved);
    }

    private int ApplyAnnotationMapping(AnnotationMappingResult mapping,AiAnnotationUpdateMode mode,bool autoJump)
    {
        var changedItems=new List<SelectionItem>();
        foreach(var item in _lastSentSelections.Distinct())
        {
            if(!mapping.BySelection.TryGetValue(item,out var notes))continue;var update=AnnotationUpdateService.Apply(item.AnnotationNotes,notes,mode,item.VideoPath is not null);if(!update.Changed)continue;
            CancelVideoAnnotationPlayback(item);item.AnnotationNotes.Clear();item.AnnotationNotes.AddRange(update.Annotations);if(update.Replaced)item.AnnotationCardPositions.Clear();var presentationTime=item.VideoPath is not null?item.VideoPreview?.LastPresentedPosition.TotalSeconds:null;RenderAnnotationsForItem(item,presentationTime);changedItems.Add(item);
        }
        if(autoJump&&changedItems.Count>0)AutoJumpToFirstVideoMarker(changedItems);
        return _lastSentSelections.Distinct().Sum(item=>item.AnnotationNotes.Count);
    }

    private static void LogAnnotationMapping(string phase,AnnotationMappingResult mapping)
    {
        new PrivacyLogger().Info("ScreenAiAnnotationMap",$"{phase}：原始 {mapping.RawCount}，时间轴候选 {mapping.TimelineCandidateCount}，句柄不匹配 {mapping.HandleMismatchCount}，句柄纠偏 {mapping.HandleRemapCount}，索引不匹配 {mapping.RegionMismatchCount}，类型不匹配 {mapping.TypeMismatchCount}，时长拒绝 {mapping.DurationRejectedCount}，质量拒绝 {mapping.QualityRejectedCount}，重复抑制 {mapping.DuplicateRemovedCount}，关键帧精简 {mapping.KeyframesRemovedCount}，单视频重映射 {mapping.SingleVideoRemapCount}，时长钳制 {mapping.DurationClampedCount}，最终渲染 {mapping.RenderedCount}");
    }

    private void AutoJumpToFirstVideoMarker(IEnumerable<SelectionItem> items)
    {
        SelectionItem? firstItem=null;AiAnnotation? firstAnnotation=null;VideoAnnotationKeyframe? firstFrame=null;
        foreach(var item in items.Where(candidate=>candidate.VideoPath is not null))
            if(VideoAnnotationTimeline.TryGetFirstMarker(item.AnnotationNotes,out var annotation,out var frame)&&(firstFrame is null||frame.Time<firstFrame.Time)){firstItem=item;firstAnnotation=annotation;firstFrame=frame;}
        if(firstItem is not null&&firstAnnotation is not null&&firstFrame is not null)_=JumpToVideoAnnotationFrameAsync(firstItem,firstAnnotation,firstFrame,true);
    }

    private void ApplyVideoAnswerActions(string answer)
    {
        var actions=new List<MarkdownAnswerAction>();
        foreach(var target in _lastSentSelections.Where(item=>item.VideoPath is not null))
        {
            foreach(var action in VideoAnnotationTimeline.CreateAnswerActions(target.AnnotationNotes))
            {
                if(actions.Count>=VideoAnnotationTimeline.MaxAnswerActions)break;
                var annotation=action.Annotation;var label=VideoAnnotationActionLabelFormatter.Create(annotation,action.Kind);
                if(action.Kind==VideoAnnotationAnswerActionKind.PlayRange)
                {
                    var currentTarget=target;var currentAnnotation=annotation;actions.Add(new MarkdownAnswerAction(label,$"播放对应动作并跟踪标注：{annotation.Text}",()=>ActivateVideoAnswerAction(currentTarget,currentAnnotation)));
                }
                else if(action.Frame is { } frame)
                {
                    var currentTarget=target;var currentAnnotation=annotation;var marker=frame;actions.Add(new MarkdownAnswerAction(label,$"跳转到这一标记帧并暂停显示标注：{annotation.Text}",()=>ActivateVideoFrameAnswerAction(currentTarget,currentAnnotation,marker)));
                }
            }
            if(actions.Count>=VideoAnnotationTimeline.MaxAnswerActions)break;
        }
        AnswerText.SetMarkdownWithActions(answer,actions);
    }

    private void ActivateVideoAnswerAction(SelectionItem item,AiAnnotation annotation)
    {
        if(_closed||!_selections.Contains(item)||item.VideoPath is null)return;var index=_selections.IndexOf(item);if(index>=0)Select(index);SetPromptBarHidden(false);ShowToolbar();_ = PlayVideoAnnotationsAsync(item,annotation);
    }

    private void ActivateVideoFrameAnswerAction(SelectionItem item,AiAnnotation annotation,VideoAnnotationKeyframe frame)
    {
        if(_closed||!_selections.Contains(item)||item.VideoPath is null)return;var index=_selections.IndexOf(item);if(index>=0)Select(index);SetPromptBarHidden(false);ShowToolbar();_ = JumpToVideoAnnotationFrameAsync(item,annotation,frame,false);
    }

    private async Task JumpToVideoAnnotationFrameAsync(SelectionItem item,AiAnnotation annotation,VideoAnnotationKeyframe frame,bool automatic)
    {
        CrashDiagnosticsService.MarkOperation("屏幕助手：视频标记帧跳转");CancelVideoAnnotationPlayback(item);var playback=new CancellationTokenSource();item.VideoAnnotationPlayback=playback;
        try
        {
            var preview=EnsureVideoPreview(item);var requested=TimeSpan.FromSeconds(frame.Time);var presented=await preview.SeekAsync(requested,pauseAfterSeek:true,playback.Token);
            if(_closed||!ReferenceEquals(item.VideoAnnotationPlayback,playback)||!_selections.Contains(item))return;item.VideoPlaying=false;RenderAnnotationsForItem(item,presented.TotalSeconds);PromptStatus.Text=automatic?$"已自动定位到第一个标记 {FormatVideoTime(presented)} · 视频已暂停":$"已跳到标记 {FormatVideoTime(presented)} · 视频已暂停";
        }
        catch(OperationCanceledException){}
        catch(Exception ex){new PrivacyLogger().Error("VideoAnnotationFrameJump",ex);if(!_closed&&ReferenceEquals(item.VideoAnnotationPlayback,playback))PromptStatus.Text=$"视频标记跳转失败：{ex.Message}";}
        finally{if(ReferenceEquals(item.VideoAnnotationPlayback,playback))item.VideoAnnotationPlayback=null;playback.Dispose();if(!_closed)CrashDiagnosticsService.MarkOperation("屏幕助手：等待操作");}
    }

    private static void RenderAnnotationsForItem(SelectionItem item,double? videoTime=null)
    {
        item.AiAnnotations.Children.Clear();var w=item.Bounds.Width;var h=item.Bounds.Height;var cardWidth=Math.Min(Math.Clamp(w*.3,145,360),Math.Max(1,w-10));var font=Math.Clamp(w/70,11,22);
        // Callout boxes are laid out against the same normalized geometry below.
        // Do not render a second primitive rectangle/ellipse layer: it is the
        // source of the offset blue duplicate boxes on dense pages.
        // Render protocol rectangles/ellipses in the same primitive layer as
        // the other AI tools. The old callout-only path silently dropped these
        // accurate red boxes while a separate blue approximation remained.
        var callouts=item.AnnotationNotes.Where(note=>note.Kind==AiAnnotationKind.Callout).ToArray();
        var primitiveNotes=item.AnnotationNotes.Where(note=>note.Kind is not (AiAnnotationKind.Callout or AiAnnotationKind.Mosaic)&&!AnnotationLayoutService.IsDuplicateTargetMarker(note,callouts)).ToArray();
        if(primitiveNotes.Length>0)
        {
            var overlay=AnnotationOverlayRenderer.CreateAiDrawingImage(Math.Max(1,(int)Math.Ceiling(w)),Math.Max(1,(int)Math.Ceiling(h)),primitiveNotes,videoTime,null,VideoAnnotationTimeline.LiveFrameToleranceSeconds);item.AiAnnotations.Children.Add(new Image{Source=overlay,Width=w,Height=h,Stretch=Stretch.Fill,IsHitTestVisible=false});
        }
        var calloutTargets=new Dictionary<AiAnnotation,Rect>(ReferenceEqualityComparer.Instance);var targetCount=0;
        foreach(var n in item.AnnotationNotes.Take(48))
        {
            var frame=new VideoAnnotationKeyframe(videoTime??0,n.X,n.Y,n.Width,n.Height);
            if(n.Kind!=AiAnnotationKind.Callout||targetCount>=VisualAnnotationProtocol.MaximumCallouts||string.IsNullOrWhiteSpace(n.Text)||n.Width<0.012||n.Height<0.012||n.Width>0.92||n.Height>0.92)continue;
            if(n.IsVideoTimeline&&(!videoTime.HasValue||!VideoAnnotationTimeline.TryInterpolateForPresentation(n,videoTime.Value,VideoAnnotationTimeline.LiveFrameToleranceSeconds,out frame)))continue;
            var x=Math.Clamp(frame.X,0,1)*w;var y=Math.Clamp(frame.Y,0,1)*h;var rw=Math.Max(14,Math.Clamp(frame.Width,0,1)*w);var rh=Math.Max(14,Math.Clamp(frame.Height,0,1)*h);
            calloutTargets[n]=new Rect(x,y,rw,rh);targetCount++;
        }
        var calloutOrder=calloutTargets.Keys.ToArray();var calloutCards=new Dictionary<AiAnnotation,(Border Card,double Height,AnnotationCalloutPlacement Placement)>(ReferenceEqualityComparer.Instance);var requests=new List<AnnotationCalloutRequest>(calloutOrder.Length);
        foreach(var note in calloutOrder)
        {
            var card=new Border{Width=cardWidth,Padding=new Thickness(font*.65,font*.5,font*.65,font*.5),CornerRadius=new CornerRadius(8),Background=new SolidColorBrush(Color.FromArgb(248,255,255,255)),BorderBrush=new SolidColorBrush(Color.FromArgb(145,61,174,242)),BorderThickness=new Thickness(1),Cursor=Cursors.SizeAll,ToolTip="拖动批注气泡",Child=new TextBlock{Text=note.Text,Foreground=new SolidColorBrush(Color.FromRgb(35,48,70)),FontSize=font,TextWrapping=TextWrapping.Wrap,LineHeight=font*1.3},Effect=new DropShadowEffect{Color=Color.FromRgb(51,71,98),BlurRadius=16,ShadowDepth=4,Opacity=.28}};
            card.Measure(new Size(cardWidth,Math.Max(40,h)));var cardHeight=Math.Min(Math.Max(font*3.2,card.DesiredSize.Height),Math.Max(1,h-10));card.Height=cardHeight;requests.Add(new AnnotationCalloutRequest(calloutTargets[note],new Size(cardWidth,cardHeight)));calloutCards[note]=(card,cardHeight,default);
        }
        var planned=AnnotationLayoutService.PlanCallouts(requests,new Size(w,h));for(var index=0;index<calloutOrder.Length;index++){var value=calloutCards[calloutOrder[index]];calloutCards[calloutOrder[index]]=(value.Card,value.Height,planned[index]);}
        if((item.VideoPath is null?item.Image.Source:item.Video.Source) is BitmapSource mosaicSource)
        {
            var mosaics=new List<(VideoAnnotationKeyframe Frame,Int32Rect Region)>();
            foreach(var note in item.AnnotationNotes.Where(note=>note.Kind==AiAnnotationKind.Mosaic).Take(16))
            {
                var frame=new VideoAnnotationKeyframe(videoTime??0,note.X,note.Y,note.Width,note.Height);if(note.IsVideoTimeline&&(!videoTime.HasValue||!VideoAnnotationTimeline.TryInterpolateForPresentation(note,videoTime.Value,VideoAnnotationTimeline.LiveFrameToleranceSeconds,out frame)))continue;
                var left=Math.Clamp((int)Math.Floor(frame.X*mosaicSource.PixelWidth),0,mosaicSource.PixelWidth-1);var top=Math.Clamp((int)Math.Floor(frame.Y*mosaicSource.PixelHeight),0,mosaicSource.PixelHeight-1);var right=Math.Clamp((int)Math.Ceiling((frame.X+frame.Width)*mosaicSource.PixelWidth),left+1,mosaicSource.PixelWidth);var bottom=Math.Clamp((int)Math.Ceiling((frame.Y+frame.Height)*mosaicSource.PixelHeight),top+1,mosaicSource.PixelHeight);mosaics.Add((frame,new Int32Rect(left,top,right-left,bottom-top)));
            }
            if(mosaics.Count>0)
            {
                var pixelated=ImagePixelationService.PixelateMany(mosaicSource,mosaics.Select(mosaic=>mosaic.Region).ToArray(),Math.Clamp((int)Math.Round(12*Math.Max(mosaicSource.PixelWidth/Math.Max(1,w),mosaicSource.PixelHeight/Math.Max(1,h))),6,40));
                foreach(var mosaic in mosaics){var crop=new CroppedBitmap(pixelated,mosaic.Region);crop.Freeze();var image=new Image{Source=crop,Width=mosaic.Frame.Width*w,Height=mosaic.Frame.Height*h,Stretch=Stretch.Fill,IsHitTestVisible=false};Canvas.SetLeft(image,mosaic.Frame.X*w);Canvas.SetTop(image,mosaic.Frame.Y*h);item.AiAnnotations.Children.Add(image);}
            }
        }
        foreach(var n in item.AnnotationNotes.Take(48))
        {
            var frame=new VideoAnnotationKeyframe(videoTime??0,n.X,n.Y,n.Width,n.Height);
            if(n.IsVideoTimeline&&(!videoTime.HasValue||!VideoAnnotationTimeline.TryInterpolateForPresentation(n,videoTime.Value,VideoAnnotationTimeline.LiveFrameToleranceSeconds,out frame)))continue;
            if(n.Kind==AiAnnotationKind.Mosaic)continue;
            if(!calloutTargets.TryGetValue(n,out var target))continue;
            var x=target.Left;var y=target.Top;var rw=target.Width;var rh=target.Height;var style=n.EffectiveStyle;var targetColor=string.Equals(style.Color,"#2AAEFF",StringComparison.OrdinalIgnoreCase)?Color.FromRgb(255,0,0):(Color)ColorConverter.ConvertFromString(style.Color);var targetOutline=new Rectangle{Width=rw,Height=rh,Stroke=new SolidColorBrush(targetColor),StrokeThickness=Math.Max(1,style.StrokeWidth*Math.Min(w,h)),RadiusX=3,RadiusY=3,IsHitTestVisible=false};Canvas.SetLeft(targetOutline,x);Canvas.SetTop(targetOutline,y);item.AiAnnotations.Children.Add(targetOutline);
            var line=new Line{Stroke=Cyan,StrokeThickness=Math.Max(1,w/1200),IsHitTestVisible=false};var dot=new Ellipse{Width=5,Height=5,Fill=Cyan,IsHitTestVisible=false};var view=calloutCards[n];var card=view.Card;var cardHeight=view.Height;var cardX=view.Placement.CardBounds.Left;var cardY=view.Placement.CardBounds.Top;if(item.AnnotationCardPositions.TryGetValue(n,out var saved)){cardX=Math.Clamp(saved.X*w,5,Math.Max(5,w-cardWidth-5));cardY=Math.Clamp(saved.Y*h,5,Math.Max(5,h-cardHeight-5));}
            void PositionCard(double left,double top)
            {
                left=Math.Clamp(left,5,Math.Max(5,w-cardWidth-5));top=Math.Clamp(top,5,Math.Max(5,h-cardHeight-5));Canvas.SetLeft(card,left);Canvas.SetTop(card,top);var cardBounds=new Rect(left,top,cardWidth,cardHeight);var connector=AnnotationLayoutService.FindConnector(target,cardBounds);var targetPoint=new Point(connector.X<=target.Left?target.Left:connector.X>=target.Right?target.Right:Math.Clamp(connector.X,target.Left,target.Right),connector.Y<=target.Top?target.Top:connector.Y>=target.Bottom?target.Bottom:Math.Clamp(connector.Y,target.Top,target.Bottom));line.X1=targetPoint.X;line.Y1=targetPoint.Y;line.X2=connector.X;line.Y2=connector.Y;Canvas.SetLeft(dot,connector.X-2.5);Canvas.SetTop(dot,connector.Y-2.5);
            }
            Point dragStart=default,cardStart=default;var dragging=false;
            card.PreviewMouseLeftButtonDown+=(_,args)=>{dragging=true;dragStart=args.GetPosition(item.AiAnnotations);cardStart=new Point(Canvas.GetLeft(card),Canvas.GetTop(card));card.CaptureMouse();args.Handled=true;};
            card.PreviewMouseMove+=(_,args)=>{if(!dragging||!card.IsMouseCaptured||args.LeftButton!=MouseButtonState.Pressed)return;var point=args.GetPosition(item.AiAnnotations);PositionCard(cardStart.X+point.X-dragStart.X,cardStart.Y+point.Y-dragStart.Y);args.Handled=true;};
            card.PreviewMouseLeftButtonUp+=(_,args)=>{if(!dragging)return;dragging=false;if(card.IsMouseCaptured)card.ReleaseMouseCapture();item.AnnotationCardPositions[n]=new Point(Canvas.GetLeft(card)/Math.Max(1,w),Canvas.GetTop(card)/Math.Max(1,h));args.Handled=true;};
            card.LostMouseCapture+=(_,_)=>{if(!dragging)return;dragging=false;item.AnnotationCardPositions[n]=new Point(Canvas.GetLeft(card)/Math.Max(1,w),Canvas.GetTop(card)/Math.Max(1,h));};
            item.AiAnnotations.Children.Add(line);item.AiAnnotations.Children.Add(dot);item.AiAnnotations.Children.Add(card);PositionCard(cardX,cardY);
        }
    }

    private async Task PlayVideoAnnotationsAsync(SelectionItem item,AiAnnotation primary)
    {
        CrashDiagnosticsService.MarkOperation("屏幕助手：视频时间轴标注播放");
        CancelVideoAnnotationPlayback(item);
        var playback=new CancellationTokenSource();item.VideoAnnotationPlayback=playback;
        Action<TimeSpan>? frameHandler=null;Action? endedHandler=null;
        try
        {
            var preview=EnsureVideoPreview(item);var start=TimeSpan.FromSeconds(primary.StartTime!.Value);var end=TimeSpan.FromSeconds(primary.EndTime!.Value);
            var presentedStart=await preview.SeekAsync(start,pauseAfterSeek:true,playback.Token);
            if(_closed||!ReferenceEquals(item.VideoAnnotationPlayback,playback)||!_selections.Contains(item))return;
            RenderAnnotationsForItem(item,presentedStart.TotalSeconds);item.VideoPlaying=false;
            if(end<=start){PromptStatus.Text=$"已定位到 {FormatVideoTime(start)} · 视频已暂停";return;}
            var completed=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            frameHandler=position=>
            {
                if(!ReferenceEquals(item.VideoAnnotationPlayback,playback)||playback.IsCancellationRequested)return;
                if(position+TimeSpan.FromSeconds(.05)<end)return;
                preview.Pause();item.VideoPlaying=false;completed.TrySetResult();
            };
            endedHandler=()=>{if(!ReferenceEquals(item.VideoAnnotationPlayback,playback))return;preview.Pause();item.VideoPlaying=false;completed.TrySetResult();};
            preview.FramePresented+=frameHandler;preview.Ended+=endedHandler;preview.Play();item.VideoPlaying=true;
            PromptStatus.Text=$"正在播放 {FormatVideoTime(start)}–{FormatVideoTime(end)} 的标注过程";
            await completed.Task.WaitAsync(playback.Token);
            if(frameHandler is not null){preview.FramePresented-=frameHandler;frameHandler=null;}if(endedHandler is not null){preview.Ended-=endedHandler;endedHandler=null;}
            var presentedEnd=await preview.SeekAsync(end,pauseAfterSeek:true,playback.Token);RenderAnnotationsForItem(item,presentedEnd.TotalSeconds);
            if(ReferenceEquals(item.VideoAnnotationPlayback,playback))PromptStatus.Text=$"已播放至 {FormatVideoTime(presentedEnd)} · 视频已暂停";
        }
        catch(OperationCanceledException){}
        catch(Exception ex){new PrivacyLogger().Error("VideoAnnotationPlayback",ex);if(!_closed&&ReferenceEquals(item.VideoAnnotationPlayback,playback))PromptStatus.Text=$"视频定位失败：{ex.Message}";}
        finally
        {
            if(frameHandler is not null&&item.VideoPreview is { } preview)preview.FramePresented-=frameHandler;
            if(endedHandler is not null&&item.VideoPreview is { } endedPreview)endedPreview.Ended-=endedHandler;
            if(ReferenceEquals(item.VideoAnnotationPlayback,playback))item.VideoAnnotationPlayback=null;
            playback.Dispose();
            if(!_closed)CrashDiagnosticsService.MarkOperation("屏幕助手：等待操作");
        }
    }

    private static string FormatVideoTime(TimeSpan value)=>value.ToString(value.TotalHours>=1?@"hh\:mm\:ss\.f":@"mm\:ss\.f");
    private static void CancelVideoAnnotationPlayback(SelectionItem item)
    {
        var playback=item.VideoAnnotationPlayback;item.VideoAnnotationPlayback=null;
        if(playback is null)return;
        try{playback.Cancel();}catch{}
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
        RemoveEmptyDrawingText(item);var includeAnnotations=AskWhetherToIncludeAnnotations(item);if(!includeAnnotations.HasValue)return;
        if(item.VideoPath is { } video)
        {
            var dialog=new SaveFileDialog{Filter=LocalizationService.T("MP4 视频|*.mp4|GIF 动图|*.gif","MP4 video|*.mp4|Animated GIF|*.gif"),DefaultExt=".mp4",FilterIndex=1,AddExtension=true,FileName=ExportFileNameService.Recording(DateTime.Now)};
            if(ShowSystemFileDialog(dialog)!=true)return;
            var exportGif=dialog.FilterIndex==2;
            var destination=System.IO.Path.ChangeExtension(dialog.FileName,exportGif?".gif":".mp4");
            var pixels=ToPixelRect(item.Bounds);var manualOverlay=includeAnnotations.Value&&HasManualAnnotations(item)?RenderManualOverlay(item,pixels.Width,pixels.Height):null;var annotations=includeAnnotations.Value?item.AnnotationNotes.ToArray():[];var operation=BeginOverlayOperation(exportGif?"正在导出 GIF…按 Esc 可取消":includeAnnotations.Value?"正在合成带标注 MP4…按 Esc 可取消":"正在保存 MP4…按 Esc 可取消");TempMediaLease? exportLease=null;
            try
            {
                exportLease=TempMediaRegistry.Shared.AcquireExistingFile(video);
                if(exportGif)
                {
                    var fps=_host.Settings.GifFps;
                    Func<BitmapSource,TimeSpan,BitmapSource>? compositor=includeAnnotations.Value?(frame,time)=>Dispatcher.Invoke(()=>AnnotationOverlayRenderer.Composite(AnnotationOverlayRenderer.ApplyAiMosaics(frame,annotations,time.TotalSeconds),manualOverlay,AnnotationOverlayRenderer.RenderAiOverlay(frame.PixelWidth,frame.PixelHeight,annotations,time.TotalSeconds,item.AnnotationCardPositions))):null;
                    var result=await GifExportService.ExportFromVideoAsync(video,destination,fps,operation.Token,compositor);
                    if(IsOverlayOperationActive(operation,item))PromptStatus.Text=$"GIF 已保存 · {result.FrameCount} 帧 / {result.EffectiveFps:0.#} FPS";
                }
                else
                {
                    if(includeAnnotations.Value)await AnnotatedVideoExportService.ExportAsync(video,destination,manualOverlay,annotations,operation.Token,item.AnnotationCardPositions);else await Task.Run(()=>AtomicFileService.Copy(video,destination),operation.Token);
                    if(IsOverlayOperationActive(operation,item))PromptStatus.Text=includeAnnotations.Value?"带标注 MP4 已保存":"MP4 原件已保存";
                }
            }
            catch(OperationCanceledException){if(IsOverlayOperationActive(operation,item))PromptStatus.Text="已取消保存视频";}
            catch(Exception ex){new PrivacyLogger().Error("SaveVideo",ex);if(IsOverlayOperationActive(operation,item))PromptStatus.Text=$"保存失败：{ex.Message}";}
            finally{exportLease?.Dispose();EndOverlayOperation(operation);}
            return;
        }

        // Flatten the selected pixels before opening the system picker. The
        // picker is external visual state and must never become source pixels.
        var image=RenderSelectionImage(item,includeAnnotations.Value,includeAnnotations.Value,includeAnnotations.Value);
        var jpeg=_host.Settings.DefaultImageFormat.Equals("jpg",StringComparison.OrdinalIgnoreCase)||_host.Settings.DefaultImageFormat.Equals("jpeg",StringComparison.OrdinalIgnoreCase);var imageDialog=new SaveFileDialog{Filter=LocalizationService.T("PNG 图片|*.png|JPEG 图片|*.jpg;*.jpeg","PNG image|*.png|JPEG image|*.jpg;*.jpeg"),DefaultExt=jpeg?".jpg":".png",FilterIndex=jpeg?2:1,AddExtension=true,FileName=ExportFileNameService.Screenshot(DateTime.Now)};if(ShowSystemFileDialog(imageDialog)!=true)return;
        var imageOperation=BeginOverlayOperation("正在保存图片…按 Esc 可取消");
        try{await Task.Run(()=>ScreenCaptureService.Save(image,imageDialog.FileName,imageDialog.FilterIndex==2),imageOperation.Token);if(IsOverlayOperationActive(imageOperation,item))PromptStatus.Text="图片已保存";}
        catch(OperationCanceledException){if(IsOverlayOperationActive(imageOperation,item))PromptStatus.Text="已取消保存图片";}
        catch(Exception ex){new PrivacyLogger().Error("SaveImage",ex);if(IsOverlayOperationActive(imageOperation,item))PromptStatus.Text=$"图片保存失败：{ex.Message}";}
        finally{EndOverlayOperation(imageOperation);}
    }
    private async void Pin(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy()||Active is not { } item)return;RemoveEmptyDrawingText(item);var pixels=ToPixelRect(item.Bounds);var region=ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY);CancellationTokenSource? operation=null;TempMediaLease? generatedVideoLease=null;
        try
        {
            if(item.VideoPath is { } video)
            {
                if(HasAnyAnnotations(item))
                {
                    operation=BeginOverlayOperation("正在生成带标注贴视频…按 Esc 可取消");var annotatedPath=new TempFileService().NewFile(".mp4");generatedVideoLease=TempMediaRegistry.Shared.Acquire(annotatedPath);var manualOverlay=HasManualAnnotations(item)?RenderManualOverlay(item,pixels.Width,pixels.Height):null;
                    await AnnotatedVideoExportService.ExportAsync(video,annotatedPath,manualOverlay,item.AnnotationNotes.ToArray(),operation.Token,item.AnnotationCardPositions);
                    if(!IsOverlayOperationActive(operation,item))return;video=annotatedPath;
                }
                var window=new PinnedVideoWindow(video,region);try{window.Show();}catch{window.Close();throw;}
            }
            else new PinnedImageWindow(RenderSelectionImage(item,true,true,true),region).Show();
            // Capture the protected pin into the frozen desktop frame, then
            // explicitly keep the live pin above the capture controls.
            RefreshDesktopFrameIncludingPinnedWindows();
            RestoreOverlayKeyboardFocusAfterPin();
            RaisePinnedWindowsAboveOverlay();
            PromptStatus.Text=item.VideoPath is null?(HasAnyAnnotations(item)?"已在原位贴出带标注图片":"已在原位贴图"):(HasAnyAnnotations(item)?"已在原位贴出带标注视频":"已在原位贴视频");SetPromptBarHidden(false);
        }
        catch(OperationCanceledException){if(!_closed&&operation is not null&&ReferenceEquals(_overlayRequest,operation))PromptStatus.Text="已取消生成贴图";}
        catch(Exception ex){new PrivacyLogger().Error("PinMedia",ex);PromptStatus.Text=$"贴图失败：{ex.Message}";}
        finally{generatedVideoLease?.Dispose();if(operation is not null)EndOverlayOperation(operation);}
    }
    private async void CaptureLongScreenshot(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy()||_longCaptureMode||Active is not {IsImplicit:false,VideoPath:null,CapturedImageOverride:null} item)return;
        if(!_captureExclusionVerified){PromptStatus.Text="覆盖层防捕获不可用，无法安全生成长截图";return;}
        _longCaptureBefore=CaptureOverlaySnapshot();_longCaptureItem=item;_longCaptureMode=true;_longCaptureFrames.Clear();_longCaptureComposite=null;_longCaptureScrollTarget=IntPtr.Zero;_longCaptureSampleVersion=0;
        try
        {
            Toolbar.Visibility=SizeText.Visibility=PointerInspector.Visibility=PromptBarHost.Visibility=Visibility.Collapsed;HideHandles();SnapPreview.Visibility=Visibility.Collapsed;BeginLongCaptureLiveRegion(item);LongCaptureBar.Visibility=Visibility.Visible;PositionFloatingBar(LongCaptureBar,item);Cursor=Cursors.Arrow;
            await Dispatcher.InvokeAsync(()=>{},DispatcherPriority.Render);await Task.Delay(100);
            var pixels=ToPixelRect(item.Bounds);var screen=ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY);var centerX=screen.X+screen.Width/2;var centerY=screen.Y+screen.Height/2;var handle=new WindowInteropHelper(this).Handle;_longCaptureScrollTarget=_windowSnap.FindTopmostTargetAt(centerX,centerY,handle)?.Handle??IntPtr.Zero;
            var live=new ScreenCaptureService().CaptureDesktop();var frame=ScreenCaptureService.Crop(live.Image,pixels);_longCaptureFrames.Add(frame);_longCaptureComposite=frame;UpdateLongCapturePreview(item,frame);LongCaptureProgressText.Text="已采集 1 段";PromptStatus.Text="滚轮向下控制截取长度 · 已截内容会在旁边向上拼接";Root.Focus();
        }
        catch(Exception ex)
        {
            new PrivacyLogger().Error("LongScreenshotStart",ex);CancelLongCaptureSession($"长截图启动失败：{ex.Message}");
        }
    }

    private void BeginLongCaptureLiveRegion(SelectionItem item)
    {
        var full=new RectangleGeometry(new Rect(0,0,Math.Max(0,Root.ActualWidth),Math.Max(0,Root.ActualHeight)));var hole=new RectangleGeometry(Normalize(item.Bounds));DesktopImage.Clip=new CombinedGeometry(GeometryCombineMode.Exclude,full,hole);Dimmer.Clip=new CombinedGeometry(GeometryCombineMode.Exclude,full,hole);item.Image.Visibility=item.Video.Visibility=item.Markup.Visibility=item.TextOverlays.Visibility=item.AiAnnotations.Visibility=item.TextSelection.Visibility=Visibility.Collapsed;
    }

    private void OnPreviewMouseWheel(object sender,MouseWheelEventArgs e)
    {
        if(!_longCaptureMode||_longCaptureItem is not { } item)return;
        if(IsInside(e.OriginalSource as DependencyObject,LongCaptureBar))return;
        var point=e.GetPosition(Root);e.Handled=true;
        if(!item.Bounds.Contains(point)){PromptStatus.Text="请把鼠标放在长截图区域内滚动";return;}
        var screen=ScreenCoordinateService.ToScreenPixelPoint(point,Root.ActualWidth,Root.ActualHeight,_frame.Image.PixelWidth,_frame.Image.PixelHeight,_frame.OriginX,_frame.OriginY);var handle=new WindowInteropHelper(this).Handle;var target=_windowSnap.FindTopmostTargetAt(screen.X,screen.Y,handle)?.Handle??_longCaptureScrollTarget;if(target!=IntPtr.Zero)_longCaptureScrollTarget=target;
        if(!MouseWheelInputService.Scroll(_longCaptureScrollTarget,screen.X,screen.Y,e.Delta)){PromptStatus.Text="当前区域没有响应滚轮，请把鼠标移到实际可滚动内容上";return;}
        if(e.Delta>=0){PromptStatus.Text="已向上滚动；继续向下滚动才会追加长图";return;}
        ScheduleLongCaptureSample();PromptStatus.Text="正在等待页面滚动完成…";
    }

    private void ScheduleLongCaptureSample()
    {
        var previous=Interlocked.Exchange(ref _longCaptureSampleRequest,new CancellationTokenSource());if(previous is not null){try{previous.Cancel();}catch(ObjectDisposedException){}previous.Dispose();}var request=_longCaptureSampleRequest!;var version=++_longCaptureSampleVersion;_longCaptureSampleTask=CaptureLongFrameAfterScrollAsync(request,version);
    }

    private async Task CaptureLongFrameAfterScrollAsync(CancellationTokenSource request,int version)
    {
        try
        {
            await Task.Delay(280,request.Token);if(!_longCaptureMode||_longCaptureItem is not { } item||!ReferenceEquals(request,_longCaptureSampleRequest)||version!=_longCaptureSampleVersion)return;
            var pixels=ToPixelRect(item.Bounds);var live=new ScreenCaptureService().CaptureDesktop();var frame=ScreenCaptureService.Crop(live.Image,pixels);var previous=_longCaptureFrames[^1];var shift=await Task.Run(()=>ScrollingCaptureComposer.EstimateVerticalShift(previous,frame),request.Token);if(shift<=0){if(_longCaptureMode)PromptStatus.Text="没有检测到新的滚动内容；可能已到底或滚动过快";return;}
            if(_longCaptureFrames.Count>=24){PromptStatus.Text="已达到 24 段安全上限，请点击完成";return;}
            _longCaptureFrames.Add(frame);var frames=_longCaptureFrames.ToArray();var composite=await Task.Run(()=>ScrollingCaptureComposer.Compose(frames),request.Token);if(!_longCaptureMode||!ReferenceEquals(request,_longCaptureSampleRequest)||version!=_longCaptureSampleVersion)return;
            _longCaptureComposite=composite;UpdateLongCapturePreview(item,composite);LongCaptureProgressText.Text=$"已采集 {_longCaptureFrames.Count} 段 · {composite.PixelHeight}px";PromptStatus.Text=$"长图已追加 {shift}px · 继续滚动或点击完成";if((long)composite.PixelWidth*composite.PixelHeight>=ScrollingCaptureComposer.MaxOutputPixels)PromptStatus.Text="已达到长图像素安全上限，请点击完成";
        }
        catch(OperationCanceledException){}
        catch(Exception ex){new PrivacyLogger().Error("LongScreenshotSample",ex);if(_longCaptureMode)PromptStatus.Text=$"本次滚动未能拼接：{ex.Message}";}
    }

    private void UpdateLongCapturePreview(SelectionItem item,BitmapSource composite)
    {
        var monitor=MonitorBounds(item.Bounds);var width=Math.Clamp(Math.Min(220,Math.Max(120,item.Bounds.Width*.34)),96,Math.Max(96,monitor.Width*.28));var height=Math.Min(monitor.Height*.68,width*composite.PixelHeight/Math.Max(1d,composite.PixelWidth));var placeLeft=item.Bounds.Left-monitor.Left>=width+12;var left=placeLeft?item.Bounds.Left-width-10:item.Bounds.Right+10;if(!placeLeft&&left+width>monitor.Right)left=monitor.Right-width-6;var bottom=Math.Min(monitor.Bottom-6,item.Bounds.Bottom);var top=Math.Max(monitor.Top+6,bottom-height);LongCapturePreviewImage.Source=composite;LongCapturePreviewHost.Width=width;LongCapturePreviewHost.Height=Math.Max(48,bottom-top);Canvas.SetLeft(LongCapturePreviewHost,Math.Clamp(left,monitor.Left+4,Math.Max(monitor.Left+4,monitor.Right-width-4)));Canvas.SetTop(LongCapturePreviewHost,top);LongCapturePreviewHost.Visibility=Visibility.Visible;
    }

    private async void FinishLongCapture(object sender,RoutedEventArgs e)
    {
        if(!_longCaptureMode||_longCaptureItem is not { } item)return;var pending=_longCaptureSampleTask;if(pending is not null&&!pending.IsCompleted)await Task.WhenAny(pending,Task.Delay(650));CancelLongCaptureSample();var result=_longCaptureComposite??_longCaptureFrames.FirstOrDefault();if(result is null){CancelLongCaptureSession("没有采集到可用画面");return;}var before=_longCaptureBefore;var original=item.Bounds;var monitor=MonitorBounds(original);ClearImageOnlyLayers(item);item.CapturedImageOverride=result;var desiredHeight=original.Width*result.PixelHeight/Math.Max(1d,result.PixelWidth);var height=Math.Clamp(desiredHeight,Math.Min(original.Height,monitor.Height),Math.Max(Math.Min(original.Height,monitor.Height),monitor.Height-8));var bottom=Math.Min(original.Bottom,monitor.Bottom-4);var top=Math.Max(monitor.Top+4,bottom-height);item.Bounds=new Rect(Math.Clamp(original.Left,monitor.Left+4,Math.Max(monitor.Left+4,monitor.Right-original.Width-4)),top,original.Width,Math.Max(1,bottom-top));EndLongCaptureSession(item);UpdateSelection(item);if(before is not null)RecordOverlayOperation(before,"滚动长截图");PromptStatus.Text=$"长截图完成 · {result.PixelWidth} × {result.PixelHeight} · 已留在原位";
    }

    private void CancelLongCapture(object sender,RoutedEventArgs e)=>CancelLongCaptureSession("已取消长截图");

    private void CancelLongCaptureSession(string status)
    {
        var item=_longCaptureItem;CancelLongCaptureSample();if(item is not null)EndLongCaptureSession(item);else ResetLongCaptureState();PromptStatus.Text=status;
    }

    private void EndLongCaptureSession(SelectionItem item)
    {
        DesktopImage.Clip=null;Dimmer.Clip=null;LongCaptureBar.Visibility=LongCapturePreviewHost.Visibility=Visibility.Collapsed;LongCapturePreviewImage.Source=null;item.Image.Visibility=Visibility.Visible;item.Video.Visibility=item.VideoPath is null?Visibility.Collapsed:Visibility.Visible;item.Markup.Visibility=item.TextOverlays.Visibility=item.AiAnnotations.Visibility=Visibility.Visible;ApplyTextLayerState(item);ResetLongCaptureState();Cursor=Cursors.Cross;PromptBarHost.Visibility=_conversationAiAvailable?Visibility.Visible:Visibility.Collapsed;if(_selections.Contains(item)){var index=_selections.IndexOf(item);if(index>=0)Select(index);ShowToolbar();PositionPromptBar();SetPromptBarHidden(false);}if(IsActive&&!_closed)Root.Focus();
    }

    private void ResetLongCaptureState()
    {
        _longCaptureMode=false;_longCaptureItem=null;_longCaptureBefore=null;_longCaptureFrames.Clear();_longCaptureComposite=null;_longCaptureScrollTarget=IntPtr.Zero;_longCaptureSampleTask=null;
    }

    private void CancelLongCaptureSample()
    {
        var request=Interlocked.Exchange(ref _longCaptureSampleRequest,null);if(request is null)return;try{request.Cancel();}catch(ObjectDisposedException){}request.Dispose();
    }
    private void Draw(object s,RoutedEventArgs e)=>EnterDrawingMode();
    private void EnterDrawingMode()
    {
        if(RejectIfOverlayOperationBusy()||Active is not {IsImplicit:false} item)return;_drawingOperationBefore=CaptureOverlaySnapshot();_drawingOperationChanged=false;_drawingMode=true;Toolbar.Visibility=Visibility.Collapsed;HideHandles();SizeText.Visibility=PointerInspector.Visibility=Visibility.Collapsed;item.Markup.Visibility=Visibility.Visible;item.Markup.IsHitTestVisible=true;EnsureDrawingControls();ApplyCurrentDrawingAttributes(item);SetDrawTool(DrawTool.Freehand);DrawingToolbar.Visibility=Visibility.Visible;PositionFloatingBar(DrawingToolbar,item);SetPromptBarHidden(true);PromptStatus.Text=item.VideoPath is null?"原位标注中 · 颜色统一作用于画笔、形状、文字和序号":"视频原位标注中 · 手工标注将贯穿整个视频";
    }
    private void ExitDrawingMode()
    {
        if(Active is { } item)
        {
            // InkCanvas can keep capture on an internal child rather than on
            // the canvas itself. Release the whole capture subtree before it
            // stops receiving input, otherwise the overlay can feel frozen.
            if(item.Markup.IsMouseCaptureWithin)Mouse.Capture(null);
            _drawPreview=null;
            ClearDrawingObjectSelection();
            Keyboard.ClearFocus();
            RemoveEmptyDrawingText(item);
            item.Markup.IsHitTestVisible=false;
        }
        _drawingMode=false;DrawingToolbar.Visibility=Visibility.Collapsed;Cursor=Cursors.Cross;SetPromptBarHidden(false);if(Active is not null){UpdateSelection(Active);ShowToolbar();}if(_drawingOperationChanged&&_drawingOperationBefore is { } before)RecordOverlayOperation(before,"原位标注");_drawingOperationBefore=null;_drawingOperationChanged=false;PromptStatus.Text="标注已保留在当前区域";
        // ClearFocus is needed to commit an active annotation TextBox, but it
        // also leaves PreviewKeyDown without a route. Give focus back to the
        // full-screen root so the next Esc can always close the overlay.
        if(IsActive&&!_closed)Root.Focus();
    }
    private void FinishInterruptedDrawingMode()
    {
        ExitDrawingMode();
    }
    private void SetDrawTool(DrawTool tool)
    {
        if(Active is not { } item)return;ClearDrawingObjectSelection();_drawTool=tool;item.Markup.EditingMode=tool==DrawTool.Freehand?InkCanvasEditingMode.Ink:InkCanvasEditingMode.None;Cursor=tool switch{DrawTool.Freehand=>Cursors.Pen,DrawTool.Select=>Cursors.Arrow,_=>Cursors.Cross};DrawingTextControls.Visibility=tool==DrawTool.Text?Visibility.Visible:Visibility.Collapsed;UpdateDrawingToolVisualState(tool);PositionFloatingBar(DrawingToolbar,item);_=Dispatcher.BeginInvoke(DispatcherPriority.Render,new Action(()=>{if(_drawingMode&&DrawingToolbar.Visibility==Visibility.Visible&&Active is { } active)PositionFloatingBar(DrawingToolbar,active);}));
    }
    private void UpdateDrawingToolVisualState(DrawTool tool)
    {
        var active=tool switch{DrawTool.Freehand when _drawHighlighter=>DrawingHighlightButton,DrawTool.Freehand=>DrawingPenButton,DrawTool.Rectangle=>DrawingRectangleButton,DrawTool.Ellipse=>DrawingEllipseButton,DrawTool.Arrow=>DrawingArrowButton,DrawTool.Mosaic=>DrawingMosaicButton,DrawTool.Text=>DrawingTextButton,DrawTool.Number=>DrawingNumberButton,DrawTool.Select=>DrawingSelectButton,DrawTool.Eraser=>DrawingEraserButton,_=>DrawingPenButton};
        foreach(var button in new[]{DrawingSelectButton,DrawingPenButton,DrawingHighlightButton,DrawingRectangleButton,DrawingEllipseButton,DrawingArrowButton,DrawingMosaicButton,DrawingTextButton,DrawingNumberButton,DrawingEraserButton})button.Style=(Style)FindResource(ReferenceEquals(button,active)?"ReferenceIconButton":"ToolbarIconButton");
    }
    private static DrawingAttributes RegularDrawingAttributes(Color color)=>new(){Color=color,Width=4,Height=4,IsHighlighter=false,FitToCurve=true};
    private static DrawingAttributes HighlightDrawingAttributes(Color color)=>new(){Color=color,Width=18,Height=18,IsHighlighter=true,FitToCurve=true};
    private void ApplyCurrentDrawingAttributes(SelectionItem item){item.Markup.DefaultDrawingAttributes=_drawHighlighter?HighlightDrawingAttributes(_drawColor):RegularDrawingAttributes(_drawColor);if(DrawColorSwatch is not null)DrawColorSwatch.Fill=new SolidColorBrush(_drawColor);}
    private void DrawPen(object s,RoutedEventArgs e){if(Active is { } item){_drawHighlighter=false;ApplyCurrentDrawingAttributes(item);SetDrawTool(DrawTool.Freehand);}}
    private void SetShapeTool(DrawTool tool){if(Active is not { } item)return;_drawHighlighter=false;ApplyCurrentDrawingAttributes(item);SetDrawTool(tool);}
    private void DrawRectangleTool(object s,RoutedEventArgs e)=>SetShapeTool(DrawTool.Rectangle);
    private void DrawEllipseTool(object s,RoutedEventArgs e)=>SetShapeTool(DrawTool.Ellipse);
    private void DrawArrowTool(object s,RoutedEventArgs e)=>SetShapeTool(DrawTool.Arrow);
    private void DrawMosaicTool(object s,RoutedEventArgs e){SetShapeTool(DrawTool.Mosaic);PromptStatus.Text="拖动绘制矩形马赛克 · 可撤销或重做";}
    private void DrawTextTool(object s,RoutedEventArgs e){_drawHighlighter=false;if(Active is { } item)ApplyCurrentDrawingAttributes(item);SetDrawTool(DrawTool.Text);PromptStatus.Text="点击截图放置文本框 · 可选系统字体、字号和荧光底色";}
    private void DrawNumberTool(object s,RoutedEventArgs e){_drawHighlighter=false;if(Active is { } item)ApplyCurrentDrawingAttributes(item);SetDrawTool(DrawTool.Number);PromptStatus.Text="点击截图依次放置实心序号";}
    private void DrawSelect(object s,RoutedEventArgs e){SetDrawTool(DrawTool.Select);Keyboard.ClearFocus();PromptStatus.Text="点击选择标注 · 拖动移动 · Delete 删除";}
    private void SetDrawColor(Color color){_drawColor=color;if(Active is { } item){ApplyCurrentDrawingAttributes(item);UpdateFocusedDrawingTextColor(item);}}
    private void UpdateFocusedDrawingTextColor(SelectionItem item)
    {
        if(Keyboard.FocusedElement is not TextBox box||box.Tag is not Guid id)return;var index=item.DrawingElements.FindIndex(element=>element.Id==id);if(index<0||item.DrawingElements[index] is not TextDrawingElement text)return;var updated=text with{Color=_drawColor};item.DrawingElements[index]=updated;box.Foreground=box.CaretBrush=new SolidColorBrush(_drawColor);box.Background=updated.Highlight?new SolidColorBrush(Color.FromArgb(150,255,237,105)):Brushes.Transparent;_drawingOperationChanged=true;
    }
    private void DrawRed(object s,RoutedEventArgs e)=>SetDrawColor(Colors.Red);
    private void DrawBlue(object s,RoutedEventArgs e)=>SetDrawColor(Color.FromRgb(49,140,255));
    private void DrawChooseColor(object s,RoutedEventArgs e)
    {
        _drawingModalOpen=true;try{if(!MewuColorDialog.TryChoose(this,_drawColor,out var selected))return;SetDrawColor(selected);PromptStatus.Text=$"当前标注颜色 RGB({_drawColor.R}, {_drawColor.G}, {_drawColor.B})";}finally{_drawingModalOpen=false;if(!_closed)Activate();}
    }
    private void DrawHighlight(object s,RoutedEventArgs e){if(Active is { } item){_drawHighlighter=true;ApplyCurrentDrawingAttributes(item);SetDrawTool(DrawTool.Freehand);}}
    private void ToggleTextHighlight(object s,RoutedEventArgs e){_drawTextHighlight=!_drawTextHighlight;TextHighlightButton.Background=new SolidColorBrush(_drawTextHighlight?Color.FromRgb(255,236,139):Colors.Transparent);PromptStatus.Text=_drawTextHighlight?"新文字使用荧光底色":"新文字使用普通颜色";}
    private void DrawEraser(object s,RoutedEventArgs e){if(Active is not null){SetDrawTool(DrawTool.Eraser);PromptStatus.Text="拖过标注即可擦除 · 支持笔迹、形状、文字、序号和马赛克";}}
    private void DrawUndo(object s,RoutedEventArgs e)
    {
        if(Active is not { } item)return;ClearDrawingObjectSelection();
        while(item.DrawingOrder.Count>0)
        {
            var action=item.DrawingOrder[^1];item.DrawingOrder.RemoveAt(item.DrawingOrder.Count-1);
            if(action is StrokeDrawingAction strokeAction&&item.Markup.Strokes.Contains(strokeAction.Stroke)){item.Markup.Strokes.Remove(strokeAction.Stroke);item.DrawingRedo.Push(action);_drawingOperationChanged=true;return;}
            if(action is ElementDrawingAction elementAction)
            {
                var current=item.DrawingElements.FirstOrDefault(element=>element.Id==elementAction.Element.Id);if(current is null)continue;item.DrawingElements.Remove(current);item.DrawingRedo.Push(new ElementDrawingAction(current));RebuildDrawingElements(item);_drawingOperationChanged=true;return;
            }
            if(action is StrokeRemovalDrawingAction removedStroke){if(item.Markup.Strokes.Contains(removedStroke.Stroke))continue;AddStrokeWithoutHistory(item,removedStroke.Stroke);item.DrawingRedo.Push(action);_drawingOperationChanged=true;return;}
            if(action is ElementRemovalDrawingAction removedElement){if(item.DrawingElements.Any(element=>element.Id==removedElement.Element.Id))continue;item.DrawingElements.Add(removedElement.Element);RebuildDrawingElements(item);item.DrawingRedo.Push(action);_drawingOperationChanged=true;return;}
            if(action is StrokeMoveDrawingAction movedStroke&&item.Markup.Strokes.Contains(movedStroke.Stroke)){ApplyStrokeState(movedStroke.Stroke,movedStroke.Before);item.DrawingRedo.Push(action);_drawingOperationChanged=true;return;}
            if(action is ElementMoveDrawingAction movedElement&&ReplaceDrawingElement(item,movedElement.Before)){RebuildDrawingElements(item);item.DrawingRedo.Push(action);_drawingOperationChanged=true;return;}
        }
    }
    private void DrawRedo(object s,RoutedEventArgs e)
    {
        if(Active is not { } item||!item.DrawingRedo.TryPop(out var action))return;ClearDrawingObjectSelection();
        if(action is StrokeDrawingAction strokeAction)AddStrokeWithoutHistory(item,strokeAction.Stroke);
        else if(action is ElementDrawingAction elementAction){item.DrawingElements.Add(elementAction.Element);RebuildDrawingElements(item);}
        else if(action is StrokeRemovalDrawingAction removedStroke)item.Markup.Strokes.Remove(removedStroke.Stroke);
        else if(action is ElementRemovalDrawingAction removedElement){item.DrawingElements.RemoveAll(element=>element.Id==removedElement.Element.Id);RebuildDrawingElements(item);}
        else if(action is StrokeMoveDrawingAction movedStroke){ApplyStrokeState(movedStroke.Stroke,movedStroke.After);}
        else if(action is ElementMoveDrawingAction movedElement){ReplaceDrawingElement(item,movedElement.After);RebuildDrawingElements(item);}
        item.DrawingOrder.Add(action);_drawingOperationChanged=true;
    }
    private void DrawClear(object s,RoutedEventArgs e){if(Active is { } item&&(item.Markup.Strokes.Count>0||item.DrawingElements.Count>0)){ClearDrawingObjectSelection();item.Markup.Strokes.Clear();item.Markup.Children.Clear();item.DrawingElements.Clear();item.DrawingOrder.Clear();item.DrawingRedo.Clear();item.NextDrawingNumber=1;_drawingOperationChanged=true;}}
    private void DrawDone(object s,RoutedEventArgs e)=>ExitDrawingMode();
    private void MarkupDown(object sender,MouseButtonEventArgs e)
    {
        if(!_drawingMode||sender is not InkCanvas canvas||Active is not { } item||!ReferenceEquals(canvas,item.Markup)||_drawTool==DrawTool.Freehand)return;var point=e.GetPosition(canvas);
        if(_drawTool==DrawTool.Eraser){canvas.Focus();Keyboard.Focus(canvas);canvas.CaptureMouse();EraseDrawingObjectsAt(item,point);_lastEraserPoint=point;e.Handled=true;return;}
        if(_drawTool==DrawTool.Select)
        {
            if(e.ClickCount>=2&&TryEditDrawingText(item,point,canvas)){e.Handled=true;return;}
            BeginDrawingObjectSelection(item,point,canvas);e.Handled=true;return;
        }
        if(_drawTool==DrawTool.Text){if(!TryEditDrawingText(item,point,canvas))AddTextDrawingElement(item,point);e.Handled=true;return;}
        if(_drawTool==DrawTool.Number){AddNumberDrawingElement(item,point);e.Handled=true;return;}
        _drawStart=point;canvas.CaptureMouse();e.Handled=true;
    }
    private void MarkupMove(object sender,MouseEventArgs e)
    {
        if(!_drawingMode||sender is not InkCanvas canvas||Active is not { } item||!ReferenceEquals(canvas,item.Markup)||e.LeftButton!=MouseButtonState.Pressed||!canvas.IsMouseCaptured)return;
        if(_drawTool==DrawTool.Eraser){var point=e.GetPosition(canvas);if(_lastEraserPoint is not { } previous||(point-previous).Length>=12){EraseDrawingObjectsAt(item,point);_lastEraserPoint=point;}e.Handled=true;return;}
        if(_drawTool==DrawTool.Select){MoveSelectedDrawingObject(item,e.GetPosition(canvas),canvas);e.Handled=true;return;}
        if(_drawTool is DrawTool.Freehand or DrawTool.Text or DrawTool.Number)return;
        if(_drawPreview is not null)canvas.Strokes.Remove(_drawPreview);_drawPreview=CreateShapeStroke(canvas,_drawStart,e.GetPosition(canvas),_drawTool,Keyboard.Modifiers.HasFlag(ModifierKeys.Shift));canvas.Strokes.Add(_drawPreview);e.Handled=true;
    }
    private void MarkupUp(object sender,MouseButtonEventArgs e)
    {
        if(sender is not InkCanvas canvas||!canvas.IsMouseCaptured)return;
        if(_drawTool==DrawTool.Eraser){_lastEraserPoint=null;canvas.ReleaseMouseCapture();e.Handled=true;return;}
        if(_drawTool==DrawTool.Select){canvas.ReleaseMouseCapture();CommitSelectedDrawingMove();e.Handled=true;return;}
        if(_drawTool is DrawTool.Freehand or DrawTool.Text or DrawTool.Number)return;var completed=_drawPreview;_drawPreview=null;canvas.ReleaseMouseCapture();if(Active is { } item&&completed is not null){if(_drawTool==DrawTool.Mosaic){canvas.Strokes.Remove(completed);AddMosaicElement(item,_drawStart,e.GetPosition(canvas));}else{item.DrawingOrder.Add(new StrokeDrawingAction(completed));item.DrawingRedo.Clear();}}_drawingOperationChanged=completed is not null||_drawingOperationChanged;e.Handled=true;
    }
    private void MarkupLostMouseCapture(object sender,MouseEventArgs e)
    {
        if(!_drawingMode||sender is not InkCanvas canvas)return;
        if(_drawTool==DrawTool.Select){CommitSelectedDrawingMove();return;}
        if(_drawTool==DrawTool.Eraser){_lastEraserPoint=null;return;}
        if(_drawTool is DrawTool.Freehand or DrawTool.Text or DrawTool.Number||_drawPreview is null)return;var completed=_drawPreview;_drawPreview=null;if(ReferenceEquals(canvas,Active?.Markup)&&Active is { } item){if(_drawTool==DrawTool.Mosaic)canvas.Strokes.Remove(completed);else{item.DrawingOrder.Add(new StrokeDrawingAction(completed));item.DrawingRedo.Clear();_drawingOperationChanged=true;}}PromptStatus.Text="标注笔划已保留，可继续编辑";
    }

    private void BeginDrawingObjectSelection(SelectionItem item,Point point,InkCanvas canvas)
    {
        ClearDrawingObjectSelection();canvas.Focus();Keyboard.Focus(canvas);
        var element=HitTestDrawingElement(item,point);
        if(element is not null)
        {
            _selectedDrawingElementId=element.Id;_drawingMoveOriginalElement=element;_drawingMovePointerStart=point;ShowDrawingObjectSelection(item);canvas.CaptureMouse();PromptStatus.Text="已选择标注 · 拖动移动 · Delete 删除";return;
        }
        var stroke=item.Markup.Strokes.Reverse().FirstOrDefault(candidate=>candidate.HitTest(point,8));
        if(stroke is null){PromptStatus.Text="没有命中标注对象";return;}
        _selectedDrawingStroke=stroke;_drawingMoveOriginalStroke=CaptureStrokeState(stroke);_drawingMovePointerStart=point;ShowDrawingObjectSelection(item);canvas.CaptureMouse();PromptStatus.Text="已选择笔迹或形状 · 拖动移动 · Delete 删除";
    }

    private bool TryEditDrawingText(SelectionItem item,Point point,InkCanvas canvas)
    {
        if(HitTestDrawingElement(item,point) is not TextDrawingElement text)return false;
        if(_drawTool!=DrawTool.Text)SetDrawTool(DrawTool.Text);
        var editor=FindDrawingElementVisual(item,text.Id) as TextBox;
        if(editor is null)return false;
        editor.Focus();Keyboard.Focus(editor);var local=canvas.TranslatePoint(point,editor);var caret=editor.GetCharacterIndexFromPoint(local,true);if(caret>=0)editor.CaretIndex=caret;editor.SelectionLength=0;PromptStatus.Text="正在重新编辑文字 · 点击空白处可新增文本框";return true;
    }

    private void MoveSelectedDrawingObject(SelectionItem item,Point point,InkCanvas canvas)
    {
        if(_drawingMoveOriginalElement is null&&_drawingMoveOriginalStroke is null)return;
        if(!_drawingObjectMoving&&!PinnedWindowInteractionPolicy.ShouldBeginDrag(_drawingMovePointerStart,point,SystemParameters.MinimumHorizontalDragDistance,SystemParameters.MinimumVerticalDragDistance))return;
        _drawingObjectMoving=true;var requested=point-_drawingMovePointerStart;
        if(_drawingMoveOriginalElement is { } original&&_selectedDrawingElementId==original.Id)
        {
            var bounds=DrawingElementBounds(item,original);var delta=DrawingAnnotationGeometry.ConstrainTranslation(bounds,requested,new Size(canvas.ActualWidth,canvas.ActualHeight));var moved=MoveDrawingElement(original,delta);ReplaceDrawingElement(item,moved);var visual=FindDrawingElementVisual(item,original.Id);if(visual is not null){InkCanvas.SetLeft(visual,DrawingElementX(moved));InkCanvas.SetTop(visual,DrawingElementY(moved));}
        }
        else if(_selectedDrawingStroke is { } stroke&&_drawingMoveOriginalStroke is { } originalStroke)
        {
            var bounds=StrokeBounds(originalStroke,stroke.DrawingAttributes);var delta=DrawingAnnotationGeometry.ConstrainTranslation(bounds,requested,new Size(canvas.ActualWidth,canvas.ActualHeight));ApplyStrokeTranslation(stroke,originalStroke,delta);
        }
        ShowDrawingObjectSelection(item);
    }

    private void CommitSelectedDrawingMove()
    {
        if(!_drawingObjectMoving){_drawingMoveOriginalElement=null;_drawingMoveOriginalStroke=null;return;}
        if(Active is { } item)
        {
            if(_drawingMoveOriginalElement is { } before&&_selectedDrawingElementId==before.Id&&item.DrawingElements.FirstOrDefault(element=>element.Id==before.Id) is { } after&&!Equals(before,after))item.DrawingOrder.Add(new ElementMoveDrawingAction(before,after));
            else if(_selectedDrawingStroke is { } stroke&&_drawingMoveOriginalStroke is { } strokeBefore&&item.Markup.Strokes.Contains(stroke))item.DrawingOrder.Add(new StrokeMoveDrawingAction(stroke,strokeBefore,CaptureStrokeState(stroke)));
            item.DrawingRedo.Clear();_drawingOperationChanged=true;ShowDrawingObjectSelection(item);PromptStatus.Text="标注位置已更新 · Delete 可删除";
        }
        _drawingMoveOriginalElement=null;_drawingMoveOriginalStroke=null;_drawingObjectMoving=false;
    }

    private bool DeleteSelectedDrawingObject()
    {
        if(Active is not { } item)return false;
        if(_selectedDrawingElementId is { } id&&item.DrawingElements.FirstOrDefault(element=>element.Id==id) is { } element)
        {
            item.DrawingElements.Remove(element);item.DrawingOrder.Add(new ElementRemovalDrawingAction(element));item.DrawingRedo.Clear();ClearDrawingObjectSelection();RebuildDrawingElements(item);_drawingOperationChanged=true;PromptStatus.Text="已删除所选标注";return true;
        }
        if(_selectedDrawingStroke is { } stroke&&item.Markup.Strokes.Contains(stroke))
        {
            item.Markup.Strokes.Remove(stroke);item.DrawingOrder.Add(new StrokeRemovalDrawingAction(stroke));item.DrawingRedo.Clear();ClearDrawingObjectSelection();_drawingOperationChanged=true;PromptStatus.Text="已删除所选笔迹或形状";return true;
        }
        return false;
    }

    private void EraseDrawingObjectsAt(SelectionItem item,Point point)
    {
        var element=HitTestDrawingElement(item,point);
        if(element is not null)
        {
            item.DrawingElements.Remove(element);item.DrawingOrder.Add(new ElementRemovalDrawingAction(element));item.DrawingRedo.Clear();ClearDrawingObjectSelection();RebuildDrawingElements(item);_drawingOperationChanged=true;PromptStatus.Text="已擦除标注对象";return;
        }
        var stroke=item.Markup.Strokes.Reverse().FirstOrDefault(candidate=>candidate.HitTest(point,9));
        if(stroke is null)return;
        item.Markup.Strokes.Remove(stroke);item.DrawingOrder.Add(new StrokeRemovalDrawingAction(stroke));
        item.DrawingRedo.Clear();ClearDrawingObjectSelection();_drawingOperationChanged=true;PromptStatus.Text="已擦除笔迹或形状";
    }

    private DrawingElementSpec? HitTestDrawingElement(SelectionItem item,Point point)
    {
        foreach(var element in item.DrawingElements.AsEnumerable().Reverse())
        {
            var bounds=DrawingElementBounds(item,element);bounds.Inflate(5,5);if(bounds.Contains(point))return element;
        }
        return null;
    }

    private Rect DrawingElementBounds(SelectionItem item,DrawingElementSpec element)
    {
        var visual=FindDrawingElementVisual(item,element.Id);var width=visual is {ActualWidth:>0}?visual.ActualWidth:element switch{TextDrawingElement text=>text.Width,NumberDrawingElement number=>number.Diameter,MosaicDrawingElement mosaic=>mosaic.Width,_=>0};var height=visual is {ActualHeight:>0}?visual.ActualHeight:element switch{TextDrawingElement text=>Math.Max(30,text.FontSize*1.55),NumberDrawingElement number=>number.Diameter,MosaicDrawingElement mosaic=>mosaic.Height,_=>0};return new Rect(DrawingElementX(element),DrawingElementY(element),Math.Max(1,width),Math.Max(1,height));
    }

    private static double DrawingElementX(DrawingElementSpec element)=>element switch{TextDrawingElement text=>text.X,NumberDrawingElement number=>number.X,MosaicDrawingElement mosaic=>mosaic.X,_=>0};
    private static double DrawingElementY(DrawingElementSpec element)=>element switch{TextDrawingElement text=>text.Y,NumberDrawingElement number=>number.Y,MosaicDrawingElement mosaic=>mosaic.Y,_=>0};
    private static DrawingElementSpec MoveDrawingElement(DrawingElementSpec element,Vector delta)=>element switch{TextDrawingElement text=>text with{X=text.X+delta.X,Y=text.Y+delta.Y},NumberDrawingElement number=>number with{X=number.X+delta.X,Y=number.Y+delta.Y},MosaicDrawingElement mosaic=>mosaic with{X=mosaic.X+delta.X,Y=mosaic.Y+delta.Y},_=>element};

    private static bool ReplaceDrawingElement(SelectionItem item,DrawingElementSpec replacement)
    {
        var index=item.DrawingElements.FindIndex(element=>element.Id==replacement.Id);if(index<0)return false;item.DrawingElements[index]=replacement;return true;
    }

    private static FrameworkElement? FindDrawingElementVisual(SelectionItem item,Guid id)=>item.Markup.Children.OfType<FrameworkElement>().FirstOrDefault(child=>child.Tag is Guid tag&&tag==id);

    private void ShowDrawingObjectSelection(SelectionItem item)
    {
        RemoveDrawingSelectionOutline();Rect bounds;
        if(_selectedDrawingElementId is { } id&&item.DrawingElements.FirstOrDefault(element=>element.Id==id) is { } element)bounds=DrawingElementBounds(item,element);
        else if(_selectedDrawingStroke is { } stroke&&item.Markup.Strokes.Contains(stroke))bounds=stroke.GetBounds();
        else return;
        bounds.Inflate(3,3);_drawingSelectionOutline=new Border{Width=Math.Max(1,bounds.Width),Height=Math.Max(1,bounds.Height),BorderBrush=new SolidColorBrush(Color.FromRgb(74,128,244)),BorderThickness=new Thickness(1.5),CornerRadius=new CornerRadius(3),Background=Brushes.Transparent,IsHitTestVisible=false};InkCanvas.SetLeft(_drawingSelectionOutline,bounds.Left);InkCanvas.SetTop(_drawingSelectionOutline,bounds.Top);Panel.SetZIndex(_drawingSelectionOutline,int.MaxValue);item.Markup.Children.Add(_drawingSelectionOutline);
    }

    private void ClearDrawingObjectSelection()
    {
        RemoveDrawingSelectionOutline();_selectedDrawingElementId=null;_selectedDrawingStroke=null;_drawingMoveOriginalElement=null;_drawingMoveOriginalStroke=null;_drawingObjectMoving=false;
    }

    private void RemoveDrawingSelectionOutline()
    {
        if(_drawingSelectionOutline?.Parent is InkCanvas parent)parent.Children.Remove(_drawingSelectionOutline);_drawingSelectionOutline=null;
    }

    private static StrokeDrawingState CaptureStrokeState(Stroke stroke)=>new(stroke.StylusPoints.Select(point=>point).ToArray());
    private static Rect StrokeBounds(StrokeDrawingState state,DrawingAttributes attributes)
    {
        if(state.Points.Count==0)return Rect.Empty;var left=state.Points.Min(point=>point.X);var top=state.Points.Min(point=>point.Y);var right=state.Points.Max(point=>point.X);var bottom=state.Points.Max(point=>point.Y);var padding=Math.Max(attributes.Width,attributes.Height)/2;return new Rect(new Point(left-padding,top-padding),new Point(right+padding,bottom+padding));
    }
    private static void ApplyStrokeState(Stroke stroke,StrokeDrawingState state)=>stroke.StylusPoints=new StylusPointCollection(state.Points);
    private static void ApplyStrokeTranslation(Stroke stroke,StrokeDrawingState state,Vector delta)
    {
        var points=state.Points.Select(point=>{var moved=point;moved.X+=delta.X;moved.Y+=delta.Y;return moved;});stroke.StylusPoints=new StylusPointCollection(points);
    }
    private void AddStrokeWithoutHistory(SelectionItem item,Stroke stroke){_restoringDrawingAction=true;try{item.Markup.Strokes.Add(stroke);}finally{_restoringDrawingAction=false;}}
    private static Stroke CreateShapeStroke(InkCanvas canvas,Point a,Point b,DrawTool tool,bool constrain)
    {
        if(tool==DrawTool.Ellipse&&constrain)b=DrawingAnnotationGeometry.ConstrainEllipseEndToCircle(a,b,new Size(canvas.ActualWidth,canvas.ActualHeight));
        var points=new StylusPointCollection();
        if(tool is DrawTool.Rectangle or DrawTool.Mosaic){points.Add(new StylusPoint(a.X,a.Y));points.Add(new StylusPoint(b.X,a.Y));points.Add(new StylusPoint(b.X,b.Y));points.Add(new StylusPoint(a.X,b.Y));points.Add(new StylusPoint(a.X,a.Y));}
        else if(tool==DrawTool.Ellipse){var left=Math.Min(a.X,b.X);var top=Math.Min(a.Y,b.Y);var rx=Math.Abs(b.X-a.X)/2;var ry=Math.Abs(b.Y-a.Y)/2;for(var index=0;index<=64;index++){var angle=index*Math.PI*2/64;points.Add(new StylusPoint(left+rx+Math.Cos(angle)*rx,top+ry+Math.Sin(angle)*ry));}}
        else{points.Add(new StylusPoint(a.X,a.Y));points.Add(new StylusPoint(b.X,b.Y));var angle=Math.Atan2(b.Y-a.Y,b.X-a.X);var length=Math.Min(24,Math.Max(10,new Vector(b.X-a.X,b.Y-a.Y).Length*.25));points.Add(new StylusPoint(b.X-length*Math.Cos(angle-Math.PI/6),b.Y-length*Math.Sin(angle-Math.PI/6)));points.Add(new StylusPoint(b.X,b.Y));points.Add(new StylusPoint(b.X-length*Math.Cos(angle+Math.PI/6),b.Y-length*Math.Sin(angle+Math.PI/6)));}
        var attributes=canvas.DefaultDrawingAttributes.Clone();attributes.IsHighlighter=false;attributes.FitToCurve=false;return new Stroke(points,attributes);
    }
    private void EnsureDrawingControls()
    {
        if(!_drawingFontsLoaded)
        {
            foreach(var family in Fonts.SystemFontFamilies.GroupBy(font=>font.Source,StringComparer.CurrentCultureIgnoreCase).Select(group=>group.First()).Select(font=>new DrawingFontChoice(font.Source,LocalizedFontName(font))).OrderBy(choice=>choice.DisplayName,StringComparer.CurrentCultureIgnoreCase))DrawingFontFamily.Items.Add(family);
            foreach(var size in new[]{12d,14,16,18,20,24,28,32,40,48,56,64,72})DrawingFontSize.Items.Add(size);
            _drawingFontsLoaded=true;
        }
        DrawingFontFamily.SelectedItem=DrawingFontFamily.Items.Cast<object>().OfType<DrawingFontChoice>().FirstOrDefault(item=>string.Equals(item.Source,_drawFontFamily,StringComparison.CurrentCultureIgnoreCase))??DrawingFontFamily.Items.Cast<object>().FirstOrDefault();
        DrawingFontSize.SelectedItem=DrawingFontSize.Items.Cast<object>().FirstOrDefault(item=>item is double value&&Math.Abs(value-_drawFontSize)<.1)??24d;
        TextHighlightButton.Background=new SolidColorBrush(_drawTextHighlight?Color.FromRgb(255,236,139):Colors.Transparent);DrawColorSwatch.Fill=new SolidColorBrush(_drawColor);
    }
    private static string LocalizedFontName(FontFamily family)
    {
        foreach(var tag in new[]{CultureInfo.CurrentUICulture.IetfLanguageTag,"zh-CN","zh-Hans","en-US"})
        {
            var language=XmlLanguage.GetLanguage(tag);if(family.FamilyNames.TryGetValue(language,out var name)&&!string.IsNullOrWhiteSpace(name))return name;
        }
        return family.FamilyNames.Values.FirstOrDefault(name=>!string.IsNullOrWhiteSpace(name))??family.Source;
    }
    private void DrawingFontFamilyChanged(object sender,SelectionChangedEventArgs e){if(DrawingFontFamily.SelectedItem is DrawingFontChoice selected)_drawFontFamily=selected.Source;UpdateFocusedDrawingTextStyle();}
    private void DrawingFontSizeChanged(object sender,SelectionChangedEventArgs e){if(DrawingFontSize.SelectedItem is double selected)_drawFontSize=selected;UpdateFocusedDrawingTextStyle();}
    private void UpdateFocusedDrawingTextStyle()
    {
        if(Active is not { } item||Keyboard.FocusedElement is not TextBox box||box.Tag is not Guid id)return;var index=item.DrawingElements.FindIndex(element=>element.Id==id);if(index<0||item.DrawingElements[index] is not TextDrawingElement text)return;var updated=text with{FontFamily=_drawFontFamily,FontSize=_drawFontSize};item.DrawingElements[index]=updated;box.FontFamily=new FontFamily(updated.FontFamily);box.FontSize=updated.FontSize;_drawingOperationChanged=true;
    }
    private void AddTextDrawingElement(SelectionItem item,Point point)
    {
        var width=Math.Clamp(Math.Min(260,Math.Max(120,item.Bounds.Width*.42)),80,Math.Max(80,item.Bounds.Width));var x=Math.Clamp(point.X,0,Math.Max(0,item.Bounds.Width-width));var y=Math.Clamp(point.Y,0,Math.Max(0,item.Bounds.Height-Math.Max(32,_drawFontSize*1.7)));var element=new TextDrawingElement(Guid.NewGuid(),x,y,width,string.Empty,_drawFontFamily,_drawFontSize,_drawColor,_drawTextHighlight);item.DrawingElements.Add(element);item.DrawingOrder.Add(new ElementDrawingAction(element));item.DrawingRedo.Clear();var editor=CreateTextDrawingEditor(item,element);item.Markup.Children.Add(editor);editor.Focus();_drawingOperationChanged=true;
    }
    private void AddNumberDrawingElement(SelectionItem item,Point point)
    {
        var diameter=Math.Clamp(_drawFontSize+12,28,56);var x=Math.Clamp(point.X-diameter/2,0,Math.Max(0,item.Bounds.Width-diameter));var y=Math.Clamp(point.Y-diameter/2,0,Math.Max(0,item.Bounds.Height-diameter));var element=new NumberDrawingElement(Guid.NewGuid(),x,y,diameter,item.NextDrawingNumber++,_drawColor);item.DrawingElements.Add(element);item.DrawingOrder.Add(new ElementDrawingAction(element));item.DrawingRedo.Clear();item.Markup.Children.Add(CreateNumberDrawingVisual(element));_drawingOperationChanged=true;PromptStatus.Text=$"已放置序号 {element.Number} · 继续点击放置 {item.NextDrawingNumber}";
    }
    private void AddMosaicElement(SelectionItem item,Point start,Point end)
    {
        var bounds=Normalize(new Rect(start,end));if(bounds.Width<3||bounds.Height<3)return;var element=new MosaicDrawingElement(Guid.NewGuid(),bounds.X,bounds.Y,bounds.Width,bounds.Height);item.DrawingElements.Add(element);item.DrawingOrder.Add(new ElementDrawingAction(element));item.DrawingRedo.Clear();item.Markup.Children.Add(CreateMosaicVisual(item,element));_drawingOperationChanged=true;
    }
    private Image CreateMosaicVisual(SelectionItem item,MosaicDrawingElement element)
    {
        var source=RenderSelectionImage(item,false,false,false);var scaleX=source.PixelWidth/Math.Max(1,item.Bounds.Width);var scaleY=source.PixelHeight/Math.Max(1,item.Bounds.Height);var left=Math.Clamp((int)Math.Floor(element.X*scaleX),0,source.PixelWidth-1);var top=Math.Clamp((int)Math.Floor(element.Y*scaleY),0,source.PixelHeight-1);var right=Math.Clamp((int)Math.Ceiling((element.X+element.Width)*scaleX),left+1,source.PixelWidth);var bottom=Math.Clamp((int)Math.Ceiling((element.Y+element.Height)*scaleY),top+1,source.PixelHeight);var region=new Int32Rect(left,top,right-left,bottom-top);var pixelated=ImagePixelationService.Pixelate(source,region,Math.Clamp((int)Math.Round(12*Math.Max(scaleX,scaleY)),6,40));var crop=new CroppedBitmap(pixelated,region);crop.Freeze();var visual=new Image{Tag=element.Id,Source=crop,Width=element.Width,Height=element.Height,Stretch=Stretch.Fill,IsHitTestVisible=false};InkCanvas.SetLeft(visual,element.X);InkCanvas.SetTop(visual,element.Y);return visual;
    }
    private TextBox CreateTextDrawingEditor(SelectionItem item,TextDrawingElement element)
    {
        var editor=new TextBox{Tag=element.Id,Text=element.Text,Width=element.Width,MinHeight=Math.Max(30,element.FontSize*1.55),AcceptsReturn=true,TextWrapping=TextWrapping.Wrap,FontFamily=new FontFamily(element.FontFamily),FontSize=element.FontSize,FontWeight=FontWeights.SemiBold,Padding=new Thickness(4,1,4,2),Foreground=new SolidColorBrush(element.Color),Background=element.Highlight?new SolidColorBrush(Color.FromArgb(150,255,237,105)):Brushes.Transparent,BorderBrush=Brushes.Transparent,BorderThickness=new Thickness(1),CaretBrush=new SolidColorBrush(element.Color)};
        InkCanvas.SetLeft(editor,element.X);InkCanvas.SetTop(editor,element.Y);
        editor.GotKeyboardFocus+=(_,_)=>editor.BorderBrush=new SolidColorBrush(Color.FromRgb(108,124,238));
        editor.LostKeyboardFocus+=(_,_)=>editor.BorderBrush=Brushes.Transparent;
        editor.TextChanged+=(_,_)=>{var index=item.DrawingElements.FindIndex(candidate=>candidate.Id==element.Id);if(index<0||item.DrawingElements[index] is not TextDrawingElement current)return;item.DrawingElements[index]=current with{Text=editor.Text};item.DrawingRedo.Clear();_drawingOperationChanged=true;};
        return editor;
    }
    private static Border CreateNumberDrawingVisual(NumberDrawingElement element)
    {
        var visual=new Border{Tag=element.Id,Width=element.Diameter,Height=element.Diameter,CornerRadius=new CornerRadius(element.Diameter/2),Background=new SolidColorBrush(element.Color),IsHitTestVisible=false,Child=new TextBlock{Text=element.Number.ToString(System.Globalization.CultureInfo.InvariantCulture),Foreground=ContrastBrush(element.Color),FontSize=Math.Clamp(element.Diameter*.48,13,25),FontWeight=FontWeights.Bold,HorizontalAlignment=HorizontalAlignment.Center,VerticalAlignment=VerticalAlignment.Center,TextAlignment=TextAlignment.Center}};InkCanvas.SetLeft(visual,element.X);InkCanvas.SetTop(visual,element.Y);return visual;
    }
    private static Brush ContrastBrush(Color color)=>color.R*.299+color.G*.587+color.B*.114>155?Brushes.Black:Brushes.White;
    private void RebuildDrawingElements(SelectionItem item)
    {
        item.Markup.Children.Clear();foreach(var element in item.DrawingElements){if(element is TextDrawingElement text)item.Markup.Children.Add(CreateTextDrawingEditor(item,text));else if(element is NumberDrawingElement number)item.Markup.Children.Add(CreateNumberDrawingVisual(number));else if(element is MosaicDrawingElement mosaic)item.Markup.Children.Add(CreateMosaicVisual(item,mosaic));}
    }
    private void RemoveEmptyDrawingText(SelectionItem item)
    {
        var empty=item.DrawingElements.OfType<TextDrawingElement>().Where(element=>string.IsNullOrWhiteSpace(element.Text)).Select(element=>element.Id).ToHashSet();if(empty.Count==0)return;item.DrawingElements.RemoveAll(element=>empty.Contains(element.Id));item.DrawingOrder.RemoveAll(action=>action is ElementDrawingAction element&&empty.Contains(element.Element.Id));RebuildDrawingElements(item);
    }
    private async void QuickSend(object s,RoutedEventArgs e)=>await SendAsync(true);
    private async void QuickPromptKeyDown(object s,KeyEventArgs e){if(e.Key==Key.Enter&&!Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)){e.Handled=true;if(ReferencePicker.IsOpen&&_referencePickerCandidates.Count>0){InsertReferenceMention(_referencePickerCandidates[0]);return;}await SendAsync(true);}}
    private async void QuickVoice(object s,RoutedEventArgs e)=>await ToggleVoiceAsync();

    private async void RecognizeTable(object sender,RoutedEventArgs e)
    {
        if(Active is not {IsImplicit:false,VideoPath:null} item){PromptStatus.Text="请先框选包含表格的图片区域";return;}
        var prompt=LocalizationService.T("识别当前图片中的所有表格并逐格精确转录。answer 只输出 Markdown 表格，不要添加标题、解释、代码围栏或表格外文字；每个原表格对应一个 Markdown 表格。严格保持行列顺序，空单元格保留为空，合并单元格把内容放在左上格，其余对应格留空；数字、小数点、正负号、百分号、日期和单位必须逐字符核对。annotationMode 必须为 preserve，annotations 必须为空数组。","Recognize every table in the current image and transcribe it cell by cell. In answer, return Markdown tables only—no heading, explanation, code fence, or text outside the tables. Preserve row and column order and keep empty cells empty. For merged cells, put the content in the top-left cell and leave the covered cells blank. Verify every digit, decimal point, sign, percentage, date, and unit. Set annotationMode to preserve and return an empty annotations array.");
        await SendAsync(false,prompt,item,true);
    }

    private void CopyRecognizedTable(object sender,RoutedEventArgs e)
    {
        if(TableClipboardService.TryCopy(AnswerText.Markdown,out var count,out var error))PromptStatus.Text=$"已复制 {count} 个表格 · Excel 可直接粘贴，文本框为 Markdown，桌面为 PNG";
        else PromptStatus.Text=error??"复制表格失败";
    }

    private void UploadAttachment(object sender,RoutedEventArgs e)
    {
        if(!_conversationAiAvailable)return;
        var dialog=new OpenFileDialog{Multiselect=true,Filter=LocalizationService.T("图片/视频/文本|*.png;*.jpg;*.jpeg;*.webp;*.gif;*.mp4;*.mov;*.webm;*.txt;*.md;*.json;*.csv|所有文件|*.*","Images, video, and text|*.png;*.jpg;*.jpeg;*.webp;*.gif;*.mp4;*.mov;*.webm;*.txt;*.md;*.json;*.csv|All files|*.*")};
        if(ShowSystemFileDialog(dialog)!=true)return;
        foreach(var path in dialog.FileNames)
        {
            try
            {
                var info=new FileInfo(path);if(!info.Exists||info.Length==0){PromptStatus.Text="附件为空，已跳过";continue;}
                var ext=info.Extension.ToLowerInvariant();
                var (type,mime,maxBytes)=ext switch
                {
                    ".png"=>(AiAttachmentType.Image,"image/png",25L*1024*1024),
                    ".jpg" or ".jpeg"=>(AiAttachmentType.Image,"image/jpeg",25L*1024*1024),
                    ".webp"=>(AiAttachmentType.Image,"image/webp",25L*1024*1024),
                    ".gif"=>(AiAttachmentType.Image,"image/gif",25L*1024*1024),
                    ".bmp"=>(AiAttachmentType.Image,"image/bmp",25L*1024*1024),
                    ".mp4"=>(AiAttachmentType.Video,"video/mp4",50L*1024*1024),
                    ".mov"=>(AiAttachmentType.Video,"video/quicktime",50L*1024*1024),
                    ".webm"=>(AiAttachmentType.Video,"video/webm",50L*1024*1024),
                    ".txt"=>(AiAttachmentType.Text,"text/plain",8L*1024*1024),
                    ".md"=>(AiAttachmentType.Text,"text/markdown",8L*1024*1024),
                    ".json"=>(AiAttachmentType.Text,"application/json",8L*1024*1024),
                    ".csv"=>(AiAttachmentType.Text,"text/csv",8L*1024*1024),
                    _=>throw new NotSupportedException("暂不支持此文件格式，请选择图片、视频或文本文件")
                };
                if(info.Length>maxBytes){PromptStatus.Text=$"附件超过 {maxBytes/(1024*1024)} MB，已跳过";continue;}
                var preview=type==AiAttachmentType.Text?ReadTextPreview(path):info.Name;
                var item=new UploadedReference("upload-"+Guid.NewGuid().ToString("N"),info.FullName,type,mime,preview){Label=$"@文件{++_nextUploadNumber}"};
                _uploadedReferences.Add(item);
            }
            catch(Exception ex){PromptStatus.Text=$"无法读取附件：{ex.Message}";}
        }
        UpdateReferenceChips(); UpdateReferencePicker(); PromptStatus.Text=$"已添加 {_uploadedReferences.Count} 个附件，可输入 @ 选择"; QuickPrompt.Focus();
    }

    private static string ReadTextPreview(string path)
    {
        try
        {
            using var reader=new StreamReader(path);var buffer=new char[91];var count=reader.ReadBlock(buffer,0,buffer.Length);var text=new string(buffer,0,count);
            return count==buffer.Length?text[..90]+"…":text;
        }
        catch{return "文本文件";}
    }
    private static (int Width,int Height) GetUploadedImageSize(string path)
    {
        try
        {
            using var stream=File.OpenRead(path);
            var frame=BitmapFrame.Create(stream,BitmapCreateOptions.PreservePixelFormat,BitmapCacheOption.OnLoad);
            return (frame.PixelWidth,frame.PixelHeight);
        }
        catch{return (0,0);}
    }
    private async Task ToggleVoiceAsync()
    {
        if(_closed||RejectIfOverlayOperationBusy())return;
        if(!_conversationAiAvailable)return;
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
        catch(Exception ex){new PrivacyLogger().Error("OverlaySpeech",ex);if(!_closed&&ReferenceEquals(_speechRequest,speechRequest))PromptStatus.Text="语音输入暂时不可用";}
        finally{speechRequest.Dispose();if(ReferenceEquals(_speechRequest,speechRequest))_speechRequest=null;if(!_closed)VoiceIcon.Data=microphone;}
    }
    private void ApplyVoiceAvailability()=>VoiceButton.Visibility=_conversationAiAvailable&&_host.Settings.EnableVoiceInput?Visibility.Visible:Visibility.Collapsed;
    private async void Translate(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy())return;
        if(!_translationAiAvailable)return;
        if(Active is not { } item||!CaptureOverlayPolicy.CanRunImageOnlyCommand(item.IsImplicit,item.VideoPath)){if(Active?.VideoPath is not null)PromptStatus.Text="视频区域不支持 OCR/翻译，请先选择截图区域";else PromptStatus.Text="请先框选截图区域";return;}var before=CaptureOverlaySnapshot();var image=CurrentImage();var operation=BeginOverlayOperation("正在识别文字…按 Esc 可取消");
        try
        {
            var document=await new WindowsOcrService().RecognizeAsync(image,operation.Token);
            if(!IsOverlayOperationActive(operation,item))return;
            if(document.Lines.Count==0){PromptStatus.Text=$"{document.Engine} 未识别到文字";return;}
            var provider=_host.CreateTranslationProvider(out var providerError);if(provider is null){PromptStatus.Text=providerError??"翻译需要先配置可用的 AI Provider";RefreshAiFeatureAvailability();return;}
            var batches=CaptureOverlayPolicy.CreateTranslationBatches(document.Lines.Select(line=>line.Text).ToArray());
            var translations=new string[document.Lines.Count];
            for(var batchIndex=0;batchIndex<batches.Count;batchIndex++)
            {
                if(!IsOverlayOperationActive(operation,item))return;
                var batch=batches[batchIndex];var batchNumber=batchIndex+1;
                PromptStatus.Text=$"{document.Engine} 已识别 {document.Lines.Count} 行 · 正在翻译 {batchNumber}/{batches.Count}…按 Esc 可取消";
                var prompt=LocalizationService.T("将 translationsSource 中的每一项翻译成简体中文。保持数组长度和顺序完全一致，只返回 JSON：{\"translations\":[\"译文1\",\"译文2\"]}。translationsSource=","Translate every item in translationsSource into natural English. Preserve the exact array length and order. Return JSON only: {\"translations\":[\"translation 1\",\"translation 2\"]}. translationsSource=")+System.Text.Json.JsonSerializer.Serialize(batch.Lines);
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
            EventHandler? restore=null;restore=(_,_)=>{settings.Closed-=restore;if(IsVisible){RefreshAiFeatureAvailability();PositionPromptBar();if(Active is not null)ShowToolbar();Topmost=true;Activate();if(_conversationAiAvailable)QuickPrompt.Focus();}};settings.Closed+=restore;settings.Activate();
        }
        catch(Exception ex){Topmost=true;Activate();new PrivacyLogger().Error("OverlaySettings",ex);PromptStatus.Text="无法打开设置，请从托盘重试";}
    }
    private void RenderTextOverlays(SelectionItem item,BitmapSource image,IReadOnlyList<OcrLine> lines,IReadOnlyList<string> texts,bool translated)
    {
        item.TextLayer=new TranslationTextLayerState(image,lines.ToArray(),texts.ToArray());
        RenderTextOverlaysCore(item,image,lines,texts,translated);
    }
    private void RenderTextOverlaysCore(SelectionItem item,BitmapSource image,IReadOnlyList<OcrLine> lines,IReadOnlyList<string> texts,bool translated)
    {
        ClearTextSelection(item);item.TextOverlays.Children.Clear();var scaleX=item.Bounds.Width/image.PixelWidth;var scaleY=item.Bounds.Height/image.PixelHeight;if(!translated)return;
        var pixelsPerDip=VisualTreeHelper.GetDpi(this).PixelsPerDip;var entries=new List<TranslationVisualEntry>();var selectableLines=new List<OcrLine>();
        for(var index=0;index<lines.Count&&index<texts.Count;index++)
        {
            var line=lines[index];var value=texts[index]?.Trim();if(string.IsNullOrWhiteSpace(value))continue;var lineBounds=new Rect(line.X*scaleX,line.Y*scaleY,Math.Max(1,line.Width*scaleX),Math.Max(1,line.Height*scaleY));var fontSize=Math.Clamp(lineBounds.Height*.78,9,28);var maxTextWidth=Math.Max(24,item.Bounds.Width-TranslationOverlayLayoutService.HorizontalPadding);IReadOnlyList<TranslationVisualRow> rows=[];double lineHeight=0;
            for(var attempt=0;attempt<5;attempt++)
            {
                rows=WrapTranslationText(value,fontSize,maxTextWidth,pixelsPerDip);lineHeight=Math.Max(fontSize*1.18,lineBounds.Height*.9);var requiredHeight=rows.Count*lineHeight+TranslationOverlayLayoutService.VerticalPadding;if(requiredHeight<=item.Bounds.Height||fontSize<=7.1)break;fontSize=Math.Max(7,fontSize*Math.Clamp((item.Bounds.Height-TranslationOverlayLayoutService.VerticalPadding)/requiredHeight,.65,.92));
            }
            if(rows.Count==0)continue;var contentWidth=rows.Max(row=>row.Width)+TranslationOverlayLayoutService.HorizontalPadding;var contentHeight=rows.Count*lineHeight+TranslationOverlayLayoutService.VerticalPadding;var placement=TranslationOverlayLayoutService.Place(lineBounds,new Size(item.Bounds.Width,item.Bounds.Height),contentWidth,contentHeight);if(placement.IsEmpty)continue;var pixelRect=TranslationOverlayLayoutService.ToImagePixelRect(placement,image,scaleX,scaleY);var backdropColor=TranslationOverlayLayoutService.GetAverageColor(image,pixelRect);entries.Add(new TranslationVisualEntry(placement,fontSize,lineHeight,rows,backdropColor));
            for(var rowIndex=0;rowIndex<rows.Count;rowIndex++)
            {
                var row=rows[rowIndex];var x=placement.Left+TranslationOverlayLayoutService.HorizontalPadding/2;var y=placement.Top+TranslationOverlayLayoutService.VerticalPadding/2+rowIndex*lineHeight;var width=Math.Min(row.Width,Math.Max(1,placement.Width-TranslationOverlayLayoutService.HorizontalPadding));var bounds=new Rect(x,y,width,Math.Min(lineHeight,Math.Max(1,placement.Bottom-y)));selectableLines.Add(new OcrLine(row.Text,bounds.X,bounds.Y,bounds.Width,bounds.Height,[new OcrWord(row.Text,bounds.X,bounds.Y,bounds.Width,bounds.Height)]));
            }
        }
        foreach(var entry in entries){var backdrop=TranslationOverlayLayoutService.CreateBackdrop(image,entry.Bounds,scaleX,scaleY,entry.BackdropColor);Canvas.SetLeft(backdrop,entry.Bounds.Left);Canvas.SetTop(backdrop,entry.Bounds.Top);item.TextOverlays.Children.Add(backdrop);}
        foreach(var entry in entries)
        {
            var luminance=.2126*entry.BackdropColor.R+.7152*entry.BackdropColor.G+.0722*entry.BackdropColor.B;var lightBackground=luminance>150;var text=new OutlinedTextVisual(entry.Rows.Select(row=>row.Text).ToArray(),"Segoe UI",entry.FontSize,entry.LineHeight,lightBackground?Color.FromRgb(24,31,42):Colors.White,lightBackground?Colors.White:Colors.Black,TranslationOverlayLayoutService.HorizontalPadding/2,TranslationOverlayLayoutService.VerticalPadding/2){Width=entry.Bounds.Width,Height=entry.Bounds.Height,ToolTip="原位译文 · 可拖选复制"};var host=new Grid{Width=entry.Bounds.Width,Height=entry.Bounds.Height,ClipToBounds=true,IsHitTestVisible=false};host.Children.Add(text);Canvas.SetLeft(host,entry.Bounds.Left);Canvas.SetTop(host,entry.Bounds.Top);item.TextOverlays.Children.Add(host);
        }
        if(selectableLines.Count>0)RenderTextSelectionCore(item,selectableLines,1,1,string.Join(Environment.NewLine,texts.Where(text=>!string.IsNullOrWhiteSpace(text))),"可跨行拖动选择译文，Ctrl+C 复制","复制全部译文");
    }

    private static IReadOnlyList<TranslationVisualRow> WrapTranslationText(string value,double fontSize,double maxWidth,double pixelsPerDip)
    {
        return TranslationOverlayLayoutService.WrapText(value,maxWidth,text=>MeasureTranslationText(text,fontSize,pixelsPerDip)).Select(text=>new TranslationVisualRow(text,Math.Min(maxWidth,MeasureTranslationText(text,fontSize,pixelsPerDip)))).ToArray();
    }

    private static double MeasureTranslationText(string value,double fontSize,double pixelsPerDip)=>new FormattedText(value,CultureInfo.CurrentUICulture,FlowDirection.LeftToRight,new Typeface("Segoe UI"),fontSize,Brushes.Black,pixelsPerDip).WidthIncludingTrailingWhitespace;

    private void RenderSelectableText(SelectionItem item,BitmapSource image,OcrDocument document)
    {
        item.TextLayer=new OcrTextLayerState(image,document);
        RenderSelectableTextCore(item,image,document);
    }
    private void RenderSelectableTextCore(SelectionItem item,BitmapSource image,OcrDocument document)
    {
        item.TextOverlays.Children.Clear();ClearTextSelection(item);if(document.Lines.Count==0)return;var scaleX=item.Bounds.Width/image.PixelWidth;var scaleY=item.Bounds.Height/image.PixelHeight;RenderTextSelectionCore(item,document.Lines,scaleX,scaleY,document.Text,"可跨行拖动选择文字，Ctrl+C 复制","复制全部识别文字");
    }
    private void RenderTextSelectionCore(SelectionItem item,IReadOnlyList<OcrLine> lines,double scaleX,double scaleY,string allText,string toolTip,string copyAllHeader)
    {
        var layout=OcrSelectionLayout.Build(lines,scaleX,scaleY);if(layout.Count==0)return;
        var flow=new FlowDocument{PagePadding=new Thickness(0),ColumnGap=0,FontFamily=new FontFamily("Segoe UI"),FontSize=1,LineHeight=1,Foreground=Brushes.Transparent};var pending=new List<(OcrSelectionGlyph Glyph,Run Run)>();
        foreach(var line in layout)
        {
            var paragraph=new Paragraph{Margin=new Thickness(0),Padding=new Thickness(0),FontSize=1,LineHeight=1,Foreground=Brushes.Transparent};
            foreach(var token in line.Tokens){if(token.Prefix.Length>0)paragraph.Inlines.Add(new Run(token.Prefix));var run=new Run(token.Text);paragraph.Inlines.Add(run);foreach(var glyph in token.Glyphs)pending.Add((glyph,run));}
            flow.Blocks.Add(paragraph);
        }
        var box=new RichTextBox{Document=flow,IsReadOnly=true,IsReadOnlyCaretVisible=false,Background=Brushes.Transparent,BorderThickness=new Thickness(0),Padding=new Thickness(0),SelectionBrush=Brushes.Transparent,SelectionTextBrush=Brushes.Transparent,Cursor=Cursors.IBeam,Width=item.Bounds.Width,Height=item.Bounds.Height,VerticalScrollBarVisibility=ScrollBarVisibility.Disabled,HorizontalScrollBarVisibility=ScrollBarVisibility.Disabled,ToolTip=toolTip};
        var highlights=new Canvas{Width=item.Bounds.Width,Height=item.Bounds.Height,IsHitTestVisible=false};var selectable=new List<SelectableGlyph>(pending.Count);
        foreach(var entry in pending){var start=entry.Run.ContentStart.GetPositionAtOffset(entry.Glyph.Utf16Start,LogicalDirection.Forward);var end=entry.Run.ContentStart.GetPositionAtOffset(entry.Glyph.Utf16Start+entry.Glyph.Utf16Length,LogicalDirection.Forward);if(start is not null&&end is not null)selectable.Add(new SelectableGlyph(entry.Glyph.Bounds,start,end));}
        if(selectable.Count==0)return;box.SelectionChanged+=(_,_)=>{var count=new TextRange(box.Selection.Start,box.Selection.End).Text.Length;if(count>0)PromptStatus.Text=$"已选择 {count} 个字符 · Ctrl+C 复制或右键";};box.ContextMenu=CreateTextContextMenu(box,allText,copyAllHeader);item.TextSelection.Children.Add(highlights);item.TextSelection.Children.Add(box);item.TextSession=new OcrTextSelectionSession(box,highlights,selectable);item.TextSelection.IsHitTestVisible=true;
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
    private ContextMenu CreateTextContextMenu(RichTextBox box,string allText,string copyAllHeader)
    {
        var menu=new ContextMenu();menu.SetResourceReference(StyleProperty,"TextSelectionContextMenu");var copy=new MenuItem{Header="复制所选文字"};var copyAll=new MenuItem{Header=copyAllHeader};foreach(var entry in new[]{copy,copyAll})entry.SetResourceReference(StyleProperty,"TextSelectionMenuItem");copy.Click+=(_,_)=>CopyTextToClipboard(new TextRange(box.Selection.Start,box.Selection.End).Text.TrimEnd('\r','\n'));copyAll.Click+=(_,_)=>CopyTextToClipboard(allText);var separator=new Separator();separator.SetResourceReference(StyleProperty,"TextSelectionSeparator");menu.Items.Add(copy);menu.Items.Add(separator);menu.Items.Add(copyAll);menu.Opened+=(_,_)=>copy.IsEnabled=!box.Selection.IsEmpty;return menu;
    }
    private void CopyTextToClipboard(string text){if(text.Length==0)return;PromptStatus.Text=ClipboardService.TrySetText(text,out var error)?"文字已复制":error;}
    private static void ClearTextSelection(SelectionItem item){item.TextSession?.Dispose();item.TextSession=null;item.TextSelection.Children.Clear();item.TextSelection.IsHitTestVisible=false;}
    private async void Record(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy())return;
        if(!_captureExclusionVerified){SetPromptBarHidden(false);PromptStatus.Text=NativeMethods.VisualQaCaptureEnabled?"视觉验收模式未启用防捕获，已阻止录屏以免录入覆盖控件":"系统未能启用窗口防捕获，为避免录入遮罩和控件，已阻止录屏；请重启软件后重试";return;}
        if(_recordingSession is not null||_recordingCountdownActive||Active is not {IsImplicit:false,VideoPath:null} item)return;
        var countdown=new CancellationTokenSource();_recordingCountdownRequest=countdown;_recordingCountdownActive=true;_recordingItem=item;_recordingItemWasReferenced=_references.Contains(item);
        try
        {
            CrashDiagnosticsService.MarkOperation("屏幕助手：区域录屏倒计时");
            EnterRecordingCountdown(item);
            await RunRecordingCountdownAsync(countdown.Token);
            countdown.Token.ThrowIfCancellationRequested();
            RecordingCountdown.Visibility=Visibility.Collapsed;if(!NativeMethods.TrySetWindowMouseTransparent(new WindowInteropHelper(this).Handle,false))throw new InvalidOperationException("无法恢复录屏控制条的鼠标交互，请重新截图");_recordingCountdownActive=false;
            var pixels=ToPixelRect(item.Bounds);var region=ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY);var session=new RecordingSession(_host.Settings,region);_recordingSession=session;session.Completed+=path=>{if(!Dispatcher.HasShutdownStarted)Dispatcher.BeginInvoke(new Action(()=>CompleteRecording(session,item,path)));};session.Failed+=error=>{if(!Dispatcher.HasShutdownStarted)Dispatcher.BeginInvoke(new Action(()=>FailRecording(session,item,error)));};EnterRecordingMode(item);session.Start();_recordingTimer.Start();PromptStatus.Text="正在录制当前区域";CrashDiagnosticsService.MarkOperation("屏幕助手：正在区域录屏");
        }
        catch(OperationCanceledException)when(countdown.IsCancellationRequested)
        {
            if(!_closed&&ReferenceEquals(_recordingCountdownRequest,countdown))RestoreAfterRecordingCountdown(item,"已取消录屏倒计时");
        }
        catch(Exception ex)
        {
            if(_recordingSession is { } failedSession&&ReferenceEquals(_recordingItem,item))FailRecording(failedSession,item,ex.Message);else{new PrivacyLogger().Error("RecordingStart",ex);if(!_closed)RestoreAfterRecordingCountdown(item,$"录屏失败：{ex.Message}");}
        }
        finally
        {
            if(ReferenceEquals(_recordingCountdownRequest,countdown))_recordingCountdownRequest=null;
            countdown.Dispose();
        }
    }

    private void EnterRecordingCountdown(SelectionItem selected)
    {
        if(!NativeMethods.TrySetWindowMouseTransparent(new WindowInteropHelper(this).Handle,true))throw new InvalidOperationException("无法启用倒计时期间的鼠标穿透，请重新截图");
        Cursor=Cursors.Arrow;if(Root.IsMouseCaptured)Root.ReleaseMouseCapture();Toolbar.Visibility=DrawingToolbar.Visibility=PromptBarHost.Visibility=SizeText.Visibility=PointerInspector.Visibility=RecordingBar.Visibility=Visibility.Collapsed;HideHandles();
        foreach(var item in _selections){item.Host.Visibility=ReferenceEquals(item,selected)?Visibility.Visible:Visibility.Collapsed;item.Badge.Visibility=Visibility.Collapsed;item.Markup.Visibility=item.TextOverlays.Visibility=item.AiAnnotations.Visibility=item.TextSelection.Visibility=Visibility.Collapsed;item.Markup.IsHitTestVisible=false;item.Image.Visibility=ReferenceEquals(item,selected)?Visibility.Collapsed:Visibility.Visible;}
        selected.Outline.BorderBrush=new SolidColorBrush(Color.FromRgb(50,151,242));selected.Outline.BorderThickness=new Thickness(2);selected.Outline.Effect=new DropShadowEffect{Color=Color.FromRgb(48,151,242),BlurRadius=18,ShadowDepth=0,Opacity=.9};
        var full=new RectangleGeometry(new Rect(0,0,Math.Max(0,Root.ActualWidth),Math.Max(0,Root.ActualHeight)));var hole=new RectangleGeometry(Normalize(selected.Bounds));DesktopImage.Clip=new CombinedGeometry(GeometryCombineMode.Exclude,full,hole);Dimmer.Clip=new CombinedGeometry(GeometryCombineMode.Exclude,full,hole);
        Canvas.SetLeft(RecordingCountdown,selected.Bounds.Left+(selected.Bounds.Width-RecordingCountdown.Width)/2);Canvas.SetTop(RecordingCountdown,selected.Bounds.Top+(selected.Bounds.Height-RecordingCountdown.Height)/2);RecordingCountdown.Visibility=Visibility.Visible;
    }

    private async Task RunRecordingCountdownAsync(CancellationToken cancellationToken)
    {
        foreach(var value in CaptureOverlayPolicy.RecordingCountdownValues)
        {
            cancellationToken.ThrowIfCancellationRequested();RecordingCountdownText.Text=value.ToString(System.Globalization.CultureInfo.InvariantCulture);RecordingCountdown.Opacity=1;RecordingCountdownScale.ScaleX=RecordingCountdownScale.ScaleY=1;
            RecordingCountdown.BeginAnimation(OpacityProperty,new DoubleAnimation(0,1,TimeSpan.FromMilliseconds(220)){FillBehavior=FillBehavior.Stop});
            RecordingCountdownScale.BeginAnimation(ScaleTransform.ScaleXProperty,new DoubleAnimation(.72,1,TimeSpan.FromMilliseconds(260)){FillBehavior=FillBehavior.Stop});
            RecordingCountdownScale.BeginAnimation(ScaleTransform.ScaleYProperty,new DoubleAnimation(.72,1,TimeSpan.FromMilliseconds(260)){FillBehavior=FillBehavior.Stop});
            await Task.Delay(CaptureOverlayPolicy.RecordingCountdownStep,cancellationToken);
        }
    }

    private void CancelRecordingCountdown()
    {
        var countdown=_recordingCountdownRequest;if(countdown is null)return;try{countdown.Cancel();}catch(ObjectDisposedException){}
    }

    private void RestoreAfterRecordingCountdown(SelectionItem selected,string status)
    {
        var interactionRestored=NativeMethods.TrySetWindowMouseTransparent(new WindowInteropHelper(this).Handle,false);_recordingCountdownActive=false;RecordingCountdown.Visibility=Visibility.Collapsed;RecordingCountdown.BeginAnimation(OpacityProperty,null);RecordingCountdownScale.BeginAnimation(ScaleTransform.ScaleXProperty,null);RecordingCountdownScale.BeginAnimation(ScaleTransform.ScaleYProperty,null);DesktopImage.Clip=null;Dimmer.Clip=null;_recordingItem=null;_recordingItemWasReferenced=false;PromptBarHost.Visibility=_conversationAiAvailable?Visibility.Visible:Visibility.Collapsed;Cursor=Cursors.Cross;
        foreach(var item in _selections){item.Host.Visibility=Visibility.Visible;item.Badge.Visibility=Visibility.Visible;var imageOnly=item.VideoPath is null?Visibility.Visible:Visibility.Collapsed;item.Image.Visibility=imageOnly;item.Video.Visibility=item.VideoPath is null?Visibility.Collapsed:Visibility.Visible;item.Markup.Visibility=Visibility.Visible;item.TextOverlays.Visibility=item.TextSelection.Visibility=imageOnly;item.AiAnnotations.Visibility=Visibility.Visible;}
        var index=_selections.IndexOf(selected);if(index>=0)Select(index);RefreshSelectionNumbers();UpdateReferenceChips();ShowToolbar();PositionPromptBar();SetPromptBarHidden(false);PromptStatus.Text=interactionRestored?status:"窗口交互恢复失败，正在安全关闭覆盖层，请重新截图";CrashDiagnosticsService.MarkOperation("屏幕助手：等待操作");if(!interactionRestored)_=Dispatcher.BeginInvoke(DispatcherPriority.Send,new Action(Close));
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
        if(_recordingSession is not { } session||_recordingItem is not { } item||_recordingStopping)return;CrashDiagnosticsService.MarkOperation("屏幕助手：停止并封装区域录屏");_recordingStopping=true;_recordingTimer.Stop();RecordingTime.Text="处理中…";try{session.Stop();StartRecordingStopWatchdog(session,item);}catch(Exception ex){FailRecording(session,item,ex.Message);}
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
        CrashDiagnosticsService.MarkOperation("屏幕助手：启动录屏原位预览");
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
        preview.FramePresented+=position=>
        {
            if(_closed||!_selections.Contains(item)||item.AnnotationNotes.Count==0)return;
            // FramePresented carries the timestamp captured together with the
            // decoded pixels. Keep annotations bound to that actual frame for
            // ordinary play/pause as well as answer-action playback.
            RenderAnnotationsForItem(item,position.TotalSeconds);
        };
        item.VideoPreview=preview;
        return preview;
    }
    private void FailRecording(RecordingSession session,SelectionItem item,string error)
    {
        if(!IsCurrentRecording(session,item))return;CancelRecordingStopWatchdog();_recordingTimer.Stop();var wasReferenced=_recordingItemWasReferenced;_recordingSession=null;_recordingItem=null;_recordingItemWasReferenced=false;try{session.Dispose();}catch(Exception ex){new PrivacyLogger().Error("RecordingDispose",ex);}var stillPresent=_selections.Contains(item);if(stillPresent)CaptureOverlayPolicy.RestoreRecordingReference(_references,item,wasReferenced);else _references.Remove(item);ResetFailedVideoPreview(item);if(stillPresent)ExitRecordingMode(item);PromptStatus.Text=$"录屏失败：{error}";CrashDiagnosticsService.MarkOperation("屏幕助手：录屏失败后等待操作");
    }
    private static void ResetFailedVideoPreview(SelectionItem item)
    {
        CancelVideoAnnotationPlayback(item);
        try{item.VideoPreview?.CloseSource();}catch(Exception ex){new PrivacyLogger().Error("RecordingPreviewReset",ex);}
        item.Video.Visibility=Visibility.Collapsed;item.Image.Visibility=Visibility.Visible;item.VideoLease?.Dispose();item.VideoLease=null;item.VideoPath=null;item.VideoDuration=TimeSpan.Zero;item.VideoPlaying=false;
    }
    private static void ClearImageOnlyLayers(SelectionItem item){item.Markup.Strokes.Clear();item.Markup.Children.Clear();item.DrawingElements.Clear();item.DrawingOrder.Clear();item.DrawingRedo.Clear();item.NextDrawingNumber=1;item.TextLayer=NoTextLayerState.Instance;item.AnnotationNotes.Clear();item.TextOverlays.Children.Clear();item.AiAnnotations.Children.Clear();ClearTextSelection(item);}
    private static void InvalidateImageDerivedLayers(SelectionItem item){if(item.VideoPath is not null)return;ClearImageOnlyLayers(item);}
    private bool IsCurrentRecording(RecordingSession session,SelectionItem item)=>ReferenceEquals(_recordingSession,session)&&ReferenceEquals(_recordingItem,item);
    private void ExitRecordingMode(SelectionItem selected)
    {
        _recordingMode=_recordingPaused=_recordingStopping=false;ClearRecordingVisualHole();RecordingBar.Visibility=Visibility.Collapsed;PromptBarHost.Visibility=_conversationAiAvailable?Visibility.Visible:Visibility.Collapsed;Cursor=Cursors.Cross;foreach(var item in _selections){item.Host.Visibility=Visibility.Visible;var isImageOnly=item.VideoPath is null;var imageOnly=isImageOnly?Visibility.Visible:Visibility.Collapsed;item.Image.Visibility=imageOnly;item.Video.Visibility=isImageOnly?Visibility.Collapsed:Visibility.Visible;item.Markup.Visibility=Visibility.Visible;item.TextOverlays.Visibility=item.TextSelection.Visibility=imageOnly;item.AiAnnotations.Visibility=Visibility.Visible;}var index=_selections.IndexOf(selected);if(index>=0)Select(index);RefreshSelectionNumbers();UpdateReferenceChips();ShowToolbar();PositionPromptBar();SetPromptBarHidden(false);
    }
    private void ToggleVideoPlayback(object s,RoutedEventArgs e)
    {
        if(RejectIfOverlayOperationBusy()||Active is not {VideoPath:not null} item)return;
        try
        {
            CancelVideoAnnotationPlayback(item);
            var preview=EnsureVideoPreview(item);
            if(item.VideoPlaying){preview.Pause();item.VideoPlaying=false;RenderAnnotationsForItem(item,preview.LastPresentedPosition.TotalSeconds);PromptStatus.Text="视频已暂停 · 标注已保留";}
            else{preview.Play();item.VideoPlaying=true;PromptStatus.Text="视频正在原位播放";}
        }
        catch(Exception ex){new PrivacyLogger().Error("RecordingPreviewToggle",ex);item.VideoPlaying=false;PromptStatus.Text="视频预览暂不可用；仍可保存或复制视频";}
    }

    private async void OnPreviewKeyDown(object s,KeyEventArgs e)
    {
        if(e.Key==Key.Escape)
        {
            HandleEscape();
            e.Handled=true;return;
        }
        if(_longCaptureMode){e.Handled=true;return;}
        if(_drawingMode&&_drawTool==DrawTool.Select&&e.Key==Key.Delete)
        {
            if(!DeleteSelectedDrawingObject())PromptStatus.Text="请先点击选择要删除的标注";
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
            if(_recordingCountdownActive||_recordingMode||_overlayRequest is not null||_request is not null){PromptStatus.Text="当前操作完成后才能撤销或重做";e.Handled=true;return;}
            if(_drawingMode){if(undo)DrawUndo(s,new());else DrawRedo(s,new());}
            else if(undo)UndoOverlayOperation();else RedoOverlayOperation();
            e.Handled=true;return;
        }
        if(Keyboard.FocusedElement is RichTextBox richTextBox&&_selections.Any(item=>IsInside(richTextBox,item.TextSelection)))
        {
            if(e.Key==Key.C&&Keyboard.Modifiers.HasFlag(ModifierKeys.Control)&&!richTextBox.Selection.IsEmpty){CopyTextToClipboard(new TextRange(richTextBox.Selection.Start,richTextBox.Selection.End).Text.TrimEnd('\r','\n'));e.Handled=true;}return;
        }
        if(Keyboard.FocusedElement is TextBox or ButtonBase)return;
        if(_recordingCountdownActive||_recordingMode||_drawingMode)return;
        if(_overlayRequest is not null||_request is not null){e.Handled=true;return;}
        if(e.Key==Key.Delete&&Active is not null){var before=CaptureOverlaySnapshot();RemoveActiveSelection(true);RecordOverlayOperation(before,"删除截图区域");e.Handled=true;return;}
        if(Active is not {IsImplicit:false} item)return;var step=Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)?10:1;
        if(e.Key is Key.Left or Key.Right or Key.Up or Key.Down){var before=CaptureOverlaySnapshot();var next=ClampSelection(new Rect(item.Bounds.X+(e.Key==Key.Left?-step:e.Key==Key.Right?step:0),item.Bounds.Y+(e.Key==Key.Up?-step:e.Key==Key.Down?step:0),item.Bounds.Width,item.Bounds.Height));if(CaptureOverlayPolicy.HasContentGeometryChanged(item.Bounds,next))InvalidateImageDerivedLayers(item);item.Bounds=next;UpdateSelection(item);RecordGeometryOperationIfChanged(before,"移动截图区域");PositionPromptBar();ShowToolbar();e.Handled=true;return;}
        if(Keyboard.Modifiers!=ModifierKeys.None)return;
        if(item.VideoPath is not null&&(e.Key==Key.T||e.Key==Key.O)){PromptStatus.Text="视频区域不支持 OCR/翻译，请先选择截图区域";e.Handled=true;return;}
        if(e.Key==Key.C)Copy(s,new());else if(e.Key==Key.S)Save(s,new());else if(e.Key==Key.P)Pin(s,new());else if(e.Key==Key.D)Draw(s,new());else if(e.Key==Key.T)Translate(s,new());else if(e.Key==Key.O)Ocr(s,new());else if(e.Key==Key.R)Record(s,new());else if(e.Key==Key.Enter){e.Handled=true;await SendAsync(true);return;}else return;e.Handled=true;
    }

    private void HandleEscape()
    {
        if(_longCaptureMode)CancelLongCaptureSession("已取消长截图");
        else if(ReferencePicker.IsOpen){ReferencePicker.IsOpen=false;_referenceMentionStart=-1;}
        else if(_recordingCountdownActive){CancelRecordingCountdown();PromptStatus.Text="正在取消录屏倒计时…";}
        else if(_recordingMode)StopRecording(this,new RoutedEventArgs());
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
    }
}
