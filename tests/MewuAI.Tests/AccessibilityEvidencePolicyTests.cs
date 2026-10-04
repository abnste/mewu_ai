// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Models;
using Xunit;

namespace MewuAI.Tests;

public sealed class AccessibilityEvidencePolicyTests
{
    [Theory]
    [InlineData(false, false, true)]
    [InlineData(true, false, false)]
    [InlineData(false, true, false)]
    [InlineData(true, true, false)]
    public void FrozenOrIndependentPixelsDoNotUseCurrentDesktopGeometry(bool hasOverride, bool frozen, bool expected)
        => Assert.Equal(expected, AccessibilityEvidencePolicy.CanRefineFromLiveDesktop(hasOverride, frozen));
    [Fact]
    public void SpatialGateAloneAcceptsAControlShiftWithoutImageAgeEvidence()
    {
        // Pure geometry characterization, not a live UIA reproduction.
        var note = new AiAnnotation(.25, .30, .20, .08, "", Kind: AiAnnotationKind.Rectangle);
        Assert.True(AccessibilityAnnotationRefinementService.TryRefine(note, new(100, 100, 800, 500), new(340, 250, 160, 40), out var refined));
        Assert.Equal(.30, refined.X, 10);
        Assert.Equal(note.Y, refined.Y);
        Assert.Equal(note.Width, refined.Width);
        Assert.Equal(note.Height, refined.Height);
    }
}
