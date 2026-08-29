namespace mewu_ai_Assistant.Models;
public readonly record struct ScreenRect(int X, int Y, int Width, int Height)
{
    public bool IsEmpty => Width <= 0 || Height <= 0; public int Right => X + Width; public int Bottom => Y + Height;
    public static ScreenRect FromPoints(int x1,int y1,int x2,int y2) => new(Math.Min(x1,x2),Math.Min(y1,y2),Math.Abs(x2-x1),Math.Abs(y2-y1));
    public ScreenRect Clamp(ScreenRect bounds) { var width=Math.Clamp(Width,0,Math.Max(0,bounds.Width));var height=Math.Clamp(Height,0,Math.Max(0,bounds.Height));var x=Math.Clamp(X,bounds.X,bounds.Right-width);var y=Math.Clamp(Y,bounds.Y,bounds.Bottom-height);return new(x,y,width,height); }
    public ScreenRect Intersect(ScreenRect other){var left=Math.Max(X,other.X);var top=Math.Max(Y,other.Y);var right=Math.Min(Right,other.Right);var bottom=Math.Min(Bottom,other.Bottom);return right<=left||bottom<=top?default:new(left,top,right-left,bottom-top);}
}
