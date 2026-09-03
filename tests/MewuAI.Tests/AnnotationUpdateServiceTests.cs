using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class AnnotationUpdateServiceTests
{
    [Fact]
    public void PreserveKeepsExistingAnnotationsAndIgnoresIncomingOnes()
    {
        var existing=Rectangle(.1,.1,"旧标注");
        var incoming=Rectangle(.5,.5,"新标注");

        var result=AnnotationUpdateService.Apply([existing],[incoming],AiAnnotationUpdateMode.Preserve,false);

        Assert.False(result.Changed);Assert.False(result.Replaced);Assert.Same(existing,Assert.Single(result.Annotations));
    }

    [Fact]
    public void EmptyReplacementNeverErasesExistingAnnotations()
    {
        var existing=Rectangle(.1,.1,"旧标注");

        var result=AnnotationUpdateService.Apply([existing],[],AiAnnotationUpdateMode.Replace,false);

        Assert.False(result.Changed);Assert.False(result.Replaced);Assert.Same(existing,Assert.Single(result.Annotations));
    }

    [Fact]
    public void AppendKeepsExistingAnnotationsAndAddsOnlyNewOnes()
    {
        var existing=Rectangle(.1,.1,"旧标注");
        var incoming=Rectangle(.5,.5,"新标注");

        var result=AnnotationUpdateService.Apply([existing],[incoming],AiAnnotationUpdateMode.Append,false);

        Assert.True(result.Changed);Assert.False(result.Replaced);Assert.Equal([existing,incoming],result.Annotations);
    }

    [Fact]
    public void AppendDeduplicatesTheSameAnnotationReturnedAgain()
    {
        var existing=Rectangle(.1,.1,"同一标注");
        var duplicate=Rectangle(.1,.1,"同一标注");

        var result=AnnotationUpdateService.Apply([existing],[duplicate],AiAnnotationUpdateMode.Append,false);

        Assert.False(result.Changed);Assert.False(result.Replaced);Assert.Same(existing,Assert.Single(result.Annotations));
    }

    [Fact]
    public void ReplaceUsesOnlyTheNewValidAnnotations()
    {
        var existing=Rectangle(.1,.1,"旧标注");
        var incoming=Rectangle(.5,.5,"重标结果");

        var result=AnnotationUpdateService.Apply([existing],[incoming],AiAnnotationUpdateMode.Replace,false);

        Assert.True(result.Changed);Assert.True(result.Replaced);Assert.Same(incoming,Assert.Single(result.Annotations));
    }

    private static AiAnnotation Rectangle(double x,double y,string text)=>new(x,y,.2,.15,text,0,ReferenceHandle:"ref-image",Kind:AiAnnotationKind.Rectangle);
}
