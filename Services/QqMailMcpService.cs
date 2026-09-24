// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net;
using System.Net.Http;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text.Json;
using System.Text.Json.Nodes;
namespace mewu_ai_Assistant.Services;

/// <summary>QQ 邮箱 MCP OAuth 令牌。访问令牌较短（约 1 小时），必须保留刷新令牌。</summary>
internal sealed record QqMailMcpToken(string AccessToken,string RefreshToken,string ClientId,DateTimeOffset ExpiresAt,string Scope)
{
    public bool IsUsable=>!string.IsNullOrWhiteSpace(AccessToken)&&ExpiresAt>DateTimeOffset.UtcNow.AddMinutes(1);
}

/// <summary>
/// QQ 邮箱官方 MCP 服务（https://api.mail.qq.com/mcp）的传输与凭据层。
/// 协议为完全无状态的 Streamable HTTP JSON-RPC：服务端不返回 Mcp-Session-Id，
/// 已验证无需 initialize 握手即可直接 tools/call（响应为 application/json），
/// 因此每次调用都是独立请求，也便于规避 10 次/分钟的速率限制。
/// 凭据完全独立于任何第三方客户端（含本机 WorkBuddy）：每个用户通过
/// 腾讯授权页展示的二维码自行扫码授权，令牌仅保存在本机 DPAPI 中。
/// </summary>
internal static class QqMailMcpService
{
    internal const string Endpoint="https://api.mail.qq.com/mcp";
    internal const string CredentialId="qqmail-mcp-token";
    private const string AuthorizeEndpoint="https://wx.mail.qq.com/oauth/authorize";
    private const string TokenEndpoint="https://wx.mail.qq.com/oauth/token";
    private const string RegisterEndpoint="https://wx.mail.qq.com/oauth/register";
    private const string DefaultScope="alias:read mail:read mail:send mail:delete";
    // 腾讯按“可信平台名单”校验动态注册的 client_name/redirect_uri（已实测：
    // mewu_ai、VS Code、Cherry Studio 等名字与自定义 scheme 均被 403 拒绝，
    // 仅 Codex 系名称允许任意 loopback 端口）。这是第三方桌面客户端接入
    // QQ 邮箱 MCP 的唯一公开通道，注册后由腾讯返回平台级 client_id。
    private const string RegisterClientName="Codex CLI";
    private static readonly TimeSpan CallTimeout=TimeSpan.FromSeconds(30);
    private static readonly TimeSpan AuthorizationTimeout=TimeSpan.FromMinutes(5);
    private static readonly SemaphoreSlim Gate=new(1,1);
    private static int _requestId;
    private static readonly JsonSerializerOptions JsonOptions=new(){PropertyNameCaseInsensitive=true};

    #region 凭据存取

    internal static QqMailMcpToken? ReadCachedToken()
    {
        try
        {
            var stored=new CredentialService().Read(CredentialId);
            if(string.IsNullOrWhiteSpace(stored))return null;
            return JsonSerializer.Deserialize<QqMailMcpToken>(stored,JsonOptions);
        }
        catch{return null;}
    }

    internal static void CacheToken(QqMailMcpToken token)
    {
        try{new CredentialService().Save(CredentialId,JsonSerializer.Serialize(token,JsonOptions));}
        catch(Exception ex){new PrivacyLogger().Info("QqMailTokenCache",ex.GetType().Name);}
    }

    internal static void ClearCachedToken()
    {
        try{new CredentialService().Save(CredentialId,string.Empty);}
        catch(Exception ex){new PrivacyLogger().Info("QqMailTokenClear",ex.GetType().Name);}
    }

