// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using mewu_ai_Assistant.Services;
using Xunit;
namespace MewuAI.Core.Tests;
public sealed class AnnotationLayoutParityTests
{
    [Fact]public void SharedPlannerMatchesOriginalForRandomizedCrowdedLayouts()
    {
        var random=new Random(3189);
        for(var test=0;test<1000;test++)
        {
            var canvas=new Size(100+random.Next(1600),100+random.Next(1000));
            var requests=Enumerable.Range(0,1+random.Next(12)).Select(_=>new AnnotationCalloutRequest(new Rect(random.NextDouble()*canvas.Width,random.NextDouble()*canvas.Height,10+random.NextDouble()*100,10+random.NextDouble()*80),new Size(70+random.NextDouble()*150,20+random.NextDouble()*80))).ToArray();
            var expected=OriginalAnnotationLayoutReference.PlanCallouts(requests,canvas);
            var actual=AnnotationLayoutService.PlanCallouts(requests,canvas);
            Assert.Equal(expected.ToArray(),actual.ToArray());
        }
    }
}
