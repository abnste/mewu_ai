using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

public sealed class EnvironmentProviderBootstrap
{
    public bool Import(AppSettings settings)
    {
        var changed=false;
        changed|=Import(settings,"XAI_API_KEY","xAI Vision","OpenAICompatible","https://api.x.ai/v1","grok-4.6",makeDefault:false);
        changed|=Import(settings,"MINIMAX_CN_API_KEY","MiniMax Text","MiniMax","https://api.minimaxi.com/v1","MiniMax-M3",makeDefault:false);
        changed|=Import(settings,"VOLCENGINE_AGENTPLAN_API_KEY","Volcengine Agent Plan","OpenAICompatible","https://ark.cn-beijing.volces.com/api/plan/v3","doubao-seed-2-0-pro-260215",makeDefault:true);
        return changed;
    }

    private static bool Import(AppSettings settings,string variable,string name,string type,string baseUrl,string model,bool makeDefault)
    {
        var secret=Environment.GetEnvironmentVariable(variable);
        if(string.IsNullOrWhiteSpace(secret))return false;
        var provider=settings.Providers.FirstOrDefault(x=>x.BaseUrl.Equals(baseUrl,StringComparison.OrdinalIgnoreCase));
        if(provider is null)
        {
            provider=new AiProviderSettings{Name=name,Type=type,BaseUrl=baseUrl,Model=model};
            settings.Providers.Add(provider);
        }
        provider.Name=name;provider.Type=type;provider.Model=model;
        if(string.IsNullOrWhiteSpace(provider.CredentialId))provider.CredentialId=Guid.NewGuid().ToString("N");
        new CredentialService().Save(provider.CredentialId,secret);
        if(makeDefault&&settings.DefaultProviderId!=provider.Id)settings.DefaultProviderId=provider.Id;
        return true;
    }
}
