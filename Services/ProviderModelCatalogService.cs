// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net.Http;
using System.Net.Http.Headers;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace mewu_ai_Assistant.Services;

internal sealed class ProviderModelCatalogService
{
    internal const int MaximumResponseBytes = 2 * 1024 * 1024;
    internal const int MaximumTotalResponseBytes = 8 * 1024 * 1024;
    internal const int MaximumPages = 32;
    internal const int MaximumModels = 4096;
    private static readonly HttpClient SharedClient = NetworkHttpClientFactory.Create();
    private readonly HttpClient _client;
    internal string? LastSuccessfulBaseUrl { get; private set; }
    internal ProviderModelCatalogService(HttpClient? client = null) => _client = client ?? SharedClient;

    internal async Task<IReadOnlyList<string>> GetModelsAsync(string baseUrl, string apiKey,
        IReadOnlyDictionary<string, string> customHeaders, CancellationToken token, string authMode="bearer")
    {
        LastSuccessfulBaseUrl = null;
        var uri = ProviderEndpointPolicy.NormalizeBaseUri(baseUrl);
        var candidates = GetEndpointCandidates(uri);
        Exception? last = null;
        foreach (var candidate in candidates)
        {
            token.ThrowIfCancellationRequested();
            try
            {
                var models = await GetModelsAtEndpointAsync(candidate.AbsoluteUri, apiKey, customHeaders, token, authMode).ConfigureAwait(false);
                LastSuccessfulBaseUrl = candidate.AbsoluteUri;
                return models;
            }
            catch (OperationCanceledException) when (token.IsCancellationRequested)
            {
                throw;
            }
            catch (Exception ex) when (IsEndpointCandidateFailure(ex) && candidate != candidates[^1])
            {
                // A bare host is commonly entered without the OpenAI-compatible
                // path. Keep the original draft untouched and try only the
                // bounded, same-host candidates selected above.
                last = ex;
            }
            catch (Exception ex)
            {
                last = ex;
                break;
            }
        }

        throw last ?? InvalidList();
    }

    internal static IReadOnlyList<Uri> GetEndpointCandidates(Uri normalizedBaseUri)
    {
        if (normalizedBaseUri.AbsolutePath != "/" ||
            (!normalizedBaseUri.Scheme.Equals(Uri.UriSchemeHttps, StringComparison.OrdinalIgnoreCase) &&
             !normalizedBaseUri.IsLoopback))
            return [normalizedBaseUri];

        // Do not reinterpret an explicitly configured provider path. For a
        // genuine bare OpenAI-compatible base, /v1 is conventional and
        // /api/v1 is the only other common suffix worth trying automatically.
        var candidates = new List<Uri> { normalizedBaseUri };
        foreach (var path in new[] { "/v1/", "/api/v1/" })
        {
            var candidate = new UriBuilder(normalizedBaseUri) { Path = path }.Uri;
            if (!candidates.Contains(candidate)) candidates.Add(candidate);
        }
        return candidates;
    }

    private static bool IsEndpointCandidateFailure(Exception exception) =>
        exception is InvalidDataException or HttpRequestException ||
        exception is InvalidOperationException invalidOperation &&
        (invalidOperation.Message.Contains("HTTP ", StringComparison.Ordinal) ||
         invalidOperation.Message.Contains("Could not load models", StringComparison.Ordinal));

