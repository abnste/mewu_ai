using System.Globalization;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class LocalizationServiceTests
{
    [Theory]
    [InlineData("zh-CN",1)]
    [InlineData("zh-SG",1)]
    [InlineData("en-US",0)]
    [InlineData("en-GB",0)]
    [InlineData("fr-FR",0)]
    public void SystemUiCultureSelectsChineseOrEnglishFallback(string culture,int expected)
        =>Assert.Equal((AppLanguage)expected,LocalizationService.ResolveLanguage(CultureInfo.GetCultureInfo(culture)));

    [Theory]
    [InlineData("system","en-US",0)]
    [InlineData("system","zh-SG",1)]
    [InlineData("zh-CN","en-US",1)]
    [InlineData("en-US","zh-CN",0)]
    [InlineData("invalid","zh-CN",1)]
    public void SavedUiLanguageOverridesOrSafelyFallsBackToWindows(string preference,string systemCulture,int expected)
        =>Assert.Equal((AppLanguage)expected,LocalizationService.ResolveLanguagePreference(preference,CultureInfo.GetCultureInfo(systemCulture)));

    [Fact]
    public void EnglishUiUsesNaturalProductCopyAndDynamicStatuses()
    {
        Assert.Equal("MewuAI Screen Assistant",LocalizationService.TranslateUiText("喵呜AI 屏幕助手",AppLanguage.English));
        Assert.Equal("Start automatically when I sign in to Windows",LocalizationService.TranslateUiText("登录 Windows 后自动启动",AppLanguage.English));
        Assert.Equal("Captured 4 segments · 2380 px",LocalizationService.TranslateUiText("已采集 4 段 · 2380px",AppLanguage.English));
        Assert.Equal("4 segments kept · Scroll back slightly to reconnect",LocalizationService.TranslateUiText("已保留 4 段 · 未接上，请回滚少许",AppLanguage.English));
        Assert.Equal("Capture complete · 1415 × 2580 · Original proportions preserved",LocalizationService.TranslateUiText("长截图完成 · 1415 × 2580 · 已按原比例显示",AppLanguage.English));
        Assert.Equal("Connected · 1 agent · 2 models",LocalizationService.TranslateUiText("连接正常 · 1 个 Agent · 2 个模型",AppLanguage.English));
        Assert.Equal("Copied 2 tables · Paste editable cells into Excel, Markdown into text fields, or a PNG onto the desktop",LocalizationService.TranslateUiText("已复制 2 个表格 · Excel 可直接粘贴，文本框为 Markdown，桌面为 PNG",AppLanguage.English));
    }

    [Fact]
    public void ChineseUiKeepsOriginalCopy()
        =>Assert.Equal("登录 Windows 后自动启动",LocalizationService.TranslateUiText("登录 Windows 后自动启动",AppLanguage.SimplifiedChinese));

    [Fact]
    public void InstallerUsesWindowsUiLanguageAndLocalizedProductNames()
    {
        var script=File.ReadAllText(Path.Combine(AppContext.BaseDirectory,"Fixtures","MewuAI.iss.txt"));
        Assert.Contains("ShowLanguageDialog=no",script);Assert.Contains("LanguageDetectionMethod=uilanguage",script);
        Assert.Contains("UsePreviousLanguage=no",script);
        Assert.Contains("english.AppDisplayName=MewuAI",script);Assert.Contains("chinesesimplified.AppDisplayName=喵呜AI",script);
        Assert.Contains("AppName={cm:AppDisplayName}",script);Assert.Contains("{autodesktop}\\{cm:AppDisplayName}",script);
        Assert.DoesNotContain("#define MyAppChineseName",script);
    }
}
