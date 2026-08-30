using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

public sealed class ProviderHeaderCredentialService
{
    private readonly CredentialService _credentials;

    public ProviderHeaderCredentialService(CredentialService? credentials=null)
    {
        _credentials=credentials??new CredentialService();
    }

    public static bool IsSensitive(string headerName)
    {
        var name=headerName.Trim();
        if(name.Length==0)return false;
        return IsAuthentication(headerName)
            || name.Contains("key",StringComparison.OrdinalIgnoreCase)
            || name.Contains("secret",StringComparison.OrdinalIgnoreCase)
            || name.Contains("password",StringComparison.OrdinalIgnoreCase);
    }

    public static bool IsAuthentication(string headerName)
    {
        var name=headerName.Trim();
        if(name.Length==0)return false;
        return name.Equals("Authorization",StringComparison.OrdinalIgnoreCase)
            || name.Equals("Proxy-Authorization",StringComparison.OrdinalIgnoreCase)
            || name.Contains("auth",StringComparison.OrdinalIgnoreCase)
            || name.Contains("token",StringComparison.OrdinalIgnoreCase)
            || name.Contains("credential",StringComparison.OrdinalIgnoreCase)
            || name.Contains("secret",StringComparison.OrdinalIgnoreCase)
            || name.Contains("password",StringComparison.OrdinalIgnoreCase)
            || name.Contains("cookie",StringComparison.OrdinalIgnoreCase)
            || name.Contains("signature",StringComparison.OrdinalIgnoreCase)
            || name.Contains("subscription",StringComparison.OrdinalIgnoreCase)
            || name.Contains("api-key",StringComparison.OrdinalIgnoreCase)
            || name.Contains("api_key",StringComparison.OrdinalIgnoreCase)
            || name.Contains("apikey",StringComparison.OrdinalIgnoreCase)
            || name.EndsWith("-key",StringComparison.OrdinalIgnoreCase)
            || name.EndsWith("_key",StringComparison.OrdinalIgnoreCase)
            || name.Equals("key",StringComparison.OrdinalIgnoreCase);
    }

    public AiProviderSettings CreateHydratedCopy(AiProviderSettings source,ISet<string>? unavailableHeaders=null)
    {
        ProviderHeaderPolicy.EnsureCredentialMappingsValid(source.CustomHeaders,source.SensitiveHeaderCredentialIds);
        var copy=Clone(source);
        foreach(var pair in source.SensitiveHeaderCredentialIds)
        {
            var value=_credentials.Read(pair.Value);
            if(value is not null)copy.CustomHeaders[pair.Key]=value;
            else unavailableHeaders?.Add(pair.Key);
        }
        return copy;
    }

    public bool MigratePlaintextHeaders(AiProviderSettings provider)
    {
        return MigratePlaintextHeaders(provider,null);
    }

    internal bool MigratePlaintextHeaders(AiProviderSettings provider,ISet<string>? createdCredentialIds)
    {
        return Protect(provider,authoritative:false,forceNewCredentials:true,createdCredentialIds);
    }

    public void ProtectEditableHeaders(AiProviderSettings provider)
    {
        Protect(provider,authoritative:true,forceNewCredentials:false,null);
    }

    internal void DeleteCredential(string credentialId)=>_credentials.Delete(credentialId);

    public static AiProviderSettings Clone(AiProviderSettings source)=>new()
    {
        Id=source.Id,
        Name=source.Name,
        Type=source.Type,
        BaseUrl=source.BaseUrl,
        Model=source.Model,
        CredentialId=source.CredentialId,
        CustomHeaders=new Dictionary<string,string>(source.CustomHeaders),
        SensitiveHeaderCredentialIds=new Dictionary<string,string>(source.SensitiveHeaderCredentialIds)
    };

    private bool Protect(
        AiProviderSettings provider,
        bool authoritative,
        bool forceNewCredentials,
        ISet<string>? createdCredentialIds)
    {
        provider.CustomHeaders??=[];
        provider.SensitiveHeaderCredentialIds??=[];
        ProviderHeaderPolicy.EnsureValid(provider.CustomHeaders);
        var sensitive=provider.CustomHeaders.Keys.Where(IsSensitive).ToList();
        var changed=false;
        foreach(var name in sensitive)
        {
            var value=provider.CustomHeaders[name];
            var existing=Find(provider.SensitiveHeaderCredentialIds,name);
            var credentialId=authoritative?null:existing?.Value;
            if(forceNewCredentials||string.IsNullOrWhiteSpace(credentialId))credentialId=Guid.NewGuid().ToString("N");
            _credentials.Save(credentialId,value);
            if(forceNewCredentials)createdCredentialIds?.Add(credentialId);
            if(existing is not null&&!existing.Value.Key.Equals(name,StringComparison.Ordinal))provider.SensitiveHeaderCredentialIds.Remove(existing.Value.Key);
            provider.SensitiveHeaderCredentialIds[name]=credentialId;
            provider.CustomHeaders.Remove(name);
            changed=true;
        }

        if(authoritative)
        {
            var currentNames=sensitive.ToHashSet(StringComparer.OrdinalIgnoreCase);
            foreach(var stale in provider.SensitiveHeaderCredentialIds.Where(pair=>!currentNames.Contains(pair.Key)).ToList())
            {
                provider.SensitiveHeaderCredentialIds.Remove(stale.Key);
                changed=true;
            }
        }
        ProviderHeaderPolicy.EnsureCredentialMappingsValid(provider.CustomHeaders,provider.SensitiveHeaderCredentialIds);
        return changed;
    }

    private static KeyValuePair<string,string>? Find(Dictionary<string,string> values,string name)
    {
        foreach(var pair in values)
            if(pair.Key.Equals(name,StringComparison.OrdinalIgnoreCase))return pair;
        return null;
    }
}
