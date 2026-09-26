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

internal enum MailChannel { Auto, Qq, NetEase }
internal sealed record MailDeliveryResult(bool Success,string Message);

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

    // A recipient's domain never chooses the sender's account. Explicit selections
    // fail closed; Auto uses the enabled QQ account, then enabled NetEase SMTP.
    internal static MailChannel? SelectChannel(MailChannel requested,bool qqReady,bool netEaseReady)
        =>requested switch
        {
            MailChannel.Qq=>qqReady?MailChannel.Qq:null,
            MailChannel.NetEase=>netEaseReady?MailChannel.NetEase:null,
            _=>qqReady?MailChannel.Qq:netEaseReady?MailChannel.NetEase:null
        };

    internal static async Task<MailDeliveryResult> DeliverAsync(QqMailDraft draft,Models.AppSettings? settings,
        Func<string,Task<bool>> confirm,CancellationToken cancellationToken,MailChannel channel=MailChannel.Auto)
    {
        cancellationToken.ThrowIfCancellationRequested();
        var token=settings?.QqMailMcpEnabled==true&&channel!=MailChannel.NetEase
            ?await QqMailMcpService.ResolveTokenAsync(cancellationToken).ConfigureAwait(false):null;
        var netEaseReady=settings is not null&&NetEaseMailService.IsConfigured(settings);
        var selected=SelectChannel(channel,token is not null,netEaseReady);
        if(selected is null)
            return new(false,T("所选发件邮箱未启用或未授权。QQ 邮箱需要扫码授权；网易邮箱发送需要账号和 SMTP 授权码，扫码会话只能读取邮件。",
                "The selected sending account is disabled or unauthorized. QQ Mail needs authorization; NetEase sending needs an account and SMTP code. A QR session only reads mail."));
        if(selected==MailChannel.NetEase)
        {
            var from=settings!.NetEaseMailAccount;
            var summary=BuildUserSummary(draft,new JsonObject{["from"]=from});
            if(!await confirm(summary).ConfigureAwait(false))return Canceled();
            cancellationToken.ThrowIfCancellationRequested();
            await NetEaseMailService.SendAsync(settings,draft.To.Select(entry=>entry.Email).ToArray(),draft.Cc.Select(entry=>entry.Email).ToArray(),draft.Subject,draft.Body,cancellationToken).ConfigureAwait(false);
            return Sent("网易邮箱 / NetEase Mail",draft);
        }
        if(!token!.Scope.Split(' ',StringSplitOptions.RemoveEmptyEntries).Contains("mail:send",StringComparer.Ordinal))
            return new(false,T("当前 QQ 邮箱授权缺少 mail:send 权限。","The QQ Mail authorization lacks the mail:send scope."));
        return await DeliverQqAsync(draft,confirm,
            (arguments,ct)=>QqMailMcpService.CallToolRawAsync(token,"SendMessage",arguments,ct),cancellationToken).ConfigureAwait(false);
    }

    internal static async Task<MailDeliveryResult> DeliverQqAsync(QqMailDraft draft,Func<string,Task<bool>> confirm,
        Func<JsonObject,CancellationToken,Task<(string Text,bool IsError)>> send,CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        // Confirm the exact draft BEFORE any potentially mutating remote call.
        if(!await confirm(BuildUserSummary(draft,null)).ConfigureAwait(false))return Canceled();
        cancellationToken.ThrowIfCancellationRequested();
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(SendTimeout);
        var (phase1,isError)=await send(BuildArguments(draft,null),timeout.Token).ConfigureAwait(false);
        string? confirmationToken=null;
        try
        {
            var payload=JsonNode.Parse(phase1) as JsonObject;
            if(payload?["error"] is JsonObject error&&error["details"] is JsonObject details&&
                details["confirmation_token"] is JsonValue value&&value.TryGetValue<string>(out var text))
                confirmationToken=text;
        }
        catch(JsonException){}
        if(string.IsNullOrWhiteSpace(confirmationToken))
            return isError?new(false,T("QQ 邮箱拒绝发送，请检查授权和收件地址。","QQ Mail refused the send. Check authorization and recipients.")):Sent("QQ 邮箱 / QQ Mail",draft);
        timeout.Token.ThrowIfCancellationRequested();
        var (_,phase2Error)=await send(BuildArguments(draft,confirmationToken),timeout.Token).ConfigureAwait(false);
        return phase2Error?new(false,T("QQ 邮箱发送失败，请检查授权和收件地址。","QQ Mail send failed. Check authorization and recipients.")):Sent("QQ 邮箱 / QQ Mail",draft);
    }

    private static MailDeliveryResult Canceled()=>new(false,T("已取消发送，邮件未发出。","Send canceled; nothing was delivered."));
    private static MailDeliveryResult Sent(string account,QqMailDraft draft)=>new(true,
        T($"邮件已通过 {account} 发送给 {string.Join("、",draft.To.Select(entry=>entry.Email))}。",
          $"The email was sent via {account} to {string.Join(", ",draft.To.Select(entry=>entry.Email))}."));

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
