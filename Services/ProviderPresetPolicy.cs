// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal enum ProviderPresetGroup { China, Global, Custom }

internal sealed record ProviderPreset(string Id, string Name, string Type, string BaseUrl, string DefaultModel,
    ProviderPresetGroup Group = ProviderPresetGroup.China, string EnglishName = "", string SearchTerms = "",string ApiFormat="auto",string AuthMode="auto",string Region="",string Plan="")
{
    internal bool RequiresBaseUrl => Id == "Custom";
    internal string LocalizedName => LocalizationService.T(Name, string.IsNullOrEmpty(EnglishName) ? Name : EnglishName);
    internal bool Matches(string query) => string.IsNullOrWhiteSpace(query) ||
        string.Join(' ', Id, Name, EnglishName, SearchTerms, BaseUrl).Contains(query.Trim(), StringComparison.OrdinalIgnoreCase);
}

internal static class ProviderPresetPolicy
{
    internal static readonly ProviderPreset[] All =
    [
        new("MiniMax", "MiniMax 国内", "MiniMax", "https://api.minimax.cn/v1", "MiniMax-M3", EnglishName: "MiniMax China"),
        new("Volcengine", "火山方舟", "OpenAICompatible", VolcengineModelPolicy.StandardBaseUrl, "", EnglishName: "Volcengine Ark", SearchTerms: "豆包 Doubao 字节 ByteDance"),
        new("DashScope", "阿里百炼", "OpenAICompatible", "https://dashscope.aliyuncs.com/compatible-mode/v1", "", EnglishName: "Alibaba Cloud Bailian", SearchTerms: "通义千问 Qwen Alibaba DashScope"),
        new("DeepSeek", "DeepSeek", "OpenAICompatible", "https://api.deepseek.com/v1", "", SearchTerms: "深度求索"),
        new("Moonshot", "Kimi 国内", "OpenAICompatible", "https://api.moonshot.cn/v1", "", EnglishName: "Kimi China", SearchTerms: "月之暗面 Moonshot"),
        new("Zhipu", "智谱 GLM", "OpenAICompatible", "https://open.bigmodel.cn/api/paas/v4", "", EnglishName: "Zhipu GLM", SearchTerms: "BigModel 智谱清言"),
        new("Tencent", "腾讯 TokenHub", "OpenAICompatible", "https://tokenhub.tencentmaas.com/v1", "", EnglishName: "Tencent TokenHub", SearchTerms: "混元 Hunyuan"),
        new("Baidu", "百度千帆", "OpenAICompatible", "https://qianfan.baidubce.com/v2", "", EnglishName: "Baidu Qianfan", SearchTerms: "文心 ERNIE"),
        new("SiliconFlow", "硅基流动", "OpenAICompatible", "https://api.siliconflow.cn/v1", "", EnglishName: "SiliconFlow China"),
        new("OpenAI", "OpenAI", "OpenAICompatible", "https://api.openai.com/v1", "", ProviderPresetGroup.Global, SearchTerms: "ChatGPT GPT"),
        new("Anthropic", "Anthropic Claude", "OpenAICompatible", "https://api.anthropic.com/v1", "", ProviderPresetGroup.Global,ApiFormat:"anthropic",AuthMode:"api_key"),
        new("AnthropicBearer", "Anthropic 兼容中转", "OpenAICompatible", "http://localhost", "", ProviderPresetGroup.Custom, "Anthropic relay", "Claude Messages Bearer proxy",ApiFormat:"anthropic",AuthMode:"bearer"),
        new("OpenAIResponses", "OpenAI Responses", "OpenAICompatible", "https://api.openai.com/v1", "", ProviderPresetGroup.Global,SearchTerms:"Responses GPT o-series",ApiFormat:"responses",AuthMode:"bearer"),
        new("Google", "Google Gemini", "OpenAICompatible", "https://generativelanguage.googleapis.com/v1beta/openai", "", ProviderPresetGroup.Global),
        new("xAI", "xAI Grok", "OpenAICompatible", "https://api.x.ai/v1", "", ProviderPresetGroup.Global),
        new("OpenRouter", "OpenRouter", "OpenAICompatible", "https://openrouter.ai/api/v1", "", ProviderPresetGroup.Global),
        new("Groq", "Groq", "OpenAICompatible", "https://api.groq.com/openai/v1", "", ProviderPresetGroup.Global),
        new("Mistral", "Mistral AI", "OpenAICompatible", "https://api.mistral.ai/v1", "", ProviderPresetGroup.Global),
        new("Together", "Together AI", "OpenAICompatible", "https://api.together.ai/v1", "", ProviderPresetGroup.Global),
        new("MiniMaxGlobal", "MiniMax 国际", "MiniMax", "https://api.minimax.io/v1", "MiniMax-M3", ProviderPresetGroup.Global, "MiniMax Global"),
        new("DashScopeGlobal", "阿里百炼国际", "OpenAICompatible", "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", "", ProviderPresetGroup.Global, "Alibaba Cloud International", "通义千问 Qwen DashScope"),
        new("MoonshotGlobal", "Kimi 国际", "OpenAICompatible", "https://api.moonshot.ai/v1", "", ProviderPresetGroup.Global, "Kimi Global", "Moonshot"),
        new("Custom", "自定义兼容服务", "OpenAICompatible", "", "", ProviderPresetGroup.Custom, "Custom compatible service", "OpenAI localhost Ollama LM Studio 中转")
    ];

