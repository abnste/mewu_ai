using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class ProviderApiKeyChangePolicy
{
    internal static void Apply(AiProviderSettings provider,string? replacement,bool deleteExisting,CredentialService credentials)
    {
        ArgumentNullException.ThrowIfNull(provider);ArgumentNullException.ThrowIfNull(credentials);
        if(deleteExisting)provider.CredentialId=string.Empty;
        if(string.IsNullOrWhiteSpace(replacement))return;
        var credentialId=Guid.NewGuid().ToString("N");credentials.Save(credentialId,replacement);provider.CredentialId=credentialId;
    }

    internal static bool HasCompetingAuthentication(AiProviderSettings provider)=>
        !string.IsNullOrWhiteSpace(provider.CredentialId)&&
        provider.CustomHeaders.Keys.Concat(provider.SensitiveHeaderCredentialIds.Keys).Any(ProviderHeaderCredentialService.IsAuthentication);
}
