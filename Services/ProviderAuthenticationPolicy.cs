using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class ProviderAuthenticationPolicy
{
    internal static void EnsureStoredCredentialReferences(AiProviderSettings provider)
    {
        ArgumentNullException.ThrowIfNull(provider);
        var hasPrimary=!string.IsNullOrWhiteSpace(provider.CredentialId);
        var hasHeader=provider.SensitiveHeaderCredentialIds.Keys.Any(ProviderHeaderCredentialService.IsAuthentication);
        EnsureExclusive(provider.Name,hasPrimary,hasHeader);
    }

    internal static void EnsureUsableCredentials(AiProviderSettings provider,string? apiKey)
    {
        ArgumentNullException.ThrowIfNull(provider);
        var hasPrimary=!string.IsNullOrWhiteSpace(apiKey);
        var hasHeader=provider.CustomHeaders.Any(pair=>ProviderHeaderCredentialService.IsAuthentication(pair.Key)&&!string.IsNullOrWhiteSpace(pair.Value));
        EnsureExclusive(provider.Name,hasPrimary,hasHeader);
    }

    private static void EnsureExclusive(string? providerName,bool hasPrimary,bool hasHeader)
    {
        var name=string.IsNullOrWhiteSpace(providerName)?"Provider":providerName;
        if(hasPrimary&&hasHeader)throw new InvalidOperationException($"{name} 同时配置了 API Key 与认证 Header，请只保留一种认证方式");
        if(!hasPrimary&&!hasHeader)throw new InvalidOperationException($"{name} 的 API Key 或认证 Header 不可用，请重新配置");
    }
}
