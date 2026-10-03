// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System;
using System.Globalization;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Data;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Effects;
using Path=System.Windows.Shapes.Path;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

/// <summary>
/// Presentation-only, source-time controls. The owner performs seeks, owns
/// history and restores playback when InteractionCompleted reports cancellation.
/// </summary>
internal sealed class VideoTrimBar : Border
{
    private enum Interaction { None, Seek, Start, End, Range }
    private const double TrackInset=12;
    private static readonly Brush Accent=new SolidColorBrush(Color.FromRgb(100,116,239));
    private static readonly Brush Muted=new SolidColorBrush(Color.FromRgb(82,99,122));
    private static readonly Geometry PlayGeometry=Geometry.Parse("M5,2 L16,9 L5,16 Z");
    private static readonly Geometry PauseGeometry=Geometry.Parse("M4,2 L7,2 L7,16 L4,16 Z M11,2 L14,2 L14,16 L11,16 Z");
    private readonly Canvas _track=new(){Height=38,Background=Brushes.Transparent,Focusable=true};
    private readonly Border _rail=new(){Height=4,CornerRadius=new CornerRadius(2),Background=new SolidColorBrush(Color.FromRgb(222,229,240)),IsHitTestVisible=false};
    private readonly Border _range=new(){Height=4,CornerRadius=new CornerRadius(2),Background=new LinearGradientBrush(Color.FromRgb(111,124,245),Color.FromRgb(83,101,233),0),IsHitTestVisible=false};
    private readonly Thumb _startThumb=CreateThumb(20,32,Accent);
    private readonly Thumb _endThumb=CreateThumb(20,32,Accent);
    private readonly Thumb _positionThumb=CreateThumb(16,36,Muted,position:true);
    private readonly TextBlock _times=new(){FontSize=12,FontFamily=new FontFamily("Segoe UI"),Foreground=Muted,VerticalAlignment=VerticalAlignment.Center,TextTrimming=TextTrimming.CharacterEllipsis};
    private readonly TextBlock _clock=new(){FontSize=13,FontWeight=FontWeights.SemiBold,Foreground=Muted,TextTrimming=TextTrimming.CharacterEllipsis};
    private readonly TextBlock _retained=new(){FontSize=12,Foreground=new SolidColorBrush(Color.FromRgb(100,115,139)),TextTrimming=TextTrimming.CharacterEllipsis,Margin=new Thickness(0,3,0,0)};
    private readonly Grid _layout=new();
    private readonly StackPanel _actions=new(){Orientation=Orientation.Horizontal,HorizontalAlignment=HorizontalAlignment.Right};
    private readonly Border _separator=new(){Width=1,Background=new SolidColorBrush(Color.FromRgb(226,232,241)),Margin=new Thickness(5,7,5,7)};
    private readonly Path _playIcon=CreateIcon(PlayGeometry,filled:true);
    private readonly Button _play,_setStart,_setEnd,_reset;
    private TimeSpan _duration,_start,_end,_position;
    private TimeSpan _beforeStart,_beforeEnd,_beforePosition;
    private bool _playing,_canTrim,_finishing;
    private Interaction _interaction;
    private Thumb? _dragThumb;
    private double _dragOffset;

    internal event Action<TimeSpan,bool>? SeekRequested;
    internal event Action<TimeSpan,TimeSpan,bool>? RangeChanged;
    internal event Action? InteractionStarted;
    internal event Action<bool>? InteractionCompleted;
    internal event Action? TogglePlaybackRequested;

    internal bool IsInteracting=>_interaction!=Interaction.None;
    internal bool IsTrimInteraction=>_interaction is Interaction.Start or Interaction.End or Interaction.Range;

