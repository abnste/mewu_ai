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
}
