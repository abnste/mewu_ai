using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;
public sealed class AiProviderFactory
{
    private readonly CredentialService? _credentials;
    private readonly Action<string,Exception>? _logError;
    public AiProviderFactory(CredentialService? credentials=null)
        :this(credentials,static (component,exception)=>new PrivacyLogger().Error(component,exception)){}
    internal AiProviderFactory(CredentialService? credentials,Action<string,Exception>? logError){_credentials=credentials;_logError=logError;}
    public IAiProvider? Create(AppSettings settings)=>Create(settings,out _);
    public IAiProvider? Create(AppSettings settings,out string? error)
    {
        error=null;
        try
        {
            ArgumentNullException.ThrowIfNull(settings);
            if(settings.ConfigurationErrors.Count>0)throw new InvalidOperationException(string.Join("；",settings.ConfigurationErrors.Distinct(StringComparer.Ordinal)));
            if(settings.Providers is not {Count:>0})throw new InvalidOperationException("尚未配置 AI Provider");
            if(settings.Providers.Any(provider=>provider is null||string.IsNullOrWhiteSpace(provider.Id))||settings.Providers.GroupBy(provider=>provider.Id,StringComparer.Ordinal).Any(group=>group.Count()>1))throw new InvalidOperationException("Provider ID 不能为空且必须唯一，请在设置中修复后保存");
            if(string.IsNullOrWhiteSpace(settings.DefaultProviderId))throw new InvalidOperationException("请选择一个默认 AI Provider");
            var matches=settings.Providers.Where(provider=>provider.Id==settings.DefaultProviderId).Take(2).ToList();
            if(matches.Count==0)throw new InvalidOperationException("默认 AI Provider 已不存在，请在设置中重新选择");
            if(matches.Count>1)throw new InvalidOperationException("默认 AI Provider 的 ID 重复，请检查 Provider 配置");
            var stored=matches[0];
            if(!string.Equals(stored.Type,"MiniMax",StringComparison.OrdinalIgnoreCase)&&!string.Equals(stored.Type,"OpenAICompatible",StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException($"{stored.Name} 的 Provider 类型无效");
            if(string.IsNullOrWhiteSpace(stored.Model))throw new InvalidOperationException($"{stored.Name} 的 Model 不能为空");
            _=ProviderEndpointPolicy.NormalizeBaseUri(stored.BaseUrl);
            if(stored.CustomHeaders is null)throw new InvalidOperationException($"{stored.Name} 的 Custom Headers 不能为空");
            if(stored.SensitiveHeaderCredentialIds is null)throw new InvalidOperationException($"{stored.Name} 的敏感 Header 凭据映射不能为空");
            ProviderHeaderPolicy.EnsureCredentialMappingsValid(stored.CustomHeaders,stored.SensitiveHeaderCredentialIds);
            var credentials=_credentials??new CredentialService();
            if(stored.CustomHeaders.Keys.Any(ProviderHeaderCredentialService.IsSensitive))throw new InvalidOperationException($"{stored.Name} 含未加密的敏感 Header，请在设置中重新保存");
            var unavailableHeaders=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            var provider=new ProviderHeaderCredentialService(credentials).CreateHydratedCopy(stored,unavailableHeaders);
            if(unavailableHeaders.Count>0)throw new InvalidOperationException($"{stored.Name} 的加密 Header 凭据不可用，请重新输入：{string.Join("、",unavailableHeaders)}");
            var key=credentials.Read(stored.CredentialId);
            ProviderAuthenticationPolicy.EnsureUsableCredentials(provider,key);
            key=!string.IsNullOrWhiteSpace(key)?key:string.Empty;
            if(string.Equals(provider.Type,"MiniMax",StringComparison.OrdinalIgnoreCase))return new MiniMaxProvider(provider,key);
            if(string.Equals(provider.Type,"OpenAICompatible",StringComparison.OrdinalIgnoreCase))return new OpenAiCompatibleProvider(provider,key);
            throw new NotSupportedException($"不支持的 Provider 类型：{provider.Type}");
        }
        catch(Exception ex)when(ex is InvalidOperationException or ArgumentException or NotSupportedException or IOException or UnauthorizedAccessException)
        {
            error=ex.Message;
            try{_logError?.Invoke("ProviderConfiguration",ex);}catch{}
            return null;
        }
    }
}
