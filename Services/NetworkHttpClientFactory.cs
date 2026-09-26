// SPDX-License-Identifier: MPL-2.0
using System.Net;
using System.Net.Http;

namespace mewu_ai_Assistant.Services;

internal static class NetworkHttpClientFactory
{
    private static readonly object Gate=new();
    private static string _mode="system";
    private static string _url=string.Empty;
    private static HttpClient? _client;

    internal static void Validate(string? mode,string? url)
    {
        if(mode is not ("system" or "direct" or "custom"))throw new InvalidOperationException("代理模式无效。");
        if(mode=="custom"&&(!Uri.TryCreate(url,UriKind.Absolute,out var uri)||uri.Scheme is not ("http" or "https" or "socks5")||!string.IsNullOrEmpty(uri.UserInfo)||uri.AbsolutePath!="/"||uri.Query.Length>0||uri.Fragment.Length>0))
            throw new InvalidOperationException("代理地址必须是有效的 http、https 或 socks5 地址，且不能包含账号密码、路径或查询参数。");
    }

    internal static void Configure(string? mode,string? url)
    {
        Validate(mode,url);
        lock(Gate)
        {
            if(_mode==mode&&_url==(url?.Trim()??string.Empty))return;
            _mode=mode!;_url=url?.Trim()??string.Empty;
            _client=null;
        }
    }

    /// <summary>Reads the currently configured proxy mode and URL without creating a client.</summary>
    internal static (string Mode,string Url) CurrentProxy()
    {
        lock(Gate){return (_mode,_url);}
    }

    internal static HttpClient Create()
    {
        lock(Gate)
        {
        if(_client is not null)return _client;
        var mode=_mode;var url=_url;
        var handler=new SocketsHttpHandler{AllowAutoRedirect=false,UseCookies=false,PooledConnectionLifetime=TimeSpan.FromMinutes(5),PooledConnectionIdleTimeout=TimeSpan.FromMinutes(1)};
        if(mode.Equals("direct",StringComparison.OrdinalIgnoreCase)) handler.UseProxy=false;
        else if(mode.Equals("custom",StringComparison.OrdinalIgnoreCase))
        {
            if(!Uri.TryCreate(url,UriKind.Absolute,out var proxyUri)||proxyUri.Scheme is not ("http" or "https" or "socks5"))
                throw new InvalidOperationException("自定义代理地址无效，请填写 http://、https:// 或 socks5:// 地址。");
            handler.UseProxy=true;handler.Proxy=new WebProxy(proxyUri);
        }
        return _client=new HttpClient(handler){Timeout=Timeout.InfiniteTimeSpan};
        }
    }

    private static string NormalizeMode(string? value)=>value?.Trim().ToLowerInvariant() switch
    {"direct"=>"direct","custom"=>"custom",_=>"system"};
}