    internal static ProviderPreset Detect(AiProviderSettings settings)
    {
        var endpoint = settings.BaseUrl.Trim().TrimEnd('/');
        // Recognize existing addresses without rewriting saved settings or credentials.
        if (settings.Type.Equals("MiniMax", StringComparison.OrdinalIgnoreCase) &&
            endpoint.Equals("https://api.minimaxi.com/v1", StringComparison.OrdinalIgnoreCase)) return All[0];
        var format=string.IsNullOrWhiteSpace(settings.BaseUrl)?"chat":ProviderProtocolPolicy.ApiFormat(settings);
        return All.FirstOrDefault(p => p.Id != "Custom" && p.Type.Equals(settings.Type, StringComparison.OrdinalIgnoreCase) &&
            p.BaseUrl.Equals(endpoint, StringComparison.OrdinalIgnoreCase) && ProviderProtocolPolicy.ApiFormat(new AiProviderSettings{Type=p.Type,BaseUrl=string.IsNullOrWhiteSpace(p.BaseUrl)?"https://localhost":p.BaseUrl,ApiFormat=p.ApiFormat})==format) ?? All[^1];
    }

    internal static AiProviderSettings Create(ProviderPreset preset) => new()
    {
        Name = preset.LocalizedName, Type = preset.Type, BaseUrl = preset.BaseUrl, Model = preset.DefaultModel,
        ApiFormat=preset.ApiFormat,AuthMode=preset.AuthMode,Region=preset.Region,Plan=preset.Plan
    };

    internal static bool IsUntouchedDraft(AiProviderSettings settings, bool hasPendingKey)
    {
        var preset = Detect(settings);
        return !hasPendingKey && string.IsNullOrEmpty(settings.CredentialId) &&
            settings.CustomHeaders.Count == 0 && settings.SensitiveHeaderCredentialIds.Count == 0 && settings.RequestParameters is { Count: 0 } &&
            settings.BaseUrl == preset.BaseUrl && settings.Model == preset.DefaultModel &&
            (settings.Name == preset.Name || settings.Name == preset.LocalizedName || settings.Name == preset.EnglishName);
    }

    internal static string DisplayName(AiProviderSettings settings) =>
        settings.Type.Equals("MiniMax", StringComparison.OrdinalIgnoreCase) && settings.Name == "MiniMax M3"
            ? "MiniMax" : settings.Name;
}
