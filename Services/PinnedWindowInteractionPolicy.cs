using System.Windows;

namespace mewu_ai_Assistant.Services;

internal static class PinnedWindowInteractionPolicy
{
    // SWP_SHOWWINDOW | SWP_NOACTIVATE. Showing or restoring a pin must never
    // steal the keyboard route from the capture overlay that created it.
    internal const uint ShowWithoutActivationFlags=0x0040|0x0010;

    internal static bool ShouldBeginDrag(Point start,Point current,double horizontalThreshold,double verticalThreshold)
    {
        ArgumentOutOfRangeException.ThrowIfNegative(horizontalThreshold);
        ArgumentOutOfRangeException.ThrowIfNegative(verticalThreshold);
        return Math.Abs(current.X-start.X)>=horizontalThreshold||Math.Abs(current.Y-start.Y)>=verticalThreshold;
    }
}
