// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
namespace mewu_ai_Assistant.Services;

/// <summary>模型生成的邮件草稿（从回答中的 mewu-mail-send 标记块解析）。</summary>
/// <param name="To">收件人列表（至少一个）。</param>
/// <param name="Cc">抄送（可为空）。</param>
/// <param name="Subject">主题。</param>
/// <param name="Body">正文（纯文本）。</param>
internal sealed record QqMailDraft(IReadOnlyList<(string Email,string? Name)> To,IReadOnlyList<(string Email,string? Name)> Cc,string Subject,string Body);

/// <summary>
/// QQ 邮箱发件流程：模型在回答里输出 ```mewu-mail-send``` 标记块表示“需要发送这封邮件”，
/// 应用解析出草稿后调用 MCP SendMessage 工具，并遵循服务端的两阶段确认协议——
/// 第一次调用返回 confirmation_token 与 operation_summary（不会发送），经用户在
/// 确认对话框中明确同意后带令牌重试才真正发出。应用自身绝不代用户静默发送。
/// </summary>
internal static partial class QqMailSendService
{
    private static readonly TimeSpan SendTimeout=TimeSpan.FromSeconds(30);

    [GeneratedRegex(@"```mewu-mail-send[ \t]*\r?\n(.*?)\r?\n?```",RegexOptions.Singleline|RegexOptions.CultureInvariant)]
    private static partial Regex MailSendBlockRegex();

    /// <summary>判断回答是否包含发件标记块。</summary>
    internal static bool ContainsMarker(string answer)=>MailSendBlockRegex().IsMatch(answer??string.Empty);

    /// <summary>解析回答中的发件标记块。<paramref name="cleanedAnswer"/> 为剔除标记块后的回答文本。</summary>
    internal static bool TryExtract(string answer,out QqMailDraft draft,out string cleanedAnswer)
    {
        draft=null!;
        cleanedAnswer=answer??string.Empty;
        var match=MailSendBlockRegex().Match(cleanedAnswer);
        if(!match.Success)return false;
        try
        {
            var payload=JsonNode.Parse(match.Groups[1].Value)?.AsObject();
            if(payload is null)return false;
            var to=ParseAddresses(payload["to"]);
            var cc=ParseAddresses(payload["cc"]);
            var subject=payload["subject"]?.GetValue<string>()??string.Empty;
            var body=payload["body"]?.GetValue<string>()??string.Empty;
            if(to.Count==0||subject.Trim().Length==0||body.Trim().Length==0)return false;
            draft=new QqMailDraft(to,cc,subject.Trim(),body.Trim());
            cleanedAnswer=CleanAround(cleanedAnswer.Remove(match.Index,match.Length));
            return true;
        }
        catch(Exception ex)
        {
            new PrivacyLogger().Info("QqMailDraftParse",ex.GetType().Name);
            return false;
        }
    }

