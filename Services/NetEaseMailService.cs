// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Net.Security;
using System.Net.Sockets;
using System.Security.Authentication;
using System.Text;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;

/// <summary>
/// 网易邮箱（163/126/yeah 等）SMTP 代发。用户在网易邮箱网页端开启 SMTP 并生成
/// 授权码后填入设置，授权码经 CredentialService（DPAPI）保存、绝不落入设置文件。
/// 网易仅开放 465 端口隐式 TLS，System.Net.Mail.SmtpClient 不支持该模式，
/// 故此处直接用 SslStream 实现最小 SMTP 对话（EHLO/AUTH LOGIN/MAIL FROM/DATA）。
/// </summary>
internal static class NetEaseMailService
{
    internal const string CredentialId="netease-mail-auth";
    private const int SmtpPort=465;
    private static readonly TimeSpan SendTimeout=TimeSpan.FromSeconds(30);

    /// <summary>网易邮箱地址域（识别屏幕/提示词中的网易后缀以此为准）。</summary>
    internal static bool IsNetEaseAddress(string email)
    {
        var at=email.LastIndexOf('@');
        if(at<0||at+1>=email.Length)return false;
        return email[(at+1)..].ToLowerInvariant() is "163.com" or "126.com" or "yeah.net" or "188.com" or "vip.163.com" or "vip.126.com" or "netease.com";
    }

    private static string HostFor(string account)
        =>account[(account.LastIndexOf('@')+1)..].ToLowerInvariant() switch
        {
            "126.com" or "vip.126.com"=>"smtp.126.com",
            "yeah.net"=>"smtp.yeah.net",
            _=>"smtp.163.com"
        };

    internal static bool IsConfigured(AppSettings settings)
        =>settings.NetEaseMailEnabled
          &&!string.IsNullOrWhiteSpace(settings.NetEaseMailAccount)
          &&settings.NetEaseMailAccount.Contains('@')
          &&!string.IsNullOrWhiteSpace(new CredentialService().Read(CredentialId));

    /// <summary>读取 DPAPI 保存的 SMTP 授权码（仅存在性/读取用途，不外泄）。</summary>
    internal static string? ReadAuthCode()=>new CredentialService().Read(CredentialId) is {Length:>0} code?code:null;

    internal static void SaveAuthCode(string code)
    {
        if(string.IsNullOrWhiteSpace(code))throw new InvalidOperationException(LocalizationService.IsEnglish?"Authorization code cannot be empty.":"授权码不能为空。");
        new CredentialService().Save(CredentialId,code.Trim());
    }

    internal static void ClearAuthCode()=>new CredentialService().Save(CredentialId,string.Empty);

