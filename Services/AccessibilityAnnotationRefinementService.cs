using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Converts a coarse visual box into the exact UI Automation control bounds
/// beneath its centre. This mirrors computer-use's accessibility-first target
/// resolution while retaining image-only fallback for pixels without UIA.
/// </summary>
internal static class AccessibilityAnnotationRefinementService
{
    internal static bool TryRefine(AiAnnotation annotation,ScreenRect selection,ScreenRect control,out AiAnnotation refined)
    {
        refined=annotation;
        if(annotation.IsVideoTimeline||annotation.Kind is not (AiAnnotationKind.Callout or AiAnnotationKind.Rectangle or AiAnnotationKind.Ellipse)||selection.IsEmpty||control.IsEmpty)return false;
        var visible=control.Intersect(selection);
        if(visible.Width<12||visible.Height<12)return false;
        refined=annotation with
        {
            X=(visible.X-selection.X)/(double)selection.Width,
            Y=(visible.Y-selection.Y)/(double)selection.Height,
            Width=visible.Width/(double)selection.Width,
            Height=visible.Height/(double)selection.Height
        };
        return true;
    }
}
