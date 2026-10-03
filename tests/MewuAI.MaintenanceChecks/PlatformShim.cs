// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
// Test-only compatibility for executing original postprocessor integration tests
// on Linux. This is not a replacement production Rect or Windows acceptance.
namespace System.Windows
{
    public readonly record struct Point(double X,double Y);
    public readonly record struct Size(double Width,double Height);
    public readonly record struct Rect(double X,double Y,double Width,double Height)
    {
        public double Left=>X;
        public double Top=>Y;
        public double Right=>X+Width;
        public double Bottom=>Y+Height;
        public bool IntersectsWith(Rect other)=>!IsEmpty&&!other.IsEmpty&&other.Left<=Right&&other.Right>=Left&&other.Top<=Bottom&&other.Bottom>=Top;
        public bool IsEmpty=>Width<0||Height<0;
        public static Rect Intersect(Rect a,Rect b)
        {
            var x=Math.Max(a.X,b.X);var y=Math.Max(a.Y,b.Y);
            var right=Math.Min(a.X+a.Width,b.X+b.Width);var bottom=Math.Min(a.Y+a.Height,b.Y+b.Height);
            return right<x||bottom<y?new Rect(0,0,-1,-1):new Rect(x,y,right-x,bottom-y);
        }
    }
}
namespace mewu_ai_Assistant.Services
{
    // Kept equal to existing CrossRegionConnectionService.MaximumConnections.
    internal static class CrossRegionConnectionService { internal const int MaximumConnections=12; }
}