    /// <summary>执行两阶段确认发送。<paramref name="confirm"/> 在 UI 线程弹出确认
    /// 对话框（入参为给用户看的摘要），返回 true 表示用户同意。返回给调用方
    /// 一段可直接展示的中文/英文结果文案。收件人为网易后缀时改走网易 SMTP
    /// 通道（必须配置授权码——网易没有官方 OAuth-like 发件通道，网页扫码
    /// 临时会话拿到的 sid 经实测可被服务端沙箱化：mbox:compose 返回 S_OK
    /// 但“已发送”箱查不到副本，因此只保留扫码会话作为读上下文，不用于发件）。</summary>
    internal static async Task<string> DeliverAsync(QqMailDraft draft,Models.AppSettings? settings,Func<string,Task<bool>> confirm,CancellationToken cancellationToken,bool forceNetEase=false)
    {
        // 通道路由：网易后缀优先网易 SMTP 通道（必须配置授权码）；QQ 未授权
        // 但网易 SMTP 已配置时同样走网易通道。网页扫码会话仅用于读邮件上下文。
        var netEaseReady=settings is not null&&NetEaseMailService.IsConfigured(settings);
        var netEaseRecipient=draft.To.Count>0&&NetEaseMailService.IsNetEaseAddress(draft.To[0].Email);
        var token=netEaseReady&&netEaseRecipient?null:await QqMailMcpService.ResolveTokenAsync(cancellationToken).ConfigureAwait(false);
        if(forceNetEase&&netEaseReady)
            return await DeliverViaNetEaseAsync(settings!,draft,confirm,cancellationToken).ConfigureAwait(false);
        if(netEaseReady&&(netEaseRecipient||token is null))
            return await DeliverViaNetEaseAsync(settings!,draft,confirm,cancellationToken).ConfigureAwait(false);
        token??=await QqMailMcpService.ResolveTokenAsync(cancellationToken).ConfigureAwait(false);
        if(token is null)
        {
            if(NetEaseMailWebMcpService.HasSession()&&NetEaseMailService.IsNetEaseAddress(draft.To[0].Email))
                return T("已扫码授权网易邮箱，但扫码会话仅用于读取邮件上下文；要实际发送必须填写 SMTP 授权码。请在 设置 → MCP → 网易邮箱 填写账号和授权码（网页端 设置 → POP3/SMTP/IMAP 生成）。",
                    "NetEase Mail is QR-authorized only for reading context. To actually send, configure the SMTP authorization code under Settings → MCP → NetEase Mail (generate one in the web client under Settings → POP3/SMTP/IMAP).");
            return T("QQ 邮箱尚未扫码授权，无法发送邮件。请先在 设置 → MCP → QQ 邮箱 扫码授权。","QQ Mail is not authorized yet. Authorize it under Settings → MCP → QQ Mail first.");
        }
        if(!token.Scope.Contains("mail:send",StringComparison.OrdinalIgnoreCase))
            return T("当前 QQ 邮箱授权缺少 mail:send 权限，无法发送。","The current QQ Mail authorization lacks the mail:send scope.");

        var arguments=BuildArguments(draft,null);
        // 第一阶段：不带确认令牌，服务端只返回待确认摘要，不会发送。
        var (phase1,isError)=await QqMailMcpService.CallToolRawAsync(token,"SendMessage",arguments,cancellationToken).ConfigureAwait(false);
        string? confirmationToken=null;
        string summary;
        try
        {
            var payload=JsonNode.Parse(phase1)?.AsObject();
            var details=payload?["error"]?["details"]?.AsObject();
            confirmationToken=details?["confirmation_token"]?.GetValue<string>();
            var operation=details?["operation_summary"]?.AsObject();
            summary=BuildUserSummary(draft,operation);
        }
        catch(JsonException)
        {
            summary=BuildUserSummary(draft,null);
        }
        if(confirmationToken is null)
        {
            // 没有确认令牌：要么服务端明确拒绝，要么异常地直接完成了发送。
            return isError
                ?T($"QQ 邮箱拒绝发送：{Trim(phase1)}",$"QQ Mail refused the send: {Trim(phase1)}")
                :T("邮件已发送。","The email was sent.");
        }

        // 用户确认是唯一放行条件；取消则放弃，令牌自然作废。
        var approved=await confirm(summary).ConfigureAwait(false);
        if(!approved)return T("已取消发送，邮件未发出。","Send canceled; nothing was delivered.");

        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(SendTimeout);
        var (phase2,phase2Error)=await QqMailMcpService.CallToolRawAsync(token,"SendMessage",BuildArguments(draft,confirmationToken),timeout.Token).ConfigureAwait(false);
        if(phase2Error)return T($"QQ 邮箱发送失败：{Trim(phase2)}",$"QQ Mail send failed: {Trim(phase2)}");
        var recipients=string.Join("、",draft.To.Select(entry=>entry.Email));
        return T($"邮件已通过 QQ 邮箱发送给 {recipients}（主题：{draft.Subject}）。",$"The email was sent via QQ Mail to {recipients} (subject: {draft.Subject}).");
    }

