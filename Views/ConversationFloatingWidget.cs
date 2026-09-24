// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Globalization;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Shapes;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

internal enum ConversationWidgetState { Idle, Thinking, Completed, Canceled, Failed }

/// <summary>A small restore surface for a minimized screenshot conversation.</summary>
internal sealed class ConversationFloatingWidget : Window
{
    private readonly CaptureOverlayWindow _owner;
    private readonly TextBlock _preview=new(){Name="ConversationProgressPreview",FontSize=10.5,Margin=new Thickness(0,1,0,0),TextTrimming=TextTrimming.CharacterEllipsis};
    private readonly Canvas _spinner=new(){Name="ConversationThinkingSpinner",Width=14,Height=14,IsHitTestVisible=false};
    private readonly RotateTransform _spinnerRotation=new();
    private readonly Ellipse _statusDot=new(){Name="ConversationStatusDot",Width=7,Height=7,IsHitTestVisible=false};
    private readonly Border _statusBadge=new(){Name="ConversationStatusBadge",Width=17,Height=17,CornerRadius=new CornerRadius(9),Background=new SolidColorBrush(Color.FromRgb(249,251,255)),HorizontalAlignment=HorizontalAlignment.Center,VerticalAlignment=VerticalAlignment.Center,Visibility=Visibility.Collapsed};
    private ConversationWidgetState _state;
    private string _previewContent=string.Empty;
    private bool _closed;
    private Point? _pressStart;

    internal ConversationFloatingWidget(CaptureOverlayWindow owner)
    {
        _owner=owner??throw new ArgumentNullException(nameof(owner));
        Title=LocalizationService.T("喵呜AI 对话","MewuAI conversation");
        Width=206;Height=54;MinWidth=180;MinHeight=48;
        WindowStyle=WindowStyle.None;ResizeMode=ResizeMode.NoResize;
        AllowsTransparency=true;Background=Brushes.Transparent;Topmost=true;ShowInTaskbar=false;
        ShowActivated=true;
        Content=BuildContent();
        _preview.SizeChanged+=(_,_)=>UpdatePreviewText();
        UpdateProgress(ConversationWidgetState.Idle,string.Empty);
        Loaded+=(_,_)=>{PlaceNearWorkArea();UpdateSpinnerAnimation();};
        IsVisibleChanged+=(_,_)=>{if(!IsVisible)CancelPointerGesture();UpdateSpinnerAnimation();};
        Closed+=(_,_)=>{_closed=true;_spinnerRotation.BeginAnimation(RotateTransform.AngleProperty,null);};
        MouseLeftButtonDown+=OnMouseDown;
        MouseMove+=OnPointerMove;
        MouseLeftButtonUp+=OnPointerUp;
        LostMouseCapture+=(_,_)=>_pressStart=null;
        Deactivated+=(_,_)=>CancelPointerGesture();
        PreviewKeyDown+=(_,e)=>{if(e.Key==Key.Escape&&_pressStart is not null){CancelPointerGesture();e.Handled=true;}};
    }

