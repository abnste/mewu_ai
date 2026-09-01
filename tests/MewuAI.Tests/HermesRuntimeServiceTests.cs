using System.Text.Json;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class HermesRuntimeServiceTests
{
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