    /// <summary>网易通道：仅 SMTP 授权码（强制）。网易没有稳定的 OAuth-like 发件
    /// 通道；网页扫码拿到的 webmail 临时会话经实测会被服务端沙箱化（mbox:compose
    /// 返回 S_OK 但“已发送”箱没有副本），因此扫码授权仅供读邮件上下文使用，
    /// 不能代发邮件。SMTP 授权码在网易网页端 设置 → POP3/SMTP/IMAP 生成后
    /// 填入设置即可（DPAPI 加密保存）。</summary>
    private static async Task<string> DeliverViaNetEaseAsync(Models.AppSettings settings,QqMailDraft draft,Func<string,Task<bool>> confirm,CancellationToken cancellationToken)
    {
        if(!NetEaseMailService.IsConfigured(settings))
            return T("网易邮箱发件必须填写 SMTP 授权码（网页端 设置 → POP3/SMTP/IMAP 生成）。扫码授权仅用于读取邮件上下文，不能代发邮件。",
                "NetEase Mail sending requires the SMTP authorization code (generated under web Settings → POP3/SMTP/IMAP). The QR-code authorization only enables context fetching, not sending.");
        var from=settings.NetEaseMailAccount;
        var recipients=string.Join("、",draft.To.Select(entry=>entry.Email));
        var cc=string.Join("、",draft.Cc.Select(entry=>entry.Email));
        var summary=T(
            $"发件人：{from}\n收件人：{recipients}{(cc.Length>0?$"\n抄送：{cc}":string.Empty)}\n主题：{draft.Subject}\n\n{draft.Body}",
            $"From: {from}\nTo: {recipients}{(cc.Length>0?$"\nCC: {cc}":string.Empty)}\nSubject: {draft.Subject}\n\n{draft.Body}");
        var approved=await confirm(summary).ConfigureAwait(false);
        if(!approved)return T("已取消发送，邮件未发出。","Send canceled; nothing was delivered.");
        await NetEaseMailService.SendAsync(settings,draft.To.Select(entry=>entry.Email).ToArray(),draft.Cc.Select(entry=>entry.Email).ToArray(),draft.Subject,draft.Body,cancellationToken).ConfigureAwait(false);
        return T($"邮件已通过网易邮箱（SMTP 授权码）发送给 {recipients}（主题：{draft.Subject}）。",$"The email was sent via NetEase Mail (SMTP authorization code) to {recipients} (subject: {draft.Subject}).");
    }

    private static JsonObject BuildArguments(QqMailDraft draft,string? confirmationToken)
    {
        var arguments=new JsonObject
        {
            ["to"]=ToJsonArray(draft.To),
            ["subject"]=draft.Subject,
            ["body"]=draft.Body,
            ["body_format"]="text"
        };
        if(draft.Cc.Count>0)arguments["cc"]=ToJsonArray(draft.Cc);
        if(confirmationToken is not null)arguments["confirmation_token"]=confirmationToken;
        return arguments;
    }

    private static JsonArray ToJsonArray(IReadOnlyList<(string Email,string? Name)> addresses)
    {
        var array=new JsonArray();
        foreach(var (email,name) in addresses)
            array.Add(new JsonObject{["email"]=email,["name"]=name??email});
        return array;
    }

    private static string BuildUserSummary(QqMailDraft draft,JsonObject? operation)
    {
        // 优先使用服务端返回的 operation_summary（含真实发件账号），失败再回退本地草稿。
        var from=operation?["from"]?.GetValue<string>()??T("已授权的 QQ 邮箱账号","your authorized QQ Mail account");
        var recipients=string.Join("、",draft.To.Select(entry=>entry.Email));
        var cc=string.Join("、",draft.Cc.Select(entry=>entry.Email));
        return T(
            $"发件人：{from}\n收件人：{recipients}{(cc.Length>0?$"\n抄送：{cc}":string.Empty)}\n主题：{draft.Subject}\n\n{draft.Body}",
            $"From: {from}\nTo: {recipients}{(cc.Length>0?$"\nCC: {cc}":string.Empty)}\nSubject: {draft.Subject}\n\n{draft.Body}");
    }

    private static IReadOnlyList<(string Email,string? Name)> ParseAddresses(JsonNode? node)
    {
        var result=new List<(string,string?)>();
        if(node is not JsonArray array)return result;
        foreach(var item in array)
        {
            var email=item?["email"]?.GetValue<string>();
            if(string.IsNullOrWhiteSpace(email))continue;
            // 地址必须能通过实体校验（防止模型编造或注入额外目标），
            // 并以实体识别的规范化结果为准（去掉首尾标点等）。
            var canonical=ScreenEntityRecognitionService.Extract(email).FirstOrDefault(entity=>entity.Type==ScreenEntityType.Email)?.Value;
            if(canonical is null)continue;
            result.Add((canonical,item?["name"]?.GetValue<string>()));
        }
        return result;
    }

    /// <summary>去掉标记块后清理多余的空行与残留的“草稿”措辞。</summary>
    private static string CleanAround(string value)
    {
        value=value.Replace("\n\n\n","\n\n",StringComparison.Ordinal);
        return value.TrimEnd();
    }

    private static string Trim(string value)
    {
        var normalized=value.Replace('\r',' ').Replace('\n',' ').Trim();
        return normalized.Length<=220?normalized:normalized[..220]+"…";
    }

    private static string T(string zh,string en)=>LocalizationService.T(zh,en);
}
