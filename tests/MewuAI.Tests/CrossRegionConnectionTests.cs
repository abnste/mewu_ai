// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class CrossRegionConnectionTests
{
    private const string Payload="""
    {"annotationProtocol":"mewu.visual-annotations/1","answer":"两个画面中的同一个按钮","annotations":[
      {"kind":"connection","target":{"regionIndex":0,"referenceHandle":"image-a"},"geometry":{"coordinateSpace":"normalized","rect":{"x":0.2,"y":0.3,"width":0.4,"height":0.2}},
       "destination":{"target":{"regionIndex":1,"referenceHandle":"image-b"},"geometry":{"coordinateSpace":"normalized","rect":{"x":0.5,"y":0.4,"width":0.2,"height":0.3}}},"label":"对应按钮"},
      {"x":0.1,"y":0.1,"width":0.1,"height":0.1,"text":"合法兄弟项"}]}
    """;

    [Fact] public void ParsesTwoIndependentNormalizedTargets()
    {
        var result=StructuredResponseParser.Parse(Payload);Assert.Equal(2,result.Annotations.Count);var link=result.Annotations[0];
        Assert.Equal(AiAnnotationKind.Connection,link.Kind);Assert.Equal("image-a",link.ReferenceHandle);Assert.Equal("image-b",link.Destination!.ReferenceHandle);
        Assert.Equal(.2,link.X);Assert.Equal(.5,link.Destination.X);
    }

    [Theory]
    [InlineData("unknown-kind")][InlineData("missing")][InlineData("same-image")][InlineData("bad-index")][InlineData("out-of-bounds")][InlineData("pixel-space")][InlineData("timeline")]
    public void InvalidConnectionDoesNotEraseAnswerOrLegalSibling(string problem)
    {
        var json=JsonNode.Parse(Payload)!;var link=json["annotations"]![0]!;
        switch(problem)
        {
            case "unknown-kind":link["kind"]="not-supported";break;
            case "missing":link.AsObject().Remove("destination");break;
            case "same-image":link["destination"]!["target"]!["referenceHandle"]="image-a";break;
            case "bad-index":link["destination"]!["target"]!["regionIndex"]="bad";break;
            case "out-of-bounds":link["destination"]!["geometry"]!["rect"]!["x"]=1.5;break;
            case "pixel-space":link["destination"]!["geometry"]!["coordinateSpace"]="pixels";break;
            case "timeline":link["timeline"]=new JsonObject();break;
        }
        var result=StructuredResponseParser.Parse(json.ToJsonString());Assert.Equal("两个画面中的同一个按钮",result.Answer);Assert.Equal("合法兄弟项",Assert.Single(result.Annotations).Text);
    }

    [Fact] public void HandlesWinOverStaleIndicesAfterReordering()
    {
        var link=StructuredResponseParser.Parse(Payload).Annotations[0];
        Assert.True(CrossRegionConnectionService.TryResolve(link,[new("image-b",false),new("image-a",false)],out var mapped,out var from,out var to));
        Assert.Equal(1,from);Assert.Equal(0,to);Assert.Equal(1,mapped.RegionIndex);Assert.Equal(0,mapped.Destination!.RegionIndex);
    }

    [Theory][InlineData(false)][InlineData(true)]
    public void DeletedOrVideoEndpointIsRejected(bool video)
    {
        var link=StructuredResponseParser.Parse(Payload).Annotations[0];
        Assert.False(CrossRegionConnectionService.TryResolve(link,[new("image-a",false),new(video?"image-b":"another-image",video)],out _,out _,out _));
    }

    [Fact] public void DifferentDestinationsAreNotDeduplicated()
    {
        var a=StructuredResponseParser.Parse(Payload).Annotations[0];var b=a with{Destination=a.Destination! with{ReferenceHandle="image-c"}};
        var result=AnnotationUpdateService.Apply([a],[b],AiAnnotationUpdateMode.Append,false);Assert.Equal(2,result.Annotations.Count);
        Assert.Single(AnnotationUpdateService.Apply([a],[a],AiAnnotationUpdateMode.Append,false).Annotations);
        Assert.Equal(a,Assert.Single(AnnotationUpdateService.Apply([a],[b],AiAnnotationUpdateMode.Preserve,false).Annotations));
    }

    [Fact] public void ProjectionUsesEachScreenshotsSizeAndOrigin()
    {
        Assert.Equal(new Rect(-50,110,40,30),CrossRegionConnectionService.Project(new Rect(-100,50,200,300),.25,.2,.2,.1));
        var from=CrossRegionConnectionService.BoundaryToward(new Rect(0,0,100,100),new Point(200,50));Assert.Equal(new Point(100,50),from);
    }

    [Fact] public void FrozenVectorConnectsAcrossTheGapAndBothExportsRetainEndpoints()
    {
        var note=StructuredResponseParser.Parse(Payload).Annotations[0];
        var link=new CrossRegionConnection(note,new Rect(50,100,80,50),new Rect(550,100,80,50),"图片1","图片2");
        var image=CrossRegionConnectionRenderer.CreateDrawing(700,300,[link]);Assert.True(image.IsFrozen);
        var visual=new DrawingVisual();using(var dc=visual.RenderOpen())dc.DrawImage(image,new Rect(0,0,700,300));
        var bitmap=new RenderTargetBitmap(700,300,96,96,PixelFormats.Pbgra32);bitmap.Render(visual);
        Assert.True(HasBlue(bitmap,220,120,480,130));
        Assert.True(HasBlue(CrossRegionConnectionRenderer.RenderRegion(250,250,new Rect(0,0,250,250),"image-a",[link]),0,0,250,250));
        Assert.True(HasBlue(CrossRegionConnectionRenderer.RenderRegion(250,250,new Rect(450,0,250,250),"image-b",[link]),0,0,250,250));
    }

    private static bool HasBlue(BitmapSource image,int left,int top,int right,int bottom)
    {
        var bytes=new byte[image.PixelWidth*image.PixelHeight*4];image.CopyPixels(bytes,image.PixelWidth*4,0);
        for(var y=top;y<bottom;y++)for(var x=left;x<right;x++){var i=(y*image.PixelWidth+x)*4;if(bytes[i+3]>100&&bytes[i]>bytes[i+2]+30)return true;}return false;
    }
}
