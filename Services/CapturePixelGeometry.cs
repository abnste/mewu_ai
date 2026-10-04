// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Pixel-space geometry shared by capture backends. Desktop origins may be
/// negative; these coordinates are not WPF DIPs or macOS logical points.
/// </summary>
internal static class CapturePixelGeometry
{
    internal const long MaximumRegionPixels = 80_000_000;

    internal static (int X,int Y) ToScreenPixelPoint(double localX,double localY,double surfaceWidth,double surfaceHeight,int pixelWidth,int pixelHeight,int originX,int originY)
    {
        if(!double.IsFinite(localX)||!double.IsFinite(localY)||!double.IsFinite(surfaceWidth)||!double.IsFinite(surfaceHeight)||surfaceWidth<=0||surfaceHeight<=0||pixelWidth<=0||pixelHeight<=0)return (originX,originY);
        // Points address pixels, unlike half-open rectangle edges. The final
        // pixel is width-1/height-1, not the adjacent screen's first pixel.
        var x=(int)Math.Clamp(Math.Round(localX*pixelWidth/surfaceWidth),0,pixelWidth-1);
        var y=(int)Math.Clamp(Math.Round(localY*pixelHeight/surfaceHeight),0,pixelHeight-1);
        return (checked(originX+x),checked(originY+y));
    }
    internal static double PixelStepInSurface(double pixels,double surfaceLength,int pixelLength)
        =>double.IsFinite(pixels)&&double.IsFinite(surfaceLength)&&surfaceLength>0&&pixelLength>0?pixels*surfaceLength/pixelLength:0;

    internal static void ValidateRegionSize(ScreenRect region)
    {
        if (region.IsEmpty || (long)region.Width * region.Height > MaximumRegionPixels)
            throw new ArgumentOutOfRangeException(nameof(region), "截取区域为空或超过像素上限");
    }

    internal static void ValidateRegion(ScreenRect region, ScreenRect desktop)
    {
        ValidateRegionSize(region);

        // Use wide sums: a wrapped right/bottom edge must never pass validation.
        if (desktop.IsEmpty || region.X < desktop.X || region.Y < desktop.Y
            || (long)region.X + region.Width > (long)desktop.X + desktop.Width
            || (long)region.Y + region.Height > (long)desktop.Y + desktop.Height)
            throw new ArgumentOutOfRangeException(nameof(region), "截取区域超出虚拟桌面");
    }

    internal static ScreenRect ClipToImage(ScreenRect rect, int pixelWidth, int pixelHeight)
    {
        if (rect.IsEmpty)
            throw new ArgumentOutOfRangeException(nameof(rect), "裁剪区域必须有正的宽高");
        if (pixelWidth <= 0) throw new ArgumentOutOfRangeException(nameof(pixelWidth));
        if (pixelHeight <= 0) throw new ArgumentOutOfRangeException(nameof(pixelHeight));

        // Clip both edges rather than moving a negative origin into the image.
        var left = Math.Clamp((long)rect.X, 0L, pixelWidth);
        var top = Math.Clamp((long)rect.Y, 0L, pixelHeight);
        var right = Math.Clamp((long)rect.X + rect.Width, 0L, pixelWidth);
        var bottom = Math.Clamp((long)rect.Y + rect.Height, 0L, pixelHeight);
        if (right <= left || bottom <= top)
            throw new ArgumentException("裁剪区域与截图没有交集", nameof(rect));
        return new((int)left, (int)top, (int)(right - left), (int)(bottom - top));
    }
}
