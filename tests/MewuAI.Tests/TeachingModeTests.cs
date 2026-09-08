// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class TeachingModeTests
{
    [Fact]
    public void ExistingSettingsWithoutTeachingModeUseDefaultSharing()
    {
        var settings=JsonSerializer.Deserialize<AppSettings>("{\"OverlayOpacity\":0.5}")!;
        Assert.True(settings.TeachingMode);
        Assert.Equal(.5,settings.OverlayOpacity);
    }

    [Theory]
    [InlineData(true)]
    [InlineData(false)]
    public void TeachingPreferenceSurvivesSettingsSaveAndReload(bool enabled)
    {
        var directory=Path.Combine(Path.GetTempPath(),"MewuTeachingTests",Guid.NewGuid().ToString("N"));
        try
        {
            var path=Path.Combine(directory,"settings.json");
            var service=new SettingsService(path);
            var settings=service.Load();
            Assert.True(settings.TeachingMode);
            settings.Providers[0].CredentialId="synthetic-test-credential-reference";
            settings.TeachingMode=enabled;
            service.Save(settings);
            var loaded=service.Load();
            Assert.Equal(enabled,loaded.TeachingMode);
            Assert.Equal(settings.DefaultProviderId,loaded.DefaultProviderId);
            Assert.Empty(loaded.ConfigurationErrors);
            settings.TeachingMode=!enabled;
            service.Save(settings);
            Assert.Equal(!enabled,service.Load().TeachingMode);
        }
        finally { if(Directory.Exists(directory))Directory.Delete(directory,true); }
    }
}
