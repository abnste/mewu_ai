using System.Windows;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class AnnotationGeometryService
{
    internal static Rect ToNormalizedRect(AiAnnotation annotation)=>new(annotation.X,annotation.Y,annotation.Width,annotation.Height);
    internal static Rect ToNormalizedRect(VideoAnnotationKeyframe frame)=>new(frame.X,frame.Y,frame.Width,frame.Height);

    internal static double IntersectionOverUnion(Rect first,Rect second)
    {
        if(first.IsEmpty||second.IsEmpty||first.Width<=0||first.Height<=0||second.Width<=0||second.Height<=0)return 0;
        var intersection=Rect.Intersect(first,second);
        if(intersection.IsEmpty)return 0;
        var intersectionArea=intersection.Width*intersection.Height;
        var union=first.Width*first.Height+second.Width*second.Height-intersectionArea;
        return union<=0?0:intersectionArea/union;
    }
}
