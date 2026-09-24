// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net;
using System.Net.Http;
using System.Text.RegularExpressions;
namespace mewu_ai_Assistant.Services;

/// <summary>
/// 内置的基础 HTTP 抓取（零依赖，无需安装 Scrapling/Python）：Chrome 请求头 +
/// 自动解压 + 自动重定向，适合静态页面（大多数博客、新闻、文档站）。
/// 页面需要 JS 渲染（微信公众号）或被反爬拦截（知乎 403）时返回 null，
/// 由调用方回退到 Scrapling（引导一键安装）。这是"下载了 mewuAI 但没装
/// Scrapling"用户的兜底链路：能抓的先抓到，抓不到的给出明确指引。
/// </summary>
internal static class BasicHttpCrawlService
{
    private static readonly HttpClient Http=CreateClient();

    private static HttpClient CreateClient()
    {
        var handler=new HttpClientHandler
        {
            AutomaticDecompression=DecompressionMethods.All,
            AllowAutoRedirect=true,
            MaxAutomaticRedirections=6,
            UseCookies=true,
        };
        var client=new HttpClient(handler){Timeout=TimeSpan.FromSeconds(25)};
        client.DefaultRequestHeaders.TryAddWithoutValidation("User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36");
        client.DefaultRequestHeaders.TryAddWithoutValidation("Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8");
        client.DefaultRequestHeaders.TryAddWithoutValidation("Accept-Language","zh-CN,zh;q=0.9,en;q=0.8");
        return client;
    }

    /// <summary>尝试抓取静态页面正文。页面需要 JS 渲染、被拦截或网络失败时返回
    /// null（不抛异常），调用方据此回退到 Scrapling 链路。</summary>
    internal static async Task<ScraplingCrawlService.CrawlResult?> TryCrawlAsync(string url,CancellationToken cancellationToken)
    {
        try
        {
            using var request=new HttpRequestMessage(HttpMethod.Get,url);
            using var response=await Http.SendAsync(request,cancellationToken).ConfigureAwait(false);
            if(!response.IsSuccessStatusCode)
            {
                new PrivacyLogger().Info("BasicHttpCrawl",$"status={(int)response.StatusCode}");
                return null;
            }
            var mediaType=response.Content.Headers.ContentType?.MediaType??string.Empty;
            if(mediaType.Length>0&&!mediaType.Contains("html",StringComparison.OrdinalIgnoreCase)&&!mediaType.Contains("xml",StringComparison.OrdinalIgnoreCase))
                return null;
            var html=await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false);
            return ExtractResult(url,html);
        }
        catch(Exception ex)
        {
            new PrivacyLogger().Info("BasicHttpCrawl",ex.GetType().Name);
            return null;
        }
    }

    /// <summary>从 HTML 抽取标题+正文（剥 script/style、块级标签转换行、解实体）。
    /// 剥完几乎没正文（典型为 JS 渲染页或错误页）时返回 null。</summary>
    private static ScraplingCrawlService.CrawlResult? ExtractResult(string url,string html)
    {
        if(string.IsNullOrWhiteSpace(html))return null;
        var titleMatch=Regex.Match(html,"<title[^>]*>(.*?)</title>",RegexOptions.Singleline|RegexOptions.IgnoreCase);
        var title=WebUtility.HtmlDecode(titleMatch.Success?titleMatch.Groups[1].Value:string.Empty).Trim();
        var body=html;
        var bodyMatch=Regex.Match(body,"<body[^>]*>(.*?)</body>",RegexOptions.Singleline|RegexOptions.IgnoreCase);
        if(bodyMatch.Success)body=bodyMatch.Groups[1].Value;
        body=Regex.Replace(body,"<script[^>]*>.*?</script>"," ",RegexOptions.Singleline|RegexOptions.IgnoreCase);
        body=Regex.Replace(body,"<style[^>]*>.*?</style>"," ",RegexOptions.Singleline|RegexOptions.IgnoreCase);
        body=Regex.Replace(body,"<noscript[^>]*>.*?</noscript>"," ",RegexOptions.Singleline|RegexOptions.IgnoreCase);
        body=Regex.Replace(body,"<svg[^>]*>.*?</svg>"," ",RegexOptions.Singleline|RegexOptions.IgnoreCase);
        body=Regex.Replace(body,"<br\\s*/?>|</p>|</div>|</li>|</tr>|</h[1-6]>|</article>|</section>","\n",RegexOptions.IgnoreCase);
        body=Regex.Replace(body,"<[^>]+>"," ");
        body=WebUtility.HtmlDecode(body);
        var lines=body.Split('\n')
            .Select(line=>Regex.Replace(line,"\\s+"," ").Trim())
            .Where(line=>line.Length>0);
        var text=string.Join("\n",lines);
        // JS 渲染页（微信等）剥完标签只剩壳或错误页文案：不足以当正文。
        if(text.Length<120)return null;
        if(text.Contains("Parameter error",StringComparison.OrdinalIgnoreCase))return null;
        return new ScraplingCrawlService.CrawlResult(url,title,text,Engine:LocalizationService.T("内置基础抓取","built-in basic fetch"));
    }
}
