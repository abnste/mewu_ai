using System.Windows;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

public static class ScreenCoordinateService
{
    public static Int32Rect ToPixelRect(Rect dipRect,double surfaceWidth,double surfaceHeight,int pixelWidth,int pixelHeight)
    {
        if(surfaceWidth<=0||surfaceHeight<=0||pixelWidth<=0||pixelHeight<=0)return Int32Rect.Empty;
        var scaleX=pixelWidth/surfaceWidth;var scaleY=pixelHeight/surfaceHeight;
        var left=Math.Clamp((int)Math.Round(dipRect.Left*scaleX),0,pixelWidth);
        var top=Math.Clamp((int)Math.Round(dipRect.Top*scaleY),0,pixelHeight);
        var right=Math.Clamp((int)Math.Round(dipRect.Right*scaleX),left,pixelWidth);
        var bottom=Math.Clamp((int)Math.Round(dipRect.Bottom*scaleY),top,pixelHeight);
        return new Int32Rect(left,top,right-left,bottom-top);
    }

    public static ScreenRect ToScreenRect(Int32Rect localPixels,int originX,int originY)=>new(originX+localPixels.X,originY+localPixels.Y,localPixels.Width,localPixels.Height);
}
