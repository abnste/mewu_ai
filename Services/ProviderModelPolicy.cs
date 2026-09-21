// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Compatibility rules for the actual API endpoint and model. A model's
/// native API capabilities do not imply the same transport on a compatible API.
/// </summary>
internal static class ProviderModelPolicy
{
    private const long MiB = 1024L * 1024;

    internal static AiProviderCapabilities GetCapabilities(AiProviderSettings settings)
    {
        var endpoint = ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl);
        var model = ModelName(settings.Model);
        if (IsMiniMaxM3(settings)) return new(true, true, true, 10 * MiB, 50 * MiB, TimeSpan.Zero,
            MimeTypes("image/jpeg", "image/png", "image/gif", "image/webp", "video/mp4", "video/avi", "video/x-msvideo", "video/mov", "video/quicktime", "video/x-matroska"));
        if (VolcengineModelPolicy.IsEndpoint(endpoint)) return VolcengineModelPolicy.GetCapabilities(settings.Model);

        // The official Flash alias now serves V4.1, while V4 Pro remains text-only.
        // Do not apply this redirect to another vendor hosting the older weights.
        if (IsOfficial(endpoint, "api.deepseek.com"))
            return Images(IsDeepSeekVision(model), 32 * MiB, "image/png", "image/jpeg", "image/webp", "image/gif");

        if (IsOfficial(endpoint, "api.moonshot.cn", "api.moonshot.ai"))
            return IsKimiMultimodal(model)
                ? new(true, true, true, 64 * MiB, 64 * MiB, TimeSpan.Zero,
                    MimeTypes("image/png", "image/jpeg", "image/webp", "image/gif", "image/bmp", "image/heic", "image/heif", "video/mp4"))
                : Images(IsKimiVision(model), 20 * MiB);

        if (IsDashScope(endpoint) && IsQwenVisualFamily(model))
            // Account for the provider's limit on the encoded data URI itself.
            return new(true, true, true, InlineRawLimit(20 * MiB), InlineRawLimit(10 * MiB), TimeSpan.FromHours(2),
                MimeTypes("image/png", "image/jpeg", "image/webp", "image/gif", "video/mp4", "video/avi", "video/x-msvideo", "video/mov", "video/quicktime", "video/x-matroska", "video/x-flv", "video/x-ms-wmv"));

        if (IsOfficial(endpoint, "api.z.ai", "open.bigmodel.cn"))
            // The API documents base64 images, but its video schema only
            // documents remote URLs. Do not claim local inline-video support.
            return Images(IsGlmVision(model), 4_999_999, "image/png", "image/jpeg");

