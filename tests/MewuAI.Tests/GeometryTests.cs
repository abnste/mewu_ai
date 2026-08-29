using mewu_ai_Assistant.Models;
using Xunit;
namespace MewuAI.Tests;
public sealed class GeometryTests
{
    [Fact] public void FromPoints_NormalizesReverseDrag(){Assert.Equal(new ScreenRect(-50,-20,150,100),ScreenRect.FromPoints(100,80,-50,-20));}
    [Fact] public void Clamp_HandlesNegativeVirtualCoordinates(){var value=new ScreenRect(-2100,-100,500,500).Clamp(new ScreenRect(-1920,0,3840,1080));Assert.Equal(new ScreenRect(-1920,0,500,500),value);}
}
