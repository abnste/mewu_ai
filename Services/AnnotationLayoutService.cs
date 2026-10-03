// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;

/// <summary>Original WPF API; geometry planning is shared with the portable renderer.</summary>
public static class AnnotationLayoutService
{
    public static double FindCardTop(double preferred,double minimum,double maximum,double minimumDistance,IReadOnlyCollection<double> occupied)
        =>AnnotationLayoutPlanner.FindCardTop(preferred,minimum,maximum,minimumDistance,occupied);
    public static AnnotationCalloutPlacement FindCalloutPlacement(Rect target,Size card,Size canvas,IReadOnlyCollection<Rect> occupied,double gap=12,double padding=5)
        =>FromCore(AnnotationLayoutPlanner.FindCalloutPlacement(Box(target),Area(card),Area(canvas),occupied.Select(Box).ToArray(),gap,padding));
    public static IReadOnlyList<AnnotationCalloutPlacement> PlanCallouts(IReadOnlyList<AnnotationCalloutRequest> requests,Size canvas,double gap=12,double padding=5)
    {
        ArgumentNullException.ThrowIfNull(requests);
        return AnnotationLayoutPlanner.PlanCallouts(requests.Select(r=>new CalloutLayoutRequest(Box(r.Target),Area(r.Card))).ToArray(),Area(canvas),gap,padding).Select(FromCore).ToArray();
    }
    public static bool IsDuplicateTargetMarker(AiAnnotation candidate,IReadOnlyList<AiAnnotation> callouts)=>AnnotationLayoutPlanner.IsDuplicateTargetMarker(candidate,callouts);
    public static Point FindConnector(Rect target,Rect card)
    {var point=AnnotationLayoutPlanner.FindConnector(Box(target),Box(card));return new(point.X,point.Y);}
    private static AnnotationBox Box(Rect rect)=>new(rect.X,rect.Y,rect.Width,rect.Height);
    private static AnnotationArea Area(Size size)=>new(size.Width,size.Height);
    private static AnnotationCalloutPlacement FromCore(PlannedCallout value)=>new(new Rect(value.CardBounds.X,value.CardBounds.Y,value.CardBounds.Width,value.CardBounds.Height),new Point(value.ConnectorPoint.X,value.ConnectorPoint.Y));
}
public readonly record struct AnnotationCalloutPlacement(Rect CardBounds,Point ConnectorPoint);
public readonly record struct AnnotationCalloutRequest(Rect Target,Size Card);
