using System.Windows;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

public static class ScreenCoordinateService
{
    private const double DefaultDpi=96d;

    /// <summary>Returns the device scale represented by a Win32 DPI value.</summary>
    public static double DpiScale(uint dpi)=>Math.Max(DefaultDpi,dpi)/DefaultDpi;

    /// <summary>Converts a physical-pixel length to the WPF DIP space.</summary>
    public static double PixelsToDip(double pixels,uint dpi)=>pixels/DpiScale(dpi);

    /// <summary>Converts a physical-pixel size to the WPF DIP space.</summary>
    public static Size PixelsToDipSize(double width,double height,uint dpi)=>new(PixelsToDip(width,dpi),PixelsToDip(height,dpi));

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

    /// <summary>
    /// Maps captured-frame physical pixels into coordinates relative to the
    /// live HWND rectangle used by SetWindowRgn.  The window can start at a
    /// different point than the virtual desktop (including a negative one),
    /// so this conversion must not assume both origins are identical.
    /// </summary>
    public static ScreenRect ToWindowRelativePixelRect(Int32Rect localPixels,int frameOriginX,int frameOriginY,ScreenRect windowBounds,int inset=0)
    {
        if(localPixels.IsEmpty||windowBounds.IsEmpty)return default;
        var safeInset=Math.Max(0,inset);
        var absoluteLeft=(long)frameOriginX+localPixels.X+safeInset;
        var absoluteTop=(long)frameOriginY+localPixels.Y+safeInset;
        var absoluteRight=(long)frameOriginX+localPixels.X+localPixels.Width-safeInset;
        var absoluteBottom=(long)frameOriginY+localPixels.Y+localPixels.Height-safeInset;
        var windowRight=(long)windowBounds.X+windowBounds.Width;
        var windowBottom=(long)windowBounds.Y+windowBounds.Height;
        var clippedLeft=Math.Max(absoluteLeft,windowBounds.X);
        var clippedTop=Math.Max(absoluteTop,windowBounds.Y);
        var clippedRight=Math.Min(absoluteRight,windowRight);
        var clippedBottom=Math.Min(absoluteBottom,windowBottom);
        if(clippedRight<=clippedLeft||clippedBottom<=clippedTop)return default;
        return new ScreenRect(
            checked((int)(clippedLeft-windowBounds.X)),
            checked((int)(clippedTop-windowBounds.Y)),
            checked((int)(clippedRight-clippedLeft)),
            checked((int)(clippedBottom-clippedTop)));
    }

    public static Rect ToLocalDipRect(ScreenRect screenPixels,int virtualOriginX,int virtualOriginY,double surfaceWidth,double surfaceHeight,int pixelWidth,int pixelHeight)
    {
        if(surfaceWidth<=0||surfaceHeight<=0||pixelWidth<=0||pixelHeight<=0||screenPixels.IsEmpty)return Rect.Empty;
        var scaleX=surfaceWidth/pixelWidth;var scaleY=surfaceHeight/pixelHeight;
        return new Rect((screenPixels.X-virtualOriginX)*scaleX,(screenPixels.Y-virtualOriginY)*scaleY,screenPixels.Width*scaleX,screenPixels.Height*scaleY);
    }
}
