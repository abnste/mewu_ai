using System.Net;
using System.Net.Http;
using System.Text;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ProviderModelCatalogTests
{
    [Theory]
    [InlineData("https://api.minimaxi.com/v1", "MiniMax-M3")]
    [InlineData("https://api.openai.com/v1", "gpt-example")]
    [InlineData("https://ark.cn-beijing.volces.com/api/v3", "glm-5-3-flash")]
    [InlineData("http://localhost:1234/v1", "local-model")]
    public async Task LoadsFromSelectedEndpointAndPreservesIds(string endpoint, string model)
    {
        using var handler = new Handler(request =>
        {
            Assert.Equal(endpoint + "/models", request.RequestUri!.AbsoluteUri);
            Assert.Equal("Bearer", request.Headers.Authorization!.Scheme);
            Assert.Equal("test-key", request.Headers.Authorization.Parameter);
            return Json("{\"data\":[null,1,{}, {\"id\":\"" + model + "\"},{\"id\":\"" + model + "\"}]}");
        });
        using var client = new HttpClient(handler);
        var result = await new ProviderModelCatalogService(client).GetModelsAsync(endpoint, "test-key", new Dictionary<string, string>(), TestContext.Current.CancellationToken);
        Assert.Equal([model], result);
    }

    [Theory]
    [InlineData("[]")]
    [InlineData("{\"data\":null}")]
    [InlineData("{\"models\":[]}")]
    public async Task RejectsInvalidEnvelopes(string body)
    {
        using var client = new HttpClient(new Handler(_ => Json(body)));
        await Assert.ThrowsAsync<InvalidDataException>(() => new ProviderModelCatalogService(client).GetModelsAsync("https://api.minimaxi.com/v1", "test", new Dictionary<string,string>(), TestContext.Current.CancellationToken));
    }

    [Fact]
    public async Task EnforcesLimitWithoutContentLength()
    {
        using var client = new HttpClient(new Handler(_ => new HttpResponseMessage(HttpStatusCode.OK)
        { Content = new StreamContent(new NonSeekableStream(new byte[ProviderModelCatalogService.MaximumResponseBytes + 1])) }));
        await Assert.ThrowsAsync<InvalidDataException>(() => new ProviderModelCatalogService(client).GetModelsAsync("https://api.minimaxi.com/v1", "test", new Dictionary<string,string>(), TestContext.Current.CancellationToken));
    }

    [Fact]
    public async Task RejectsCompetingCredentialsBeforeNetwork()
    {
        using var client = new HttpClient(new Handler(_ => throw new Xunit.Sdk.XunitException("Unexpected request")));
        await Assert.ThrowsAsync<InvalidOperationException>(() => new ProviderModelCatalogService(client).GetModelsAsync("https://api.minimaxi.com/v1", "test", new Dictionary<string,string> { ["Authorization"] = "Bearer header-key" }, TestContext.Current.CancellationToken));
    }

    [Fact]
    public async Task DoesNotExposeResponseSecretsOrFollowRedirect()
    {
        using var client = new HttpClient(new Handler(_ => new HttpResponseMessage(HttpStatusCode.Redirect) { Content = new StringContent("secret-path-key") }));
        var error = await Assert.ThrowsAsync<InvalidOperationException>(() => new ProviderModelCatalogService(client).GetModelsAsync("https://api.minimaxi.com/v1", "test", new Dictionary<string,string>(), TestContext.Current.CancellationToken));
        Assert.Contains("302", error.Message);
        Assert.DoesNotContain("secret-path-key", error.Message);
    }

    [Fact]
    public async Task VolcengineFiltersGenerationButDoesNotInventRecommendedModels()
    {
        using var client = new HttpClient(new Handler(_ => Json("{\"data\":[{\"id\":\"doubao-seedream\"},{\"id\":\"glm-5-3-flash\"}]}")));
        var models = await new ProviderModelCatalogService(client).GetModelsAsync(VolcengineModelPolicy.StandardBaseUrl, "test", new Dictionary<string,string>(), TestContext.Current.CancellationToken);
        Assert.Equal(["glm-5-3-flash"], models);
    }

    [Fact]
    public void PresetsUseProviderNamesAndKeepCredentialsIsolated()
    {
        foreach (var preset in ProviderPresetPolicy.All)
        {
            var first = ProviderPresetPolicy.Create(preset);
            var second = ProviderPresetPolicy.Create(preset);
            Assert.NotEqual(first.Id, second.Id);
            Assert.Empty(second.CredentialId);
            Assert.Empty(second.CustomHeaders);
            Assert.Empty(second.SensitiveHeaderCredentialIds);
            Assert.Equal(preset.Id, ProviderPresetPolicy.Detect(first).Id);
        }
        Assert.Equal("MiniMax", new AiProviderSettings().Name);
        Assert.Equal("MiniMax", ProviderPresetPolicy.DisplayName(new AiProviderSettings { Name = "MiniMax M3" }));
        Assert.Equal("My API", ProviderPresetPolicy.DisplayName(new AiProviderSettings { Name = "My API" }));
        Assert.Equal("Custom", ProviderPresetPolicy.Detect(new AiProviderSettings { BaseUrl = "https://api.minimaxi.com.attacker.test/v1" }).Id);
    }

    private static HttpResponseMessage Json(string body) => new(HttpStatusCode.OK) { Content = new StringContent(body, Encoding.UTF8, "application/json") };
    [Fact]
    public void ExactlyFourProvidersOnlyOpenAiCompatibleRequiresUrl()
    {
        Assert.Equal(4,ProviderPresetPolicy.All.Length);
        Assert.Equal(new[]{"MiniMax (CN)","MiniMax","火山引擎","OpenAI 通用"},ProviderPresetPolicy.All.Select(p=>p.Name));
        Assert.Equal("Custom",Assert.Single(ProviderPresetPolicy.All,p=>p.RequiresBaseUrl).Id);
        Assert.All(ProviderPresetPolicy.All.Where(p=>!p.RequiresBaseUrl),p=>Assert.True(Uri.IsWellFormedUriString(p.BaseUrl,UriKind.Absolute)));
        Assert.Equal("Custom",ProviderPresetPolicy.Detect(new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://api.openai.com/v1"}).Id);
        Assert.Equal("Custom",ProviderPresetPolicy.Detect(new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.test/v1"}).Id);
    }

    [Fact]
    public void OnlyUntouchedAutomaticallyCreatedDraftsMayBeDiscarded()
    {
        var draft = ProviderPresetPolicy.Create(ProviderPresetPolicy.All.Single(p => p.Id == "Custom"));
        Assert.True(ProviderPresetPolicy.IsUntouchedDraft(draft, false));
        Assert.False(ProviderPresetPolicy.IsUntouchedDraft(draft, true));
        draft.Model = "chosen-model";
        Assert.False(ProviderPresetPolicy.IsUntouchedDraft(draft, false));
        draft.Model = "";
        draft.CustomHeaders["X-Tenant"] = "tenant";
        Assert.False(ProviderPresetPolicy.IsUntouchedDraft(draft, false));
    }
    private sealed class Handler(Func<HttpRequestMessage,HttpResponseMessage> respond) : HttpMessageHandler
    {
        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) => Task.FromResult(respond(request));
    }
    private sealed class NonSeekableStream(byte[] bytes) : MemoryStream(bytes)
    {
        public override bool CanSeek => false;
    }
}
