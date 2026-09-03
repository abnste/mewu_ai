using System.Windows;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

public static class AnnotationLayoutService
{
    public static double FindCardTop(double preferred,double minimum,double maximum,double minimumDistance,IReadOnlyCollection<double> occupied)
    {
        maximum=Math.Max(minimum,maximum);
        preferred=Math.Clamp(preferred,minimum,maximum);
        minimumDistance=Math.Max(0,minimumDistance);
        if(occupied.Count==0||occupied.All(value=>Math.Abs(value-preferred)>=minimumDistance))return preferred;

        var candidates=new List<double>{preferred,minimum,maximum};
        foreach(var value in occupied)
        {
            candidates.Add(Math.Clamp(value-minimumDistance,minimum,maximum));
            candidates.Add(Math.Clamp(value+minimumDistance,minimum,maximum));
        }

        var ordered=candidates.Distinct().OrderBy(value=>Math.Abs(value-preferred)).ToList();
        var available=ordered.FirstOrDefault(candidate=>occupied.All(value=>Math.Abs(value-candidate)>=minimumDistance),double.NaN);
        if(!double.IsNaN(available))return available;
        return ordered.OrderByDescending(candidate=>occupied.Min(value=>Math.Abs(value-candidate))).ThenBy(candidate=>Math.Abs(candidate-preferred)).First();
    }

    /// <summary>
    /// Places a callout card independently from its target.  The target
    /// rectangle is model geometry; the card is presentation only and must not
    /// hide that rectangle when another side of the selection is available.
    /// </summary>
    public static AnnotationCalloutPlacement FindCalloutPlacement(Rect target,Size card,Size canvas,IReadOnlyCollection<Rect> occupied,double gap=12,double padding=5)
    {
        var width=Math.Max(1,canvas.Width);var height=Math.Max(1,canvas.Height);var cardWidth=Math.Min(Math.Max(1,card.Width),Math.Max(1,width-padding*2));var cardHeight=Math.Min(Math.Max(1,card.Height),Math.Max(1,height-padding*2));
        Rect Fit(double left,double top)=>new(Math.Clamp(left,padding,Math.Max(padding,width-cardWidth-padding)),Math.Clamp(top,padding,Math.Max(padding,height-cardHeight-padding)),cardWidth,cardHeight);
        var center=new Point(target.Left+target.Width/2,target.Top+target.Height/2);
        var candidates=new[]
        {
            Fit(target.Right+gap,center.Y-cardHeight/2),
            Fit(target.Left-gap-cardWidth,center.Y-cardHeight/2),
            Fit(center.X-cardWidth/2,target.Bottom+gap),
            Fit(center.X-cardWidth/2,target.Top-gap-cardHeight),
            Fit(target.Right+gap,target.Top-gap-cardHeight),
            Fit(target.Right+gap,target.Bottom+gap),
            Fit(target.Left-gap-cardWidth,target.Top-gap-cardHeight),
            Fit(target.Left-gap-cardWidth,target.Bottom+gap)
        };
        var evaluated=candidates.Select((bounds,index)=>new{bounds,index,score=Score(bounds,target,occupied,index),targetOverlap=OverlapArea(bounds,target),occupiedOverlap=occupied.Sum(other=>OverlapArea(bounds,other))}).ToArray();
        // Prefer the conventional right/left/below/above order when a clean
        // slot exists. Scoring is only a bounded fallback for crowded content.
        var clean=evaluated.FirstOrDefault(entry=>entry.targetOverlap<=.01&&entry.occupiedOverlap<=.01);
        var chosen=(clean??evaluated.OrderBy(entry=>entry.score).ThenBy(entry=>entry.index).First()).bounds;
        return new AnnotationCalloutPlacement(chosen,FindConnector(target,chosen));
    }

    public static IReadOnlyList<AnnotationCalloutPlacement> PlanCallouts(IReadOnlyList<AnnotationCalloutRequest> requests,Size canvas,double gap=12,double padding=5)
    {
        ArgumentNullException.ThrowIfNull(requests);if(requests.Count==0)return [];
        var placements=new AnnotationCalloutPlacement[requests.Count];var cards=new List<Rect>(requests.Count);
        for(var index=0;index<requests.Count;index++)
        {
            var targets=requests.Where((_,other)=>other!=index).Select(request=>request.Target).ToArray();
            // In a fully crowded canvas overlap may be unavoidable. Weight
            // semantic targets above prior cards so explanatory text covers a
            // card before it hides the pixels the annotation is pointing at.
            var occupied=targets.Concat(targets).Concat(targets).Concat(cards).ToArray();
            var placement=FindCalloutPlacement(requests[index].Target,requests[index].Card,canvas,occupied,gap,padding);placements[index]=placement;cards.Add(placement.CardBounds);
        }
        return placements;
    }

    public static bool IsDuplicateTargetMarker(AiAnnotation candidate,IReadOnlyList<AiAnnotation> callouts)
    {
        if(candidate.Kind is not (AiAnnotationKind.Rectangle or AiAnnotationKind.Ellipse))return false;
        return callouts.Any(callout=>callout.RegionIndex==candidate.RegionIndex&&string.Equals(callout.ReferenceHandle,candidate.ReferenceHandle,StringComparison.Ordinal)&&
            AnnotationGeometryService.IntersectionOverUnion(AnnotationGeometryService.ToNormalizedRect(callout),AnnotationGeometryService.ToNormalizedRect(candidate))>=.72);
    }

    private static double Score(Rect card,Rect target,IReadOnlyCollection<Rect> occupied,int order)
    {
        var targetOverlap=OverlapArea(card,target);var occupiedOverlap=occupied.Sum(bounds=>OverlapArea(card,bounds));
        var distance=Math.Abs((card.Left+card.Width/2)-(target.Left+target.Width/2))+Math.Abs((card.Top+card.Height/2)-(target.Top+target.Height/2));
        return targetOverlap*1_000_000+occupiedOverlap*20_000+distance+order*.01;
    }

    private static double OverlapArea(Rect first,Rect second){var overlap=Rect.Intersect(first,second);return Math.Max(0,overlap.Width)*Math.Max(0,overlap.Height);}

    public static Point FindConnector(Rect target,Rect card)
    {
        var center=new Point(target.Left+target.Width/2,target.Top+target.Height/2);
        if(card.Left>=target.Right)return new Point(card.Left,Math.Clamp(center.Y,card.Top,card.Bottom));
        if(card.Right<=target.Left)return new Point(card.Right,Math.Clamp(center.Y,card.Top,card.Bottom));
        if(card.Top>=target.Bottom)return new Point(Math.Clamp(center.X,card.Left,card.Right),card.Top);
        if(card.Bottom<=target.Top)return new Point(Math.Clamp(center.X,card.Left,card.Right),card.Bottom);
        var candidates=new[]{new Point(card.Left,Math.Clamp(center.Y,card.Top,card.Bottom)),new Point(card.Right,Math.Clamp(center.Y,card.Top,card.Bottom)),new Point(Math.Clamp(center.X,card.Left,card.Right),card.Top),new Point(Math.Clamp(center.X,card.Left,card.Right),card.Bottom)};
        return candidates.OrderBy(point=>(point.X-center.X)*(point.X-center.X)+(point.Y-center.Y)*(point.Y-center.Y)).First();
    }
}

public readonly record struct AnnotationCalloutPlacement(Rect CardBounds,Point ConnectorPoint);
public readonly record struct AnnotationCalloutRequest(Rect Target,Size Card);
