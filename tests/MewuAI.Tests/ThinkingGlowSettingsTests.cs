// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ThinkingGlowSettingsTests
{
    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("red")]
    [InlineData("#123")]
    [InlineData("#FFAABBCC")]
    [InlineData("#１２３４５６")]
    [InlineData("#XYZ123")]
    public void InvalidColorFallsBackWithoutThrowing(string? value)
    {
        Assert.Equal(ThinkingGlowAppearance.DefaultColor,ThinkingGlowAppearance.NormalizeColor(value));
        Assert.Equal(ThinkingGlowAppearance.ParseColor(ThinkingGlowAppearance.DefaultColor),ThinkingGlowAppearance.ParseColor(value));
    }
    [Fact]
    public void LegacySettingsEnableDefaultGlow()
    {
        var settings=JsonSerializer.Deserialize<AppSettings>("{}")!;
        Assert.True(settings.ThinkingGlowEnabled);
        Assert.Equal(ThinkingGlowAppearance.DefaultColor,settings.ThinkingGlowColor);
    }
    [Fact]
    public void SettingsPersistDisabledChoiceAndCustomColor()
    {
        var root=Path.Combine(Path.GetTempPath(),"MewuAI-GlowTests-"+Guid.NewGuid().ToString("N"));
        try
        {
            var provider=new AiProviderSettings{CredentialId="test-reference"};
            var settings=new AppSettings{ThinkingGlowEnabled=false,ThinkingGlowColor=" #f0aB91 ",Providers=[provider],DefaultProviderId=provider.Id};
            var service=new SettingsService(Path.Combine(root,"settings.json"));
            service.Save(settings);var loaded=service.Load();
            Assert.False(loaded.ThinkingGlowEnabled);Assert.Equal("#F0AB91",loaded.ThinkingGlowColor);
            Assert.Equal(240,ThinkingGlowAppearance.ParseColor(loaded.ThinkingGlowColor).R);
        }
        finally{if(Directory.Exists(root))Directory.Delete(root,true);}
    }
}
