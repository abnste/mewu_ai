using System.Windows;

namespace mewu_ai_Assistant.Services;

internal static class PinnedWindowInteractionPolicy
{
    internal static bool ShouldBeginDrag(Point start,Point current,double horizontalThreshold,double verticalThreshold)
    {
        ArgumentOutOfRangeException.ThrowIfNegative(horizontalThreshold);
        ArgumentOutOfRangeException.ThrowIfNegative(verticalThreshold);
        return Math.Abs(current.X-start.X)>=horizontalThreshold||Math.Abs(current.Y-start.Y)>=verticalThreshold;
    }
}
