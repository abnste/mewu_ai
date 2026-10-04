// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class TranslationSingleLineTests
{
    [Theory]
    [InlineData("\r\n")]
    [InlineData("\r")]
    [InlineData("\n")]
    [InlineData("\u0085")]
    [InlineData("\u2028")]
    [InlineData("\u2029")]
    public void ModelLineSeparatorsBecomeOneSpaceWithoutLosingTranslation(string separator)
    {
        Assert.Equal("第一部分 😀 second part", TranslationOverlayLayoutService.NormalizeLineBreaks("第一部分 😀" + separator + "second part"));
    }

    [Fact]
    public void NormalizationPreservesIntentionalInternalSpacesAndSourceRowIdentity()
    {
        string[] physicalRows = ["  第一条  保留空格\r\n仍是第一条  ", "  第二条\n仍是第二条  "];
        var normalized = physicalRows.Select(TranslationOverlayLayoutService.NormalizeLineBreaks).ToArray();
        Assert.Equal(["第一条  保留空格 仍是第一条", "第二条 仍是第二条"], normalized);
    }

    [Fact]
    public void GenericWrappingRemainsAvailableForOtherConsumers()
    {
        var rows = TranslationOverlayLayoutService.WrapText("完整文字不能丢失", 3, text => text.Length);
        Assert.Equal(3, rows.Count);
        Assert.Equal("完整文字不能丢失", string.Concat(rows));
    }
}
