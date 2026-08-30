using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class AnnotationLayoutServiceTests
{
    [Fact] public void KeepsPreferredPositionWhenItIsAvailable(){Assert.Equal(40,AnnotationLayoutService.FindCardTop(40,5,100,20,[]));}

    [Fact] public void MovesAwayFromOccupiedMaximumInsteadOfLooping(){var result=AnnotationLayoutService.FindCardTop(100,5,100,20,[100]);Assert.InRange(result,5,80);}

    [Fact] public void ReturnsBoundedFallbackWhenNoCollisionFreeSlotExists(){var result=AnnotationLayoutService.FindCardTop(10,5,15,20,[5,15]);Assert.InRange(result,5,15);Assert.True(double.IsFinite(result));}
}
