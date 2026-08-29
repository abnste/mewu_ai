using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;
namespace MewuAI.Tests;
public sealed class ServicesTests
{
    [Fact] public void CredentialService_RoundTripsWithCurrentUserDpapi(){var id="test-"+Guid.NewGuid().ToString("N");var service=new CredentialService();service.Save(id,"secret-value");Assert.Equal("secret-value",service.Read(id));var path=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","Credentials",id+".bin");File.Delete(path);}
    [Fact] public void MiniMaxProvider_RejectsImageBeforeNetwork(){var provider=new MiniMaxProvider(new AiProviderSettings{Type="MiniMax",BaseUrl="https://api.minimax.io/v1",Model="MiniMax-M2.7"},"unused");Assert.False(provider.Capabilities.SupportsImage);}
    [Fact] public void OpenAiProvider_DeclaresSupportedImageMimeTypes(){var provider=new OpenAiCompatibleProvider(new AiProviderSettings(),"unused");Assert.Contains("image/png",provider.Capabilities.AcceptedMimeTypes);Assert.False(provider.Capabilities.SupportsVideo);}
}
