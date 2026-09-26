using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ScraplingE2eTests
{
    [Fact]
    public void LocateFindsVerifiedDesktopScraplingInstall()
    {
        var expected=@"D:\scrapling（爬虫）";
        if(!File.Exists(Path.Combine(expected,"venv","Scripts","python.exe")))return;
        Assert.Equal(expected,ScraplingCrawlService.Locate(new AppSettings()));
    }

    [Fact(Skip="Requires an explicitly enabled local Scrapling install and live network; run manually with MEWU_RUN_NETWORK_TESTS=1.")]
    public async Task VerifiedScraplingEnvironmentExtractsWeChatBody()
    {
        if(Environment.GetEnvironmentVariable("MEWU_RUN_NETWORK_TESTS")!="1")return;
        var dir=@"D:\scrapling（爬虫）";
        var python=Path.Combine(dir,"venv","Scripts","python.exe");
        if(!File.Exists(python))return;
        var result=await ScraplingCrawlService.CrawlAsync(
            "https://mp.weixin.qq.com/s/5svePaascIHkohiGhxZQiw",dir,CancellationToken.None);
        Assert.True(result.Text.Length>=200,result.Text);
        Assert.Contains("CURA",result.Text,StringComparison.OrdinalIgnoreCase);
    }

    [Fact(Skip="Requires an explicitly enabled local Scrapling install and live network; run manually with MEWU_RUN_NETWORK_TESTS=1.")]
    public async Task MisreadWeChatKeyIsAutoCorrected()
    {
        // 圈选识别用的本地 OCR 会把大写 I 认成小写 l：clH 变体应被自动纠正回 IH。
        if(Environment.GetEnvironmentVariable("MEWU_RUN_NETWORK_TESTS")!="1")return;
        var dir=@"D:\scrapling（爬虫）";
        var python=Path.Combine(dir,"venv","Scripts","python.exe");
        if(!File.Exists(python))return;
        var result=await ScraplingCrawlService.CrawlAsync(
            "https://mp.weixin.qq.com/s/5svePaasclHkohiGhxZQiw",dir,CancellationToken.None);
        Assert.Equal("https://mp.weixin.qq.com/s/5svePaascIHkohiGhxZQiw",result.Url);
        Assert.NotNull(result.CorrectedFrom);
        Assert.True(result.Text.Length>=200,result.Text);
        Assert.Contains("CURA",result.Text,StringComparison.OrdinalIgnoreCase);
    }

    [Fact(Skip="Requires an explicitly enabled local Scrapling install and live network; run manually with MEWU_RUN_NETWORK_TESTS=1.")]
    public async Task InvalidWeChatKeyFailsWithClearMessage()
    {
        if(Environment.GetEnvironmentVariable("MEWU_RUN_NETWORK_TESTS")!="1")return;
        var dir=@"D:\scrapling（爬虫）";
        var python=Path.Combine(dir,"venv","Scripts","python.exe");
        if(!File.Exists(python))return;
        var ex=await Assert.ThrowsAsync<InvalidOperationException>(()=>
            ScraplingCrawlService.CrawlAsync(
                "https://mp.weixin.qq.com/s/zzzzzzzzzzzzzzzzzzzzzzzz",dir,CancellationToken.None));
        // 不再把 "Parameter error" 错误页当正文透传，而是结构化报错。
        Assert.Contains("Parameter error",ex.Message);
    }

    [Fact(Skip="Requires live network; run manually with MEWU_RUN_NETWORK_TESTS=1.")]
    public async Task BasicHttpCrawlHandlesStaticPageOrFailsGracefully()
    {
        // 内置基础抓取（未装 Scrapling 的兜底）：静态页出正文；拿不到时返回 null，
        // 不抛异常（网络不可用时不失败）。
        var result=await BasicHttpCrawlService.TryCrawlAsync("https://example.com",CancellationToken.None);
        if(result is null)return;
        Assert.True(result.Text.Length>=80,result.Text);
        Assert.Contains("Example Domain",result.Title,StringComparison.OrdinalIgnoreCase);
    }
}