    private FrameworkElement BuildContent()
    {
        var shell=new Border{Background=new SolidColorBrush(Color.FromRgb(249,251,255)),BorderBrush=new SolidColorBrush(Color.FromRgb(211,220,235)),BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(18),Padding=new Thickness(9,6,9,6)};
        var row=new DockPanel{LastChildFill=true};
        var close=new Button{Name="CloseConversationButton",Style=(Style)_owner.FindResource("ConversationHeaderButton"),Width=22,Height=22,Margin=new Thickness(0),ToolTip=LocalizationService.T("关闭会话","Close conversation"),Visibility=Visibility.Hidden};
        close.Content=new System.Windows.Shapes.Path{Width=12,Height=12,Stroke=new SolidColorBrush(Color.FromRgb(113,126,151)),StrokeThickness=1.5,StrokeStartLineCap=PenLineCap.Round,StrokeEndLineCap=PenLineCap.Round,Data=Geometry.Parse("M3,3 L9,9 M9,3 L3,9")};
        System.Windows.Automation.AutomationProperties.SetName(close,LocalizationService.T("关闭会话","Close conversation"));
        MouseEnter+=(_,_)=>close.Visibility=Visibility.Visible;
        MouseLeave+=(_,_)=>close.Visibility=Visibility.Hidden;
        close.Click+=(_,e)=>{e.Handled=true;_owner.Close();};
        var statusSlot=new Grid{Width=22,Height=22,VerticalAlignment=VerticalAlignment.Top,Margin=new Thickness(5,0,0,0)};
        statusSlot.Children.Add(_statusBadge);statusSlot.Children.Add(close);Panel.SetZIndex(close,1);
        DockPanel.SetDock(statusSlot,Dock.Right);row.Children.Add(statusSlot);
        var icon=new Border{Width=28,Height=28,CornerRadius=new CornerRadius(10),Background=new SolidColorBrush(Color.FromRgb(232,237,255)),Margin=new Thickness(0,0,8,0)};
        icon.Child=new System.Windows.Shapes.Path{Width=15,Height=15,Stretch=Stretch.Uniform,Stroke=new SolidColorBrush(Color.FromRgb(82,99,217)),StrokeThickness=1.5,Data=Geometry.Parse("M2,2 L14,2 L14,11 L8,11 L4,14 L4,11 L2,11 Z M5,5 L11,5 M5,8 L9,8"),HorizontalAlignment=HorizontalAlignment.Center,VerticalAlignment=VerticalAlignment.Center};
        DockPanel.SetDock(icon,Dock.Left);row.Children.Add(icon);
        var text=new StackPanel{VerticalAlignment=VerticalAlignment.Center};
        text.Children.Add(new TextBlock{Text=LocalizationService.T("截图会话","Screenshot conversation"),Foreground=new SolidColorBrush(Color.FromRgb(45,59,84)),FontSize=12.5,FontWeight=FontWeights.SemiBold});
        _preview.Foreground=new SolidColorBrush(Color.FromRgb(116,130,153));text.Children.Add(_preview);
        row.Children.Add(text);
        shell.Child=row;
        _spinner.RenderTransform=_spinnerRotation;_spinner.RenderTransformOrigin=new Point(.5,.5);
        for(var index=0;index<8;index++)
        {
            var dot=new Ellipse{Width=2.3,Height=2.3,Fill=new SolidColorBrush(Color.FromRgb(82,99,217)),Opacity=.25+index*.1};
            var angle=index*Math.PI/4;
            Canvas.SetLeft(dot,5.85+4.6*Math.Cos(angle));Canvas.SetTop(dot,5.85+4.6*Math.Sin(angle));_spinner.Children.Add(dot);
        }
        var indicator=new Grid();indicator.Children.Add(_spinner);indicator.Children.Add(_statusDot);_statusBadge.Child=indicator;
        return shell;
    }

    internal void UpdateProgress(ConversationWidgetState state,string preview)
    {
        if(_closed)return;
        var changed=_state!=state;_state=state;
        var label=state switch
        {
            ConversationWidgetState.Thinking=>LocalizationService.T("正在思考…","Thinking…"),
            ConversationWidgetState.Completed=>LocalizationService.T("回答已完成","Answer ready"),
            ConversationWidgetState.Canceled=>LocalizationService.T("已取消","Canceled"),
            ConversationWidgetState.Failed=>LocalizationService.T("请求失败","Request failed"),
            _=>LocalizationService.T("点击恢复对话","Click to restore")
        };
        _previewContent=string.IsNullOrWhiteSpace(preview)?label:preview;
        UpdatePreviewText();
        _preview.ToolTip=label+(string.IsNullOrWhiteSpace(preview)?string.Empty:"\n"+preview);
        System.Windows.Automation.AutomationProperties.SetName(_preview,label+" "+preview);
        _statusBadge.Visibility=state==ConversationWidgetState.Idle?Visibility.Collapsed:Visibility.Visible;
        _statusBadge.ToolTip=label;
        System.Windows.Automation.AutomationProperties.SetName(_statusBadge,label);
        _spinner.Visibility=state==ConversationWidgetState.Thinking?Visibility.Visible:Visibility.Collapsed;
        _statusDot.Visibility=state is ConversationWidgetState.Idle or ConversationWidgetState.Thinking?Visibility.Collapsed:Visibility.Visible;
        _statusDot.Fill=new SolidColorBrush(state switch
        {
            ConversationWidgetState.Completed=>Color.FromRgb(34,170,106),
            ConversationWidgetState.Failed=>Color.FromRgb(214,94,94),
            _=>Color.FromRgb(142,153,169)
        });
        if(changed)UpdateSpinnerAnimation();
    }

