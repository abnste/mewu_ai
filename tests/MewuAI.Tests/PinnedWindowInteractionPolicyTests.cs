using System.Windows;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class PinnedWindowInteractionPolicyTests
{
    [Theory]
    [InlineData(0,0,3.9,3.9,false)]
    [InlineData(0,0,4,0,true)]
    [InlineData(10,10,10,6,true)]
    [InlineData(10,10,5,10,true)]
    public void ShouldBeginDrag_UsesSystemStyleAxisThreshold(double startX,double startY,double currentX,double currentY,bool expected)
    {
        Assert.Equal(expected,PinnedWindowInteractionPolicy.ShouldBeginDrag(new Point(startX,startY),new Point(currentX,currentY),4,4));
    }
}
