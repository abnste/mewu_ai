using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using System.Windows;
using Xunit;
namespace MewuAI.Tests;
public sealed class GeometryTests
{
    [Fact] public void FromPoints_NormalizesReverseDrag(){Assert.Equal(new ScreenRect(-50,-20,150,100),ScreenRect.FromPoints(100,80,-50,-20));}
    [Fact] public void Clamp_HandlesNegativeVirtualCoordinates(){var value=new ScreenRect(-2100,-100,500,500).Clamp(new ScreenRect(-1920,0,3840,1080));Assert.Equal(new ScreenRect(-1920,0,500,500),value);}
    [Fact] public void Clamp_MovesRegionInsideRightAndBottomEdges(){var value=new ScreenRect(1800,1000,500,500).Clamp(new ScreenRect(-1920,0,3840,1080));Assert.Equal(new ScreenRect(1420,580,500,500),value);}
    [Fact] public void Intersect_ReturnsTrueOverlap(){Assert.Equal(new ScreenRect(0,50,100,50),new ScreenRect(-100,50,200,100).Intersect(new ScreenRect(0,0,100,100)));}
    [Fact] public void DipSelection_MapsToPhysicalPixelsAt150Percent(){var result=ScreenCoordinateService.ToPixelRect(new Rect(100,50,200,100),1280,720,1920,1080);Assert.Equal(new Int32Rect(150,75,300,150),result);}
    [Fact] public void DipSelection_ClampsRoundingAtSurfaceEdge(){var result=ScreenCoordinateService.ToPixelRect(new Rect(1279.8,719.8,10,10),1280,720,1920,1080);Assert.Equal(new Int32Rect(1920,1080,0,0),result);}
}
