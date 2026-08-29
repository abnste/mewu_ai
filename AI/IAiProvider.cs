using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.AI;
public interface IAiProvider
{
    string Id { get; } AiProviderCapabilities Capabilities { get; }
    Task<AiResult> SendAsync(AiRequest request,CancellationToken cancellationToken);
    Task<bool> TestConnectionAsync(CancellationToken cancellationToken);
}
