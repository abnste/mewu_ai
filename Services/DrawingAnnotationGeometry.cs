using System.Windows;

namespace mewu_ai_Assistant.Services;

internal static class DrawingAnnotationGeometry
{
    internal static Point ConstrainEllipseEndToCircle(Point start,Point end,Size canvas)
    {
        if(!IsFinite(start.X)||!IsFinite(start.Y)||!IsFinite(end.X)||!IsFinite(end.Y))return start;
        var directionX=end.X<start.X?-1d:1d;var directionY=end.Y<start.Y?-1d:1d;
        var desired=Math.Max(Math.Abs(end.X-start.X),Math.Abs(end.Y-start.Y));
        var horizontalLimit=directionX>0?Math.Max(0,canvas.Width-start.X):Math.Max(0,start.X);
        var verticalLimit=directionY>0?Math.Max(0,canvas.Height-start.Y):Math.Max(0,start.Y);
        var side=Math.Min(desired,Math.Min(horizontalLimit,verticalLimit));
        return new Point(start.X+directionX*side,start.Y+directionY*side);
    }

    private static bool IsFinite(double value)=>double.IsFinite(value);
}
