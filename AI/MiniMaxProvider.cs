using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.AI;
public sealed class MiniMaxProvider : OpenAiCompatibleProvider
{
    public override AiProviderCapabilities Capabilities { get; }=new(true,false,false,false,false,0,TimeSpan.Zero,new HashSet<string>());
    public MiniMaxProvider(AiProviderSettings settings,string key):base(settings,key){}
}