    internal VideoTrimBar()
    {
        MinWidth=300;
        Background=Brushes.Transparent;
        ClipToBounds=false;
        Focusable=false;
        SnapsToDevicePixels=true;
        UseLayoutRounding=true;
        InputMethod.SetIsInputMethodEnabled(this,false);
        InputMethod.SetIsInputMethodEnabled(_track,false);
        AutomationProperties.SetName(this,T("视频进度与裁切","Video progress and trim"));
        AutomationProperties.SetName(_track,T("视频进度","Video position"));
        _track.Children.Add(_rail);
        _track.Children.Add(_range);
        _track.Children.Add(_positionThumb);
        _track.Children.Add(_startThumb);
        _track.Children.Add(_endThumb);
        ConfigureThumb(_startThumb,Interaction.Start,T("裁切起点","Trim start"));
        ConfigureThumb(_endThumb,Interaction.End,T("裁切终点","Trim end"));
        ConfigureThumb(_positionThumb,Interaction.Seek,T("播放位置","Playback position"));
        _play=CreateButton(_playIcon,T("播放或暂停视频","Play or pause video"),()=>TogglePlaybackRequested?.Invoke());
        _setStart=CreateButton(CreateIcon(Geometry.Parse("M4,2 L4,16 M15,9 L7,9 M10,5 L6,9 L10,13")),T("将当前位置设为起点","Set current position as start"),()=>ChangeRange(Clamp(_position,TimeSpan.Zero,_end-MinimumRange),_end));
        _setEnd=CreateButton(CreateIcon(Geometry.Parse("M14,2 L14,16 M3,9 L11,9 M8,5 L12,9 L8,13")),T("将当前位置设为终点","Set current position as end"),()=>ChangeRange(_start,Clamp(_position,_start+MinimumRange,_duration)));
        _reset=CreateButton(CreateIcon(Geometry.Parse("M3,7 A6.5,6.5 0 1 1 3.8,13 M3,2 L3,7 L8,7")),T("恢复完整视频","Restore full video"),()=>ChangeRange(TimeSpan.Zero,_duration));
        _actions.Children.Add(_setStart);_actions.Children.Add(_setEnd);_actions.Children.Add(_reset);
        _layout.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        _layout.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});
        _layout.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        _layout.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        _layout.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        _layout.RowDefinitions.Add(new(){Height=GridLength.Auto});
        _layout.RowDefinitions.Add(new(){Height=GridLength.Auto});
        var clockPanel=new StackPanel{MinWidth=104,MaxWidth=124,VerticalAlignment=VerticalAlignment.Center,Margin=new Thickness(8,0,4,0)};
        clockPanel.Children.Add(_clock);clockPanel.Children.Add(_retained);
        _layout.Children.Add(_play);_layout.Children.Add(_track);_layout.Children.Add(clockPanel);
        _layout.Children.Add(_times);_layout.Children.Add(_separator);_layout.Children.Add(_actions);
        Grid.SetColumn(_track,1);Grid.SetColumn(clockPanel,2);Grid.SetRowSpan(clockPanel,2);
        Grid.SetColumn(_separator,3);Grid.SetRowSpan(_separator,2);
        var layers=new Grid{ClipToBounds=false};
        // Keep the effect off the crisp border and content, just like Toolbar.
        layers.Children.Add(new Border{Background=Brushes.White,CornerRadius=new CornerRadius(18),IsHitTestVisible=false,
            Effect=new DropShadowEffect{Color=Color.FromRgb(51,71,98),BlurRadius=24,ShadowDepth=6,Opacity=.24}});
        layers.Children.Add(new Border{Background=new SolidColorBrush(Color.FromArgb(250,255,255,255)),BorderBrush=new SolidColorBrush(Color.FromRgb(215,225,239)),
            BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(18),Padding=new Thickness(6),Child=_layout});
        Child=layers;
        Loaded+=(_,_)=>
        {
            foreach(var button in new[]{_play,_setStart,_setEnd,_reset})
                if(button.TryFindResource("ToolbarIconButton") is Style)button.SetResourceReference(StyleProperty,"ToolbarIconButton");
        };
        UpdateResponsiveLayout(300);
        _track.SizeChanged+=(_,_)=>RefreshVisuals();
        _track.MouseLeftButtonDown+=TrackMouseDown;
        _track.MouseMove+=TrackMouseMove;
        _track.MouseLeftButtonUp+=TrackMouseUp;
        _track.LostMouseCapture+=TrackLostCapture;
        PreviewKeyDown+=HandleKeyDown;
        MouseLeftButtonDown+=(_,e)=>e.Handled=true;
        MouseLeftButtonUp+=(_,e)=>e.Handled=true;
        // Idle movement still reaches the owner so it can dismiss the pixel
        // inspector and maintain the toolbar's hover zone. Only an active
        // timeline gesture owns movement exclusively.
        MouseMove+=(_,e)=>{if(IsInteracting)e.Handled=true;};
        MouseWheel+=(_,e)=>e.Handled=true;
        Unloaded+=(_,_)=>CancelInteraction();
        IsEnabledChanged+=(_,_)=>{if(!IsEnabled)CancelInteraction();};
        RefreshVisuals();
    }

    internal void SetState(TimeSpan duration,TimeSpan start,TimeSpan end,TimeSpan position,bool playing,bool canTrim)
    {
        Dispatcher.VerifyAccess();
        // Frame delivery and asynchronous seeks may report an older position
        // while the user is dragging. Only a later idle update can replace it.
        if(IsInteracting){_playing=playing;RefreshVisuals();return;}
        _duration=duration>TimeSpan.Zero?duration:TimeSpan.Zero;
        _start=Clamp(start,TimeSpan.Zero,_duration);
        _end=Clamp(end,TimeSpan.Zero,_duration);
        if(_end-_start<MinimumRange)
        {
            _start=Clamp(_start,TimeSpan.Zero,_duration-MinimumRange);
            _end=_start+MinimumRange;
        }
        _position=Clamp(position,TimeSpan.Zero,_duration);
        _playing=playing;
        _canTrim=canTrim;
        RefreshVisuals();
    }

    protected override Size MeasureOverride(Size constraint)
    {
        // Decide before measuring so the owner receives the correct height
        // when positioning the bar beside a screen edge.
        UpdateResponsiveLayout(double.IsFinite(constraint.Width)?constraint.Width:double.IsFinite(Width)?Width:720);
        return base.MeasureOverride(constraint);
    }

    private void UpdateResponsiveLayout(double width)
    {
        var narrow=width<540;
        Grid.SetRowSpan(_play,narrow?1:2);
        Grid.SetRowSpan(_layout.Children[2],narrow?1:2);
        Grid.SetRow(_times,1);Grid.SetColumn(_times,narrow?0:1);Grid.SetColumnSpan(_times,narrow?2:1);
        _times.Margin=narrow?new Thickness(10,0,8,0):new Thickness(12,0,12,1);
        Grid.SetRow(_actions,narrow?1:0);Grid.SetColumn(_actions,narrow?2:4);
        Grid.SetRowSpan(_actions,narrow?1:2);Grid.SetColumnSpan(_actions,narrow?3:1);
        _separator.Visibility=narrow?Visibility.Collapsed:Visibility.Visible;
    }

    internal void CancelInteraction()
    {
        Dispatcher.VerifyAccess();
        CompleteInteraction(cancelled:true);
    }

    private TimeSpan MinimumRange=>TimeSpan.FromTicks(Math.Min(TimeSpan.TicksPerMillisecond*100,_duration.Ticks));
    private double TrackWidth=>Math.Max(1,_track.ActualWidth-TrackInset*2);
    private double ToX(TimeSpan value)=>TrackInset+(_duration.Ticks>0?value.Ticks/(double)_duration.Ticks*TrackWidth:0);
    private TimeSpan FromX(double value)
    {
        if(!double.IsFinite(value)||_duration<=TimeSpan.Zero)return TimeSpan.Zero;
        var fraction=Math.Clamp((value-TrackInset)/TrackWidth,0,1);
        if(fraction<=0)return TimeSpan.Zero;
        if(fraction>=1)return _duration;
        return TimeSpan.FromTicks((long)(fraction*_duration.Ticks));
    }

    private static TimeSpan Clamp(TimeSpan value,TimeSpan minimum,TimeSpan maximum)
        =>TimeSpan.FromTicks(Math.Clamp(value.Ticks,minimum.Ticks,Math.Max(minimum.Ticks,maximum.Ticks)));

    private bool BeginInteraction(Interaction interaction,Thumb? thumb=null)
    {
        if(IsInteracting||_finishing||!IsEnabled||_duration<=TimeSpan.Zero||interaction!=Interaction.Seek&&!_canTrim)return false;
        _beforeStart=_start;_beforeEnd=_end;_beforePosition=_position;
        _interaction=interaction;
        _dragThumb=thumb;
        _dragOffset=thumb is null?0:Mouse.GetPosition(_track).X-ToX(interaction==Interaction.Start?_start:interaction==Interaction.End?_end:_position);
        // Mouse capture can synchronously re-enter input handlers. The state
        // and original values must be established before Thumb captures.
        try{InteractionStarted?.Invoke();}
        catch{CancelInteraction();throw;}
        return IsInteracting&&!_finishing;
    }

    private void ConfigureThumb(Thumb thumb,Interaction interaction,string name)
    {
        AutomationProperties.SetName(thumb,name);
        thumb.ToolTip=name;
        thumb.PreviewMouseLeftButtonDown+=(_,e)=>
        {
            if(!BeginInteraction(interaction,thumb)){e.Handled=true;return;}
            thumb.Focus();
        };
        thumb.DragStarted+=(_,e)=>
        {
            if(!IsInteracting&&!BeginInteraction(interaction,thumb))thumb.CancelDrag();
            e.Handled=true;
        };
        thumb.DragDelta+=(_,e)=>
        {
            if(ReferenceEquals(_dragThumb,thumb)&&!_finishing)UpdateInteraction(FromX(Mouse.GetPosition(_track).X-_dragOffset));
            e.Handled=true;
        };
        thumb.DragCompleted+=(_,e)=>
        {
            if(ReferenceEquals(_dragThumb,thumb)&&!_finishing)
            {
                if(!e.Canceled)UpdateInteraction(FromX(Mouse.GetPosition(_track).X-_dragOffset));
                CompleteInteraction(e.Canceled);
            }
            e.Handled=true;
        };
        thumb.LostMouseCapture+=(_,_)=>
        {
            // On a normal release Thumb finishes its drag before raising its
            // completed event. Unexpected capture loss cancels via Thumb too.
            if(ReferenceEquals(_dragThumb,thumb)&&!_finishing&&thumb.IsDragging)CompleteInteraction(cancelled:true);
        };
    }

    private void TrackMouseDown(object sender,MouseButtonEventArgs e)
    {
        if(FindThumb(e.OriginalSource as DependencyObject) is not null)return;
        e.Handled=true;
        if(!BeginInteraction(Interaction.Seek))return;
        _track.Focus();
        if(!_track.CaptureMouse()){CompleteInteraction(cancelled:true);return;}
        if(IsInteracting)UpdateInteraction(FromX(e.GetPosition(_track).X));
    }

    private void TrackMouseMove(object sender,MouseEventArgs e)
    {
        if(_interaction!=Interaction.Seek||_dragThumb is not null||!_track.IsMouseCaptured||_finishing)return;
        e.Handled=true;
        if(e.LeftButton!=MouseButtonState.Pressed){CompleteInteraction(cancelled:true);return;}
        UpdateInteraction(FromX(e.GetPosition(_track).X));
    }

    private void TrackMouseUp(object sender,MouseButtonEventArgs e)
    {
        if(_interaction!=Interaction.Seek||_dragThumb is not null||!_track.IsMouseCaptured)return;
        e.Handled=true;
        UpdateInteraction(FromX(e.GetPosition(_track).X));
        CompleteInteraction(cancelled:false);
    }

    private void TrackLostCapture(object sender,MouseEventArgs e)
    {
        if(_dragThumb is null&&ReferenceEquals(e.OriginalSource,_track)&&IsInteracting&&!_finishing)CompleteInteraction(cancelled:true);
    }

    private void UpdateInteraction(TimeSpan position)
    {
        if(!IsInteracting||_finishing)return;
        if(_interaction==Interaction.Seek)
        {
            _position=Clamp(position,TimeSpan.Zero,_duration);
            RefreshVisuals();
            SeekRequested?.Invoke(_position,false);
        }
        else
        {
            if(_interaction==Interaction.Start)_start=Clamp(position,TimeSpan.Zero,_end-MinimumRange);
            else if(_interaction==Interaction.End)_end=Clamp(position,_start+MinimumRange,_duration);
            RefreshVisuals();
            RangeChanged?.Invoke(_start,_end,false);
        }
    }

    private void CompleteInteraction(bool cancelled)
    {
        if(!IsInteracting||_finishing)return;
        _finishing=true;
        try
        {
            if(cancelled){_start=_beforeStart;_end=_beforeEnd;_position=_beforePosition;}
            if(_dragThumb is {IsDragging:true} thumb)thumb.CancelDrag();
            if(_track.IsMouseCaptured)_track.ReleaseMouseCapture();
            RefreshVisuals();
            try
            {
                if(!cancelled)
                {
                    if(IsTrimInteraction)RangeChanged?.Invoke(_start,_end,true);
                    else SeekRequested?.Invoke(_position,true);
                }
            }
            finally{InteractionCompleted?.Invoke(cancelled);}
        }
        finally{_interaction=Interaction.None;_dragThumb=null;_finishing=false;RefreshVisuals();}
    }

    private void ChangeRange(TimeSpan start,TimeSpan end)
    {
        if(!_canTrim||start==_start&&end==_end||!BeginInteraction(Interaction.Range))return;
        _start=start;_end=end;
        CompleteInteraction(cancelled:false);
    }

    private void HandleKeyDown(object sender,KeyEventArgs e)
    {
        if(e.Key==Key.Escape&&IsInteracting){CancelInteraction();e.Handled=true;return;}
        if(IsInteracting){e.Handled=true;return;}
        var modifiers=Keyboard.Modifiers;
        if((modifiers&~ModifierKeys.Shift)!=ModifierKeys.None)return;
        if(e.Key==Key.Space)
        {
            if(_duration>TimeSpan.Zero)TogglePlaybackRequested?.Invoke();
            e.Handled=true;return;
        }
        if(e.Key is not (Key.Left or Key.Right or Key.Up or Key.Down or Key.Home or Key.End))return;
        var focused=Keyboard.FocusedElement;
        var interaction=ReferenceEquals(focused,_startThumb)?Interaction.Start:ReferenceEquals(focused,_endThumb)?Interaction.End:Interaction.Seek;
        if(focused is Button)return;
        e.Handled=true;
        if(!BeginInteraction(interaction))return;
        var current=interaction==Interaction.Start?_start:interaction==Interaction.End?_end:_position;
        var step=TimeSpan.FromMilliseconds(modifiers.HasFlag(ModifierKeys.Shift)?1000:100);
        var next=e.Key==Key.Home?TimeSpan.Zero:e.Key==Key.End?_duration:
            e.Key is Key.Left or Key.Down?TimeSpan.FromTicks(Math.Max(0,current.Ticks-step.Ticks)):
            TimeSpan.FromTicks(current.Ticks+Math.Min(step.Ticks,Math.Max(0,_duration.Ticks-current.Ticks)));
        UpdateInteraction(next);
        CompleteInteraction(cancelled:false);
    }

    private void RefreshVisuals()
    {
        var start=ToX(_start);var end=ToX(_end);
        Canvas.SetLeft(_rail,TrackInset);Canvas.SetTop(_rail,17);_rail.Width=TrackWidth;
        Canvas.SetLeft(_range,start);Canvas.SetTop(_range,17);_range.Width=Math.Max(0,end-start);
        PlaceThumb(_startThumb,start,3);PlaceThumb(_endThumb,end,3);PlaceThumb(_positionThumb,ToX(_position),1);
        var ready=_duration>TimeSpan.Zero;
        _startThumb.IsEnabled=_endThumb.IsEnabled=ready&&_canTrim;
        _startThumb.Opacity=_endThumb.Opacity=ready&&_canTrim?1:.4;
        _positionThumb.IsEnabled=_track.IsEnabled=ready;
        _play.IsEnabled=ready;
        _setStart.IsEnabled=_setEnd.IsEnabled=_reset.IsEnabled=ready&&_canTrim&&!IsInteracting;
        _playIcon.Data=_playing?PauseGeometry:PlayGeometry;
        _play.ToolTip=T(_playing?"暂停视频":"播放视频",_playing?"Pause video":"Play video");
        _clock.Text=$"{Format(_position)} / {Format(_duration)}";
        _clock.ToolTip=_clock.Text;
        _retained.Text=$"{T("保留","Keep")} {Format(_end-_start)}";
        _times.Text=$"{Format(_start)} – {Format(_end)}";
        _times.ToolTip=$"{T("裁切起点","Trim start")} {Format(_start)} · {T("裁切终点","Trim end")} {Format(_end)}";
        _startThumb.ToolTip=$"{T("裁切起点","Trim start")} {Format(_start)}";
        _endThumb.ToolTip=$"{T("裁切终点","Trim end")} {Format(_end)}";
        _positionThumb.ToolTip=$"{T("播放位置","Playback position")} {Format(_position)}";
    }

    private static void PlaceThumb(Thumb thumb,double center,double top)
    {
        Canvas.SetLeft(thumb,center-thumb.Width/2);Canvas.SetTop(thumb,top);
    }

    private static Thumb? FindThumb(DependencyObject? element)
    {
        while(element is not null)
        {
            if(element is Thumb thumb)return thumb;
            element=element is Visual?VisualTreeHelper.GetParent(element):null;
        }
        return null;
    }

    private static Thumb CreateThumb(double width,double height,Brush fill,bool position=false)
    {
        var hitTarget=new FrameworkElementFactory(typeof(Grid));
        hitTarget.SetValue(Panel.BackgroundProperty,Brushes.Transparent);
        var border=new FrameworkElementFactory(typeof(Border)){Name="Surface"};
        border.SetValue(FrameworkElement.WidthProperty,position?5d:12d);
        border.SetValue(FrameworkElement.HorizontalAlignmentProperty,HorizontalAlignment.Center);
        border.SetValue(FrameworkElement.MarginProperty,new Thickness(0,position?1:3,0,position?1:3));
        border.SetValue(Border.BackgroundProperty,fill);
        border.SetValue(Border.BorderBrushProperty,Brushes.White);
        border.SetValue(Border.BorderThicknessProperty,new Thickness(1));
        border.SetValue(Border.CornerRadiusProperty,new CornerRadius(position?2.5:4));
        if(!position)
        {
            var grip=new FrameworkElementFactory(typeof(Path));
            grip.SetValue(Path.DataProperty,Geometry.Parse("M4,7 L4,17 M7,7 L7,17"));
            grip.SetValue(Path.StrokeProperty,Brushes.White);grip.SetValue(Path.StrokeThicknessProperty,1d);
            grip.SetValue(Path.StrokeStartLineCapProperty,PenLineCap.Round);grip.SetValue(Path.StrokeEndLineCapProperty,PenLineCap.Round);
            border.AppendChild(grip);
        }
        hitTarget.AppendChild(border);
        var template=new ControlTemplate(typeof(Thumb)){VisualTree=hitTarget};
        var hover=new Trigger{Property=IsMouseOverProperty,Value=true};
        hover.Setters.Add(new Setter(Border.BorderBrushProperty,new SolidColorBrush(Color.FromRgb(183,196,248)),"Surface"));
        template.Triggers.Add(hover);
        var focused=new Trigger{Property=IsKeyboardFocusedProperty,Value=true};
        focused.Setters.Add(new Setter(Border.BorderBrushProperty,new SolidColorBrush(Color.FromRgb(19,42,82)),"Surface"));
        focused.Setters.Add(new Setter(Border.BorderThicknessProperty,new Thickness(2),"Surface"));
        template.Triggers.Add(focused);
        var thumb=new Thumb{Width=width,Height=height,Template=template,Focusable=true,Cursor=Cursors.SizeWE};
        InputMethod.SetIsInputMethodEnabled(thumb,false);
        return thumb;
    }

    private static Path CreateIcon(Geometry geometry,bool filled=false)
    {
        var icon=new Path{Width=18,Height=18,Data=geometry,Stretch=Stretch.Uniform,StrokeThickness=1.7,
            StrokeStartLineCap=PenLineCap.Round,StrokeEndLineCap=PenLineCap.Round,StrokeLineJoin=PenLineJoin.Round,IsHitTestVisible=false};
        icon.SetBinding(filled?Path.FillProperty:Path.StrokeProperty,new Binding(nameof(Control.Foreground))
        {RelativeSource=new RelativeSource(RelativeSourceMode.FindAncestor,typeof(Button),1)});
        return icon;
    }

    private static Button CreateButton(UIElement icon,string name,Action action)
    {
        var button=new Button{Content=icon,ToolTip=name,Style=CreateFallbackButtonStyle(),VerticalAlignment=VerticalAlignment.Center};
        InputMethod.SetIsInputMethodEnabled(button,false);
        AutomationProperties.SetName(button,name);
        button.Click+=(_,e)=>{action();e.Handled=true;};
        return button;
    }

    private static Style CreateFallbackButtonStyle()
    {
        // Isolated previews may have no Window.Resources. Real overlays replace
        // this fallback with their shared ToolbarIconButton resource on Loaded.
        var surface=new FrameworkElementFactory(typeof(Border)){Name="ToolIcon"};
        surface.SetBinding(Border.BackgroundProperty,new Binding(nameof(Control.Background)){RelativeSource=RelativeSource.TemplatedParent});
        surface.SetValue(Border.BorderBrushProperty,new SolidColorBrush(Color.FromRgb(110,131,242)));
        surface.SetValue(Border.CornerRadiusProperty,new CornerRadius(12));
        var content=new FrameworkElementFactory(typeof(ContentPresenter));
        content.SetValue(FrameworkElement.HorizontalAlignmentProperty,HorizontalAlignment.Center);
        content.SetValue(FrameworkElement.VerticalAlignmentProperty,VerticalAlignment.Center);
        surface.AppendChild(content);
        var template=new ControlTemplate(typeof(Button)){VisualTree=surface};
        var hover=new Trigger{Property=IsMouseOverProperty,Value=true};
        hover.Setters.Add(new Setter(Border.BackgroundProperty,new SolidColorBrush(Color.FromRgb(234,241,250)),"ToolIcon"));
        var focus=new Trigger{Property=IsKeyboardFocusedProperty,Value=true};
        focus.Setters.Add(new Setter(Border.BackgroundProperty,new SolidColorBrush(Color.FromRgb(234,241,250)),"ToolIcon"));
        focus.Setters.Add(new Setter(Border.BorderThicknessProperty,new Thickness(2),"ToolIcon"));
        var pressed=new Trigger{Property=Button.IsPressedProperty,Value=true};
        pressed.Setters.Add(new Setter(OpacityProperty,.68,"ToolIcon"));
        var disabled=new Trigger{Property=IsEnabledProperty,Value=false};
        disabled.Setters.Add(new Setter(OpacityProperty,.4,"ToolIcon"));
        template.Triggers.Add(hover);template.Triggers.Add(focus);template.Triggers.Add(pressed);template.Triggers.Add(disabled);
        var style=new Style(typeof(Button));
        style.Setters.Add(new Setter(WidthProperty,38d));style.Setters.Add(new Setter(HeightProperty,38d));
        style.Setters.Add(new Setter(MarginProperty,new Thickness(2)));style.Setters.Add(new Setter(Control.PaddingProperty,new Thickness(0)));
        style.Setters.Add(new Setter(Control.ForegroundProperty,Muted));style.Setters.Add(new Setter(Control.BackgroundProperty,Brushes.Transparent));
        style.Setters.Add(new Setter(Control.BorderThicknessProperty,new Thickness(0)));style.Setters.Add(new Setter(CursorProperty,Cursors.Hand));
        style.Setters.Add(new Setter(Control.TemplateProperty,template));
        return style;
    }

    private static string Format(TimeSpan value)=>value.ToString(value.TotalHours>=1?@"h\:mm\:ss\.f":@"mm\:ss\.f",CultureInfo.InvariantCulture);
    private static string T(string chinese,string english)=>LocalizationService.T(chinese,english);
}
