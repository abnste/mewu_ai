using System.Windows;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class DrawingAnnotationGeometryTests
{
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
