namespace mewu_ai_Assistant.Services;

internal static class PinnedWindowZoomPolicy
{
    // 2400 DIP was an arbitrary product cap and made large monitors and long
    // screenshots impossible to inspect.  Keep only a Win32-safe finite bound
    // so repeated wheel zoom cannot overflow SetWindowPos or WPF layout.
    internal const double MaxWindowDimension=32_768;

    internal static double GetMaximumWidth(int imageWidth,int imageHeight,double outerPadding)
    {
        Validate(imageWidth,imageHeight,outerPadding);
        var maxContent=Math.Max(1,MaxWindowDimension-outerPadding);
        var width=maxContent;
        var height=width*imageHeight/imageWidth+outerPadding;
        if(height>MaxWindowDimension)width=(MaxWindowDimension-outerPadding)*imageWidth/imageHeight;
        return Math.Max(outerPadding+1,width+outerPadding);
    }

    internal static double GetMaximumHeight(int imageWidth,int imageHeight,double outerPadding)
    {
        Validate(imageWidth,imageHeight,outerPadding);
        var maxContent=Math.Max(1,MaxWindowDimension-outerPadding);
        var height=maxContent;
        var width=height*imageWidth/imageHeight+outerPadding;
        if(width>MaxWindowDimension)height=(MaxWindowDimension-outerPadding)*imageHeight/imageWidth;
        return Math.Max(outerPadding+1,height+outerPadding);
    }

    private static void Validate(int imageWidth,int imageHeight,double outerPadding)
    {
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(imageWidth);
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(imageHeight);
        ArgumentOutOfRangeException.ThrowIfNegative(outerPadding);
        if(!double.IsFinite(outerPadding)||outerPadding>=MaxWindowDimension)throw new ArgumentOutOfRangeException(nameof(outerPadding));
    }
}
