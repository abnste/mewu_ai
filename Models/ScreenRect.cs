namespace mewu_ai_Assistant.Models;
public readonly record struct ScreenRect(int X, int Y, int Width, int Height)
{
    public bool IsEmpty => Width <= 0 || Height <= 0; public int Right => X + Width; public int Bottom => Y + Height;
    public static ScreenRect FromPoints(int x1,int y1,int x2,int y2) => new(Math.Min(x1,x2),Math.Min(y1,y2),Math.Abs(x2-x1),Math.Abs(y2-y1));
    public ScreenRect Clamp(ScreenRect b) { var x=Math.Clamp(X,b.X,b.Right); var y=Math.Clamp(Y,b.Y,b.Bottom); return new(x,y,Math.Max(0,Math.Min(Width,b.Right-x)),Math.Max(0,Math.Min(Height,b.Bottom-y))); }
}