    private void UpdatePreviewText()
    {
        var width=_preview.ActualWidth;
        if(width<=0){_preview.Text=_previewContent;return;}
        var typeface=new Typeface(_preview.FontFamily,_preview.FontStyle,_preview.FontWeight,_preview.FontStretch);
        var dpi=VisualTreeHelper.GetDpi(_preview).PixelsPerDip;
        bool Fits(string value)=>new FormattedText(value,CultureInfo.CurrentUICulture,FlowDirection.LeftToRight,
            typeface,_preview.FontSize,_preview.Foreground,dpi).WidthIncludingTrailingWhitespace<=width;
        if(Fits(_previewContent)){_preview.Text=_previewContent;return;}
        // Trim the beginning, so the newest words stay visible instead of
        // being hidden by TextBlock's ordinary trailing ellipsis.
        var elements=StringInfo.ParseCombiningCharacters(_previewContent);
        for(var index=1;index<elements.Length;index++)
        {
            var suffix="…"+_previewContent[elements[index]..];
            if(Fits(suffix)){_preview.Text=suffix;return;}
        }
        _preview.Text="…";
    }

    private void UpdateSpinnerAnimation()
    {
        var animate=!_closed&&IsVisible&&_state==ConversationWidgetState.Thinking&&SystemParameters.ClientAreaAnimation;
        if(animate==_spinnerRotation.HasAnimatedProperties)return;
        _spinnerRotation.BeginAnimation(RotateTransform.AngleProperty,animate
            ?new DoubleAnimation(0,360,TimeSpan.FromMilliseconds(900)){RepeatBehavior=RepeatBehavior.Forever}
            :null);
    }

    private void OnMouseDown(object? sender,MouseButtonEventArgs e)
    {
        for(var source=e.OriginalSource as DependencyObject;source is not null;source=VisualTreeHelper.GetParent(source))
            if(source is Button)return;
        if(e.ChangedButton!=MouseButton.Left)return;
        _pressStart=e.GetPosition(this);
        if(!CaptureMouse())_pressStart=null;
        e.Handled=true;
    }

    private void OnPointerMove(object sender,MouseEventArgs e)
    {
        if(_pressStart is not {} start)return;
        if(e.LeftButton!=MouseButtonState.Pressed){CancelPointerGesture();return;}
        if(!PinnedWindowInteractionPolicy.ShouldBeginDrag(start,e.GetPosition(this),
            SystemParameters.MinimumHorizontalDragDistance,SystemParameters.MinimumVerticalDragDistance))return;
        // The native move loop handles monitor/DPI changes and Escape. End
        // click tracking first: a drag must never restore the conversation.
        CancelPointerGesture();
        e.Handled=true;
        if(!_closed&&IsVisible&&Mouse.LeftButton==MouseButtonState.Pressed)DragMove();
    }

    private void OnPointerUp(object sender,MouseButtonEventArgs e)
    {
        if(e.ChangedButton!=MouseButton.Left||_pressStart is null)return;
        var point=e.GetPosition(this);
        CancelPointerGesture();
        e.Handled=true;
        if(!_closed&&IsVisible&&new Rect(RenderSize).Contains(point))_owner.RestoreFromConversationWidget();
    }

    private void CancelPointerGesture()
    {
        _pressStart=null;
        if(IsMouseCaptured)ReleaseMouseCapture();
    }

    private void PlaceNearWorkArea()
    {
        var area=SystemParameters.WorkArea;
        var index=Application.Current?.Windows.OfType<ConversationFloatingWidget>().Count(window=>window.IsVisible&&!ReferenceEquals(window,this))??0;
        var column=index%4;var row=index/4;
        Left=Math.Max(area.Left+8,area.Right-Width-18-column*(Width+10));
        Top=Math.Max(area.Top+8,area.Bottom-Height-22-row*(Height+10));
    }

    internal void CloseForOwnerExit()
    {
        try{Close();}catch{}
    }
}
