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
        var coarseWidth=Math.Max(1,annotation.Width*selection.Width);var coarseHeight=Math.Max(1,annotation.Height*selection.Height);
        var coarseCenterX=selection.X+(annotation.X+annotation.Width/2)*selection.Width;var coarseCenterY=selection.Y+(annotation.Y+annotation.Height/2)*selection.Height;
        // ElementFromPoint may legitimately fall back to the application root.
        // Never turn a local model marker into the whole screenshot merely
        // because that root contains its centre. The selected semantic control
        // must cover or closely border the coarse centre and remain within a
        // conservative size envelope around the model estimate.
        var centerGapX=coarseCenterX<visible.X?visible.X-coarseCenterX:coarseCenterX>visible.Right?coarseCenterX-visible.Right:0;
        var centerGapY=coarseCenterY<visible.Y?visible.Y-coarseCenterY:coarseCenterY>visible.Bottom?coarseCenterY-visible.Bottom:0;
        if(centerGapX>Math.Max(24,coarseWidth*.4)||centerGapY>Math.Max(24,coarseHeight*.4))return false;
        var widthRatio=visible.Width/coarseWidth;var heightRatio=visible.Height/coarseHeight;
        if(widthRatio is <.2 or >5||heightRatio is <.2 or >5)return false;
        var visibleArea=(long)visible.Width*visible.Height;var selectionArea=Math.Max(1L,(long)selection.Width*selection.Height);var coarseArea=coarseWidth*coarseHeight;
        if(visibleArea*100>=selectionArea*88&&coarseArea<selectionArea*.65)return false;
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
