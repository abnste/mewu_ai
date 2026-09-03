using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class AccessibilityAnnotationRefinementServiceTests
{
    [Fact]
    public void ReplacesCoarseCalloutWithExactPhysicalControlBounds()
    {
        var note=new AiAnnotation(.12,.15,.18,.12,"新对话",Kind:AiAnnotationKind.Callout);
        Assert.True(AccessibilityAnnotationRefinementService.TryRefine(note,new ScreenRect(100,200,1000,600),new ScreenRect(340,290,260,44),out var exact));
        Assert.Equal(.24,exact.X,6);Assert.Equal(.15,exact.Y,6);Assert.Equal(.26,exact.Width,6);Assert.Equal(44d/600,exact.Height,6);
    }

    [Fact]
    public void DoesNotReplaceVideoOrControlOutsideTheSelection()
    {
        var note=new AiAnnotation(.1,.1,.2,.2,"视频",StartTime:1,EndTime:1,Keyframes:[new(1,.1,.1,.2,.2)]);
        Assert.False(AccessibilityAnnotationRefinementService.TryRefine(note,new ScreenRect(0,0,200,100),new ScreenRect(20,20,40,30),out _));
        var image=note with{StartTime=null,EndTime=null,Keyframes=null};
        Assert.False(AccessibilityAnnotationRefinementService.TryRefine(image,new ScreenRect(0,0,200,100),new ScreenRect(300,20,40,30),out _));
    }
}
