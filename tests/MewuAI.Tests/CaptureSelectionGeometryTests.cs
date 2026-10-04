// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Services;
using Xunit;
namespace MewuAI.Core.Tests;
public sealed class CaptureSelectionGeometryTests
{
    [Fact]public void EveryAcceptedSmallSelectionAndHandleAtEveryEdgeIsSafe()
    {
        foreach(var width in new[]{8d,9,10,11,12})foreach(var height in new[]{8d,9,10,11,12})
        foreach(var x in new[]{0d,100-width})foreach(var y in new[]{0d,80-height})
        foreach(var handle in new[]{"W","E","N","S","NW","NE","SW","SE"})
        foreach(var delta in new[]{-1000d,-1,0,1,1000})
        {
            var b=CaptureSelectionGeometry.Resize(new(x,y,width,height),handle,delta,delta,100,80);
            Assert.InRange(b.X,0,100);Assert.InRange(b.Y,0,80);Assert.InRange(b.Width,8,100);Assert.InRange(b.Height,8,80);
            Assert.True(b.X+b.Width<=100&&b.Y+b.Height<=80);
        }
    }
    [Fact]public void ResizingPreservesOppositeAnchorAndDoesNotInvert()
    {
        var b=CaptureSelectionGeometry.Resize(new(20,20,30,30),"W",100,0,100,100);Assert.Equal(42,b.X);Assert.Equal(8,b.Width);Assert.Equal(50,b.X+b.Width);
        b=CaptureSelectionGeometry.Resize(new(20,20,30,30),"SE",100,100,100,100);Assert.Equal(20,b.X);Assert.Equal(20,b.Y);Assert.Equal(80,b.Width);Assert.Equal(80,b.Height);
    }
    [Fact]public void TinyCanvasAndOutsideSnapResultsRemainBounded()
    {
        var tiny=CaptureSelectionGeometry.Resize(new(0,0,8,8),"NW",1,1,4,5);Assert.Equal(new CaptureSelectionBounds(0,0,4,5),tiny);
        var outside=CaptureSelectionGeometry.Resize(new(-20,-10,180,200),"SE",0,0,100,80);Assert.Equal(new CaptureSelectionBounds(0,0,100,80),outside);
    }
    [Fact]public void InvalidInputsAreConservativelyEmpty()
    {
        Assert.Equal(default,CaptureSelectionGeometry.Resize(new(0,0,8,8),"W",double.NaN,0,100,100));
        Assert.Equal(default,CaptureSelectionGeometry.Resize(new(0,0,8,8),"W",1,0,0,100));
        Assert.Equal(default,CaptureSelectionGeometry.Resize(new(0,0,-8,8),"W",1,0,100,100));
    }
}
