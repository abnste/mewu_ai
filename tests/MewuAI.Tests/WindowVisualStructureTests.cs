// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Xml.Linq;
using Xunit;

namespace MewuAI.Tests;

public sealed class WindowVisualStructureTests
{
    [Fact]
    public void MainWindowKeepsShadowOutsideClippedShell()
    {
        var path=Path.Combine(AppContext.BaseDirectory,"Fixtures","MainWindow.xaml.xml");
        var document=XDocument.Load(path,LoadOptions.SetLineInfo);
        XNamespace presentation="http://schemas.microsoft.com/winfx/2006/xaml/presentation";
        XNamespace x="http://schemas.microsoft.com/winfx/2006/xaml";

        XElement FindNamed(string name)=>document
            .Descendants()
            .Single(element=>string.Equals((string?)element.Attribute(x+"Name"),name,StringComparison.Ordinal));

        var shadow=FindNamed("WindowShadow");
        var shell=FindNamed("Shell");
        var shadowEffect=shadow.Element(presentation+"Border.Effect")?.Element(presentation+"DropShadowEffect");

        Assert.NotNull(shadowEffect);
        Assert.Null(shadow.Attribute("Clip"));
        Assert.NotEqual("True",(string?)shadow.Attribute("ClipToBounds"));
        Assert.Same(shadow.Parent,shell.Parent);
        Assert.Equal("True",(string?)shell.Attribute("ClipToBounds"));
        Assert.Equal("14",(string?)shell.Attribute("CornerRadius"));
        Assert.Null(shell.Element(presentation+"Border.Effect"));
    }

    [Fact]
    public void CapturePromptKeepsShadowsOutsideClippedContent()
    {
        var path=Path.Combine(AppContext.BaseDirectory,"Fixtures","CaptureOverlayWindow.xaml.xml");
        var document=XDocument.Load(path,LoadOptions.SetLineInfo);
        XNamespace presentation="http://schemas.microsoft.com/winfx/2006/xaml/presentation";
        XNamespace x="http://schemas.microsoft.com/winfx/2006/xaml";

        XElement FindNamed(string name)=>document
            .Descendants()
            .Single(element=>string.Equals((string?)element.Attribute(x+"Name"),name,StringComparison.Ordinal));

        var host=FindNamed("PromptBarHost");
        var content=FindNamed("PromptBar");
        var shadowBorders=host.Elements(presentation+"Border")
            .Where(border=>border.Element(presentation+"Border.Effect")?.Element(presentation+"DropShadowEffect") is not null)
            .ToArray();

        Assert.Equal(2,shadowBorders.Length);
        Assert.All(shadowBorders,border=>Assert.Equal("False",(string?)border.Attribute("IsHitTestVisible")));
        Assert.Equal("True",(string?)content.Attribute("ClipToBounds"));
        Assert.Null(content.Element(presentation+"Border.Effect"));
        Assert.Same(host,content.Parent);
    }

    [Theory]
    [InlineData("Toolbar","ToolbarShadow","ToolbarSurface")]
    [InlineData("DrawingToolbar","DrawingToolbarShadow","DrawingToolbarSurface")]
    [InlineData("LongCaptureBar","LongCaptureBarShadow","LongCaptureBarSurface")]
    [InlineData("RecordingBar","RecordingBarShadow","RecordingBarSurface")]
    public void CaptureToolbarsKeepShadowSeparateFromVisibleBorder(string hostName,string shadowName,string surfaceName)
    {
        var path=Path.Combine(AppContext.BaseDirectory,"Fixtures","CaptureOverlayWindow.xaml.xml");
        var document=XDocument.Load(path,LoadOptions.SetLineInfo);
        XNamespace presentation="http://schemas.microsoft.com/winfx/2006/xaml/presentation";
        XNamespace x="http://schemas.microsoft.com/winfx/2006/xaml";
        XElement FindNamed(string name)=>document.Descendants().Single(element=>string.Equals((string?)element.Attribute(x+"Name"),name,StringComparison.Ordinal));

        var host=FindNamed(hostName);var shadow=FindNamed(shadowName);var surface=FindNamed(surfaceName);
        Assert.Equal("False",(string?)host.Attribute("ClipToBounds"));
        Assert.Equal("False",(string?)shadow.Attribute("IsHitTestVisible"));
        Assert.Equal("0",(string?)shadow.Attribute("BorderThickness"));
        Assert.NotNull(shadow.Element(presentation+"Border.Effect")?.Element(presentation+"DropShadowEffect"));
        Assert.Null(surface.Element(presentation+"Border.Effect"));
        Assert.Same(host,shadow.Parent);Assert.Same(host,surface.Parent);
    }

    [Fact]
    public void CaptureOverlayHasNamedReasoningScrollAndNonInteractiveRecordingCountdown()
    {
        var path=Path.Combine(AppContext.BaseDirectory,"Fixtures","CaptureOverlayWindow.xaml.xml");
        var document=XDocument.Load(path,LoadOptions.SetLineInfo);
        XNamespace x="http://schemas.microsoft.com/winfx/2006/xaml";
        XElement FindNamed(string name)=>document.Descendants().Single(element=>string.Equals((string?)element.Attribute(x+"Name"),name,StringComparison.Ordinal));

        Assert.Equal("ScrollViewer",FindNamed("ReasoningScroll").Name.LocalName);
        var countdown=FindNamed("RecordingCountdown");
        Assert.Equal("Collapsed",(string?)countdown.Attribute("Visibility"));
        Assert.Equal("False",(string?)countdown.Attribute("IsHitTestVisible"));
    }

    [Fact]
    public void CaptureSelectionToolbarFollowsTheUserWorkflow()
    {
        var path=Path.Combine(AppContext.BaseDirectory,"Fixtures","CaptureOverlayWindow.xaml.xml");
        var document=XDocument.Load(path,LoadOptions.SetLineInfo);
        XNamespace presentation="http://schemas.microsoft.com/winfx/2006/xaml/presentation";
        XNamespace x="http://schemas.microsoft.com/winfx/2006/xaml";
        var toolbar=document.Descendants().Single(element=>string.Equals((string?)element.Attribute(x+"Name"),"Toolbar",StringComparison.Ordinal));
        var names=toolbar.Descendants(presentation+"Button")
            .Select(button=>(string?)button.Attribute(x+"Name"))
            .Where(name=>name is not null)
            .ToArray();

        Assert.Equal(new[]
        {
            "ReferenceButton","DrawButton","OcrButton","TranslateButton","TableButton",
            "LongCaptureButton","RecordButton","VideoPlayButton","CopyButton","SaveButton","PinButton",
            // MCP 分享按钮（钉钉/飞书/Obsidian/IMA）在配置后隐藏显示，但结构上始终存在。
            "DingTalkButton","FeishuButton","ObsidianButton","ImaButton"
        },names);
        // MCP 分享按钮（钉钉/飞书/Obsidian/IMA）自带第 4 个分组分隔符。
        Assert.Equal(4,toolbar.Descendants(presentation+"Border").Count(border=>string.Equals((string?)border.Attribute("Style"),"{StaticResource ToolbarSeparator}",StringComparison.Ordinal)));
    }
}
