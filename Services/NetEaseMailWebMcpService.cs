// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Net;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
namespace mewu_ai_Assistant.Services;

/// <summary>网易邮箱网页版扫码登录会话（sid + Cookie），经 DPAPI 保存在本机。</summary>
/// <param name="Account">授权账号（可能为“（未知）”占位，网页端未直接回传时）。</param>
/// <param name="Sid">webmail 会话 ID（js6/main.jsp?sid=…）。</param>
/// <param name="Cookies">mail.163.com 域会话 Cookie（名→值）。</param>
/// <param name="ObtainedAt">会话获取时间。</param>
internal sealed partial record NetEaseWebSession(string Account,string Sid,IReadOnlyDictionary<string,string> Cookies,DateTimeOffset ObtainedAt)
{
    [GeneratedRegex(@"sid=([A-Za-z0-9]+)")]
    internal static partial Regex SidRegex();

    /// <summary>从任意网页响应文本/跳转地址中提取 sid。</summary>
    internal static string? ExtractSid(string text)
    {
        if(string.IsNullOrEmpty(text))return null;
        var match=SidRegex().Match(text);
        return match.Success?match.Groups[1].Value:null;
    }
}

/// <summary>
/// 网易邮箱（163/126 等）网页版“扫码登录”客户端。网易没有官方 MCP/开放 API，
/// 社区项目（如 NetEaseEmailConnector/netease-mail-mcp）全部依赖 IMAP/SMTP 授权码，
/// 不满足“仅二维码、不依赖 IMAP/SMTP”的接入要求，故这里直接复刻 mail.163.com
/// 登录页（mailscanlogin-1.3.2.js）的二维码登录协议：
///   1. GET  scanlogin.mail.163.com/proxy/getqrcodeid        → 二维码 uuid
///   2. 二维码内容 https://reg.163.com/qr.do?u=…&amp;p=mail163（本地渲染，用“网易邮箱大师”App 扫）
///   3. GET  …/proxy/ngxqrcodeauthstatus（轮询 408 等待/409 已扫/200 已确认/404 过期）
///   4. GET  …/proxy/qrcodeauth                                → 登录票据 ticket
///   5. POST reg.163.com/services/ticketlogin                  → webmail 会话 sid + Cookie
/// 授权后用 js6 网页接口（func=mbox:listMessages / func=mbox:compose&action=deliver）
/// 提供收件箱读取与发件能力，等价于 QQ 邮箱 MCP 的上下文/发件通道。
/// 会话仅保存在本机（CredentialService/DPAPI），不经过任何第三方。
/// 步骤 1–3 已实测可达；步骤 4–6 需用户真实扫码才能触发，代码按防御式解析实现。
/// </summary>
internal static partial class NetEaseMailWebMcpService
{
    internal const string CredentialId="netease-mail-web-session";
    private const string Product="mail163";
    private const string ScanBase="https://scanlogin.mail.163.com";
    private const string QrLandingBase="https://reg.163.com/qr.do";
    private const string TicketLoginUrl="https://reg.163.com/services/ticketlogin";
    // 官方 mailscanlogin-1.3.2.js：_loginMasterURL=rt+"/ma/ticketlogin"，rt 为 scanlogin 基址。
    private const string MasterTicketLoginUrl="https://scanlogin.mail.163.com/ma/ticketlogin";
    // 官方 accountType==2（邮箱大师账号）异步登录入口。
    private const string MaAsyncLoginUrl="https://dashi.163.com/account/ma/login";
    private const string ErrorUrl="https://mail.163.com/errorpage/error163.htm";

    /// <summary>分步记录扫码授权流程（不含票据/会话明文），便于诊断“已确认却登录失败”。</summary>
    private static void Log(string step,string detail)=>new PrivacyLogger().Info("NetEaseScan",$"{step}: {detail}");
    private static readonly TimeSpan AuthorizationTimeout=TimeSpan.FromMinutes(5);
    private static readonly TimeSpan CallTimeout=TimeSpan.FromSeconds(25);
    private static readonly TimeSpan PollInterval=TimeSpan.FromSeconds(2);
    private static readonly JsonSerializerOptions JsonOptions=new(){PropertyNameCaseInsensitive=true};

    private static string UserAgent=>"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

    #region 会话存取

    internal static NetEaseWebSession? ReadSession()
    {
        try
        {
            var stored=new CredentialService().Read(CredentialId);
            if(string.IsNullOrWhiteSpace(stored))return null;
            var session=JsonSerializer.Deserialize<SessionDto>(stored,JsonOptions);
            if(session is null||string.IsNullOrWhiteSpace(session.Sid))return null;
            var cookies=session.Cookies??new Dictionary<string,string>();
            var account=session.Account;
            // 旧缓存无账号（“（未知）”）时，从 P_INFO Cookie 回填真实邮箱账号（仅内存）。
            if(string.IsNullOrWhiteSpace(account)||!account.Contains('@'))
            {
                var derived=AccountFromCookies(cookies);
                if(derived.Contains('@'))account=derived;
            }
            return new NetEaseWebSession(
                string.IsNullOrWhiteSpace(account)||!account.Contains('@')?LocalizationService.IsEnglish?"(unknown)":"（未知）":account,
                session.Sid!,cookies,session.ObtainedAt==default?DateTimeOffset.UtcNow:session.ObtainedAt);
        }
        catch{return null;}
    }

    internal static void CacheSession(NetEaseWebSession session)
        =>new CredentialService().Save(CredentialId,JsonSerializer.Serialize(new SessionDto
        {
            Account=session.Account,
            Sid=session.Sid,
            Cookies=new Dictionary<string,string>(session.Cookies),
            ObtainedAt=session.ObtainedAt
        },JsonOptions));

    internal static void ClearSession()
    {
        try{new CredentialService().Save(CredentialId,string.Empty);}
        catch(Exception ex){new PrivacyLogger().Info("NetEaseWebSessionClear",ex.GetType().Name);}
    }

