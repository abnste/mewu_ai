// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Ink;
using System.Windows.Input;

namespace mewu_ai_Assistant.Services;

internal static class EditableShapeStroke
{
    private static readonly Guid KindKey=new("0dcb4b2d-6ed6-455b-a75c-64c574c7af4e");
    internal static bool IsArrow(Stroke stroke)=>stroke.ContainsPropertyData(KindKey)&&Equals(stroke.GetPropertyData(KindKey),"arrow");

    internal static Stroke Create(Point a,Point b,string kind,DrawingAttributes attributes)
    {
        var points=new StylusPointCollection();
        if(kind=="rectangle")
        {
            points.Add(new StylusPoint(a.X,a.Y));points.Add(new StylusPoint(b.X,a.Y));points.Add(new StylusPoint(b.X,b.Y));points.Add(new StylusPoint(a.X,b.Y));points.Add(new StylusPoint(a.X,a.Y));
        }
        else if(kind=="ellipse")
        {
            var left=Math.Min(a.X,b.X);var top=Math.Min(a.Y,b.Y);var rx=Math.Abs(b.X-a.X)/2;var ry=Math.Abs(b.Y-a.Y)/2;
            for(var i=0;i<=64;i++){var angle=i*Math.PI*2/64;points.Add(new StylusPoint(left+rx+Math.Cos(angle)*rx,top+ry+Math.Sin(angle)*ry));}
        }
        else if(kind=="arrow")
        {
            points.Add(new StylusPoint(a.X,a.Y));points.Add(new StylusPoint(b.X,b.Y));var angle=Math.Atan2(b.Y-a.Y,b.X-a.X);
            var length=Math.Min(24,Math.Max(0,(b-a).Length*.25));
            points.Add(new StylusPoint(b.X-length*Math.Cos(angle-Math.PI/6),b.Y-length*Math.Sin(angle-Math.PI/6)));
            points.Add(new StylusPoint(b.X,b.Y));points.Add(new StylusPoint(b.X-length*Math.Cos(angle+Math.PI/6),b.Y-length*Math.Sin(angle+Math.PI/6)));
        }
        else throw new ArgumentOutOfRangeException(nameof(kind));
        var drawing=attributes.Clone();drawing.IsHighlighter=false;drawing.FitToCurve=false;
        var result=new Stroke(points,drawing);result.AddPropertyData(KindKey,kind);return result;
    }

    internal static Rect Bounds(IReadOnlyList<StylusPoint> points)
    {
        if(points.Count==0)return Rect.Empty;
        return new Rect(new Point(points.Min(p=>p.X),points.Min(p=>p.Y)),new Point(points.Max(p=>p.X),points.Max(p=>p.Y)));
    }

    internal static StylusPointCollection Resize(IReadOnlyList<StylusPoint> points,Rect target)
    {
        var source=Bounds(points);
        return new StylusPointCollection(points.Select(p=>
        {
            p.X=target.Left+(source.Width>0?(p.X-source.Left)/source.Width:0.5)*target.Width;
            p.Y=target.Top+(source.Height>0?(p.Y-source.Top)/source.Height:0.5)*target.Height;
            return p;
        }));
    }
}
