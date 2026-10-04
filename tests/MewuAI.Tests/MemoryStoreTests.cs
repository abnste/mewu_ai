// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using System.Windows;
using Xunit;

namespace MewuAI.Tests;

public sealed class MemoryStoreTests
{
    [Fact]
    public void MatchesAnyKeywordToOneMemoryEntry()
    {
        var entry = new MemoryEntry { Keywords = ["用户名", "账号"] };
        var matches = MemoryStore.Match([entry], "登录页面\n账号");

        var match = Assert.Single(matches);
        Assert.Same(entry, match.Entry);
        Assert.Equal("账号", match.Keyword);
    }

    [Fact]
    public void NormalizationIgnoresSpacesAndPunctuation()
    {
        Assert.Equal("登录密码", MemoryStore.Normalize("登录 密码："));
    }

    [Fact]
    public void IgnoresOneCharacterKeys()
    {
        var entry = new MemoryEntry { Keywords = ["码"] };

        Assert.Empty(MemoryStore.Match([entry], "验证码"));
    }

    [Theory]
    [InlineData("password", true, .90)]
    [InlineData("password", false, .65)]
    [InlineData("username", false, .90)]
    [InlineData("username", true, .73)]
    public void CandidateScoreAccountsForFieldKindAndGeometry(string fieldKind, bool isPassword, double minimumExpected)
    {
        var entry = new MemoryEntry { FieldKind = fieldKind, Sensitive = fieldKind == "password" };
        var candidate = new MemoryInputCandidate(null, new Rect(100, 100, 180, 30), isPassword, false, "Edit");

        var score = MemoryFillService.Score(entry, candidate, new Point(110, 115), new ScreenRect(0, 0, 400, 300));

        Assert.True(score >= minimumExpected, $"Expected at least {minimumExpected:P0}, got {score:P0}");
    }

    [Fact]
    public void PasswordEntryCannotFillARegularInput()
    {
        var candidate = new MemoryInputCandidate(null, new Rect(0, 0, 160, 30), false, false, "Edit");

        Assert.False(MemoryFillService.TryFill(candidate, "secret", IntPtr.Zero));
    }
}
