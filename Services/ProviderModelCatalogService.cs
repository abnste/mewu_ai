using System.Net.Http;
using System.Net.Http.Headers;
using System.Text.Json;

namespace mewu_ai_Assistant.Services;

internal sealed class ProviderModelCatalogService
{
    internal const int MaximumResponseBytes = 2 * 1024 * 1024;
    private static readonly HttpClient SharedClient = new(new HttpClientHandler { AllowAutoRedirect = false, UseCookies = false }) { Timeout = Timeout.InfiniteTimeSpan };
    private readonly HttpClient _client;
    internal ProviderModelCatalogService(HttpClient? client = null) => _client = client ?? SharedClient;

    internal async Task<IReadOnlyList<string>> GetModelsAsync(string baseUrl, string apiKey,
        IReadOnlyDictionary<string, string> customHeaders, CancellationToken token)
    {
        var uri = ProviderEndpointPolicy.NormalizeBaseUri(baseUrl);
        ProviderHeaderPolicy.EnsureValid(customHeaders);
        if (!string.IsNullOrWhiteSpace(apiKey) && customHeaders.Keys.Any(ProviderHeaderCredentialService.IsAuthentication))
            throw new InvalidOperationException(LocalizationService.T("API Key 与认证 Custom Header 不能同时发送", "Use either an API key or an authentication header, not both."));
        using var request = new HttpRequestMessage(HttpMethod.Get, new Uri(uri, "models"));
        if (!string.IsNullOrWhiteSpace(apiKey)) request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", apiKey);
        foreach (var header in customHeaders)
            if (!request.Headers.TryAddWithoutValidation(header.Key, header.Value))
                throw new InvalidOperationException(LocalizationService.T("无法添加模型列表请求头", "Could not add a model-list request header."));
        using var response = await _client.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, token).ConfigureAwait(false);
        if (!response.IsSuccessStatusCode)
            throw new InvalidOperationException(LocalizationService.T($"获取模型失败（HTTP {(int)response.StatusCode}）。请检查此提供商的密钥与地址；也可手动输入模型 ID。", $"Could not load models (HTTP {(int)response.StatusCode}). Check this provider's key and endpoint, or enter a model ID manually."));
        if (response.Content.Headers.ContentLength is > MaximumResponseBytes) throw TooLarge();
        using var stream = await response.Content.ReadAsStreamAsync(token).ConfigureAwait(false);
        using var buffer = new MemoryStream();
        var chunk = new byte[8192];
        while (true)
        {
            var count = await stream.ReadAsync(chunk, token).ConfigureAwait(false);
            if (count == 0) break;
            if (buffer.Length + count > MaximumResponseBytes) throw TooLarge();
            buffer.Write(chunk, 0, count);
        }
        using var document = JsonDocument.Parse(buffer.ToArray());
        if (document.RootElement.ValueKind != JsonValueKind.Object ||
            !document.RootElement.TryGetProperty("data", out var data) || data.ValueKind != JsonValueKind.Array)
            throw new InvalidDataException(LocalizationService.T("模型列表格式无效，可手动输入模型 ID。", "Invalid model list. You can still enter a model ID manually."));
        var models = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var item in data.EnumerateArray())
        {
            if (item.ValueKind != JsonValueKind.Object || !item.TryGetProperty("id", out var id) || id.ValueKind != JsonValueKind.String) continue;
            var value = id.GetString()?.Trim();
            if (value is not { Length: > 0 and <= 200 } || value.Any(char.IsControl)) continue;
            if (VolcengineModelPolicy.IsEndpoint(uri) && !VolcengineModelPolicy.IsChatModel(value)) continue;
            models.Add(value);
            if (models.Count >= 4096) break;
        }
        return models.Order(StringComparer.OrdinalIgnoreCase).ToArray();
    }

    private static InvalidDataException TooLarge() => new(LocalizationService.T("模型列表超过安全大小限制", "The model list exceeds the size limit."));
}
