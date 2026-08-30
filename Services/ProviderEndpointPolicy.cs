namespace mewu_ai_Assistant.Services;

public static class ProviderEndpointPolicy
{
    public static Uri NormalizeBaseUri(string? value)
    {
        if(string.IsNullOrWhiteSpace(value)||!Uri.TryCreate(value.Trim(),UriKind.Absolute,out var uri)||string.IsNullOrWhiteSpace(uri.Host))
            throw new InvalidOperationException("Provider Base URL 必须是有效的绝对地址");
        if(!string.IsNullOrEmpty(uri.UserInfo))
            throw new InvalidOperationException("Provider Base URL 不能包含用户名或密码");
        if(!string.IsNullOrEmpty(uri.Query)||!string.IsNullOrEmpty(uri.Fragment))
            throw new InvalidOperationException("Provider Base URL 不能包含查询参数或片段");

        var isHttps=uri.Scheme.Equals(Uri.UriSchemeHttps,StringComparison.OrdinalIgnoreCase);
        var isLoopbackHttp=uri.Scheme.Equals(Uri.UriSchemeHttp,StringComparison.OrdinalIgnoreCase)&&uri.IsLoopback;
        if(!isHttps&&!isLoopbackHttp)
            throw new InvalidOperationException("Provider Base URL 必须使用 HTTPS；仅本机 loopback 服务允许 HTTP");

        var builder=new UriBuilder(uri){Scheme=uri.Scheme.ToLowerInvariant(),Path=uri.AbsolutePath.TrimEnd('/')+"/"};
        return builder.Uri;
    }
}
