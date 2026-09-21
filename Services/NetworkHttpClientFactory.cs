using System.Net;
using System.Net.Http;

namespace mewu_ai_Assistant.Services;

internal static class NetworkHttpClientFactory
{
    private static readonly object Gate=new();
    private static string _mode="system";
    private static string _url=string.Empty;

    internal static void Configure(string? mode,string? url)
    {
        lock(Gate){_mode=NormalizeMode(mode);_url=url?.Trim()??string.Empty;}
    }

    internal static HttpClient Create()
    {
        string mode,url;
        lock(Gate){mode=_mode;url=_url;}
        var handler=new HttpClientHandler{AllowAutoRedirect=false,UseCookies=false};
        if(mode.Equals("direct",StringComparison.OrdinalIgnoreCase)) handler.UseProxy=false;
        else if(mode.Equals("custom",StringComparison.OrdinalIgnoreCase))
        {
            if(!Uri.TryCreate(url,UriKind.Absolute,out var proxyUri)||proxyUri.Scheme is not ("http" or "https" or "socks5"))
                throw new InvalidOperationException("自定义代理地址无效，请填写 http://、https:// 或 socks5:// 地址。");
            handler.UseProxy=true;handler.Proxy=new WebProxy(proxyUri);
        }
        return new HttpClient(handler){Timeout=Timeout.InfiniteTimeSpan};
    }

    private static string NormalizeMode(string? value)=>value?.Trim().ToLowerInvariant() switch
    {"direct"=>"direct","custom"=>"custom",_=>"system"};
}
