using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ScreenEntityRecognitionTests
{
    [Fact]
    public void ExtractsUrlsAndProviderEmailsWithoutTrailingPunctuation()
    {
        var entities=ScreenEntityRecognitionService.Extract("访问 https://example.com/path?q=1。联系 a@qq.com、b@163.com.");
        Assert.Contains(entities,entity=>entity.Type==ScreenEntityType.Url&&entity.Value=="https://example.com/path?q=1");
        Assert.Contains(entities,entity=>entity.Type==ScreenEntityType.Email&&entity.Value=="a@qq.com"&&entity.MailProvider=="qq");
        Assert.Contains(entities,entity=>entity.Type==ScreenEntityType.Email&&entity.Value=="b@163.com"&&entity.MailProvider=="netease");
    }

    [Fact]
    public void IgnoresUnsupportedSchemesAndDeduplicates()
    {
        var entities=ScreenEntityRecognitionService.Extract("ftp://example.com https://example.com https://example.com x@example.org x@example.org");
        Assert.Equal(2,entities.Count);
        Assert.Contains(entities,entity=>entity.Type==ScreenEntityType.Url);
        Assert.Contains(entities,entity=>entity.Type==ScreenEntityType.Email&&entity.MailProvider is null);
    }
}
