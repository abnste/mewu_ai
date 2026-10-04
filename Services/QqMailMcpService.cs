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

internal sealed class QqMailTokenCoordinator(Func<QqMailMcpToken?> read,Action<QqMailMcpToken?> write)
{
    private readonly object _storeGate=new();
    private readonly SemaphoreSlim _refreshGate=new(1,1);
    private long _revision;
    internal QqMailMcpToken? Read(){lock(_storeGate)return read();}
    internal void Write(QqMailMcpToken? value){lock(_storeGate){_revision++;write(value);}}
    internal async Task<QqMailMcpToken?> ResolveAsync(Func<QqMailMcpToken,CancellationToken,Task<QqMailMcpToken>> refresh,CancellationToken token)
    {
        if(Read() is {IsUsable:true} current)return current;
        await _refreshGate.WaitAsync(token).ConfigureAwait(false);
        try
        {
            QqMailMcpToken? latest;long revision;
            lock(_storeGate){latest=read();revision=_revision;}
            if(latest is {IsUsable:true})return latest;
            if(latest is null||string.IsNullOrWhiteSpace(latest.RefreshToken)||string.IsNullOrWhiteSpace(latest.ClientId))return null;
            var result=await refresh(latest,token).ConfigureAwait(false);
            token.ThrowIfCancellationRequested();
            lock(_storeGate)
            {
                // A clear, a new authorization, or a concurrent credential write
                // wins over an older in-flight refresh, including failed writes.
                if(revision!=_revision||read()!=latest)return read() is {IsUsable:true} replacement?replacement:null;
                if(!result.IsUsable)return null;
                _revision++;write(result);return result;
            }
        }
        finally{_refreshGate.Release();}
    }
}

