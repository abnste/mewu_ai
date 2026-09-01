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
}
