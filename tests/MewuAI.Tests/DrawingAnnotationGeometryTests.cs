using System.Windows;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class DrawingAnnotationGeometryTests
{
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
