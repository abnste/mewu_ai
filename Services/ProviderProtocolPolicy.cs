using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

/// <summary>Single source of truth for upstream protocol, endpoint and auth selection.</summary>
internal static class ProviderProtocolPolicy
{
    internal static void Validate(AiProviderSettings settings)
    {
        var format=(settings.ApiFormat??"auto").Trim().ToLowerInvariant();
        if(format is not ("" or "auto" or "chat" or "openai_chat" or "openai-chat" or "completions" or "responses" or "openai_responses" or "openai-responses" or "anthropic" or "messages" or "anthropic_messages"))throw new InvalidOperationException("API 格式必须是 auto、OpenAI Chat、OpenAI Responses 或 Anthropic Messages");
        var auth=(settings.AuthMode??"auto").Trim().ToLowerInvariant();
        if(auth is not ("" or "auto" or "none" or "anonymous" or "bearer" or "authorization" or "api_key" or "api-key" or "x-api-key" or "anthropic_api_key" or "anthropic_auth_token"))throw new InvalidOperationException("认证模式必须是 auto、Bearer、API Key 或 none");
        if(!string.IsNullOrWhiteSpace(settings.RequestPath))
        {
            var path=settings.RequestPath.Trim();
            if(Uri.TryCreate(path,UriKind.Absolute,out _)||path.Contains("..",StringComparison.Ordinal)||path.Contains('\\')||path.Contains('#'))throw new InvalidOperationException("请求路径必须是安全的相对 API 路径");
        }
    }
    internal static string ApiFormat(AiProviderSettings settings)
    {
        var value=(settings.ApiFormat??string.Empty).Trim().ToLowerInvariant();
        if(value is "anthropic" or "messages" or "anthropic_messages")return "anthropic";
        if(value is "responses" or "openai_responses" or "openai-responses")return "responses";
        if(value is "chat" or "openai_chat" or "openai-chat" or "completions")return "chat";
        var host=ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl).Host.ToLowerInvariant();
        if(settings.Type.Equals("Anthropic",StringComparison.OrdinalIgnoreCase))return "anthropic";
        return "chat";
    }

    internal static string AuthMode(AiProviderSettings settings)
    {
        var value=(settings.AuthMode??string.Empty).Trim().ToLowerInvariant();
        if(value is "none" or "anonymous")return "none";
        if(value is "api_key" or "api-key" or "x-api-key" or "anthropic_api_key")return "api_key";
        if(value is "anthropic_auth_token" or "bearer" or "authorization")return "bearer";
        // Auto must preserve legacy OpenAI-compatible configurations. Only an
        // explicitly selected Anthropic Messages route gets x-api-key by
        // default on the official endpoint; relays keep Bearer authentication.
        if(ApiFormat(settings)=="anthropic"&&IsOfficialAnthropic(settings))return "api_key";
        return "bearer";
    }

    internal static Uri BuildRequestUri(AiProviderSettings settings,string defaultPath)
    {
        var baseUri=ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl);
        var path=string.IsNullOrWhiteSpace(settings.RequestPath)?defaultPath:settings.RequestPath.Trim();
        if(!path.StartsWith('/'))path="/"+path;
        // A user may provide a complete /messages, /responses or /chat/completions
        // base. Avoid appending a second protocol path in that case.
        var basePath=baseUri.AbsolutePath.TrimEnd('/');
        var requested=path.TrimEnd('/');
        if(basePath.EndsWith(requested,StringComparison.OrdinalIgnoreCase))return baseUri;

        // Many OpenAI-compatible gateways are entered as a bare HTTPS host,
        // while their API is mounted below /v1. Model discovery already probes
        // that conventional suffix; use the same route for real requests so
        // a successful /v1/models probe cannot be followed by a request to
        // the provider's HTML root (/chat/completions).
        if(basePath.Length==0 && !baseUri.IsLoopback &&
            requested is "/chat/completions" or "/responses" or "/messages")
            requested="/v1"+requested;

        // RequestPath may itself include the version prefix (for example
        // /v1/messages) while BaseUrl is https://host/v1/. Do not duplicate
        // that prefix when composing the final endpoint.
        if(requested.StartsWith(basePath+"/",StringComparison.OrdinalIgnoreCase))
        {
            var direct=new UriBuilder(baseUri){Path=requested};
            return direct.Uri;
        }
        // Regional gateways are often entered as /api/v1 while the custom
        // request path is copied from documentation as /v1/messages. Treat a
        // shared trailing version segment as a prefix rather than generating
        // /api/v1/v1/messages.
        var versionPrefix=basePath.LastIndexOf("/v1",StringComparison.OrdinalIgnoreCase);
        if(versionPrefix>=0 && requested.StartsWith("/v1/",StringComparison.OrdinalIgnoreCase))
        {
            var suffix=requested[3..];
            var versioned=new UriBuilder(baseUri){Path=basePath+suffix};
            return versioned.Uri;
        }

        // Uri(base, "/child") replaces the complete base path and silently
        // turns https://host/v1 into https://host/child. Append explicitly so
        // provider, region and plan path prefixes remain intact.
        var combined=$"{basePath}/{requested.TrimStart('/')}";
        var builder=new UriBuilder(baseUri){Path=combined};
        return builder.Uri;
    }

    internal static bool IsOfficialAnthropic(AiProviderSettings settings)=>
        ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl).Host.Equals("api.anthropic.com",StringComparison.OrdinalIgnoreCase);

    internal static bool IsOfficialOpenAi(AiProviderSettings settings)=>
        ProviderEndpointPolicy.NormalizeBaseUri(settings.BaseUrl).Host.Equals("api.openai.com",StringComparison.OrdinalIgnoreCase);
}
