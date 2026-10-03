// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
namespace mewu_ai_Assistant.Services;

internal readonly record struct CaptureSelectionBounds(double X,double Y,double Width,double Height);

/// <summary>Pure DIP-space geometry; independent of WPF/Avalonia and screen origin.</summary>
internal static class CaptureSelectionGeometry
{
    internal const double MinimumSize=8;
    internal static CaptureSelectionBounds Resize(CaptureSelectionBounds bounds,string handle,double dx,double dy,double canvasWidth,double canvasHeight)
    {
        if(!double.IsFinite(canvasWidth)||!double.IsFinite(canvasHeight)||canvasWidth<=0||canvasHeight<=0||
            !double.IsFinite(bounds.X)||!double.IsFinite(bounds.Y)||!double.IsFinite(bounds.Width)||!double.IsFinite(bounds.Height)||
            bounds.Width<0||bounds.Height<0||!double.IsFinite(dx)||!double.IsFinite(dy))return default;
        var width=Math.Min(bounds.Width,canvasWidth);var height=Math.Min(bounds.Height,canvasHeight);
        var left=Math.Clamp(bounds.X,0,canvasWidth-width);var top=Math.Clamp(bounds.Y,0,canvasHeight-height);
        var right=left+width;var bottom=top+height;
        var minimumWidth=Math.Min(MinimumSize,canvasWidth);var minimumHeight=Math.Min(MinimumSize,canvasHeight);
        if(handle.Contains('W'))left=Math.Clamp(left+dx,0,Math.Max(0,right-minimumWidth));
        if(handle.Contains('E'))right=Math.Clamp(right+dx,Math.Min(canvasWidth,left+minimumWidth),canvasWidth);
        if(handle.Contains('N'))top=Math.Clamp(top+dy,0,Math.Max(0,bottom-minimumHeight));
        if(handle.Contains('S'))bottom=Math.Clamp(bottom+dy,Math.Min(canvasHeight,top+minimumHeight),canvasHeight);
        return new(left,top,Math.Max(0,right-left),Math.Max(0,bottom-top));
    }
}
