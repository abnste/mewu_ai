// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class DrawingAnnotationGeometryTests
{
    [Theory]
    [InlineData(0,10,10,10,10,110,80)]
    [InlineData(1,160,10,20,10,140,80)]
    [InlineData(2,160,140,20,30,140,110)]
    [InlineData(3,10,140,10,30,110,110)]
    public void CornerResizeKeepsOppositeCornerFixed(int corner,double x,double y,double left,double top,double width,double height)
    {
        Assert.Equal(new Rect(left,top,width,height),DrawingAnnotationGeometry.ResizeCorner(new Rect(20,30,100,60),corner,new Point(x,y),new Size(200,150)));
    }

    [Fact]
    public void CornerResizeStopsAtCanvasAndDoesNotInvertAtTheOppositeCorner()
    {
        Assert.Equal(new Rect(0,0,120,90),DrawingAnnotationGeometry.ResizeCorner(new Rect(20,30,100,60),0,new Point(-200,-200),new Size(200,150)));
        Assert.Equal(new Rect(118,88,2,2),DrawingAnnotationGeometry.ResizeCorner(new Rect(20,30,100,60),0,new Point(500,500),new Size(200,150)));
    }

    [Fact]
    public void ArrowRemainsEditableAfterCloningAndEndpointChangeKeepsItsOtherEnd()
    {
        var stroke=EditableShapeStroke.Create(new Point(10,20),new Point(110,120),"arrow",new System.Windows.Ink.DrawingAttributes{Width=4,Height=4});
        Assert.True(EditableShapeStroke.IsArrow(stroke.Clone()));
        var moved=EditableShapeStroke.Create(new Point(70,20),new Point(stroke.StylusPoints[1].X,stroke.StylusPoints[1].Y),"arrow",stroke.DrawingAttributes);
        Assert.Equal(110,moved.StylusPoints[1].X);Assert.Equal(120,moved.StylusPoints[1].Y);
        Assert.False(moved.DrawingAttributes.FitToCurve);Assert.Equal(4,moved.DrawingAttributes.Width);
        var zero=EditableShapeStroke.Create(new Point(10,20),new Point(10,20),"arrow",stroke.DrawingAttributes);
        Assert.All(zero.StylusPoints,p=>{Assert.Equal(10,p.X);Assert.Equal(20,p.Y);});
    }

    [Theory]
    [InlineData("rectangle")]
    [InlineData("ellipse")]
    public void ShapeResizeChangesCoordinatesWithoutChangingPenWidth(string kind)
    {
        var stroke=EditableShapeStroke.Create(new Point(10,20),new Point(110,120),kind,new System.Windows.Ink.DrawingAttributes{Width=4,Height=4});
        var before=stroke.StylusPoints.ToArray();var target=new Rect(30,40,200,50);
        stroke.StylusPoints=EditableShapeStroke.Resize(before,target);
        Assert.Equal(target,EditableShapeStroke.Bounds(stroke.StylusPoints.ToArray()));Assert.Equal(4,stroke.DrawingAttributes.Width);
        stroke.StylusPoints=new System.Windows.Input.StylusPointCollection(before);
        Assert.Equal(new Rect(10,20,100,100),EditableShapeStroke.Bounds(stroke.StylusPoints.ToArray()));
    }
    [Fact]
    public void TranslationIsClampedInsideCanvas()
    {
        var delta=DrawingAnnotationGeometry.ConstrainTranslation(new Rect(80,60,40,30),new Vector(500,-500),new Size(200,150));
        Assert.Equal(new Vector(80,-60),delta);
    }

    [Fact]
    public void TranslationInsideCanvasIsPreserved()
    {
        var delta=DrawingAnnotationGeometry.ConstrainTranslation(new Rect(20,20,40,30),new Vector(12,18),new Size(200,150));
        Assert.Equal(new Vector(12,18),delta);
    }

    [Fact]
    public void ShiftEllipseProducesEqualWidthAndHeight()
    {
        var end=DrawingAnnotationGeometry.ConstrainEllipseEndToCircle(new Point(40,30),new Point(110,70),new Size(300,200));
        Assert.Equal(110,end.X);Assert.Equal(100,end.Y);
    }

    [Fact]
    public void ShiftCircleIsClampedInsideSelectionBounds()
    {
        var end=DrawingAnnotationGeometry.ConstrainEllipseEndToCircle(new Point(260,170),new Point(400,260),new Size(300,200));
        Assert.Equal(290,end.X);Assert.Equal(200,end.Y);
    }

    [Fact]
    public void ShiftCirclePreservesNegativeDragDirection()
    {
        var end=DrawingAnnotationGeometry.ConstrainEllipseEndToCircle(new Point(120,100),new Point(80,30),new Size(300,200));
        Assert.Equal(50,end.X);Assert.Equal(30,end.Y);
    }
}
