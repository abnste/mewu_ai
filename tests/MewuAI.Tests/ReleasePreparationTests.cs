using System.Text.Json;
using System.Xml.Linq;
using Xunit;

namespace MewuAI.Tests;

public sealed class ReleasePreparationTests
{
    private static string Fixture(string name)=>Path.Combine(AppContext.BaseDirectory,"Fixtures",name);

    [Fact]
    public void ProductInstallerAndReleaseNotesUseSameVersion()
    {
        var project=XDocument.Load(Fixture("Product.csproj.xml"));
        var propertyGroup=project.Root!.Elements("PropertyGroup").First();
        Assert.Equal("0.2.5",propertyGroup.Element("Version")?.Value);
        Assert.Equal("0.2.5.0",propertyGroup.Element("AssemblyVersion")?.Value);
        Assert.Equal("0.2.5.0",propertyGroup.Element("FileVersion")?.Value);

        var installer=File.ReadAllText(Fixture("MewuAI.iss.txt"));
        Assert.Contains("#define MyAppVersion \"0.2.5\"",installer,StringComparison.Ordinal);
        Assert.Contains("VersionInfoVersion=0.2.5.0",installer,StringComparison.Ordinal);
        Assert.True(File.Exists(Fixture("release-notes-v0.2.5.md")));
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