    /// <summary>发送一封纯文本邮件（UTF-8）。收件人列表至少一个地址。</summary>
    internal static async Task SendAsync(AppSettings settings,IReadOnlyList<string> to,IReadOnlyList<string> cc,string subject,string body,CancellationToken cancellationToken)
    {
        if(!IsConfigured(settings))
            throw new InvalidOperationException(LocalizationService.T("网易邮箱未启用或缺少授权码。请到 设置 → MCP → 网易邮箱 填写账号并保存授权码。","NetEase Mail is not enabled or has no authorization code. Set the account and code under Settings → MCP → NetEase Mail."));
        if(to.Count==0)throw new InvalidOperationException(LocalizationService.T("缺少收件人。","No recipients."));
        var account=settings.NetEaseMailAccount.Trim();
        var authCode=new CredentialService().Read(CredentialId);
        if(string.IsNullOrWhiteSpace(authCode))throw new InvalidOperationException(LocalizationService.T("网易邮箱授权码缺失，请重新保存。","NetEase Mail authorization code is missing; save it again."));
        var host=HostFor(account);

        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(SendTimeout);
        using var tcp=new TcpClient();
        try
        {
            await tcp.ConnectAsync(host,SmtpPort,timeout.Token).ConfigureAwait(false);
            using var ssl=new SslStream(tcp.GetStream(),false,(_,_,_,_)=>true);
            await ssl.AuthenticateAsClientAsync(new SslClientAuthenticationOptions{TargetHost=host,EnabledSslProtocols=SslProtocols.Tls12|SslProtocols.Tls13},timeout.Token).ConfigureAwait(false);
            using var writer=new StreamWriter(ssl,new UTF8Encoding(false)){AutoFlush=true,NewLine="\r\n"};
            using var reader=new StreamReader(ssl,Encoding.UTF8);
            await Expect(reader,"220",timeout.Token).ConfigureAwait(false);           // 服务器问候
            await Command(writer,reader,"EHLO mewu_ai","250",timeout.Token).ConfigureAwait(false);
            await Command(writer,reader,"AUTH LOGIN","334",timeout.Token).ConfigureAwait(false);
            await Command(writer,reader,Convert.ToBase64String(Encoding.UTF8.GetBytes(account)),"334",timeout.Token).ConfigureAwait(false);
            await Command(writer,reader,Convert.ToBase64String(Encoding.UTF8.GetBytes(authCode)),"235",timeout.Token).ConfigureAwait(false);
            await Command(writer,reader,$"MAIL FROM:<{account}>","250",timeout.Token).ConfigureAwait(false);
            foreach(var address in to.Concat(cc).Distinct(StringComparer.OrdinalIgnoreCase))
                await Command(writer,reader,$"RCPT TO:<{address}>","25",timeout.Token).ConfigureAwait(false);
            await Command(writer,reader,"DATA","354",timeout.Token).ConfigureAwait(false);
            await writer.WriteAsync(BuildMessage(settings,account,to,cc,subject,body)).ConfigureAwait(false);
            await Command(writer,reader,"\r\n.","250",timeout.Token).ConfigureAwait(false);
            try{await Command(writer,reader,"QUIT","221",timeout.Token).ConfigureAwait(false);}catch(SmtpReplyException){}
        }
        catch(SmtpReplyException ex)
        {
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("网易邮箱发送失败（{0}）。请检查授权码是否有效、SMTP 服务是否已在邮箱设置中开启。","NetEase Mail send failed ({0}). Check that the authorization code is valid and SMTP is enabled in the mailbox settings."),
                ex.Message));
        }
        catch(AuthenticationException)
        {
            throw new InvalidOperationException(LocalizationService.T("无法与网易邮箱服务器建立加密连接。","Could not establish an encrypted connection to the NetEase mail server."));
        }
        catch(SocketException ex)
        {
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,LocalizationService.T("连接网易邮箱服务器失败（{0}）。","Failed to reach the NetEase mail server ({0})."),ex.Message));
        }
    }

    /// <summary>验证授权码：完整走一次握手 + 认证（不发信）。</summary>
    internal static async Task TestConnectionAsync(AppSettings settings,CancellationToken cancellationToken)
    {
        if(string.IsNullOrWhiteSpace(settings.NetEaseMailAccount)||!settings.NetEaseMailAccount.Contains('@'))
            throw new InvalidOperationException(LocalizationService.T("请先填写网易邮箱账号。","Enter the NetEase mail account first."));
        var account=settings.NetEaseMailAccount.Trim();
        var authCode=new CredentialService().Read(CredentialId);
        if(string.IsNullOrWhiteSpace(authCode))throw new InvalidOperationException(LocalizationService.T("尚未保存授权码。","No authorization code saved yet."));
        var host=HostFor(account);
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(SendTimeout);
        using var tcp=new TcpClient();
        await tcp.ConnectAsync(host,SmtpPort,timeout.Token).ConfigureAwait(false);
        using var ssl=new SslStream(tcp.GetStream(),false,(_,_,_,_)=>true);
        await ssl.AuthenticateAsClientAsync(new SslClientAuthenticationOptions{TargetHost=host,EnabledSslProtocols=SslProtocols.Tls12|SslProtocols.Tls13},timeout.Token).ConfigureAwait(false);
        using var writer=new StreamWriter(ssl,new UTF8Encoding(false)){AutoFlush=true,NewLine="\r\n"};
        using var reader=new StreamReader(ssl,Encoding.UTF8);
        await Expect(reader,"220",timeout.Token).ConfigureAwait(false);
        await Command(writer,reader,"EHLO mewu_ai","250",timeout.Token).ConfigureAwait(false);
        await Command(writer,reader,"AUTH LOGIN","334",timeout.Token).ConfigureAwait(false);
        await Command(writer,reader,Convert.ToBase64String(Encoding.UTF8.GetBytes(account)),"334",timeout.Token).ConfigureAwait(false);
        await Command(writer,reader,Convert.ToBase64String(Encoding.UTF8.GetBytes(authCode)),"235",timeout.Token).ConfigureAwait(false);
        try{await Command(writer,reader,"QUIT","221",timeout.Token).ConfigureAwait(false);}catch(SmtpReplyException){}
    }

    private static string BuildMessage(AppSettings settings,string from,IReadOnlyList<string> to,IReadOnlyList<string> cc,string subject,string body)
    {
        var fromName=settings.NetEaseMailFromName.Trim();
        var fromHeader=string.IsNullOrEmpty(fromName)?from:$"{EncodeHeader(fromName)} <{from}>";
        var builder=new StringBuilder();
        builder.Append("From:").Append(fromHeader).Append("\r\n");
        builder.Append("To:").Append(string.Join(",",to)).Append("\r\n");
        if(cc.Count>0)builder.Append("Cc:").Append(string.Join(",",cc)).Append("\r\n");
        builder.Append("Subject:").Append(EncodeHeader(subject)).Append("\r\n");
        builder.Append("Date:").Append(DateTimeOffset.Now.ToString("R",System.Globalization.CultureInfo.InvariantCulture)).Append("\r\n");
        builder.Append("MIME-Version:1.0\r\n");
        builder.Append("Content-Type:text/plain;charset=utf-8\r\n");
        builder.Append("Content-Transfer-Encoding:base64\r\n");
        builder.Append("\r\n");
        var raw=Encoding.UTF8.GetBytes(body.Replace("\r\n","\n",StringComparison.Ordinal).Replace("\n","\r\n",StringComparison.Ordinal));
        var base64=Convert.ToBase64String(raw);
        for(var offset=0;offset<base64.Length;offset+=76)
            builder.Append(base64,offset,Math.Min(76,base64.Length-offset)).Append("\r\n");
        return builder.ToString();
    }

    private static string EncodeHeader(string value)
    {
        var ascii=value.All(ch=>ch>=32&&ch<=126);
        return ascii?value:$"=?utf-8?B?{Convert.ToBase64String(Encoding.UTF8.GetBytes(value))}?=";
    }

    private static async Task Command(StreamWriter writer,StreamReader reader,string command,string expect,CancellationToken cancellationToken)
    {
        await writer.WriteLineAsync(command.AsMemory(),cancellationToken).ConfigureAwait(false);
        await Expect(reader,expect,cancellationToken).ConfigureAwait(false);
    }

    /// <summary>读取（可能多行的）SMTP 应答并校验前三位数字。</summary>
    private static async Task Expect(StreamReader reader,string expectPrefix,CancellationToken cancellationToken)
    {
        string last=string.Empty;
        while(true)
        {
            var line=await reader.ReadLineAsync(cancellationToken).ConfigureAwait(false);
            if(line is null)throw new SmtpReplyException(LocalizationService.IsEnglish?"connection closed by server":"服务器关闭了连接");
            last=line;
            // 形如 "250-..." 表示还有续行；"250 ..." 为最后一行。
            if(line.Length<4||line[3]!='-')break;
        }
        if(!last.StartsWith(expectPrefix,StringComparison.Ordinal))
            throw new SmtpReplyException(last.Length<=200?last:last[..200]);
    }

    private sealed class SmtpReplyException(string message):Exception(message);
}
