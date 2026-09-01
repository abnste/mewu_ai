using System.Text.Json;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class HermesRuntimeServiceTests
{
    [Fact]
    public void AgentOptionsExposeDisplayNameDescriptionAndDefault()
    {
        using var document=JsonDocument.Parse("""
        {"profiles":[
          {"name":"default","display_name":"Hermes","description":"通用助手","model":"m1","provider":"p1","is_default":true},
          {"name":"coder","display_name":"代码猫","description":"专注开发","model":"m2","provider":"p2","is_default":false},
          {"name":"bad profile","display_name":"忽略"}
        ]}
        """);

        var options=HermesRuntimeService.ParseAgentOptions(document.RootElement);

        Assert.Equal(2,options.Count);
        Assert.Equal("default",options[0].Name);Assert.True(options[0].IsDefault);Assert.Equal("Hermes",options[0].Label);
        Assert.Equal("coder",options[1].Name);Assert.Equal("代码猫",options[1].Label);Assert.Equal("专注开发",options[1].Description);
    }

    [Theory]
    [InlineData(null,"default")]
    [InlineData(" coder ","coder")]
    public void ProfileNormalizationAcceptsOnlySafeHermesSlugs(string? value,string expected)
        =>Assert.Equal(expected,HermesRuntimeService.NormalizeProfile(value));

    [Theory]
    [InlineData("bad profile")]
    [InlineData("../escape")]
    [InlineData("--profile")]
    public void ProfileNormalizationRejectsUnsafeValues(string value)
        =>Assert.Throws<InvalidOperationException>(()=>HermesRuntimeService.NormalizeProfile(value));

    [Fact]
    public void ModelOptionsPutTheCurrentHermesModelFirst()
    {
        using var document=JsonDocument.Parse("""
        {
          "provider":"provider-b",
          "model":"model-2",
          "providers":[
            {"slug":"provider-a","name":"A","models":["model-1"]},
            {"slug":"provider-b","name":"B","models":["model-2","model-3"]}
          ]
        }
        """);

        var options=HermesRuntimeService.ParseModelOptions(document.RootElement);

        Assert.Equal("model-2",options[0].Model);
        Assert.True(options[0].IsCurrent);
        Assert.Single(options,option=>option.IsCurrent);
    }
}
