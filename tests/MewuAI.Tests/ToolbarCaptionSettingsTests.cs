// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ToolbarCaptionSettingsTests
{
    [Fact]
    public void ExistingSettingsWithoutCaptionChoiceEnableLabels()
        =>Assert.True(JsonSerializer.Deserialize<AppSettings>("{}")!.ShowToolbarCaptions);

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void SavedChoiceSurvivesReloadWithoutResettingIntegrations(bool enabled)
    {
        var directory=Path.Combine(Path.GetTempPath(),"MewuAI-CaptionSettings-"+Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        File.WriteAllText(Path.Combine(directory,".mini-temp"),"toolbar settings regression");
        try
        {
            var provider=new AiProviderSettings{CredentialId="synthetic-reference"};
            var settings=new AppSettings{ShowToolbarCaptions=enabled,Providers=[provider],DefaultProviderId=provider.Id,
                FeishuTargetId="synthetic-chat",ImaKnowledgeBaseId="synthetic-library",ScraplingPath="synthetic-crawler"};
            var service=new SettingsService(Path.Combine(directory,"settings.json"));
            service.Save(settings);var reloaded=service.Load();
            Assert.Equal(enabled,reloaded.ShowToolbarCaptions);
            Assert.Equal("synthetic-chat",reloaded.FeishuTargetId);Assert.Equal("synthetic-library",reloaded.ImaKnowledgeBaseId);
            Assert.Equal("synthetic-crawler",reloaded.ScraplingPath);
        }
        finally{Directory.Delete(directory,true);}
    }
}
