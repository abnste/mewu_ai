// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using System.Xml.Linq;
using Xunit;

namespace MewuAI.Tests;

public sealed class ReleasePreparationTests
{
    [Fact]
    public void PromptInputKeepsFlexibleSpaceWithoutOverlappingActionButtons()
    {
        var document=XDocument.Load(Fixture("CaptureOverlayWindow.xaml.xml"));
        XNamespace names="http://schemas.microsoft.com/winfx/2006/xaml";
        var input=document.Descendants().Single(element=>(string?)element.Attribute(names+"Name")=="PromptInputBorder");
        var grid=input.Parent!;var column=int.Parse(input.Attribute("Grid.Column")!.Value,System.Globalization.CultureInfo.InvariantCulture);
        var definitions=grid.Elements().Single(element=>element.Name.LocalName=="Grid.ColumnDefinitions").Elements().ToArray();
        Assert.InRange(column,0,definitions.Length-1);
        Assert.True(definitions[column].Attribute("Width") is null||(string?)definitions[column].Attribute("Width")=="*");
        var occupied=grid.Elements().Where(element=>element.Attribute("Grid.Column") is not null).Select(element=>(string)element.Attribute("Grid.Column")!).ToArray();
        Assert.Equal(occupied.Length,occupied.Distinct().Count());
    }

    private static string Fixture(string name)=>Path.Combine(AppContext.BaseDirectory,"Fixtures",name);

    [Fact]
    public void ProductInstallerAndReleaseNotesUseSameVersion()
    {
        var project=XDocument.Load(Fixture("Product.csproj.xml"));
        var propertyGroup=project.Root!.Elements("PropertyGroup").First();
        Assert.Equal("0.4.4",propertyGroup.Element("Version")?.Value);
        Assert.Equal("0.4.4.0",propertyGroup.Element("AssemblyVersion")?.Value);
        Assert.Equal("0.4.4.0",propertyGroup.Element("FileVersion")?.Value);

        var installer=File.ReadAllText(Fixture("MewuAI.iss.txt"));
        Assert.Contains("#define MyAppVersion \"0.4.4\"",installer,StringComparison.Ordinal);
        Assert.Contains("VersionInfoVersion=0.4.4.0",installer,StringComparison.Ordinal);
        Assert.True(File.Exists(Fixture("release-notes-v0.4.4.md")));
    }

    [Fact]
    public void LockedDependenciesAndProviderSmokeAreReleaseInputs()
    {
        using var lockDocument=JsonDocument.Parse(File.ReadAllText(Fixture("Product.packages.lock.json")));
        Assert.Equal(1,lockDocument.RootElement.GetProperty("version").GetInt32());
        var dependencies=lockDocument.RootElement.GetProperty("dependencies");
        Assert.Contains(dependencies.EnumerateObject(),framework=>framework.Value.TryGetProperty("Microsoft.Graphics.Win2D",out _));

        var solution=XDocument.Load(Fixture("Product.slnx.xml"));
        Assert.Contains(solution.Descendants("Project"),project=>string.Equals((string?)project.Attribute("Path"),"tests/MewuAI.ProviderSmoke/MewuAI.ProviderSmoke.csproj",StringComparison.Ordinal));
    }

    [Fact]
    public void RedistributableLicensePathsSupportPackageRootsWithoutTrailingSeparator()
    {
        var project=XDocument.Load(Fixture("Product.csproj.xml"));
        var sources=project.Descendants("Content").Select(element=>(string?)element.Attribute("Include"))
            .Where(value=>value?.StartsWith("$(NuGetPackageRoot)",StringComparison.Ordinal)==true).ToArray();
        Assert.Equal(4,sources.Length);
        foreach(var source in sources)Assert.StartsWith("$(NuGetPackageRoot)\\",source,StringComparison.Ordinal);
    }

    [Fact]
    public void TransitiveRedistributableLicensesAreDeclaredAndPackaged()
    {
        var project=XDocument.Load(Fixture("Product.csproj.xml"));
        var links=project.Descendants("Content").Select(element=>(string?)element.Attribute("Link")).Where(value=>value is not null).ToHashSet(StringComparer.OrdinalIgnoreCase);
        Assert.Contains("Licenses\\Microsoft.Web.WebView2-LICENSE.txt",links);
        Assert.Contains("Licenses\\Microsoft.Web.WebView2-NOTICE.txt",links);
        Assert.Contains("Licenses\\Microsoft.WindowsAppSDK-LICENSE.txt",links);
        Assert.Contains("Licenses\\Microsoft.WindowsAppSDK-NOTICE.txt",links);

        var notices=File.ReadAllText(Fixture("THIRD-PARTY-NOTICES.md"));
        Assert.Contains("Microsoft Windows App SDK",notices,StringComparison.Ordinal);
        Assert.Contains("Microsoft WebView2 SDK",notices,StringComparison.Ordinal);
    }
}
