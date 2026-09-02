using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

internal static class PinnedImageTransform
{
    internal static BitmapSource RotateQuarterTurns(BitmapSource source,int quarterTurns)
    {
        ArgumentNullException.ThrowIfNull(source);
        var normalized=((quarterTurns%4)+4)%4;
        if(normalized==0)return source;
        var rotated=new TransformedBitmap(source,new RotateTransform(normalized*90));
        rotated.Freeze();
        return rotated;
    }
}
