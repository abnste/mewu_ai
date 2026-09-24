// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Animation;

namespace mewu_ai_Assistant.Views;

// Presentation only: never added to the annotation model or exported pixels.
internal sealed class AnnotationBusyLayer(IEnumerable<Grid> targets):IDisposable
{
    private readonly Grid[] _targets=targets.Distinct().ToArray();
    private readonly List<(Grid Host,Border Layer,RotateTransform Rotation)> _layers=[];
    private bool _disposed;
    internal void Show()
    {
        if(_disposed||_layers.Count>0)return;
        foreach(var target in _targets)
        {
            var rotation=new RotateTransform();
            var spinner=new System.Windows.Shapes.Path{Width=26,Height=26,Stroke=Brushes.White,StrokeThickness=3,
                Data=Geometry.Parse("M 13,2 A 11,11 0 1 1 2,13"),RenderTransform=rotation,RenderTransformOrigin=new Point(.5,.5)};
            var content=new StackPanel{HorizontalAlignment=HorizontalAlignment.Center,VerticalAlignment=VerticalAlignment.Center};
            content.Children.Add(spinner);
            content.Children.Add(new TextBlock{Text="AI标注中",Foreground=Brushes.White,FontSize=15,Margin=new Thickness(0,8,0,0)});
            var layer=new Border{Background=new SolidColorBrush(Color.FromArgb(115,0,0,0)),Child=content,IsHitTestVisible=false,ClipToBounds=true};
            Panel.SetZIndex(layer,10000);target.Children.Add(layer);_layers.Add((target,layer,rotation));
            rotation.BeginAnimation(RotateTransform.AngleProperty,new DoubleAnimation(0,360,TimeSpan.FromSeconds(.9)){RepeatBehavior=RepeatBehavior.Forever});
        }
    }
    internal void Hide()
    {
        foreach(var (host,layer,rotation) in _layers){rotation.BeginAnimation(RotateTransform.AngleProperty,null);host.Children.Remove(layer);}
        _layers.Clear();
    }
    public void Dispose(){Hide();_disposed=true;}
}