        var image = IsKnownImageModel(model);
        if (IsOfficial(endpoint, "api.anthropic.com"))
            // Anthropic's 10 MiB limit applies to the base64-encoded image.
            return Images(image, 10 * MiB / 4 * 3, "image/png", "image/jpeg", "image/webp", "image/gif");
        if (IsOfficial(endpoint, "api.x.ai"))
            return Images(image, 20 * MiB, "image/png", "image/jpeg");
        if (IsOfficial(endpoint, "api.mistral.ai"))
            return Images(image, 10_000_000, "image/png", "image/jpeg", "image/webp", "image/gif");
        return Images(image, 20 * MiB);
    }

    internal static bool IsMiniMaxM3(AiProviderSettings settings)
    {
        if (!settings.Model.Equals("MiniMax-M3", StringComparison.OrdinalIgnoreCase)) return false;
        return settings.Type.Equals("MiniMax", StringComparison.OrdinalIgnoreCase) ||
            IsOfficial(ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl), "api.minimaxi.com", "api.minimax.cn", "api.minimax.io");
    }

    internal static bool UsesCumulativeContent(AiProviderSettings settings) => IsMiniMaxM3(settings);

    internal static bool RequiresAssistantContinuation(AiProviderSettings settings)
    {
        var endpoint = ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl);
        var model = ModelName(settings.Model);
        return IsOfficial(endpoint, "api.moonshot.cn", "api.moonshot.ai") &&
            (Family(model, "kimi-k3") || Family(model, "kimi-k2.7-code"));
    }

    internal static long MaximumRequestBytes(AiProviderSettings settings)
    {
        var endpoint = ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl);
        if (IsOfficial(endpoint, "api.anthropic.com")) return 32 * MiB;
        if (IsOfficial(endpoint, "api.deepseek.com")) return 48 * MiB;
        // Kimi documents a larger body limit, but the existing application
        // attachment pipeline is still bounded to 64 MiB before encoding.
        return 64 * MiB;
    }

    internal static bool UsesVideoSamplingField(AiProviderSettings settings) => IsMiniMaxM3(settings) ||
        VolcengineModelPolicy.IsEndpoint(ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl));

    internal static TimeSpan MinimumVideoDuration(AiProviderSettings settings) =>
        IsDashScope(ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl)) && IsQwenVisualFamily(ModelName(settings.Model))
            ? TimeSpan.FromSeconds(2) : TimeSpan.Zero;

    internal static int MaximumImageCount(AiProviderSettings settings)
    {
        var endpoint = ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl);
        var model = ModelName(settings.Model);
        if (IsOfficial(endpoint, "api.mistral.ai")) return 8;
        if (IsOfficial(endpoint, "api.groq.com"))
        {
            if (Family(model, "qwen3.8-27b")) return 3;
            if (Family(model, "qwen3.6-27b")) return 5;
        }
        return 16;
    }

    internal static void ValidateRequestParameters(AiProviderSettings settings)
    {
        ProviderRequestParameterPolicy.Validate(settings.RequestParameters);
        var endpoint = ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl);
        var model = ModelName(settings.Model);
        if (IsOfficial(endpoint, "api.openai.com") && ProviderProtocolPolicy.ApiFormat(settings)!="responses" && ProviderModelCatalogService.IsOpenAiResponsesOnly(settings.Model))
            throw new InvalidOperationException(LocalizationService.T(
                $"{settings.Model} 仅支持 OpenAI Responses API。当前连接使用 Chat Completions，请选择支持该接口的模型，例如 gpt-6-astra。",
                $"{settings.Model} requires the OpenAI Responses API. This connection uses Chat Completions; choose a compatible model such as gpt-6-astra."));
        var fixedSampling = IsOfficial(endpoint, "api.openai.com") && Family(model, "gpt-6-astra") ||
            IsOfficial(endpoint, "api.moonshot.cn", "api.moonshot.ai") && Family(model, "kimi-k3");
        if (fixedSampling && settings.RequestParameters.Keys.Any(key => key is "temperature" or "top_p"))
            throw new InvalidOperationException(LocalizationService.T(
                $"{settings.Model} 不支持自定义 temperature 或 top_p，请在高级请求设置中移除这两个参数。思考能力保持模型默认。",
                $"{settings.Model} does not support custom temperature or top_p. Remove these parameters from advanced request settings. The model's default reasoning remains enabled."));
    }

    internal static void ApplyRequestParameters(IDictionary<string, object?> body, AiProviderSettings settings, AiRequest request)
    {
        ValidateRequestParameters(settings);
        foreach (var parameter in settings.RequestParameters) body[parameter.Key] = parameter.Value;
        var endpoint = ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl);
        var model = ModelName(settings.Model);
        var miniMax = IsMiniMaxM3(settings);
        var openAi = IsOfficial(endpoint, "api.openai.com");
        if (openAi && settings.RequestParameters.TryGetValue("service_tier", out var tier) && tier.GetString() == "standard")
            body["service_tier"] = "default";
        // An explicit utility budget wins. Unknown models retain the backend's
        // default, and no request acquires an invented universal output ceiling.
        var maximum = request.MaxOutputTokens;
        if (maximum is null && request.UseModelMaximumOutputTokens)
        {
            if (miniMax) maximum = 524288;
            else if (openAi && IsLatestOpenAi(model)) maximum = 128000;
            else if (IsOfficial(endpoint, "api.deepseek.com") && (IsDeepSeekVision(model) || model == "deepseek-v4-pro")) maximum = 393216;
        }
        if (maximum is { } tokens)
        {
            var completionBudget = miniMax || openAi && UsesOpenAiCompletionBudget(model) ||
                IsOfficial(endpoint, "api.moonshot.cn", "api.moonshot.ai") && Family(model, "kimi-k3");
            body[completionBudget ? "max_completion_tokens" : "max_tokens"] = tokens;
        }

        if (miniMax)
        {
            body["reasoning_split"] = true;
            body["thinking"] = new { type = request.DisableReasoning ? "disabled" : "adaptive" };
        }
        else if (request.DisableReasoning && VolcengineModelPolicy.IsEndpoint(endpoint))
        {
            body["thinking"] = new { type = "disabled" };
            body["reasoning_effort"] = "minimal";
        }
        else if (request.DisableReasoning && IsOfficial(endpoint, "api.deepseek.com"))
        {
            body["thinking"] = new { type = "disabled" };
        }
    }

    internal static bool NeedsAnthropicVersion(AiProviderSettings settings) =>
        IsOfficial(ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl), "api.anthropic.com");

    private static bool IsLatestOpenAi(string model) => Family(model, "gpt-6-astra") ||
        Family(model, "gpt-5.6-sol") || Family(model, "gpt-5.6-terra") || Family(model, "gpt-5.6-luna");

    private static bool UsesOpenAiCompletionBudget(string model) => IsLatestOpenAi(model) ||
        Family(model, "gpt-5") || model.StartsWith("gpt-5.", StringComparison.Ordinal) ||
        Family(model, "o1") || Family(model, "o3") || Family(model, "o4");

    private static bool IsDeepSeekVision(string model) => model is "deepseek-flash" or "deepseek-v4-flash" or "deepseek-v4-flash-vision-exp";

    private static bool IsKimiMultimodal(string model) => Family(model, "kimi-k3") || Family(model, "kimi-k2.7-code") || Family(model, "kimi-k2.6");
    private static bool IsKimiVision(string model) => IsKimiMultimodal(model) || model.Contains("vision", StringComparison.Ordinal);

    private static bool IsKnownImageModel(string model)
    {
        // These text-only variants do not inherit their family's image input.
        if (Family(model, "o3-mini") || Family(model, "gpt-4o-search-preview")) return false;
        if (model.Contains("embedding", StringComparison.Ordinal) || model.Contains("rerank", StringComparison.Ordinal) ||
            model.Contains("realtime", StringComparison.Ordinal) || model.Contains("audio", StringComparison.Ordinal) ||
            model.Contains("tts", StringComparison.Ordinal) || model.Contains("transcribe", StringComparison.Ordinal) ||
            model.Contains("image-generation", StringComparison.Ordinal) || model.StartsWith("gpt-image", StringComparison.Ordinal)) return false;
        if (IsLatestOpenAi(model) || Family(model, "gpt-4o") || Family(model, "gpt-4.1") || Family(model, "gpt-4-turbo") ||
            Family(model, "gpt-5") || model.StartsWith("gpt-5.", StringComparison.Ordinal) ||
            Family(model, "o3") || Family(model, "o4-mini")) return true;
        if (Family(model, "claude-3") || Family(model, "claude-fable-5") || Family(model, "claude-opus-5") ||
            Family(model, "claude-opus-4") || Family(model, "claude-sonnet-5") || Family(model, "claude-sonnet-4") ||
            Family(model, "claude-haiku-4")) return true;
        if ((Family(model, "gemini-1.5") || Family(model, "gemini-2.0") || Family(model, "gemini-2.5") ||
             Family(model, "gemini-3") || Family(model, "gemini-3.1") || Family(model, "gemini-3.5") || Family(model, "gemini-3.8")) &&
            !model.Contains("-image", StringComparison.Ordinal) && !model.Contains("-live", StringComparison.Ordinal)) return true;
        if (Family(model, "grok-4") || Family(model, "grok-4.6") || Family(model, "grok-4.20")) return true;
        if (IsKimiVision(model)) return true;
        if (IsQwenVisualFamily(model) || IsGlmVision(model) ||
            Family(model, "deepseek-v4.1-flash") || Family(model, "mistral-medium-3-5") || Family(model, "mistral-small-2603")) return true;
        return model.Contains("vision", StringComparison.Ordinal) || model.Contains("-vl", StringComparison.Ordinal) ||
            model.Contains("vl-", StringComparison.Ordinal) || Family(model, "pixtral");
    }

    private static string ModelName(string model)
    {
        var value = model.Trim().ToLowerInvariant();
        return value[(value.LastIndexOf('/') + 1)..];
    }

    private static bool Family(string value, string family) => value == family || value.StartsWith(family + "-", StringComparison.Ordinal);
    private static bool IsGlmVision(string model) => Family(model, "glm-5.3-flash") || Family(model, "glm-5-3-flash") ||
        Family(model, "glm-4.5v") || Family(model, "glm-4.6v") || Family(model, "glm-5v");
    private static bool IsQwenVisualFamily(string model) => Family(model, "qwen3.8-max") || Family(model, "qwen3.8-flash") ||
        Family(model, "qwen3.8-27b") || Family(model, "qwen3.8-2.4t-a95b") || Family(model, "qwen3.7-plus") ||
        Family(model, "qwen3.7-flash") || Family(model, "qwen3.6-27b") || Family(model, "qwen3-vl") ||
        Family(model, "qwen3.6-plus") || Family(model, "qwen3.6-flash") || Family(model, "qwen3.5-plus") || Family(model, "qwen3.5-flash");
    private static bool IsDashScope(Uri endpoint) => endpoint.Scheme == Uri.UriSchemeHttps && endpoint.IsDefaultPort &&
        endpoint.AbsolutePath == "/compatible-mode/v1/" && ProviderModelCatalogService.IsDashScopeHost(endpoint.Host);
    private static long InlineRawLimit(long encodedLimit) => (encodedLimit - 128) / 4 * 3;
    private static bool IsOfficial(Uri endpoint, params string[] hosts) => endpoint.IsDefaultPort && hosts.Contains(endpoint.Host, StringComparer.OrdinalIgnoreCase);
    private static IReadOnlySet<string> MimeTypes(params string[] values) => new HashSet<string>(values, StringComparer.OrdinalIgnoreCase);
    private static AiProviderCapabilities Images(bool image, long limit, params string[] mimeTypes) =>
        new(image, false, true, image ? limit : 0, 0, TimeSpan.Zero,
            image ? MimeTypes(mimeTypes.Length == 0 ? ["image/png", "image/jpeg", "image/webp"] : mimeTypes) : MimeTypes());
}