    /// <summary>取得可用令牌：本地缓存 → refresh_token 自动刷新。全部失败返回 null。
    /// 不读取、不导入任何第三方客户端（如 WorkBuddy）的凭据。</summary>
    internal static async Task<QqMailMcpToken?> ResolveTokenAsync(CancellationToken cancellationToken)
    {
        var cached=ReadCachedToken();
        if(cached is {IsUsable:true})return cached;
        if(cached is null||string.IsNullOrWhiteSpace(cached.RefreshToken)||string.IsNullOrWhiteSpace(cached.ClientId))return null;
        await Gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            // 双重检查：并发调用可能已完成刷新。
            if(ReadCachedToken() is {IsUsable:true} refreshed) return refreshed;
            var result=await RefreshAsync(cached,cancellationToken).ConfigureAwait(false);
            if(result is {IsUsable:true})
            {
                CacheToken(result);
                return result;
            }
            return null;
        }
        catch(OperationCanceledException){throw;}
        catch(Exception ex){new PrivacyLogger().Info("QqMailRefresh",ex.GetType().Name);return null;}
        finally{Gate.Release();}
    }

    #endregion

    #region 扫码授权（OAuth 2.0 授权码 + PKCE）

    /// <summary>
    /// 执行完整扫码授权：动态注册客户端 → 构造 PKCE 授权 URL → 通过
    /// <paramref name="openInBrowser"/> 打开（腾讯授权页会直接渲染二维码，
    /// 用户用手机 QQ 邮箱 App 扫码确认）→ 本地 loopback HttpListener 接收
    /// 授权码回调 → 换取令牌并保存。每个用户授权的都是自己的账号。
    /// </summary>
    /// <param name="openInBrowser">在 UI 线程打开授权 URL（例如系统默认浏览器）。</param>
    internal static async Task<QqMailMcpToken> AuthorizeAsync(Action<string> openInBrowser,CancellationToken cancellationToken)
    {
        // 先探测一个空闲 loopback 端口，注册与回调保持一致。
        int port;
        var probe=new TcpListener(IPAddress.Loopback,0);
        try{probe.Start();port=((IPEndPoint)probe.LocalEndpoint).Port;}
        finally{probe.Stop();}
        var redirectUri=$"http://localhost:{port}/callback";

        var clientId=await RegisterClientAsync(redirectUri,cancellationToken).ConfigureAwait(false);

        var codeVerifier=Base64Url(RandomNumberGenerator.GetBytes(32));
        var codeChallenge=Base64Url(SHA256.HashData(System.Text.Encoding.ASCII.GetBytes(codeVerifier)));
        var state=Base64Url(RandomNumberGenerator.GetBytes(16));
        var authorizeUrl=$"{AuthorizeEndpoint}?response_type=code&client_id={Uri.EscapeDataString(clientId)}"+
            $"&redirect_uri={Uri.EscapeDataString(redirectUri)}&scope={Uri.EscapeDataString(DefaultScope)}"+
            $"&code_challenge={codeChallenge}&code_challenge_method=S256&state={state}";

        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(AuthorizationTimeout);
        var listener=new HttpListener();
        listener.Prefixes.Add($"http://localhost:{port}/");
        listener.Start();
        try
        {
            openInBrowser(authorizeUrl);
            var context=await listener.GetContextAsync().WaitAsync(timeout.Token).ConfigureAwait(false);
            var query=context.Request.QueryString;
            RespondCallbackPage(context);
            var error=query["error"];
            if(!string.IsNullOrEmpty(error))
                throw new InvalidOperationException(string.Equals(error,"access_denied",StringComparison.OrdinalIgnoreCase)
                    ?LocalizationService.IsEnglish?"Authorization was denied in the QQ Mail app.":"用户在 QQ 邮箱 App 中拒绝了授权。"
                    :string.Format(System.Globalization.CultureInfo.CurrentCulture,LocalizationService.IsEnglish?"Authorization failed: {0}":"授权失败：{0}",error));
            var code=query["code"];
            var returnedState=query["state"];
            if(string.IsNullOrEmpty(code))throw new InvalidOperationException(LocalizationService.IsEnglish?"The authorization callback did not contain a code.":"授权回调中没有授权码。");
            if(!string.Equals(returnedState,state,StringComparison.Ordinal))throw new InvalidOperationException(LocalizationService.IsEnglish?"Authorization state mismatch; please retry.":"授权 state 校验失败，请重试。");
            var token=await ExchangeCodeAsync(clientId,code,codeVerifier,redirectUri,cancellationToken).ConfigureAwait(false);
            CacheToken(token);
            return token;
        }
        finally
        {
            try{listener.Stop();}catch{}
            try{listener.Close();}catch{}
        }
    }

    /// <summary>RFC 7591 动态注册：腾讯按平台名单校验后返回平台级 client_id（已实测幂等）。</summary>
    private static async Task<string> RegisterClientAsync(string redirectUri,CancellationToken cancellationToken)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(CallTimeout);
        var payload=new JsonObject
        {
            ["client_name"]=RegisterClientName,
            ["redirect_uris"]=new JsonArray(redirectUri),
            ["grant_types"]=new JsonArray("authorization_code","refresh_token"),
            ["response_types"]=new JsonArray("code"),
            ["token_endpoint_auth_method"]="none"
        };
        using var content=new StringContent(payload.ToJsonString(),System.Text.Encoding.UTF8,"application/json");
        using var response=await NetworkHttpClientFactory.Create().PostAsync(RegisterEndpoint,content,timeout.Token).ConfigureAwait(false);
        var body=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
        if(!response.IsSuccessStatusCode)throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
            LocalizationService.IsEnglish?"QQ Mail client registration failed (HTTP {0}).":"QQ 邮箱客户端注册失败（HTTP {0}）。",(int)response.StatusCode));
        using var document=JsonDocument.Parse(body);
        var clientId=GetString(document.RootElement,"client_id");
        if(string.IsNullOrWhiteSpace(clientId))throw new InvalidOperationException(LocalizationService.IsEnglish?"QQ Mail client registration response is missing client_id.":"QQ 邮箱客户端注册响应缺少 client_id。");
        return clientId!;
    }

    /// <summary>用授权码 + PKCE code_verifier 换取令牌（token_endpoint_auth_method=none，无需 client_secret）。</summary>
    private static async Task<QqMailMcpToken> ExchangeCodeAsync(string clientId,string code,string codeVerifier,string redirectUri,CancellationToken cancellationToken)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(CallTimeout);
        using var content=new FormUrlEncodedContent(new Dictionary<string,string>
        {
            ["grant_type"]="authorization_code",
            ["code"]=code,
            ["redirect_uri"]=redirectUri,
            ["client_id"]=clientId,
            ["code_verifier"]=codeVerifier
        });
        using var response=await NetworkHttpClientFactory.Create().PostAsync(TokenEndpoint,content,timeout.Token).ConfigureAwait(false);
        var body=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
        if(!response.IsSuccessStatusCode)throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
            LocalizationService.IsEnglish?"QQ Mail token exchange failed (HTTP {0}): {1}":"QQ 邮箱令牌交换失败（HTTP {0}）：{1}",(int)response.StatusCode,ExtractOAuthError(body)));
        return ParseTokenResponse(body,clientId,requireRefreshToken:true);
    }

    private static async Task<QqMailMcpToken> RefreshAsync(QqMailMcpToken token,CancellationToken cancellationToken)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(CallTimeout);
        using var content=new FormUrlEncodedContent(new Dictionary<string,string>
        {
            ["grant_type"]="refresh_token",
            ["refresh_token"]=token.RefreshToken,
            ["client_id"]=token.ClientId
        });
        using var response=await NetworkHttpClientFactory.Create().PostAsync(TokenEndpoint,content,timeout.Token).ConfigureAwait(false);
        var body=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
        if(!response.IsSuccessStatusCode)throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
            LocalizationService.IsEnglish?"QQ Mail token refresh failed (HTTP {0}).":"QQ 邮箱令牌刷新失败（HTTP {0}）。",(int)response.StatusCode));
        // 刷新响应可能不回传 refresh_token；为空时沿用旧值。
        var refreshed=ParseTokenResponse(body,token.ClientId,requireRefreshToken:false);
        return new QqMailMcpToken(refreshed.AccessToken,
            refreshed.RefreshToken.Length>0?refreshed.RefreshToken:token.RefreshToken,
            token.ClientId,refreshed.ExpiresAt,refreshed.Scope);
    }

    private static QqMailMcpToken ParseTokenResponse(string body,string clientId,bool requireRefreshToken)
    {
        using var document=JsonDocument.Parse(body);
        var root=document.RootElement;
        var accessToken=GetString(root,"access_token")??GetString(root,"accessToken");
        if(string.IsNullOrWhiteSpace(accessToken))throw new InvalidOperationException(LocalizationService.IsEnglish
            ?$"QQ Mail token response is invalid: {ExtractOAuthError(body)}"
            :$"QQ 邮箱令牌响应无效：{ExtractOAuthError(body)}");
        var refreshToken=GetString(root,"refresh_token")??GetString(root,"refreshToken");
        if(requireRefreshToken&&string.IsNullOrWhiteSpace(refreshToken))
            throw new InvalidOperationException(LocalizationService.IsEnglish?"QQ Mail token response is missing refresh_token.":"QQ 邮箱令牌响应缺少 refresh_token。");
        var expiresAt=ReadEpochMs(root,"expires_at")??ReadEpochMs(root,"expiresAt");
        if(expiresAt is null)
        {
            var seconds=GetInt(root,"expires_in")??GetInt(root,"expiresIn");
            expiresAt=seconds is null?DateTimeOffset.UtcNow.AddMinutes(55):DateTimeOffset.UtcNow.AddSeconds(seconds.Value);
        }
        var scope=GetString(root,"scope")??DefaultScope;
        return new QqMailMcpToken(accessToken!,refreshToken??string.Empty,clientId,expiresAt.Value,scope);
    }

    private static string ExtractOAuthError(string body)
    {
        try
        {
            using var document=JsonDocument.Parse(body);
            var error=GetString(document.RootElement,"error")??string.Empty;
            var description=GetString(document.RootElement,"error_description")??string.Empty;
            var message=string.Join("：",new[]{error,description}.Where(part=>part.Length>0));
            return message.Length>0?message:body.Length>160?body[..160]:body;
        }
        catch{return body.Length>160?body[..160]:body;}
    }

    private static void RespondCallbackPage(HttpListenerContext context)
    {
        const string html="""<!doctype html><html><head><meta charset="utf-8"><title>mewu_ai</title></head><body style="font-family:system-ui;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#f6f8fa"><div style="text-align:center"><h2 style="color:#2e7d32">✓ 授权成功</h2><p>已连接 QQ 邮箱，请返回 mewu_ai。</p><p style="color:#888">Authorization complete — return to mewu_ai.</p></div></body></html>""";
        var buffer=System.Text.Encoding.UTF8.GetBytes(html);
        context.Response.StatusCode=200;
        context.Response.ContentType="text/html; charset=utf-8";
        context.Response.ContentLength64=buffer.Length;
        try{context.Response.OutputStream.Write(buffer);}catch{}
        try{context.Response.Close();}catch{}
    }

    private static string Base64Url(byte[] data)=>Convert.ToBase64String(data).TrimEnd('=').Replace('+','-').Replace('/','_');

    #endregion

    #region MCP JSON-RPC

    /// <summary>执行一次 MCP JSON-RPC 调用并返回 result 节点。</summary>
    internal static async Task<JsonElement> CallAsync(QqMailMcpToken token,string method,JsonObject? parameters,CancellationToken cancellationToken)
    {
        return await RpcAsync(token,method,parameters,cancellationToken).ConfigureAwait(false);
    }

    /// <summary>调用一个 MCP 工具并返回 content[0].text 原文（工具报错时抛出）。</summary>
    internal static async Task<string> CallToolAsync(QqMailMcpToken token,string tool,JsonObject arguments,CancellationToken cancellationToken)
    {
        var (text,isError)=await CallToolRawAsync(token,tool,arguments,cancellationToken).ConfigureAwait(false);
        if(isError)throw new InvalidOperationException(text.Length>0?text:$"QQ 邮箱工具 {tool} 执行失败");
        if(text.Length==0)throw new InvalidOperationException($"QQ 邮箱工具 {tool} 未返回文本内容");
        return text;
    }

    /// <summary>调用一个 MCP 工具，返回 (text,isError)。两阶段确认类工具的
    /// 第一阶段响应（含 confirmation_token 的错误体）通过本方法原样返回。</summary>
    internal static async Task<(string Text,bool IsError)> CallToolRawAsync(QqMailMcpToken token,string tool,JsonObject arguments,CancellationToken cancellationToken)
    {
        var result=await CallAsync(token,"tools/call",new JsonObject{["name"]=tool,["arguments"]=DeepCopy(arguments)},cancellationToken).ConfigureAwait(false);
        if(result.ValueKind!=JsonValueKind.Object)throw new InvalidOperationException("QQ 邮箱 MCP 返回了无法解析的工具结果");
        var isError=result.TryGetProperty("isError",out var errorFlag)&&errorFlag.ValueKind==JsonValueKind.True;
        return (GetTextContent(result),isError);
    }

    private static async Task<JsonElement> RpcAsync(QqMailMcpToken token,string method,JsonObject? parameters,CancellationToken cancellationToken)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(CallTimeout);
        var payload=new JsonObject
        {
            ["jsonrpc"]="2.0",
            ["id"]=Interlocked.Increment(ref _requestId),
            ["method"]=method
        };
        if(parameters is not null)payload["params"]=DeepCopy(parameters);
        using var request=new HttpRequestMessage(HttpMethod.Post,Endpoint)
        {
            Content=new StringContent(payload.ToJsonString(),System.Text.Encoding.UTF8,"application/json")
        };
        request.Headers.TryAddWithoutValidation("Accept","application/json, text/event-stream");
        request.Headers.Authorization=new System.Net.Http.Headers.AuthenticationHeaderValue("Bearer",token.AccessToken);
        using var response=await NetworkHttpClientFactory.Create().SendAsync(request,HttpCompletionOption.ResponseHeadersRead,timeout.Token).ConfigureAwait(false);
        if((int)response.StatusCode==401)throw new InvalidOperationException("QQ 邮箱令牌已失效，请在设置中重新扫码授权");
        if(!response.IsSuccessStatusCode)throw new InvalidOperationException($"QQ 邮箱 MCP 请求失败（HTTP {(int)response.StatusCode}）");
        var body=await ReadBodyAsync(response,timeout.Token).ConfigureAwait(false);
        using var document=JsonDocument.Parse(body);
        var root=document.RootElement;
        if(root.TryGetProperty("error",out var error))
        {
            var message=GetString(error,"message")??error.GetRawText();
            throw new InvalidOperationException($"QQ 邮箱 MCP 错误：{message}");
        }
        if(!root.TryGetProperty("result",out var result))throw new InvalidOperationException("QQ 邮箱 MCP 响应缺少 result");
        return result.Clone();
    }

    /// <summary>响应可能是 application/json，也可能是 text/event-stream（逐行 data: 前缀）。</summary>
    private static async Task<string> ReadBodyAsync(HttpResponseMessage response,CancellationToken cancellationToken)
    {
        var contentType=response.Content.Headers.ContentType?.MediaType??"application/json";
        using var stream=await response.Content.ReadAsStreamAsync(cancellationToken).ConfigureAwait(false);
        using var reader=new StreamReader(stream,System.Text.Encoding.UTF8);
        if(!contentType.Contains("event-stream",StringComparison.OrdinalIgnoreCase))
            return await reader.ReadToEndAsync(cancellationToken).ConfigureAwait(false);
        var builder=new System.Text.StringBuilder();
        while(await reader.ReadLineAsync(cancellationToken).ConfigureAwait(false) is { } line)
        {
            if(!line.StartsWith("data:",StringComparison.OrdinalIgnoreCase))continue;
            var data=line["data:".Length..].TrimStart();
            if(data.Length==0||data=="[DONE]")continue;
            builder.Append(data);
        }
        return builder.ToString();
    }

    private static string GetTextContent(JsonElement result)
    {
        if(!result.TryGetProperty("content",out var content)||content.ValueKind!=JsonValueKind.Array)return string.Empty;
        foreach(var item in content.EnumerateArray())
        {
            if(item.ValueKind!=JsonValueKind.Object)continue;
            var type=GetString(item,"type");
            if(!string.Equals(type,"text",StringComparison.OrdinalIgnoreCase))continue;
            return GetString(item,"text")??string.Empty;
        }
        return string.Empty;
    }

    private static JsonObject DeepCopy(JsonObject source)=>JsonNode.Parse(source.ToJsonString())!.AsObject();

    internal static string? GetString(JsonElement parent,string name)
    {
        if(parent.ValueKind!=JsonValueKind.Object)return null;
        return parent.TryGetProperty(name,out var value)&&value.ValueKind is JsonValueKind.String?value.GetString():null;
    }

    private static int? GetInt(JsonElement parent,string name)
    {
        if(parent.ValueKind!=JsonValueKind.Object)return null;
        return parent.TryGetProperty(name,out var value)&&value.ValueKind is JsonValueKind.Number&&value.TryGetInt32(out var parsed)?parsed:null;
    }

    private static DateTimeOffset? ReadEpochMs(JsonElement parent,string name)
    {
        if(parent.ValueKind!=JsonValueKind.Object)return null;
        if(!parent.TryGetProperty(name,out var value)||value.ValueKind is not JsonValueKind.Number||!value.TryGetInt64(out var milliseconds))return null;
        return DateTimeOffset.FromUnixTimeMilliseconds(milliseconds);
    }

    #endregion
}
