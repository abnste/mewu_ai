using System.Diagnostics;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class HermesLaunchPolicyTests : IDisposable
{
    private readonly string _home=Path.Combine(Path.GetTempPath(),"MewuAI-HermesLaunchTests",Guid.NewGuid().ToString("N"));
    private string Agent=>Path.Combine(_home,"hermes-agent");

    private string Add(string relative)
    {
        var path=Path.GetFullPath(Path.Combine(_home,relative));
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path,string.Empty);
        return path;
    }

    private HermesInstallation Installation(string launcher)=>new(_home,Agent,launcher,Path.Combine(_home,"config.yaml"));

    [Theory]
    [InlineData("venv")]
    [InlineData(".venv")]
    public void LaunchUsesTheSameVenvAndIgnoresForeignPythonEnvironment(string venv)
    {
        var launcher=Add($"hermes-agent/{venv}/Scripts/hermes.exe");
        var python=Add($"hermes-agent/{venv}/Scripts/python.exe");
        Add($"hermes-agent/{(venv=="venv"?".venv":"venv")}/Scripts/python.exe");
        Add("node/node.exe");
        var start=new ProcessStartInfo{FileName=launcher};
        start.Environment["PYTHONHOME"]="Z:\\unrelated-python";
        start.Environment["PYTHONPATH"]="Z:\\untrusted-modules";
        start.Environment["VIRTUAL_ENV"]="Z:\\wrong-venv";
        start.Environment["PATH"]="C:\\Windows\\System32";

        HermesLaunchPolicy.Configure(start,Installation(launcher));

        Assert.Equal(python,start.FileName);
        Assert.Equal(["-m","hermes_cli.main"],start.ArgumentList);
        Assert.Equal(Agent,start.Environment["PYTHONPATH"]);
        Assert.False(start.Environment.ContainsKey("PYTHONHOME"));
        Assert.Equal(Path.Combine(Agent,venv),start.Environment["VIRTUAL_ENV"]);
        Assert.Equal("utf-8",start.Environment["PYTHONIOENCODING"]);
        Assert.Equal("1",start.Environment["PYTHONNOUSERSITE"]);
        Assert.StartsWith(Path.Combine(Agent,venv,"Scripts"),start.Environment["PATH"],StringComparison.OrdinalIgnoreCase);
        Assert.Contains(Path.Combine(_home,"node"),start.Environment["PATH"],StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain("wrong-venv",start.Environment["PATH"],StringComparison.Ordinal);
        Assert.EndsWith("C:\\Windows\\System32",start.Environment["PATH"],StringComparison.Ordinal);
    }

    [Fact]
    public void CopiedBinShimIsBypassedWhenManagedPythonExists()
    {
        var launcher=Add("bin/hermes.exe");
        var python=Add("hermes-agent/venv/Scripts/python.exe");
        var start=HermesBackendService.CreateStartInfo(Installation(launcher),"test-token");
        Assert.Equal(python,start.FileName);
        Assert.Equal(["-m","hermes_cli.main","-p","default","serve","--host","127.0.0.1","--port","0"],start.ArgumentList);
        Assert.False(start.Environment.ContainsKey("HERMES_DESKTOP"));
        Assert.Equal(_home,start.Environment["HERMES_HOME"]);
        Assert.DoesNotContain("--insecure",start.ArgumentList);
    }

    [Fact]
    public void MissingInterpreterDoesNotSelectAnUnrelatedVenvOrGlobalPython()
    {
        var launcher=Add("hermes-agent/.venv/Scripts/hermes.exe");
        Add("hermes-agent/venv/Scripts/python.exe");
        var start=new ProcessStartInfo{FileName=launcher};
        HermesLaunchPolicy.Configure(start,Installation(launcher));
        Assert.Equal(launcher,start.FileName);
        Assert.Empty(start.ArgumentList);
        Assert.False(start.Environment.ContainsKey("VIRTUAL_ENV"));
        Assert.False(start.Environment.ContainsKey("PYTHONPATH"));
    }

    [Theory]
    [InlineData("ModuleNotFoundError: No module named 'private-name'", "Python")]
    [InlineData("Fatal error in launcher: token=private-token", "Python")]
    [InlineData("PermissionError: C:\\private-path", "拒绝")]
    [InlineData("UnicodeEncodeError: private-prompt", "编码")]
    [InlineData("error: unrecognized arguments: private-flag", "参数")]
    public void DiagnosticsExplainTheCategoryWithoutEchoingPrivateOutput(string input,string expected)
    {
        var diagnostics=new HermesStartupDiagnostics();
        diagnostics.Observe(input);
        var message=diagnostics.Describe(1);
        Assert.Contains(expected,message,StringComparison.Ordinal);
        Assert.DoesNotContain("private",message,StringComparison.Ordinal);
        Assert.Contains("1",message,StringComparison.Ordinal);
    }

    [Fact]
    public void UnknownAndOversizedOutputIsNotRetainedOrPresentedAsBrokenInstallation()
    {
        var diagnostics=new HermesStartupDiagnostics();
        diagnostics.Observe("api_key=private-token");
        diagnostics.Observe(new string('a',4097)+"ModuleNotFoundError");
        var message=diagnostics.Describe(1);
        Assert.DoesNotContain("private",message,StringComparison.Ordinal);
        Assert.DoesNotContain("Python",message,StringComparison.Ordinal);
        Assert.Contains("不能仅凭",message,StringComparison.Ordinal);
    }

    public void Dispose(){if(Directory.Exists(_home))Directory.Delete(_home,true);}
}
