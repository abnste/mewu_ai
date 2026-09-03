using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class PinnedImageDropPlacement
{
    internal const int PreferredMaximumWidth=900;
    private const int WorkingAreaMargin=16;

    internal static ScreenRect Place(
        ScreenRect workingArea,
        int dropX,
        int dropY,
        int imageWidth,
        int imageHeight)
    {
        if(workingArea.IsEmpty)throw new ArgumentOutOfRangeException(nameof(workingArea));
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(imageWidth);
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(imageHeight);

        var availableWidth=Math.Max(1,workingArea.Width-WorkingAreaMargin*2);
        var availableHeight=Math.Max(1,workingArea.Height-WorkingAreaMargin*2);
        var scale=Math.Min(1d,Math.Min(
            Math.Min(PreferredMaximumWidth,availableWidth)/(double)imageWidth,
            availableHeight/(double)imageHeight));
        var width=Math.Clamp((int)Math.Round(imageWidth*scale),1,availableWidth);
        var height=Math.Clamp((int)Math.Round(imageHeight*scale),1,availableHeight);
        var minimumX=workingArea.X+Math.Min(WorkingAreaMargin,Math.Max(0,(workingArea.Width-width)/2));
        var minimumY=workingArea.Y+Math.Min(WorkingAreaMargin,Math.Max(0,(workingArea.Height-height)/2));
        var maximumX=workingArea.Right-width-Math.Min(WorkingAreaMargin,Math.Max(0,(workingArea.Width-width)/2));
        var maximumY=workingArea.Bottom-height-Math.Min(WorkingAreaMargin,Math.Max(0,(workingArea.Height-height)/2));
        var left=Math.Clamp(dropX-width/2,minimumX,Math.Max(minimumX,maximumX));
        var top=Math.Clamp(dropY-height/2,minimumY,Math.Max(minimumY,maximumY));
        return new ScreenRect(left,top,width,height);
    }
}