    /// <summary>本机存在可用的网页会话（不含网络验证）。发件/上下文路由据此选择通道。</summary>
    internal static bool HasSession()=>ReadSession() is not null;

    private sealed class SessionDto
    {
        public string? Account{get;set;}
        public string? Sid{get;set;}
        public Dictionary<string,string>? Cookies{get;set;}
        public DateTimeOffset ObtainedAt{get;set;}
    }

    #endregion

    #region 扫码授权

    /// <summary>执行完整扫码授权。<paramref name="showQr"/> 收到本地渲染的二维码 PNG
    /// 字节（由 UI 展示）；<paramref name="report"/> 接收阶段性状态文本。二维码内容为
    /// 网易官方扫码落地页，需用手机“网易邮箱大师”App 扫描并在手机上确认登录。</summary>
    internal static async Task<NetEaseWebSession> AuthorizeAsync(
        Func<byte[],Task> showQr,
        Action<string> report,
        CancellationToken cancellationToken)
    {
        using var http=CreateHttpClient();
        report(LocalizationService.T("正在获取二维码…","Fetching the QR code…"));
        var uuid=await GetQrcodeIdAsync(http,cancellationToken).ConfigureAwait(false);

        var qrUrl=$"{QrLandingBase}?u={uuid}&p={Product}";
        var png=RenderQrPng(qrUrl);
        await showQr(png).ConfigureAwait(false);
        // 官方 mailscanlogin-1.3.2.js 的二维码内容为 reg.163.com/qr.do?u=…&p=mail163，
        // 邮箱大师 App 的扫码器只拦截该地址并弹出授权确认；相机/微信/浏览器打开
        // 只会看到“立即安装邮箱大师”的下载页，因此文案必须点名 App 内扫码。
        report(LocalizationService.T(
            "请打开手机「网易邮箱大师」App，用 App 内的“扫一扫”扫描二维码并在手机上确认登录（5 分钟内有效）。注意：用相机/微信扫码只会打开下载页，无法授权。",
            "Open the NetEase Mailmaster app on your phone, scan with its built-in scanner, then confirm (valid for 5 minutes). Camera/WeChat scanners only open a download page and cannot authorize."));

        // 轮询扫码状态：408 等待 / 409 已扫待确认 / 200 已确认 / 404 过期。
        // 官方协议：status 响应还带 accountType（1=邮箱账号走 qrcodeauth→ticketlogin，
        // 2=邮箱大师账号走 dashi 异步登录）。
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(AuthorizationTimeout);
        var confirmed=false;
        var accountType=1;
        var hintShown=false;
        var startedAt=Environment.TickCount64;
        try
        {
            while(!timeout.IsCancellationRequested)
            {
                await Task.Delay(PollInterval,timeout.Token).ConfigureAwait(false);
                var poll=await PollStatusAsync(http,uuid,timeout.Token).ConfigureAwait(false);
                switch(poll.Status)
                {
                    case "408":
                        // 长时间无响应多为“用相机/微信扫了码”。给一次针对性提示。
                        if(!hintShown&&Environment.TickCount64-startedAt>25_000)
                        {
                            hintShown=true;
                            report(LocalizationService.T(
                                "仍未检测到扫码。请确认是用「网易邮箱大师」App 内的“扫一扫”在扫（相机/微信扫此码只会跳到下载页）。",
                                "Still no scan detected. Make sure you are scanning with the built-in scanner inside the NetEase Mailmaster app; the camera or WeChat only opens a download page."));
                        }
                        break;
                    case "409":
                        report(LocalizationService.T("已扫码，请在手机上点击“确认登录”。","Scanned; tap “Confirm login” on the phone."));
                        break;
                    case "200":
                        confirmed=true;
                        accountType=poll.AccountType;
                        Log("PollConfirmed",$"accountType={accountType}");
                        break;
                    case "404":
                        throw new InvalidOperationException(LocalizationService.T("二维码已过期，请重新发起扫码授权。","The QR code expired; start the scan again."));
                    default:
                        throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                            LocalizationService.T("扫码状态异常（{0}），请重试。","Unexpected scan status ({0}); retry."),poll.Status));
                }
                if(confirmed)break;
            }
        }
        catch(OperationCanceledException)when(!cancellationToken.IsCancellationRequested)
        {
            // 轮询超时（非用户取消）：落到下方统一文案。
        }
        if(!confirmed)throw new OperationCanceledException(LocalizationService.T("扫码授权超时未确认。","The scan authorization timed out."));

        NetEaseWebSession session;
        if(accountType==2)
        {
            // 邮箱大师账号：POST dashi.163.com/account/ma/login {uuid, session:true}。
            report(LocalizationService.T("已确认（邮箱大师账号），正在换取邮箱会话…","Confirmed (Mailmaster account); exchanging for a mailbox session…"));
            try{session=await MasterLoginAsync(http,uuid,cancellationToken).ConfigureAwait(false);}
            catch(Exception error) when(error is InvalidOperationException or HttpRequestException or JsonException)
            {
                // Some Mailmaster builds report accountType=2 while the public
                // qrcodeauth/ticketlogin flow is the only route that returns a
                // webmail sid. Retry that documented route before failing.
                Log("MasterFallback",error.GetType().Name);
                var ticket=await GetTicketAsync(http,uuid,cancellationToken).ConfigureAwait(false);
                session=await ExchangeSessionAsync(http,uuid,ticket,cancellationToken).ConfigureAwait(false);
            }
        }
        else
        {
            var ticket=await GetTicketAsync(http,uuid,timeout.Token).ConfigureAwait(false);
            report(LocalizationService.T("已确认，正在换取邮箱会话…","Confirmed; exchanging for a mailbox session…"));
            session=await ExchangeSessionAsync(http,uuid,ticket,cancellationToken).ConfigureAwait(false);
        }
        // 旧缓存可能没有账号（“（未知）”）：从 P_INFO Cookie 回填真实邮箱账号。
        if(!session.Account.Contains('@')&&AccountFromCookies(session.Cookies).Contains('@'))
            session=new NetEaseWebSession(AccountFromCookies(session.Cookies),session.Sid,session.Cookies,session.ObtainedAt);
        CacheSession(session);
        return session;
    }

    private sealed record PollResult(string Status,int AccountType);

    private static async Task<PollResult> PollStatusAsync(HttpClient http,string uuid,CancellationToken cancellationToken)
    {
        var url=$"{ScanBase}/proxy/ngxqrcodeauthstatus?uuid={uuid}&product={Product}";
        using var request=NewRequest(HttpMethod.Get,url);
        var text=await ReadBodyAsync(http,request,cancellationToken).ConfigureAwait(false);
        var result=JsonNode.Parse(text)?["result"]?.AsObject();
        var status=result?["qrcodeStatus"]?.ToString()??string.Empty;
        var accountType=1;
        if(int.TryParse(result?["accountType"]?.ToString(),out var parsed)&&(parsed==1||parsed==2))
            accountType=parsed;
        return new PollResult(status,accountType);
    }

    private static async Task<string> GetQrcodeIdAsync(HttpClient http,CancellationToken cancellationToken)
    {
        var url=$"{ScanBase}/proxy/getqrcodeid?product={Product}&usage=0&deviceId=&clientVersion=";
        using var request=NewRequest(HttpMethod.Get,url);
        var text=await ReadBodyAsync(http,request,cancellationToken).ConfigureAwait(false);
        var uuid=JsonNode.Parse(text)?["result"]?["l"]?["i"]?.GetValue<string>();
        if(string.IsNullOrWhiteSpace(uuid))
            throw new InvalidOperationException(LocalizationService.T("网易扫码服务未返回二维码，请稍后重试。","The NetEase scan service returned no QR code; retry later."));
        return uuid!;
    }

    /// <summary>确认后取登录票据（官方 getAuth：GET qrcodeauth?uuid&product，带 deviceId 头）。
    /// 返回 (ticket, type, domain, 可选账号)。</summary>
    private static async Task<TicketResult> GetTicketAsync(HttpClient http,string uuid,CancellationToken cancellationToken)
    {
        var url=$"{ScanBase}/proxy/qrcodeauth?uuid={uuid}&product={Product}";
        using var request=NewRequest(HttpMethod.Get,url);
        request.Headers.TryAddWithoutValidation("deviceId",string.Empty);
        var text=await ReadBodyAsync(http,request,cancellationToken).ConfigureAwait(false);
        var root=JsonNode.Parse(text)?.AsObject();
        var result=root?["result"]?.AsObject();
        var code=root?["code"]?.ToString();
        var retCode=result?["retCode"]?.ToString();
        var ticket=result?["ticket"]?.GetValue<string>();
        var ticketType=result?["type"]?.ToString();
        var ticketDomain=result?["domain"]?.ToString();
        Log("GetTicket",$"code={code} retCode={retCode} type={ticketType} domain={ticketDomain} hasTicket={!string.IsNullOrWhiteSpace(ticket)}");
        if(string.IsNullOrWhiteSpace(ticket))
            throw new InvalidOperationException(LocalizationService.T("扫码确认后未能取得登录票据，请重试。","No login ticket after confirming the scan; retry."));
        return new TicketResult(
            ticket!,
            result?["type"]?.GetValue<string>()??string.Empty,
            result?["domain"]?.GetValue<string>()??string.Empty,
            result?["account"]?.GetValue<string>()??result?["username"]?.GetValue<string>()??result?["email"]?.GetValue<string>());
    }

    private sealed record TicketResult(string Ticket,string Type,string Domain,string? Account);

    /// <summary>邮箱大师账号（accountType==2）路径：POST dashi.163.com/account/ma/login
    /// （官方 asyncLogin：param={uuid,session:true}，头带 deviceId/masterfp）。
    /// 响应 result 中应含跳转地址（或设置会话 Cookie），跟随至取得 sid。</summary>
    private static async Task<NetEaseWebSession> MasterLoginAsync(HttpClient http,string uuid,CancellationToken cancellationToken)
    {
        var cookies=new Dictionary<string,string>(StringComparer.OrdinalIgnoreCase);
        string? redirect=null;
        using(var request=NewRequest(HttpMethod.Post,MaAsyncLoginUrl))
        {
            request.Headers.TryAddWithoutValidation("deviceId",string.Empty);
            request.Headers.TryAddWithoutValidation("masterfp",string.Empty);
            request.Content=new FormUrlEncodedContent(new Dictionary<string,string>{{"uuid",uuid},{"session","true"}});
            using var response=await http.SendAsync(request,HttpCompletionOption.ResponseHeadersRead,cancellationToken).ConfigureAwait(false);
            CollectCookies(response,cookies);
            var body=await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false);
            redirect=response.Headers.Location?.ToString();
            var root=TryParseJson(body);
            if(root is not null)
            {
                var maCode=root["code"]?.ToString();
                Log("MaAsyncLogin",$"http={(int)response.StatusCode} code={maCode} fields=[{string.Join(",",root.Select(pair=>pair.Key))}]");
                var result=root["result"]?.AsObject();
                redirect??=result?["url"]?.GetValue<string>()??result?["redirect"]?.GetValue<string>()??result?["location"]?.GetValue<string>();
            }
            else Log("MaAsyncLogin",$"http={(int)response.StatusCode} nonJson body={TruncateForLog(body)}");
        }
        var sid=await FollowForSidAsync(http,redirect,cookies,cancellationToken).ConfigureAwait(false)
            ??throw new InvalidOperationException(LocalizationService.T(
                "已确认（邮箱大师账号）但未能取得邮箱会话。请重试；若持续失败请改用 SMTP 授权码方式。",
                "Confirmed (Mailmaster account) but no mailbox session was returned. Retry, or fall back to the SMTP authorization code."));
        var account=AccountFromCookies(cookies);
        return new NetEaseWebSession(account,sid,cookies,DateTimeOffset.UtcNow);
    }

    /// <summary>P_INFO Cookie 形如 “账号|时间戳|…”：从中提取邮箱账号，取不到返回“（未知）”。</summary>
    private static string AccountFromCookies(IReadOnlyDictionary<string,string> cookies)
    {
        if(cookies.TryGetValue("P_INFO",out var info))
        {
            var account=info.Split('|')[0];
            if(account.Contains('@'))return account;
        }
        return LocalizationService.IsEnglish?"(unknown)":"（未知）";
    }

    private static JsonObject? TryParseJson(string text)
    {
        try{return string.IsNullOrWhiteSpace(text)?null:JsonNode.Parse(text)?.AsObject();}
        catch(JsonException){return null;}
    }

    private static string TruncateForLog(string? text)
    {
        if(string.IsNullOrEmpty(text))return string.Empty;
        var t=text.Replace("\r"," ").Replace("\n"," ");
        return t.Length>400?t[..400]+"…":t;
    }

    /// <summary>用票据换 webmail 会话：POST ticketlogin（noRedirect=1），收集 Cookie 并
    /// 从响应/跳转中提取 sid。入口 URL 必须带 uuid（官方 submitLogin 拼法），且
    /// ticketlogin 下发的 Cookie 必须随入口请求带回，否则拿不到 sid。</summary>
    private static async Task<NetEaseWebSession> ExchangeSessionAsync(HttpClient http,string uuid,TicketResult ticket,CancellationToken cancellationToken)
    {
        var isMaster=ticket.Type=="master";
        var loginUrl=isMaster?MasterTicketLoginUrl:TicketLoginUrl;
        var entryHost=HostForDomain(ticket.Domain);
        // 官方：entry + "?df=mailmaster_mail_<product>&uuid=…&style=13&allssl=true&from=web&t=…&type=…"
        var entryUrl=$"https://{entryHost}/entry/login.jsp?df=mailmaster_mail_{Product}&uuid={Uri.EscapeDataString(uuid)}&style=13&allssl=true&from=web&t={DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}";
        if(!string.IsNullOrEmpty(ticket.Type))entryUrl+="&type="+Uri.EscapeDataString(ticket.Type);
        if(isMaster)entryUrl+="&verifyMasterToken=1";

        var cookies=new Dictionary<string,string>(StringComparer.OrdinalIgnoreCase);
        var form=new Dictionary<string,string>
        {
            ["product"]=Product,
            ["domains"]=DomainsFor(ticket.Domain),
            ["noRedirect"]="1",
            ["url"]=entryUrl,
            ["url2"]=ErrorUrlFor(ticket.Domain),
            ["ticket"]=ticket.Ticket
        };
        using var request=NewRequest(HttpMethod.Post,loginUrl);
        request.Content=new FormUrlEncodedContent(form);
        using var response=await http.SendAsync(request,HttpCompletionOption.ResponseHeadersRead,cancellationToken).ConfigureAwait(false);
        CollectCookies(response,cookies);

        // noRedirect=1：服务端一般直接在响应体中给出入口地址；仍可能 302 跳转。
        var body=await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false);
        Log("TicketLogin",$"http={(int)response.StatusCode} cookies=[{string.Join(",",cookies.Keys)}] body={TruncateForLog(body)}");
        var sid=NetEaseWebSession.ExtractSid(body)??NetEaseWebSession.ExtractSid(response.Headers.Location?.ToString()??string.Empty);

        // 未拿到 sid：跟随入口（Location 或响应体中的地址），并带回 ticketlogin 下发的 Cookie。
        if(string.IsNullOrEmpty(sid))
        {
            var follow=response.Headers.Location?.ToString()??ExtractRedirect(body)??entryUrl;
            if(!string.IsNullOrWhiteSpace(follow))
            {
                sid=await FollowForSidAsync(http,follow,cookies,cancellationToken).ConfigureAwait(false);
            }
        }
        if(string.IsNullOrEmpty(sid))
            throw new InvalidOperationException(LocalizationService.T("已扫码但未能取得邮箱会话（sid）。请重试或改用下方 SMTP 授权码方式。","Scanned, but no mailbox session (sid) was returned. Retry or fall back to the SMTP authorization code below."));

        var account=ticket.Account??ExtractAccount(body)??AccountFromCookies(cookies);
        if(account.Length==0)account=AccountFromCookies(cookies);
        return new NetEaseWebSession(account,sid!,cookies,DateTimeOffset.UtcNow);
    }

    /// <summary>带 Cookie 跟随跳转地址找 sid；过程中继续收集 Set-Cookie。</summary>
    private static async Task<string?> FollowForSidAsync(HttpClient http,string? url,IDictionary<string,string> cookies,CancellationToken cancellationToken)
    {
        for(var hop=0;hop<4&&!string.IsNullOrWhiteSpace(url);hop++)
        {
            if(!url.StartsWith("http",StringComparison.OrdinalIgnoreCase))
                url="https://mail.163.com/"+url.TrimStart('/');
            using var request=NewRequest(HttpMethod.Get,url);
            if(cookies.Count>0)
                request.Headers.TryAddWithoutValidation("Cookie",string.Join("; ",cookies.Select(pair=>$"{pair.Key}={pair.Value}")));
            using var response=await http.SendAsync(request,HttpCompletionOption.ResponseHeadersRead,cancellationToken).ConfigureAwait(false);
            CollectCookies(response,cookies);
            var sid=NetEaseWebSession.ExtractSid(url)??NetEaseWebSession.ExtractSid(response.Headers.Location?.ToString()??string.Empty);
            if(!string.IsNullOrEmpty(sid))
            {
                Log("FollowSid",$"hop={hop} http={(int)response.StatusCode} sidFound=true url={MaskSid(url)}");
                return sid;
            }
            string? body=null;
            if(response.Headers.Location is null&&response.Content.Headers.ContentType?.MediaType=="text/html")
            {
                body=await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false);
                sid=NetEaseWebSession.ExtractSid(body);
                if(!string.IsNullOrEmpty(sid))
                {
                    Log("FollowSid",$"hop={hop} http={(int)response.StatusCode} sidFound=true(body)");
                    return sid;
                }
            }
            url=response.Headers.Location?.ToString()??ExtractRedirect(body??string.Empty);
            Log("FollowSid",$"hop={hop} http={(int)response.StatusCode} next={MaskSid(url)}");
        }
        return null;
    }

    private static string MaskSid(string? url)
    {
        if(string.IsNullOrEmpty(url))return string.Empty;
        return NetEaseWebSession.SidRegex().Replace(url,"sid=***");
    }

    [GeneratedRegex(@"(?:top\.location\.href|location\.replace)\s*[=(]\s*[""']([^""']+)[""']")]
    private static partial Regex RedirectRegex();

    /// <summary>官方 _urls 映射：各域名登录失败回退页。</summary>
    private static string ErrorUrlFor(string domain)=>domain.ToLowerInvariant() switch
    {
        "@126.com"=>"https://mail.126.com/errorpage/error126.htm",
        "@yeah.net"=>"https://mail.yeah.net/errorpage/err_yeah.htm",
        "@vip.163.com"=>"https://mail.163.com/errorpage/error163.htm",
        "@188.com"=>"https://mail.188.com/errorpage/error188.htm",
        _=>ErrorUrl
    };

    [GeneratedRegex(@"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")]
    private static partial Regex EmailRegex();

    private static string? ExtractRedirect(string body)
    {
        if(string.IsNullOrEmpty(body))return null;
        var match=RedirectRegex().Match(body);
        return match.Success?match.Groups[1].Value:null;
    }

    private static string? ExtractAccount(string body)
    {
        if(string.IsNullOrEmpty(body))return null;
        var match=EmailRegex().Match(body);
        return match.Success?match.Value:null;
    }

    /// <summary>二维码内容仅在本地渲染（QRCoder PNG），不经过任何第三方二维码服务。</summary>
    internal static byte[] RenderQrPng(string content)
    {
        using var generator=new QRCoder.QRCodeGenerator();
        using var data=generator.CreateQrCode(content,QRCoder.QRCodeGenerator.ECCLevel.M);
        using var qr=new QRCoder.PngByteQRCode(data);
        return qr.GetGraphic(8);
    }

    private static string HostForDomain(string domain)=>domain.ToLowerInvariant() switch
    {
        "@126.com" or "@vip.126.com"=>"mail.126.com",
        "@yeah.net"=>"mail.yeah.net",
        "@188.com"=>"mail.188.com",
        "@vip.163.com"=>"vip.mail.163.com",
        _=>"mail.163.com"
    };

    private static string DomainsFor(string domain)=>domain.ToLowerInvariant() switch
    {
        "@126.com"=>"126.com",
        "@vip.126.com"=>".vip.126.com",
        "@vip.163.com"=>".vip.163.com",
        "@188.com"=>".188.com",
        "@yeah.net"=>"yeah.net",
        _=>string.Empty
    };

    #endregion

    #region 网页版邮件接口（js6 网关）

    /// <summary>验证会话：拉取 1 封收件箱邮件（不发信、不修改已读状态）。</summary>
    internal static async Task<NetEaseWebSession> ValidateSessionAsync(NetEaseWebSession session,CancellationToken cancellationToken)
    {
        var list=await ListInboxAsync(session,1,cancellationToken).ConfigureAwait(false);
        if(list is null)
            throw new InvalidOperationException(LocalizationService.T("网易邮箱会话已失效，请重新扫码授权。","The NetEase Mail session expired; scan the QR code again."));
        return session;
    }

    /// <summary>读取收件箱最近邮件（func=mbox:listMessages）。2026 版 js6 网关要求
    /// var= 为 XML 负载（官方 bjs obj2Str 序列化：fids 数组 + order=date），且
    /// Accept: application/json 时响应为 XML（&lt;result&gt;…&lt;/result&gt;）。
    /// 返回消息对象数组（from/subject/receivedDate 等字段已统一为 JSON 属性）；
    /// 会话失效（需重新登录）时返回 null。</summary>
    internal static async Task<JsonArray?> ListInboxAsync(NetEaseWebSession session,int pageSize,CancellationToken cancellationToken)
    {
        var limit=Math.Clamp(pageSize,1,50);
        var xml=new StringBuilder("<?xml version=\"1.0\"?><object>")
            .Append("<array name=\"fids\"><int>1</int></array>")
            .Append("<string name=\"order\">date</string>")
            .Append("<boolean name=\"desc\">true</boolean>")
            .Append("<boolean name=\"skipLockedFolders\">true</boolean>")
            .Append("<boolean name=\"returnTotal\">true</boolean>")
            .Append("<int name=\"limit\">").Append(limit).Append("</int>")
            .Append("</object>").ToString();
        var payload=await CallWebmailAsync(session,"func=mbox:listMessages",xml,false,cancellationToken).ConfigureAwait(false);
        if(payload is null)return null;
        return payload["var"] as JsonArray
            ??(payload["result"]??payload["data"])?["var"] as JsonArray
            ??(payload["result"]??payload["data"])?["list"] as JsonArray
            ??(payload["result"]??payload["data"])?["messages"] as JsonArray;
    }

    /// <summary>网页发件的二次验证结果：S_OK 仅表示网易接口接受请求，不代表已投递。
    /// 必须用 ListSentMessagesAsync 查已发送箱确认服务端是否真的写入了副本。</summary>
    internal sealed record NetEaseWebSendOutcome(string Code,bool Accepted,bool VerifiedInSentFolder);

    /// <summary>网页版发件（func=mbox:compose&action=deliver）。S_OK 后必须查
    /// “已发送”箱确认服务端真的写入了副本：二维码拿到的网页临时会话存在沙箱化
    /// 风险（接口接受请求但不写入发件箱），所以单凭 S_OK 不能判定发件成功。
    /// 返回 NetEaseWebSendOutcome；若 VerifiedInSentFolder=false 表示服务端
    /// 没真正写入，建议改用 SMTP 授权码通道。</summary>
    internal static async Task<NetEaseWebSendOutcome> SendAsync(NetEaseWebSession session,IReadOnlyList<string> to,IReadOnlyList<string> cc,string subject,string body,CancellationToken cancellationToken)
    {
        if(to.Count==0)throw new InvalidOperationException(LocalizationService.T("缺少收件人。","No recipients."));
        var account=session.Account.Contains('@')?session.Account:string.Empty;
        var xml=new StringBuilder("<object>");
        xml.Append("<string name=\"id\">c:").Append(DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()).Append(Random.Shared.Next(100,999)).Append("</string>");
        if(account.Length>0)xml.Append("<string name=\"account\">").Append(EscapeXml(account)).Append("</string>");
        xml.Append("<string name=\"to\">{").Append(string.Join(",",to.Select(address=>$"\"{EscapeJson(address)}\":\"\""))).Append("}</string>");
        xml.Append("<string name=\"cc\">{").Append(string.Join(",",cc.Select(address=>$"\"{EscapeJson(address)}\":\"\""))).Append("</string>");
        xml.Append("<string name=\"bcc\">{}</string>");
        xml.Append("<string name=\"subject\">").Append(EscapeXml(subject)).Append("</string>");
        xml.Append("<string name=\"content\">").Append(EscapeXml(body)).Append("</string>");
        xml.Append("<string name=\"priority\">3</string>");
        xml.Append("<string name=\"sendLater\">false</string>");
        xml.Append("<string name=\"isHtml\">false</string>");
        xml.Append("<string name=\"charset\">UTF-8</string>");
        xml.Append("</object>");

        var payload=await CallWebmailAsync(session,"func=mbox:compose&cl_send=2&l=compose&action=deliver",xml.ToString(),true,cancellationToken).ConfigureAwait(false);
        var code=payload?["code"]?.ToString();
        if(!string.Equals(code,"S_OK",StringComparison.OrdinalIgnoreCase))
        {
            var reason=string.IsNullOrEmpty(code)?(LocalizationService.IsEnglish?"no response":"无响应"):code!;
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("网易邮箱网页发件未成功（{0}）。请在设置中改用 SMTP 授权码方式发送。","NetEase web send did not succeed ({0}). Switch to the SMTP authorization code in settings."),
                reason));
        }
        // 二次验证：S_OK 后等 2 秒查已发送箱，避免把“接口已接收”误报成“已发送”。
        // 网页扫码临时会话被服务端沙箱化时常见：mfg=compose 返回 S_OK，但 fid=3
        // （已发送）箱不写入副本，实际未投递。
        var verified=false;
        try
        {
            using var verifyWait=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            verifyWait.CancelAfter(TimeSpan.FromSeconds(4));
            await Task.Delay(TimeSpan.FromSeconds(2),verifyWait.Token).ConfigureAwait(false);
            verified=await VerifyInSentFolderAsync(session,subject,to,cancellationToken).ConfigureAwait(false);
        }
        catch(OperationCanceledException){/* 验证超时不影响 S_OK 的事实 */ }
        catch(Exception ex){new PrivacyLogger().Info("NetEaseScan",$"verify_failed: {ex.GetType().Name}"); }
        if(!verified)
        {
            new PrivacyLogger().Info("NetEaseScan","send_verification_missing");
        }
        return new NetEaseWebSendOutcome(code??"S_OK",true,verified);
    }

    /// <summary>读取已发送箱（fid=3）最近邮件。返回 JsonArray；会话失效返回 null。</summary>
    internal static async Task<JsonArray?> ListSentMessagesAsync(NetEaseWebSession session,int pageSize,CancellationToken cancellationToken)
    {
        var limit=Math.Clamp(pageSize,1,50);
        var xml=new StringBuilder("<?xml version=\"1.0\"?><object>")
            .Append("<array name=\"fids\"><int>3</int></array>")
            .Append("<string name=\"order\">date</string>")
            .Append("<boolean name=\"desc\">true</boolean>")
            .Append("<boolean name=\"skipLockedFolders\">true</boolean>")
            .Append("<boolean name=\"returnTotal\">true</boolean>")
            .Append("<int name=\"limit\">").Append(limit).Append("</int>")
            .Append("</object>").ToString();
        var payload=await CallWebmailAsync(session,"func=mbox:listMessages",xml,false,cancellationToken).ConfigureAwait(false);
        if(payload is null)return null;
        return payload["var"] as JsonArray
            ??(payload["result"]??payload["data"])?["var"] as JsonArray
            ??(payload["result"]??payload["data"])?["list"] as JsonArray
            ??(payload["result"]??payload["data"])?["messages"] as JsonArray;
    }

    /// <summary>二次验证发件结果：在已发送箱（fid=3）查找最近 90 秒内 subject+to
    /// 同时匹配的邮件（xml 中字段名因网易 bjs 序列化而异：to 字段为 JSON 字符串
    /// "{ \"a@x.com\":\"\" }"，subject 为字符串，receivedDate 为 unix 毫秒时间戳）。
    /// 找到返回 true，否则 false。</summary>
    private static async Task<bool> VerifyInSentFolderAsync(NetEaseWebSession session,string subject,IReadOnlyList<string> to,CancellationToken cancellationToken)
    {
        var messages=await ListSentMessagesAsync(session,15,cancellationToken).ConfigureAwait(false);
        if(messages is null||messages.Count==0)return false;
        var nowMs=DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        var primary=to.FirstOrDefault()??string.Empty;
        foreach(var node in messages)
        {
            if(node is not JsonObject msg)continue;
            var msgSubject=ReadString(msg,"subject");
            if(!string.Equals(msgSubject,subject,StringComparison.OrdinalIgnoreCase))continue;
            // 解析 to 字段："{\"a@x.com\":\"\",\"b@y.com\":\"\"}"
            var toField=ReadString(msg,"to");
            var hasRecipient=toField.Contains(primary,StringComparison.OrdinalIgnoreCase)
                ||to.Any(address=>toField.Contains(address,StringComparison.OrdinalIgnoreCase));
            if(!hasRecipient)continue;
            // receivedDate 可能是 unix 毫秒或可解析日期
            long unix=0;
            var raw=msg["receivedDate"]??msg["sentDate"]??msg["date"];
            if(raw is JsonValue v)
            {
                if(v.TryGetValue<long>(out var l))unix=l;
                else if(v.TryGetValue<double>(out var d))unix=(long)d;
                else if(DateTimeOffset.TryParse(v.ToString(),System.Globalization.CultureInfo.InvariantCulture,System.Globalization.DateTimeStyles.AssumeUniversal,out var dt))unix=dt.ToUnixTimeMilliseconds();
            }
            if(unix<=0)return true; // 时间字段缺失时按主题+收件人匹配
            if(Math.Abs(nowMs-unix)<=90_000)return true;
        }
        return false;
    }

    private static string ReadString(JsonObject obj,string name)
    {
        var node=obj[name];
        if(node is null)return string.Empty;
        if(node is JsonValue v)
        {
            try{return v.GetValue<string>()??string.Empty;}catch{return v.ToString();}
        }
        return node.ToString();
    }

    /// <summary>调用 js6 网关：POST var=…（var 为 XML 字符串，官方 bjs obj2Str 序列化）。
    /// Accept: application/json 时 2026 版网关返回 XML（&lt;result&gt;…）；旧版/异常时
    /// 可能返回 “var {json}” 或裸 JSON，均兼容。会话失效（需重新登录）返回 null；
    /// 网关拒绝（FR_/FA_ 错误码）抛出带错误码的异常以便诊断。</summary>
    private static async Task<JsonObject?> CallWebmailAsync(NetEaseWebSession session,string query,string varPayload,bool xmlMode,CancellationToken cancellationToken)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(CallTimeout);
        using var http=CreateHttpClient();
        var url=$"https://mail.163.com/js6/s?sid={Uri.EscapeDataString(session.Sid)}&{query}";
        using var request=NewRequest(HttpMethod.Post,url);
        request.Headers.TryAddWithoutValidation("Accept","application/json");
        request.Headers.TryAddWithoutValidation("Referer",$"https://mail.163.com/js6/main.jsp?sid={Uri.EscapeDataString(session.Sid)}&df=mail163_letter");
        request.Headers.TryAddWithoutValidation("Cookie",BuildCookieHeader(session));
        if(!varPayload.StartsWith("<?xml",StringComparison.OrdinalIgnoreCase))
            varPayload="<?xml version=\"1.0\"?>"+varPayload;
        var body=$"var={Uri.EscapeDataString(varPayload)}";
        request.Content=new StringContent(body,Encoding.UTF8,"application/x-www-form-urlencoded");
        using var response=await http.SendAsync(request,HttpCompletionOption.ResponseHeadersRead,timeout.Token).ConfigureAwait(false);
        if((int)response.StatusCode==401||(int)response.StatusCode==403)
            throw new InvalidOperationException(LocalizationService.T("网易邮箱拒绝访问，会话可能已失效，请重新扫码授权。","NetEase Mail refused the request; the session may have expired. Scan the QR code again."));
        var text=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
        Log("Webmail",$"query={query} http={(int)response.StatusCode} resp={TruncateForLog(text)}");
        if(NeedsRelogin(text))return null;
        var payload=ParseWebmailResponse(text);
        var code=payload?["code"]?.ToString()
            ??payload?["result"]?["code"]?.ToString()
            ??payload?["data"]?["code"]?.ToString();
        Log("WebmailParsed",$"query={query} code={code??"(none)"} payload={(payload is null?"none":"ok")}");
        if(string.Equals(code,"FA_INVALID_SESSION",StringComparison.OrdinalIgnoreCase)||string.Equals(code,"FA_NOT_LOGIN",StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException(LocalizationService.T("网易邮箱二维码网页会话已失效，请重新扫码授权后再发送。","The NetEase QR web session expired. Scan again before sending."));
        if(!string.IsNullOrEmpty(code)&&!string.Equals(code,"S_OK",StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("网易邮箱网页接口返回错误（{0}）。若持续失败请重新扫码授权。","The NetEase web API returned an error ({0}). Scan the QR code again if it keeps failing."),
                code));
        // 网易网页网关的 S_OK 仅表示请求被受理，不是最终投递凭证。
        // 调用方必须把结果展示为“已提交”，不能宣称收件人已收到。
        return payload;
    }

    private static bool NeedsRelogin(string text)
        =>text.Contains("SID_EXPIRED",StringComparison.OrdinalIgnoreCase)
          ||text.Contains("FA_NOT_LOGIN",StringComparison.Ordinal)
          ||text.Contains("FA_INVALID_SESSION",StringComparison.Ordinal)
          ||text.Contains("login.jsp",StringComparison.OrdinalIgnoreCase)
          &&text.Contains("top.location",StringComparison.OrdinalIgnoreCase);

    /// <summary>解析网关响应：优先 XML（&lt;result&gt;…），退回旧版 “var {json};”/裸 JSON。
    /// XML 元素 → JSON 属性：&lt;string name="x"&gt;v&lt;/string&gt; → x:"v"；
    /// int/long → 数字；boolean → 布尔；date → 字符串；array → 数组；object → 对象。</summary>
    private static JsonObject? ParseWebmailResponse(string text)
    {
        if(string.IsNullOrWhiteSpace(text))return null;
        text=text.Trim();
        if(text.StartsWith("<",StringComparison.Ordinal))
        {
            try
            {
                var xml=System.Xml.Linq.XDocument.Parse(text);
                return xml.Root is { } root?XmlElementToJson(root) as JsonObject:null;
            }
            catch(System.Xml.XmlException){return null;}
        }
        return ParseWebmailJson(text);
    }

    /// <summary>把 &lt;result&gt; 的子元素转换为 JSON 对象（带 name 属性的元素作为属性名，
    /// 其余按元素名收进 “items” 数组；根元素本身映射为其子级对象）。</summary>
    private static JsonNode? XmlElementToJson(System.Xml.Linq.XElement element)
    {
        System.Diagnostics.Debug.Assert(element is not null);
        switch(element.Name.LocalName)
        {
            case "string":
            case "date":
                return element.Value;
            case "int":
            case "long":
                return long.TryParse(element.Value,out var number)?number:element.Value;
            case "number":
                return double.TryParse(element.Value,System.Globalization.CultureInfo.InvariantCulture,out var real)?real:element.Value;
            case "boolean":
                return bool.TryParse(element.Value,out var flag)?flag:element.Value;
            case "null":
                return null;
            case "array":
            {
                var array=new JsonArray();
                foreach(var child in element.Elements())array.Add(XmlElementToJson(child));
                return array;
            }
            default:
            {
                // 网关根级叶节点（例如 <code>S_OK</code>）没有 name，
                // 仍应保留文本值，不能转换成空对象。
                if(!element.Elements().Any())return element.Value;
                var obj=new JsonObject();
                foreach(var child in element.Elements())
                {
                    var name=child.Attribute("name")?.Value??child.Name.LocalName;
                    obj[name]=XmlElementToJson(child);
                }
                if(obj.Count==0&&element.Attribute("name") is not null)
                    return element.Value.Length==0?null:element.Value;
                return obj;
            }
        }
    }

    /// <summary>旧版“var {json}”前缀格式兼容（单引号 JS 对象无法解析时返回 null）。</summary>
    private static JsonObject? ParseWebmailJson(string text)
    {
        if(string.IsNullOrWhiteSpace(text))return null;
        text=text.Trim();
        if(text.StartsWith("var ",StringComparison.OrdinalIgnoreCase))text=text[4..].Trim();
        var semi=text.LastIndexOf(';');
        if(semi==text.Length-1)text=text[..^1];
        try{return JsonNode.Parse(text)?.AsObject();}
        catch(JsonException){return null;}
    }

    private static string BuildCookieHeader(NetEaseWebSession session)
        =>string.Join("; ",session.Cookies.Select(pair=>$"{pair.Key}={pair.Value}"));

    #endregion

    #region HTTP 基础设施

    private static HttpClient CreateHttpClient()
    {
        var handler=new HttpClientHandler
        {
            AllowAutoRedirect=false,
            AutomaticDecompression=DecompressionMethods.GZip|DecompressionMethods.Deflate|DecompressionMethods.Brotli,
            UseProxy=false
        };
        return new HttpClient(handler){Timeout=Timeout.InfiniteTimeSpan};
    }

    private static HttpRequestMessage NewRequest(HttpMethod method,string url)
    {
        var request=new HttpRequestMessage(method,url);
        request.Headers.TryAddWithoutValidation("User-Agent",UserAgent);
        request.Headers.TryAddWithoutValidation("Origin","https://mail.163.com");
        request.Headers.TryAddWithoutValidation("Referer","https://mail.163.com/");
        return request;
    }

    /// <summary>发送 GET 请求并读取文本响应（不跟随跳转，由调用方决定）。</summary>
    private static async Task<string> ReadBodyAsync(HttpClient http,HttpRequestMessage request,CancellationToken cancellationToken)
    {
        using var response=await http.SendAsync(request,HttpCompletionOption.ResponseHeadersRead,cancellationToken).ConfigureAwait(false);
        var body=await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false);
        if(!response.IsSuccessStatusCode)
        {
            Log("HttpError",$"http={(int)response.StatusCode} body={TruncateForLog(body)}");
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("网易扫码服务返回 HTTP {0}，请稍后重试。","NetEase scan service returned HTTP {0}; retry later."),(int)response.StatusCode));
        }
        return body;
    }

    /// <summary>收集 Set-Cookie（名=值部分）到共享字典；过期/清空指令则移除。</summary>
    private static void CollectCookies(HttpResponseMessage response,IDictionary<string,string> cookies)
    {
        if(!response.Headers.TryGetValues("Set-Cookie",out var values))return;
        foreach(var raw in values)
        {
            var pair=raw.Split(';',2)[0].Trim();
            var eq=pair.IndexOf('=');
            if(eq<=0)continue;
            var name=pair[..eq].Trim();
            var value=pair[(eq+1)..].Trim();
            if(value.Length==0||value.Equals("EXPIRED",StringComparison.OrdinalIgnoreCase))cookies.Remove(name);
            else cookies[name]=value;
        }
    }

    private static string EscapeXml(string value)
        =>value.Replace("&","&amp;",StringComparison.Ordinal).Replace("<","&lt;",StringComparison.Ordinal).Replace(">","&gt;",StringComparison.Ordinal).Replace("\"","&quot;",StringComparison.Ordinal);

    private static string EscapeJson(string value)
        =>value.Replace("\\","\\\\",StringComparison.Ordinal).Replace("\"","\\\"",StringComparison.Ordinal);

    #endregion
}