/// <summary>
/// QQ 邮箱官方 MCP 服务（https://api.mail.qq.com/mcp）的传输与凭据层。
/// 使用 Streamable HTTP JSON-RPC；每次独立调用先协商协议并初始化，
/// 如果服务端返回 Mcp-Session-Id，则仅在本次调用内透传，不跨账号缓存。
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
    // Identify this application truthfully. A server allowlist rejection must not
    // be worked around by registering under another application's identity.
    internal const string RegisterClientName="MewuAI";
    private static readonly TimeSpan CallTimeout=TimeSpan.FromSeconds(30);
    private static readonly TimeSpan AuthorizationTimeout=TimeSpan.FromMinutes(5);
    private static readonly QqMailTokenCoordinator Tokens=new(ReadTokenStore,WriteTokenStore);
    private static int _requestId;
    private static readonly JsonSerializerOptions JsonOptions=new(){PropertyNameCaseInsensitive=true};

    #region 凭据存取

    internal static QqMailMcpToken? ReadCachedToken()=>Tokens.Read();

    private static QqMailMcpToken? ReadTokenStore()
    {
        try
        {
            var stored=new CredentialService().Read(CredentialId);
            if(string.IsNullOrWhiteSpace(stored))return null;
            return JsonSerializer.Deserialize<QqMailMcpToken>(stored,JsonOptions);
        }
        catch{return null;}
    }

    internal static void CacheToken(QqMailMcpToken token)=>Tokens.Write(token);

    private static void WriteTokenStore(QqMailMcpToken? token)
        =>new CredentialService().Save(CredentialId,token is null?string.Empty:JsonSerializer.Serialize(token,JsonOptions));

    internal static void ClearCachedToken()=>Tokens.Write(null);

    /// <summary>取得可用令牌：本地缓存 → refresh_token 自动刷新。全部失败返回 null。
    /// 不读取、不导入任何第三方客户端（如 WorkBuddy）的凭据。</summary>
    internal static async Task<QqMailMcpToken?> ResolveTokenAsync(CancellationToken cancellationToken)
    {
        try
        {
            return await Tokens.ResolveAsync(RefreshAsync,cancellationToken).ConfigureAwait(false);
        }
        catch(OperationCanceledException)when(cancellationToken.IsCancellationRequested){throw;}
        catch(Exception ex){new PrivacyLogger().Info("QqMailRefresh",ex.GetType().Name);return null;}
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
            while(true)
            {
                var context=await listener.GetContextAsync().WaitAsync(timeout.Token).ConfigureAwait(false);
                var query=context.Request.QueryString;
                if(!IsExpectedCallback(context.Request.HttpMethod,context.Request.Url?.AbsolutePath,query["state"],state))
                {
                    RespondCallbackPage(context,false,400);
                    continue; // Ignore unrelated browser requests (including favicon).
                }
                var error=query["error"];
                if(!string.IsNullOrEmpty(error))
                {
                    RespondCallbackPage(context,false,400);
                    throw new InvalidOperationException(LocalizationService.T("QQ 邮箱授权未完成，请返回应用重试。","QQ Mail authorization did not complete. Return to the app and retry."));
                }
                var code=query["code"];
                if(string.IsNullOrWhiteSpace(code)){RespondCallbackPage(context,false,400);continue;}
                try
                {
                    var token=await ExchangeCodeAsync(clientId,code,codeVerifier,redirectUri,timeout.Token).ConfigureAwait(false);
                    CacheToken(token);
                    RespondCallbackPage(context,true,200);
                    return token;
                }
                catch{RespondCallbackPage(context,false,400);throw;}
            }
        }
        finally
        {
            try{listener.Stop();}catch{}
            try{listener.Close();}catch{}
        }
    }

    /// <summary>RFC 7591 动态注册：使用本应用身份，由平台决定是否接受注册。</summary>
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
        if(response.StatusCode==HttpStatusCode.Forbidden)throw new InvalidOperationException(LocalizationService.T(
            "QQ 邮箱平台尚未接受 MewuAI 客户端注册，无法开始新授权；已有有效授权不受影响。",
            "QQ Mail has not accepted MewuAI client registration, so new authorization cannot start. Existing valid authorization is unchanged."));
        if(!response.IsSuccessStatusCode)throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
            LocalizationService.IsEnglish?"QQ Mail client registration failed (HTTP {0}).":"QQ 邮箱客户端注册失败（HTTP {0}）。",(int)response.StatusCode));
        using var document=ParseResponseJson(body);
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
        var refreshed=ParseTokenResponse(body,token.ClientId,requireRefreshToken:false,fallbackScope:token.Scope);
        return new QqMailMcpToken(refreshed.AccessToken,
            refreshed.RefreshToken.Length>0?refreshed.RefreshToken:token.RefreshToken,
            token.ClientId,refreshed.ExpiresAt,refreshed.Scope);
    }

    internal static QqMailMcpToken ParseTokenResponse(string body,string clientId,bool requireRefreshToken,string? fallbackScope=null)
    {
        using var document=ParseResponseJson(body);
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
        var scope=GetString(root,"scope")??fallbackScope??DefaultScope;
        return new QqMailMcpToken(accessToken!,refreshToken??string.Empty,clientId,expiresAt.Value,scope);
    }

    private static string ExtractOAuthError(string body)
    {
        try
        {
            using var document=JsonDocument.Parse(body);
            var error=GetString(document.RootElement,"error")??string.Empty;
            return error is "invalid_request" or "invalid_client" or "invalid_grant" or "unauthorized_client" or "unsupported_grant_type" or "invalid_scope" or "access_denied"?error:"invalid_response";
        }
        catch{return "invalid_response";}
    }

    internal static bool IsExpectedCallback(string method,string? path,string? actualState,string expectedState)
        =>method=="GET"&&path=="/callback"&&!string.IsNullOrEmpty(actualState)&&string.Equals(actualState,expectedState,StringComparison.Ordinal);

    private static void RespondCallbackPage(HttpListenerContext context,bool success,int status)
    {
        var message=success?"授权成功，请返回 MewuAI。 / Authorization complete — return to MewuAI.":"授权尚未完成，请返回 MewuAI 查看状态。 / Authorization is not complete — return to MewuAI.";
        var html=$"<!doctype html><html><head><meta charset=\"utf-8\"><title>MewuAI</title></head><body><p>{message}</p></body></html>";
        var buffer=System.Text.Encoding.UTF8.GetBytes(html);
        context.Response.StatusCode=status;
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
        if(isError)throw new InvalidOperationException(LocalizationService.T("QQ 邮箱工具执行失败，请检查授权和请求。","The QQ Mail tool failed. Check authorization and the request."));
        if(text.Length==0)throw new InvalidOperationException(LocalizationService.T($"QQ 邮箱工具 {tool} 未返回文本内容",$"QQ Mail tool {tool} returned no text"));
        return text;
    }

    /// <summary>调用一个 MCP 工具，返回 (text,isError)。两阶段确认类工具的
    /// 第一阶段响应（含 confirmation_token 的错误体）通过本方法原样返回。</summary>
    internal static async Task<(string Text,bool IsError)> CallToolRawAsync(QqMailMcpToken token,string tool,JsonObject arguments,CancellationToken cancellationToken)
    {
        var result=await CallAsync(token,"tools/call",new JsonObject{["name"]=tool,["arguments"]=DeepCopy(arguments)},cancellationToken).ConfigureAwait(false);
        if(result.ValueKind!=JsonValueKind.Object)throw new InvalidOperationException(LocalizationService.T("QQ 邮箱 MCP 返回了无法解析的工具结果","QQ Mail MCP returned an unreadable tool result"));
        if(result.TryGetProperty("isError",out var errorFlag)&&errorFlag.ValueKind is not (JsonValueKind.True or JsonValueKind.False))throw new InvalidDataException("Invalid MCP isError flag.");
        var isError=errorFlag.ValueKind==JsonValueKind.True;
        return (GetTextContent(result),isError);
    }

    private static async Task<JsonElement> RpcAsync(QqMailMcpToken token,string method,JsonObject? parameters,CancellationToken cancellationToken)
        =>await CallWithClientAsync(NetworkHttpClientFactory.Create(),token,method,parameters,cancellationToken).ConfigureAwait(false);

    internal static async Task<JsonElement> CallWithClientAsync(HttpClient client,QqMailMcpToken token,string method,JsonObject? parameters,CancellationToken cancellationToken)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(CallTimeout);
        var operationDispatched=false;
        try
        {
        var initializeId=Interlocked.Increment(ref _requestId);
        var initialize=new JsonObject
        {
            ["jsonrpc"]="2.0",["id"]=initializeId,["method"]="initialize",
            ["params"]=new JsonObject
            {
                ["protocolVersion"]="2025-11-25",["capabilities"]=new JsonObject(),
                ["clientInfo"]=new JsonObject{["name"]="MewuAI",["version"]=typeof(QqMailMcpService).Assembly.GetName().Version?.ToString()??"0.0.0"}
            }
        };
        using var initializeRequest=CreateRpcRequest(token,initialize,null,null);
        using var initializeResponse=await client.SendAsync(initializeRequest,HttpCompletionOption.ResponseHeadersRead,timeout.Token).ConfigureAwait(false);
        EnsureHttpSuccess(initializeResponse);
        var negotiated=await ReadRpcResultAsync(initializeResponse,initializeId,timeout.Token).ConfigureAwait(false);
        var version=GetString(negotiated,"protocolVersion");
        if(version is not ("2025-03-26" or "2025-06-18" or "2025-11-25"))throw new InvalidDataException("Unsupported QQ Mail MCP protocol version.");
        if(method.StartsWith("tools/",StringComparison.Ordinal)&&
            !(negotiated.TryGetProperty("capabilities",out var capabilities)&&capabilities.ValueKind==JsonValueKind.Object&&capabilities.TryGetProperty("tools",out var tools)&&tools.ValueKind==JsonValueKind.Object))
            throw new InvalidDataException("QQ Mail MCP did not negotiate tool support.");
        string? session=null;
        if(initializeResponse.Headers.TryGetValues("Mcp-Session-Id",out var sessions))
        {
            var values=sessions.ToArray();
            if(values.Length!=1||values[0].Length is 0 or >256||values[0].Any(ch=>ch<0x21||ch>0x7e))throw new InvalidDataException("Invalid QQ Mail MCP session header.");
            session=values[0];
        }
        using var readyRequest=CreateRpcRequest(token,new JsonObject{["jsonrpc"]="2.0",["method"]="notifications/initialized"},version,session);
        using var readyResponse=await client.SendAsync(readyRequest,HttpCompletionOption.ResponseHeadersRead,timeout.Token).ConfigureAwait(false);
        EnsureHttpSuccess(readyResponse);
        if(readyResponse.StatusCode!=HttpStatusCode.Accepted)throw new InvalidDataException("QQ Mail MCP did not accept initialization.");
        var requestId=Interlocked.Increment(ref _requestId);
        var payload=new JsonObject
        {
            ["jsonrpc"]="2.0",
            ["id"]=requestId,
            ["method"]=method
        };
        if(parameters is not null)payload["params"]=DeepCopy(parameters);
        using var request=CreateRpcRequest(token,payload,version,session);
        timeout.Token.ThrowIfCancellationRequested();
        operationDispatched=true;
        using var response=await client.SendAsync(request,HttpCompletionOption.ResponseHeadersRead,timeout.Token).ConfigureAwait(false);
        if((int)response.StatusCode>=500)
            throw new HttpRequestException($"QQ Mail MCP request failed (HTTP {(int)response.StatusCode}).",null,response.StatusCode);
        EnsureHttpSuccess(response);
        return await ReadRpcResultAsync(response,requestId,timeout.Token).ConfigureAwait(false);
        }
        catch(Exception ex)when(!operationDispatched&&ex is OperationCanceledException or IOException or InvalidDataException or HttpRequestException)
        {
            throw new InvalidOperationException(LocalizationService.T("QQ 邮箱连接未完成，发送请求尚未开始。","The QQ Mail connection did not complete; the send request has not started."));
        }
    }

    private static HttpRequestMessage CreateRpcRequest(QqMailMcpToken token,JsonObject payload,string? version,string? session)
    {
        var request=new HttpRequestMessage(HttpMethod.Post,Endpoint)
        {
            Content=new StringContent(payload.ToJsonString(),System.Text.Encoding.UTF8,"application/json")
        };
        request.Headers.TryAddWithoutValidation("Accept","application/json, text/event-stream");
        request.Headers.Authorization=new System.Net.Http.Headers.AuthenticationHeaderValue("Bearer",token.AccessToken);
        if(version is not null)request.Headers.Add("MCP-Protocol-Version",version);
        if(session is not null)request.Headers.Add("Mcp-Session-Id",session);
        return request;
    }

    private static void EnsureHttpSuccess(HttpResponseMessage response)
    {
        if((int)response.StatusCode==401)throw new InvalidOperationException(LocalizationService.T("QQ 邮箱令牌已失效，请在设置中重新扫码授权","The QQ Mail token has expired. Scan the QR code again in Settings."));
        if(!response.IsSuccessStatusCode)throw new InvalidOperationException(LocalizationService.T($"QQ 邮箱 MCP 请求失败（HTTP {(int)response.StatusCode}）",$"QQ Mail MCP request failed (HTTP {(int)response.StatusCode})"));
    }

    internal static JsonElement ParseRpcResult(string body,int requestId)
    {
        using var document=ParseResponseJson(body);
        var root=document.RootElement;
        if(root.ValueKind!=JsonValueKind.Object||GetString(root,"jsonrpc")!="2.0"||
            !root.TryGetProperty("id",out var id)||id.ValueKind!=JsonValueKind.Number||!id.TryGetInt32(out var receivedId)||receivedId!=requestId)
            throw new InvalidDataException("QQ Mail MCP response does not match the request.");
        var hasError=root.TryGetProperty("error",out var error);
        var hasResult=root.TryGetProperty("result",out var result);
        if(hasError==hasResult)throw new InvalidDataException("Invalid QQ Mail MCP response envelope.");
        if(hasError)
        {
            var code=GetInt(error,"code");
            throw new InvalidOperationException(LocalizationService.T($"QQ 邮箱 MCP 请求失败（代码 {code?.ToString()??"unknown"}）。",$"QQ Mail MCP request failed (code {code?.ToString()??"unknown"})."));
        }
        return result.Clone();
    }

    /// <summary>Read one correlated JSON-RPC response, not an entire persistent SSE stream.</summary>
    internal static async Task<JsonElement> ReadRpcResultAsync(HttpResponseMessage response,int requestId,CancellationToken cancellationToken)
    {
        var contentType=response.Content.Headers.ContentType?.MediaType??"application/json";
        using var stream=await response.Content.ReadAsStreamAsync(cancellationToken).ConfigureAwait(false);
        using var reader=new StreamReader(stream,System.Text.Encoding.UTF8);
        const int maxCharacters=4*1024*1024;
        if(!contentType.Equals("text/event-stream",StringComparison.OrdinalIgnoreCase))
        {
            var body=new System.Text.StringBuilder();var buffer=new char[4096];int count;
            while((count=await reader.ReadAsync(buffer.AsMemory(),cancellationToken).ConfigureAwait(false))>0)
            {
                if(body.Length+count>maxCharacters)throw new InvalidDataException("QQ Mail MCP response is too large.");
                body.Append(buffer,0,count);
            }
            return ParseRpcResult(body.ToString(),requestId);
        }
        var builder=new System.Text.StringBuilder();
        var total=0;
        while(await reader.ReadLineAsync(cancellationToken).ConfigureAwait(false) is { } line)
        {
            total+=line.Length;
            if(total>maxCharacters)throw new InvalidDataException("QQ Mail MCP response is too large.");
            if(line.Length==0)
            {
                if(TryReadEvent(builder,requestId,out var result))return result;
                builder.Clear();continue;
            }
            if(!line.StartsWith("data:",StringComparison.Ordinal))continue;
            var data=line.AsSpan(5);if(data.Length>0&&data[0]==' ')data=data[1..];
            if(builder.Length>0)builder.Append('\n');
            builder.Append(data);
        }
        if(TryReadEvent(builder,requestId,out var finalResult))return finalResult;
        throw new InvalidDataException("QQ Mail MCP stream ended without the requested response.");
    }

    private static bool TryReadEvent(System.Text.StringBuilder data,int requestId,out JsonElement result)
    {
        result=default;
        if(data.Length==0)return false;
        using var document=ParseResponseJson(data.ToString());var root=document.RootElement;
        if(root.ValueKind!=JsonValueKind.Object||GetString(root,"jsonrpc")!="2.0")throw new InvalidDataException("Invalid QQ Mail MCP event.");
        if(!root.TryGetProperty("id",out _)&&GetString(root,"method") is not null)return false;
        result=ParseRpcResult(data.ToString(),requestId);return true;
    }

    internal static string GetTextContent(JsonElement result)
    {
        if(!result.TryGetProperty("content",out var content)||content.ValueKind!=JsonValueKind.Array)return string.Empty;
        var parts=new List<string>();
        foreach(var item in content.EnumerateArray())
        {
            if(item.ValueKind!=JsonValueKind.Object)continue;
            var type=GetString(item,"type");
            if(!string.Equals(type,"text",StringComparison.OrdinalIgnoreCase))continue;
            if(GetString(item,"text") is { } text)parts.Add(text);
        }
        return string.Join("\n",parts);
    }

    private static JsonObject DeepCopy(JsonObject source)=>JsonNode.Parse(source.ToJsonString())!.AsObject();

    private static JsonDocument ParseResponseJson(string text)
    {
        try{return JsonDocument.Parse(text);}
        catch(JsonException){throw new InvalidDataException(LocalizationService.T("QQ 邮箱返回了无法解析的响应。","QQ Mail returned an unreadable response."));}
    }

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
