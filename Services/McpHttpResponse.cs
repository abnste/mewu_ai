// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net.Http;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace mewu_ai_Assistant.Services;

/// <summary>Checks external API envelopes without echoing response bodies or credentials.</summary>
internal static class McpHttpResponse
{
    internal static async Task<JsonObject> ReadAsync(HttpResponseMessage response,string service,string codeField,CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        if(!response.IsSuccessStatusCode)
            throw new InvalidOperationException(LocalizationService.T($"{service} 请求失败（HTTP {(int)response.StatusCode}）。",$"{service} request failed (HTTP {(int)response.StatusCode})."));
        JsonObject result;
        try
        {
            var body=await response.Content.ReadAsStringAsync(token).ConfigureAwait(false);
            result=JsonNode.Parse(body) as JsonObject ?? throw Invalid(service);
        }
        catch(JsonException){throw Invalid(service);}
        if(result[codeField] is not JsonValue value || !value.TryGetValue<int>(out var code))throw Invalid(service);
        if(code!=0)throw new InvalidOperationException(LocalizationService.T($"{service} 请求失败（错误码 {code}）。",$"{service} request failed (code {code})."));
        return result;
    }

    internal static string RequiredString(JsonObject result,string field,string service)
        => result[field] is JsonValue value && value.TryGetValue<string>(out var text) && !string.IsNullOrWhiteSpace(text)
            ? text : throw Invalid(service);

    internal static JsonObject RequiredObject(JsonObject result,string field,string service)
        => result[field] as JsonObject ?? throw Invalid(service);

    internal static InvalidOperationException Invalid(string service)
        =>new(LocalizationService.T($"{service} 返回的响应不完整或格式无效。",$"{service} returned an incomplete or invalid response."));
}
