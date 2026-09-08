// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal sealed record CrossRegionConnection(AiAnnotation Annotation,Rect Source,Rect Destination,string SourceLabel,string DestinationLabel);

internal static class CrossRegionConnectionService
{
    internal const int MaximumConnections=12;

    internal static bool TryResolve(AiAnnotation annotation,IReadOnlyList<AnnotationReferenceTarget> targets,out AiAnnotation resolved,out int sourceIndex,out int destinationIndex)
    {
        resolved=annotation;sourceIndex=destinationIndex=-1;
        if(annotation.Kind!=AiAnnotationKind.Connection||annotation.IsVideoTimeline||annotation.Destination is not {} destination||
            string.IsNullOrWhiteSpace(annotation.ReferenceHandle)||string.IsNullOrWhiteSpace(destination.ReferenceHandle)||annotation.ReferenceHandle==destination.ReferenceHandle||
            !IsNormalizedRect(annotation.X,annotation.Y,annotation.Width,annotation.Height)||!IsNormalizedRect(destination.X,destination.Y,destination.Width,destination.Height))return false;
        if(targets.Count(target=>target.ReferenceHandle==annotation.ReferenceHandle)!=1||targets.Count(target=>target.ReferenceHandle==destination.ReferenceHandle)!=1)return false;
        var source=CaptureOverlayPolicy.ResolveAnnotationTarget(annotation.RegionIndex,annotation.ReferenceHandle,false,targets);
        var end=CaptureOverlayPolicy.ResolveAnnotationTarget(destination.RegionIndex,destination.ReferenceHandle,false,targets);
        if(!source.Success||!end.Success||source.TargetIndex==end.TargetIndex)return false;
        sourceIndex=source.TargetIndex;destinationIndex=end.TargetIndex;
        resolved=annotation with{RegionIndex=sourceIndex,Destination=destination with{RegionIndex=destinationIndex}};return true;
    }

    private static bool IsNormalizedRect(double x,double y,double w,double h)=>double.IsFinite(x)&&double.IsFinite(y)&&double.IsFinite(w)&&double.IsFinite(h)&&x>=0&&y>=0&&w>0&&h>0&&x+w<=1.000001&&y+h<=1.000001;
    internal static Rect Project(Rect selection,double x,double y,double width,double height)=>new(selection.X+x*selection.Width,selection.Y+y*selection.Height,width*selection.Width,height*selection.Height);

    internal static Point BoundaryToward(Rect bounds,Point other)
    {
        var center=new Point(bounds.X+bounds.Width/2,bounds.Y+bounds.Height/2);var vector=other-center;
        if(vector.LengthSquared<.0001)return center;
        var factor=1/Math.Max(Math.Abs(vector.X)/Math.Max(.001,bounds.Width/2),Math.Abs(vector.Y)/Math.Max(.001,bounds.Height/2));
        return center+vector*factor;
    }
}
