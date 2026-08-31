using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Xunit;
namespace MewuAI.Tests;
public sealed class GeometryTests
{
    [Fact] public void FromPoints_NormalizesReverseDrag(){Assert.Equal(new ScreenRect(-50,-20,150,100),ScreenRect.FromPoints(100,80,-50,-20));}
    [Fact] public void Clamp_HandlesNegativeVirtualCoordinates(){var value=new ScreenRect(-2100,-100,500,500).Clamp(new ScreenRect(-1920,0,3840,1080));Assert.Equal(new ScreenRect(-1920,0,500,500),value);}
    [Fact] public void Clamp_MovesRegionInsideRightAndBottomEdges(){var value=new ScreenRect(1800,1000,500,500).Clamp(new ScreenRect(-1920,0,3840,1080));Assert.Equal(new ScreenRect(1420,580,500,500),value);}
    [Fact] public void Intersect_ReturnsTrueOverlap(){Assert.Equal(new ScreenRect(0,50,100,50),new ScreenRect(-100,50,200,100).Intersect(new ScreenRect(0,0,100,100)));}
    [Fact] public void DipSelection_MapsToPhysicalPixelsAt150Percent(){var result=ScreenCoordinateService.ToPixelRect(new Rect(100,50,200,100),1280,720,1920,1080);Assert.Equal(new Int32Rect(150,75,300,150),result);}
    [Fact] public void PhysicalPixels_ConvertToDipUsingPerMonitorDpi(){Assert.InRange(ScreenCoordinateService.PixelsToDip(2560,168),1462.856,1462.858);var size=ScreenCoordinateService.PixelsToDipSize(2560,1440,168);Assert.InRange(size.Width,1462.856,1462.858);Assert.InRange(size.Height,822.856,822.858);}
    [Fact] public void DipSelection_ClampsRoundingAtSurfaceEdge(){var result=ScreenCoordinateService.ToPixelRect(new Rect(1279.8,719.8,10,10),1280,720,1920,1080);Assert.Equal(new Int32Rect(1920,1080,0,0),result);}
    [Fact] public void PhysicalMonitor_MapsToLocalDipWithNegativeVirtualOrigin(){var result=ScreenCoordinateService.ToLocalDipRect(new ScreenRect(-1920,0,1920,1080),-1920,0,2560,720,3840,1080);Assert.Equal(new Rect(0,0,1280,720),result);}
    [Fact] public void PrimaryMonitor_MapsAfterNegativePhysicalDisplay(){var result=ScreenCoordinateService.ToLocalDipRect(new ScreenRect(0,0,1920,1080),-1920,0,2560,720,3840,1080);Assert.Equal(new Rect(1280,0,1280,720),result);}
    [Fact] public void CropClipsBothEdgesWhenSelectionStartsOutsideSource()
    {
        var pixels=new byte[100*60*4];var source=BitmapSource.Create(100,60,96,96,PixelFormats.Bgra32,null,pixels,100*4);source.Freeze();
        var crop=ScreenCaptureService.Crop(source,new Int32Rect(-10,8,20,12));
        Assert.Equal(10,crop.PixelWidth);Assert.Equal(12,crop.PixelHeight);
    }

    [Fact] public void CropRejectsEmptyIntersectionInsteadOfCreatingAnInvalidBitmap()
    {
        var source=BitmapSource.Create(100,60,96,96,PixelFormats.Bgra32,null,new byte[100*60*4],100*4);source.Freeze();
        Assert.Throws<ArgumentException>(()=>ScreenCaptureService.Crop(source,new Int32Rect(120,8,20,12)));
    }
}
