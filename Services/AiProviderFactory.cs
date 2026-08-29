using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;
public sealed class AiProviderFactory
{
    private readonly CredentialService _credentials=new();
    public IAiProvider? Create(AppSettings settings){var p=settings.Providers.FirstOrDefault(x=>x.Id==settings.DefaultProviderId)??settings.Providers.FirstOrDefault();if(p is null)return null;var key=_credentials.Read(p.CredentialId);if(string.IsNullOrEmpty(key))return null;return p.Type.Equals("MiniMax",StringComparison.OrdinalIgnoreCase)?new MiniMaxProvider(p,key):new OpenAiCompatibleProvider(p,key);}
}
