using System.Text.Json;

namespace mewu_ai_Assistant.Services;

internal static class ProviderRequestParameterPolicy
{
    internal static Dictionary<string,JsonElement> Parse(string json)
    {
        if (json.Length > 16384) throw Invalid();
        using var document = JsonDocument.Parse(string.IsNullOrWhiteSpace(json) ? "{}" : json, new JsonDocumentOptions { MaxDepth = 8 });
        if (document.RootElement.ValueKind != JsonValueKind.Object) throw Invalid();
        var result = new Dictionary<string,JsonElement>(StringComparer.Ordinal);
        foreach (var property in document.RootElement.EnumerateObject())
            if (!result.TryAdd(property.Name, property.Value.Clone())) throw Invalid();
        Validate(result);
        return result;
    }

    internal static void Validate(IReadOnlyDictionary<string,JsonElement>? values)
    {
        if (values is null || values.Count > 3) throw Invalid();
        foreach (var (key, value) in values)
        {
            var valid = key switch
            {
                "service_tier" => value.ValueKind == JsonValueKind.String && value.GetString() is "standard" or "priority",
                "temperature" => value.ValueKind == JsonValueKind.Number && value.TryGetDouble(out var t) && double.IsFinite(t) && t is >= 0 and <= 2,
                "top_p" => value.ValueKind == JsonValueKind.Number && value.TryGetDouble(out var p) && double.IsFinite(p) && p is > 0 and <= 1,
                _ => false
            };
            if (!valid) throw Invalid();
        }
    }

    private static InvalidOperationException Invalid() => new(LocalizationService.T(
        "请求参数 JSON 仅支持 service_tier（standard/priority）、temperature（0–2）和 top_p（大于 0 且不超过 1）；不得重复字段或填写密钥、模型、消息、流控制。",
        "Request JSON supports service_tier (standard/priority), temperature (0–2), and top_p (greater than 0, up to 1). Duplicate fields, credentials, model, messages and streaming controls are not allowed."));
}
