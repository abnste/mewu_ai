// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Keeps the editor's original text, including incomplete JSON, while another
/// connection is open. Validation only happens when the draft is applied.
/// </summary>
internal sealed record ApiConnectionDraft(string BaseUrl, string Model, string HeadersJson, string ParametersJson,string ApiFormat="auto",string AuthMode="auto",string RequestPath="",string Region="",string Plan="")
{
    private static readonly JsonSerializerOptions DisplayJsonOptions = new() { WriteIndented = true };

    internal static ApiConnectionDraft FromProvider(AiProviderSettings provider)
    {
        ArgumentNullException.ThrowIfNull(provider);
        return new(provider.BaseUrl, provider.Model,
            JsonSerializer.Serialize(provider.CustomHeaders, DisplayJsonOptions),
            JsonSerializer.Serialize(provider.RequestParameters, DisplayJsonOptions),provider.ApiFormat,provider.AuthMode,provider.RequestPath,provider.Region,provider.Plan);
    }

    internal void ApplyTo(AiProviderSettings provider)
    {
        ArgumentNullException.ThrowIfNull(provider);
        // Parse both fields before changing the editable connection. A broken
        // header must not partially apply a valid request-parameter change.
        var parameters = ProviderRequestParameterPolicy.Parse(ParametersJson);
        var headers = JsonSerializer.Deserialize<Dictionary<string, string>>(
            string.IsNullOrWhiteSpace(HeadersJson) ? "{}" : HeadersJson) ?? [];
        ProviderHeaderPolicy.EnsureValid(headers);
        var baseUrl = BaseUrl.TrimEnd('/');
        var model = Model.Trim();

        provider.BaseUrl = baseUrl;
        provider.Model = model;
        provider.CustomHeaders = headers;
        provider.RequestParameters = parameters;
        provider.ApiFormat=ApiFormat.Trim().ToLowerInvariant();
        provider.AuthMode=AuthMode.Trim().ToLowerInvariant();
        provider.RequestPath=RequestPath.Trim();
        provider.Region=Region.Trim();
        provider.Plan=Plan.Trim();
        ProviderProtocolPolicy.Validate(provider);
    }
}
