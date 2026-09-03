using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Models;
using System.Windows;
using Xunit;

namespace MewuAI.Tests;

public sealed class AnnotationLayoutServiceTests
{
    [Fact] public void KeepsPreferredPositionWhenItIsAvailable(){Assert.Equal(40,AnnotationLayoutService.FindCardTop(40,5,100,20,[]));}

    [Fact] public void MovesAwayFromOccupiedMaximumInsteadOfLooping(){var result=AnnotationLayoutService.FindCardTop(100,5,100,20,[100]);Assert.InRange(result,5,80);}

    [Fact] public void ReturnsBoundedFallbackWhenNoCollisionFreeSlotExists(){var result=AnnotationLayoutService.FindCardTop(10,5,15,20,[5,15]);Assert.InRange(result,5,15);Assert.True(double.IsFinite(result));}

    [Fact]
    public void PlacesCalloutCardBesideTargetWithoutCoveringIt()
    {
        var target=new Rect(110,55,80,34);var placement=AnnotationLayoutService.FindCalloutPlacement(target,new Size(150,48),new Size(400,180),[]);
        Assert.False(placement.CardBounds.IntersectsWith(target));
        Assert.InRange(placement.CardBounds.Left,202,202);
        Assert.InRange(placement.ConnectorPoint.X,202,202);
    }

    [Fact]
    public void FallsBackBelowTargetInsteadOfCoveringItWhenBothSidesAreBlocked()
    {
        var target=new Rect(95,20,110,30);var placement=AnnotationLayoutService.FindCalloutPlacement(target,new Size(190,45),new Size(300,180),[]);
        Assert.False(placement.CardBounds.IntersectsWith(target));
        Assert.True(placement.CardBounds.Top>=target.Bottom);
    }

    [Fact]
    public void RecognizesOverlappingProtocolRectangleAsCalloutDuplicate()
    {
        var callout=new AiAnnotation(.2,.3,.16,.08,"新对话",0,ReferenceHandle:"ref-1",Kind:AiAnnotationKind.Callout);
        var rectangle=callout with{Kind=AiAnnotationKind.Rectangle,Text="矩形标记"};
        Assert.True(AnnotationLayoutService.IsDuplicateTargetMarker(rectangle,[callout]));
    }
}
