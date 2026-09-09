// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Animation;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

// A bounded gradient in the existing window: no blur pass, timer or input hitbox.
public sealed class ThinkingGlow : Border
{
    private readonly ScaleTransform _breath=new(1,1);
    private bool _running;
    internal bool IsRunning=>_running;
    public ThinkingGlow()
    {
        IsHitTestVisible=false;Focusable=false;Visibility=Visibility.Collapsed;
        RenderTransform=_breath;RenderTransformOrigin=new Point(.5,1);
        Unloaded+=(_,_)=>Stop();
    }
    internal void Start(string? color)
    {
        var tint=ThinkingGlowAppearance.ParseColor(color);
        var brush=new RadialGradientBrush
        {
            Center=new Point(.5,1.05),GradientOrigin=new Point(.5,1.05),RadiusX=.62,RadiusY=1.05,
            GradientStops=new GradientStopCollection
            {
                new(Color.FromArgb(155,tint.R,tint.G,tint.B),0),
                new(Color.FromArgb(90,tint.R,tint.G,tint.B),.35),
                new(Color.FromArgb(28,tint.R,tint.G,tint.B),.7),
                new(Color.FromArgb(0,tint.R,tint.G,tint.B),1)
            }
        };
        brush.Freeze();Background=brush;
        if(_running)return;
        _running=true;Visibility=Visibility.Visible;
        // Honor the Windows animation preference while retaining a quiet status glow.
        if(!SystemParameters.ClientAreaAnimation){Opacity=.7;return;}
        var cycle=TimeSpan.FromSeconds(1.8);
        BeginAnimation(OpacityProperty,new DoubleAnimation(.48,.95,cycle){AutoReverse=true,RepeatBehavior=RepeatBehavior.Forever,EasingFunction=new SineEase{EasingMode=EasingMode.EaseInOut}});
        _breath.BeginAnimation(ScaleTransform.ScaleYProperty,new DoubleAnimation(.85,1,cycle){AutoReverse=true,RepeatBehavior=RepeatBehavior.Forever,EasingFunction=new SineEase{EasingMode=EasingMode.EaseInOut}});
    }
    internal void Stop()
    {
        _running=false;BeginAnimation(OpacityProperty,null);_breath.BeginAnimation(ScaleTransform.ScaleYProperty,null);
        Visibility=Visibility.Collapsed;Opacity=1;
    }
}
