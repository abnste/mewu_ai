using System.Drawing;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class TrayMenuRenderLayoutTests
{
    [Fact]
    public void HoverBoundsUseItemLocalCoordinates()
    {
        Assert.Equal(new Rectangle(2,1,152,34),TrayMenuRenderLayout.GetHoverBounds(new Size(156,36)));
    }

    [Theory]
    [InlineData(4,36)]
    [InlineData(156,2)]
    [InlineData(0,0)]
    public void HoverBoundsRejectItemsWithoutDrawableInterior(int width,int height)
    {
        Assert.True(TrayMenuRenderLayout.GetHoverBounds(new Size(width,height)).IsEmpty);
    }
}
