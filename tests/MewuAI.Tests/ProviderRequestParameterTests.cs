using System.Net;
using System.Text.Json;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ProviderRequestParameterTests
{
    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task PriorityIsSentInBodyForBothRequestPaths(bool stream)
    {
        var settings = new AiProviderSettings { RequestParameters = ProviderRequestParameterPolicy.Parse("{\"service_tier\":\"priority\",\"temperature\":0.7,\"top_p\":0.9}") };
        var provider = new OpenAiCompatibleProvider(settings, "test", async (request, _, token) =>
        {
            Assert.False(request.Headers.Contains("service_tier"));
            using var body = JsonDocument.Parse(await request.Content!.ReadAsStringAsync(token));
            Assert.Equal("priority", body.RootElement.GetProperty("service_tier").GetString());
            Assert.Equal(0.7, body.RootElement.GetProperty("temperature").GetDouble());
            Assert.Equal(stream, body.RootElement.GetProperty("stream").GetBoolean());
            Assert.Equal("disabled", body.RootElement.GetProperty("thinking").GetProperty("type").GetString());
            return new HttpResponseMessage(HttpStatusCode.OK) { Content = new StringContent(stream
                ? "data: {\"choices\":[{\"delta\":{\"content\":\"MEWU_OK\"},\"finish_reason\":\"stop\"}]}\n\n"
                : "{\"choices\":[{\"message\":{\"content\":\"MEWU_OK\"}}]}") };
        }, _ => TimeSpan.FromSeconds(10));
        var result = await provider.SendAsync(new AiRequest { Prompt = "test", DisableReasoning = true, StreamingProgress = stream ? new Progress<AiStreamDelta>() : null }, TestContext.Current.CancellationToken);
        Assert.Equal("MEWU_OK", result.Answer);
    }

    [Theory]
    [InlineData("{\"messages\":[]}")]
    [InlineData("{\"stream\":false}")]
    [InlineData("{\"api_key\":\"secret\"}")]
    [InlineData("{\"thinking\":{\"type\":\"adaptive\"}}")]
    [InlineData("{\"service_tier\":\"invalid\"}")]
    [InlineData("{\"temperature\":3}")]
    [InlineData("{\"top_p\":0}")]
    [InlineData("{\"service_tier\":\"standard\",\"service_tier\":\"priority\"}")]
    [InlineData("null")]
    [InlineData("[]")]
    public void RejectsUnsafeOverridesAndInvalidValues(string json) =>
        Assert.Throws<InvalidOperationException>(() => ProviderRequestParameterPolicy.Parse(json));

    [Fact]
    public void DefaultsAndOldSettingsDoNotEnablePriority()
    {
        Assert.Empty(new AiProviderSettings().RequestParameters);
        Assert.Empty(ProviderRequestParameterPolicy.Parse("{}"));
        var old = JsonSerializer.Deserialize<AiProviderSettings>("{\"Id\":\"old\",\"Type\":\"MiniMax\",\"BaseUrl\":\"https://api.minimaxi.com/v1\",\"Model\":\"MiniMax-M3\"}")!;
        Assert.Empty(old.RequestParameters);
    }

    [Fact]
    public void CloneAndSettingsRoundTripRetainParametersAndOriginalHeaders()
    {
        var provider = new AiProviderSettings { RequestParameters = ProviderRequestParameterPolicy.Parse("{\"service_tier\":\"priority\"}"), CustomHeaders = new() { ["X-Tenant"] = "test" } };
        var copy = ProviderHeaderCredentialService.Clone(provider);
        Assert.NotSame(provider.RequestParameters, copy.RequestParameters);
        var root = Path.Combine(Path.GetTempPath(), "mewu-parameters-" + Guid.NewGuid().ToString("N"));
        try
        {
            var service = new SettingsService(Path.Combine(root, "settings.json"));
            service.Save(new AppSettings { HermesEnabled = true, HermesProvider = "test", HermesModel = "test", Providers = [copy], DefaultProviderId = copy.Id });
            var loaded = Assert.Single(service.Load().Providers);
            Assert.Equal("priority", loaded.RequestParameters["service_tier"].GetString());
            Assert.Equal("test", loaded.CustomHeaders["X-Tenant"]);
        }
        finally { if (Directory.Exists(root)) Directory.Delete(root, true); }
    }
}