    private async Task<IReadOnlyList<string>> GetModelsAtEndpointAsync(string baseUrl, string apiKey,
        IReadOnlyDictionary<string, string> customHeaders, CancellationToken token, string authMode)
    {
        var uri = ProviderEndpointPolicy.NormalizeBaseUri(baseUrl);
        ProviderHeaderPolicy.EnsureValid(customHeaders);
        if (!string.IsNullOrWhiteSpace(apiKey) && customHeaders.Keys.Any(ProviderHeaderCredentialService.IsAuthentication))
            throw new InvalidOperationException(LocalizationService.T("API Key 与认证 Custom Header 不能同时发送", "Use either an API key or an authentication header, not both."));
        var normalizedAuth=(authMode??"bearer").Trim().ToLowerInvariant();
        if(normalizedAuth is "auto" or "")normalizedAuth="bearer";
        if(normalizedAuth is "none" or "anonymous")normalizedAuth="none";
        else if(normalizedAuth is "api_key" or "api-key" or "x-api-key" or "anthropic_api_key")normalizedAuth="api_key";
        else normalizedAuth="bearer";
        AuthenticationHeaderValue? authorization = null;
        try
        {
            if (normalizedAuth=="bearer"&&!string.IsNullOrWhiteSpace(apiKey)) authorization = new AuthenticationHeaderValue("Bearer", apiKey);
        }
        catch (FormatException)
        {
            // Classify invalid editor input before networking. Never echo the
            // credential or let a header-format exception escape a UI event.
            throw new InvalidOperationException(LocalizationService.T(
                "API Key 格式无效，请检查是否包含换行或空字符。",
                "Invalid API key format. Check for line breaks or null characters."));
        }
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(token);
        timeout.CancelAfter(TimeSpan.FromSeconds(60));
        token = timeout.Token;
        var kind = GetCatalogKind(uri);
        var models = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var cursors = new HashSet<string>(StringComparer.Ordinal);
        var pageFingerprints = new HashSet<string>(StringComparer.Ordinal);
        string? cursor = null;
        var totalBytes = 0;
        var receivedItems = 0;
        for (var page = 1; page <= MaximumPages; page++)
        {
            token.ThrowIfCancellationRequested();
            using var request = new HttpRequestMessage(HttpMethod.Get, GetPageUri(uri, kind, page, cursor));
            if (authorization is not null) request.Headers.Authorization = authorization;
            if(normalizedAuth=="api_key"&&!string.IsNullOrWhiteSpace(apiKey))request.Headers.TryAddWithoutValidation("x-api-key",apiKey);
            foreach (var header in customHeaders)
                if (!request.Headers.TryAddWithoutValidation(header.Key, header.Value))
                    throw new InvalidOperationException(LocalizationService.T("无法添加模型列表请求头", "Could not add a model-list request header."));
            if (kind == CatalogKind.Anthropic && !request.Headers.Contains("anthropic-version"))
                request.Headers.Add("anthropic-version", "2023-06-01");
            using var response = await _client.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, token).ConfigureAwait(false);
            if (!response.IsSuccessStatusCode)
                throw new InvalidOperationException(LocalizationService.T($"获取模型失败（HTTP {(int)response.StatusCode}）。请检查此提供商的密钥与地址；也可手动输入模型 ID。", $"Could not load models (HTTP {(int)response.StatusCode}). Check this provider's key and endpoint, or enter a model ID manually."));
            var bytes = await ReadPageAsync(response.Content, MaximumTotalResponseBytes - totalBytes, token).ConfigureAwait(false);
            totalBytes += bytes.Length;
            JsonDocument document;
            try { document = JsonDocument.Parse(bytes); }
            catch (JsonException) { throw InvalidList(); }
            using (document)
            {
                var root = document.RootElement;
                var envelope = root;
                JsonElement data;
                if (kind == CatalogKind.ArrayCompatible && root.ValueKind == JsonValueKind.Array) data = root;
                else
                {
                    if (root.ValueKind != JsonValueKind.Object) throw InvalidList();
                    if (kind == CatalogKind.DashScope &&
                        (!root.TryGetProperty("output", out envelope) || envelope.ValueKind != JsonValueKind.Object)) throw InvalidList();
                    if (!envelope.TryGetProperty(kind == CatalogKind.DashScope ? "models" : "data", out data) || data.ValueKind != JsonValueKind.Array)
                        throw InvalidList();
                }
                var pageCount = data.GetArrayLength();
                receivedItems += pageCount;
                var pageIds = new StringBuilder();
                foreach (var item in data.EnumerateArray())
                {
                    token.ThrowIfCancellationRequested();
                    if (item.ValueKind != JsonValueKind.Object) continue;
                    var value = GetString(item, kind == CatalogKind.DashScope ? "model" : "id")?.Trim();
                    if (!IsValidId(value)) continue;
                    pageIds.Append(value).Append('\n');
                    if (!IsConversationModel(uri, item, value!)) continue;
                    models.Add(value!);
                    if (models.Count > MaximumModels) throw TooLarge();
                }
                var hasMore = false;
                if (kind == CatalogKind.DashScope)
                {
                    if (!envelope.TryGetProperty("total", out var total) || total.ValueKind != JsonValueKind.Number || !total.TryGetInt32(out var count) || count < 0)
                        throw InvalidList();
                    if (envelope.TryGetProperty("page_no", out var pageNumber) &&
                        (pageNumber.ValueKind != JsonValueKind.Number || !pageNumber.TryGetInt32(out var returnedPage) || returnedPage != page)) throw InvalidList();
                    hasMore = receivedItems < count;
                }
                else if (kind == CatalogKind.Anthropic && root.TryGetProperty("has_more", out var more))
                {
                    if (more.ValueKind is not (JsonValueKind.True or JsonValueKind.False)) throw InvalidList();
                    hasMore = more.GetBoolean();
                    if (hasMore)
                    {
                        cursor = GetString(root, "last_id");
                        if (!IsValidId(cursor) || !cursors.Add(cursor!)) throw IncompleteList();
                    }
                }
                if (pageCount > 0)
                {
                    var fingerprint = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(pageIds.ToString())));
                    if (!pageFingerprints.Add(fingerprint)) throw IncompleteList();
                }
                if (!hasMore)
                {
                    token.ThrowIfCancellationRequested();
                    return models.Order(StringComparer.OrdinalIgnoreCase).ToArray();
                }
                if (pageCount == 0) throw IncompleteList();
            }
        }
        throw IncompleteList();
    }

    private enum CatalogKind { OpenAi, Anthropic, DashScope, SiliconFlow, ArrayCompatible }

    private static CatalogKind GetCatalogKind(Uri uri)
    {
        if (!uri.IsDefaultPort || uri.Scheme != Uri.UriSchemeHttps) return CatalogKind.OpenAi;
        if (uri.Host.Equals("api.anthropic.com", StringComparison.OrdinalIgnoreCase) && uri.AbsolutePath == "/v1/") return CatalogKind.Anthropic;
        if (uri.AbsolutePath == "/compatible-mode/v1/" && IsDashScopeHost(uri.Host)) return CatalogKind.DashScope;
        if (uri.AbsolutePath == "/v1/" && (uri.Host.Equals("api.siliconflow.cn", StringComparison.OrdinalIgnoreCase) ||
            uri.Host.Equals("api.siliconflow.com", StringComparison.OrdinalIgnoreCase))) return CatalogKind.SiliconFlow;
        if (uri.AbsolutePath == "/v1/" && new[] { "api.together.xyz", "api.together.ai", "api.mistral.ai" }
                .Contains(uri.Host, StringComparer.OrdinalIgnoreCase)) return CatalogKind.ArrayCompatible;
        return CatalogKind.OpenAi;
    }

    internal static bool IsDashScopeHost(string host)
    {
        if (host.Equals("dashscope.aliyuncs.com", StringComparison.OrdinalIgnoreCase) ||
            host.Equals("dashscope-intl.aliyuncs.com", StringComparison.OrdinalIgnoreCase) ||
            host.Equals("dashscope-us.aliyuncs.com", StringComparison.OrdinalIgnoreCase) ||
            host.Equals("cn-hongkong.dashscope.aliyuncs.com", StringComparison.OrdinalIgnoreCase)) return true;
        // A workspace occupies exactly one DNS label; arbitrary lookalike domains are never rewritten.
        string[] regions = ["cn-beijing", "ap-southeast-1", "ap-northeast-1", "eu-central-1", "us-east-1"];
        return regions.Any(region =>
        {
            var suffix = "." + region + ".maas.aliyuncs.com";
            if (!host.EndsWith(suffix, StringComparison.OrdinalIgnoreCase)) return false;
            var workspace = host[..^suffix.Length];
            return workspace.Length is > 0 and <= 63 && workspace.All(c => char.IsAsciiLetterOrDigit(c) || c == '-') &&
                workspace[0] != '-' && workspace[^1] != '-';
        });
    }

    private static Uri GetPageUri(Uri baseUri, CatalogKind kind, int page, string? cursor) => kind switch
    {
        CatalogKind.DashScope => new UriBuilder(baseUri) { Path = "/api/v1/models", Query = $"page_no={page}&page_size=100&capabilities=TG&capabilities=VU&capabilities=Reasoning" }.Uri,
        CatalogKind.SiliconFlow => new Uri(baseUri, "models?sub_type=chat"),
        CatalogKind.Anthropic => new Uri(baseUri, "models?limit=100" + (cursor is null ? "" : "&after_id=" + Uri.EscapeDataString(cursor))),
        _ => new Uri(baseUri, "models")
    };

    private static async Task<byte[]> ReadPageAsync(HttpContent content, int remainingBytes, CancellationToken token)
    {
        var limit = Math.Min(MaximumResponseBytes, remainingBytes);
        if (limit <= 0 || content.Headers.ContentLength > limit) throw TooLarge();
        using var stream = await content.ReadAsStreamAsync(token).ConfigureAwait(false);
        using var buffer = new MemoryStream();
        var chunk = new byte[8192];
        while (true)
        {
            var count = await stream.ReadAsync(chunk, token).ConfigureAwait(false);
            if (count == 0) break;
            if (buffer.Length + count > limit) throw TooLarge();
            buffer.Write(chunk, 0, count);
        }
        return buffer.ToArray();
    }

    private static bool IsConversationModel(Uri uri, JsonElement item, string id)
    {
        if (uri.Scheme == Uri.UriSchemeHttps && uri.IsDefaultPort && uri.Host.Equals("api.openai.com", StringComparison.OrdinalIgnoreCase) &&
            uri.AbsolutePath == "/v1/" && IsOpenAiResponsesOnly(id)) return false;
        var status = GetString(item, "status");
        if (status is not null && (status.Equals("offline", StringComparison.OrdinalIgnoreCase) ||
            status.Equals("retired", StringComparison.OrdinalIgnoreCase) || status.Equals("disabled", StringComparison.OrdinalIgnoreCase))) return false;
        if (IsTokenHubEndpoint(uri) && status is not null && !status.Equals("online", StringComparison.OrdinalIgnoreCase)) return false;
        var type = GetString(item, "type")?.ToLowerInvariant();
        if (type is "embedding" or "embeddings" or "rerank" or "reranker" or "text2image" or "image2image" or "text2video" or
            "text-to-image" or "image-to-image" or "text-to-video" or "speech-to-text" or "text-to-speech" or "audio" or "image" or "video" or "moderation" or "tts" or "asr") return false;
        if (item.TryGetProperty("capabilities", out var capabilities) && capabilities.ValueKind == JsonValueKind.Object &&
            capabilities.TryGetProperty("completion_chat", out var chat) && chat.ValueKind == JsonValueKind.False) return false;
        if (item.TryGetProperty("architecture", out var architecture) && architecture.ValueKind == JsonValueKind.Object &&
            ExcludesTextOutput(architecture, "output_modalities")) return false;
        if (item.TryGetProperty("inference_metadata", out var metadata) && metadata.ValueKind == JsonValueKind.Object &&
            ExcludesTextOutput(metadata, "response_modality")) return false;
        if (VolcengineModelPolicy.IsEndpoint(uri)) return VolcengineModelPolicy.IsChatModel(id);
        // Older /models responses carry no capability metadata. Only unambiguous non-chat families are excluded.
        var name = id.ToLowerInvariant();
        var leaf = name[(name.LastIndexOf('/') + 1)..];
        return !name.EndsWith(":batch", StringComparison.Ordinal) && !name.Contains("embedding", StringComparison.Ordinal) && !name.Contains("rerank", StringComparison.Ordinal) &&
            !leaf.StartsWith("whisper", StringComparison.Ordinal) && !leaf.StartsWith("tts-", StringComparison.Ordinal) &&
            !leaf.StartsWith("dall-e", StringComparison.Ordinal) && !leaf.StartsWith("sora", StringComparison.Ordinal) &&
            !leaf.StartsWith("gpt-image", StringComparison.Ordinal) && !leaf.StartsWith("imagen-", StringComparison.Ordinal) &&
            !leaf.StartsWith("veo-", StringComparison.Ordinal) && !leaf.StartsWith("flux", StringComparison.Ordinal) &&
            !leaf.StartsWith("stable-diffusion", StringComparison.Ordinal) && !leaf.Contains("-transcribe", StringComparison.Ordinal) &&
            !leaf.Contains("-moderation", StringComparison.Ordinal) && !leaf.Contains("-audio", StringComparison.Ordinal) &&
            !leaf.Contains("-tts", StringComparison.Ordinal) && !leaf.Contains("-realtime", StringComparison.Ordinal);
    }

    internal static bool IsOpenAiResponsesOnly(string id)
    {
        var value = id.ToLowerInvariant();
        string[] families = ["gpt-5-pro", "gpt-5.2-pro", "gpt-5.4-pro", "gpt-5.5-pro", "o1-pro", "o3-pro",
            "gpt-5-codex", "gpt-5.1-codex", "gpt-5.2-codex", "gpt-5.3-codex", "codex-mini-latest",
            "o3-deep-research", "o4-mini-deep-research", "computer-use-preview"];
        return families.Any(family => value == family || value.StartsWith(family + "-", StringComparison.Ordinal));
    }

    private static bool ExcludesTextOutput(JsonElement item, string property) =>
        item.TryGetProperty(property, out var values) && values.ValueKind == JsonValueKind.Array &&
        values.GetArrayLength() > 0 && !values.EnumerateArray().Any(value => value.ValueKind == JsonValueKind.String &&
            string.Equals(value.GetString(), "text", StringComparison.OrdinalIgnoreCase));

    private static bool IsTokenHubEndpoint(Uri uri) => uri.Scheme == Uri.UriSchemeHttps && uri.IsDefaultPort && uri.AbsolutePath == "/v1/" &&
        new[] { "tokenhub.tencentmaas.com", "tokenhub-intl.tencentmaas.com", "tokenhub.tencentcloudmaas.com", "tokenhub-intl.tencentcloudmaas.com", "tokenhub-us.tencentcloudmaas.com" }
            .Contains(uri.Host, StringComparer.OrdinalIgnoreCase);

    private static string? GetString(JsonElement element, string property) =>
        element.TryGetProperty(property, out var value) && value.ValueKind == JsonValueKind.String ? value.GetString() : null;

    private static bool IsValidId(string? value) => value is { Length: > 0 and <= 200 } && !string.IsNullOrWhiteSpace(value) && !value.Any(char.IsControl);
    private static InvalidDataException InvalidList() => new(LocalizationService.T("模型列表格式无效，可手动输入模型 ID。", "Invalid model list. You can still enter a model ID manually."));
    private static InvalidDataException IncompleteList() => new(LocalizationService.T("模型列表未完整加载，请稍后重试或手动输入模型 ID。", "The model list was not fully loaded. Retry later or enter a model ID manually."));
    private static InvalidDataException TooLarge() => new(LocalizationService.T("模型列表超过安全大小限制", "The model list exceeds the size limit."));
}
