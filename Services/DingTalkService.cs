// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json.Nodes;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;

/// <summary>
/// 钉钉图片分享：走企业内部应用（开放平台 oapi.dingtalk.com）。
/// 用户创建应用后填入 AppKey/AppSecret/AgentId；AppSecret 经 CredentialService（DPAPI）保存。
/// 流程：gettoken 换 access_token → media/upload 传图得 media_id →
/// 工作通知 asyncsend_v2 把图片推送给 userid_list 指定的联系人。
/// </summary>
internal static class DingTalkService
{
    internal const string CredentialId="dingtalk-app-secret";
    private static readonly TimeSpan Timeout=TimeSpan.FromSeconds(40);

    internal static bool IsConfigured(AppSettings settings)
        =>settings.DingTalkEnabled
          &&!string.IsNullOrWhiteSpace(settings.DingTalkAppKey)
          &&!string.IsNullOrWhiteSpace(settings.DingTalkTargetUsers)
          &&new CredentialService().Read(CredentialId) is {Length:>0};

    internal static void SaveSecret(string secret)
    {
        if(string.IsNullOrWhiteSpace(secret))throw new InvalidOperationException(LocalizationService.IsEnglish?"App Secret cannot be empty.":"App Secret 不能为空。");
        new CredentialService().Save(CredentialId,secret.Trim());
    }

    internal static void ClearSecret()=>new CredentialService().Save(CredentialId,string.Empty);

    /// <summary>换取 access_token（约 2 小时有效；调用频率低，不做缓存）。</summary>
    private static async Task<string> GetAccessTokenAsync(AppSettings settings,CancellationToken cancellationToken)
    {
        var secret=new CredentialService().Read(CredentialId);
        if(string.IsNullOrWhiteSpace(secret))throw new InvalidOperationException(LocalizationService.T("尚未保存钉钉 App Secret。","No DingTalk App Secret saved yet."));
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        var url=$"https://oapi.dingtalk.com/gettoken?appkey={Uri.EscapeDataString(settings.DingTalkAppKey.Trim())}&appsecret={Uri.EscapeDataString(secret)}";
        using var client=NetworkHttpClientFactory.Create();
        using var response=await client.GetAsync(url,timeout.Token).ConfigureAwait(false);
        var body=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
        var payload=JsonNode.Parse(body)?.AsObject();
        var errcode=payload?["errcode"]?.GetValue<int>()??-1;
        if(errcode!=0)
        {
            var errmsg=payload?["errmsg"]?.GetValue<string>()??body;
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("钉钉获取 access_token 失败（{0}）。请核对 AppKey/AppSecret。","DingTalk access_token request failed ({0}). Check the AppKey/AppSecret."),
                errmsg));
        }
        return payload?["access_token"]?.GetValue<string>()??throw new InvalidOperationException(LocalizationService.T("钉钉响应缺少 access_token。","DingTalk response has no access_token."));
    }

    /// <summary>验证凭据：能否换取 access_token。</summary>
    internal static async Task TestConnectionAsync(AppSettings settings,CancellationToken cancellationToken)
    {
        if(string.IsNullOrWhiteSpace(settings.DingTalkAppKey))throw new InvalidOperationException(LocalizationService.T("请先填写 AppKey。","Enter the AppKey first."));
        await GetAccessTokenAsync(settings,cancellationToken).ConfigureAwait(false);
    }

    /// <summary>把 PNG 截图发送给设置中指定的钉钉联系人（工作通知）。</summary>
    internal static async Task<string> SendImageAsync(AppSettings settings,byte[] png,CancellationToken cancellationToken)
    {
        if(!IsConfigured(settings))
            throw new InvalidOperationException(LocalizationService.T("钉钉未启用或配置不完整。请到 设置 → MCP → 钉钉 填写应用信息并保存 App Secret。","DingTalk is not enabled or incomplete. Fill in the app details under Settings → MCP → DingTalk."));
        if(string.IsNullOrWhiteSpace(settings.DingTalkAgentId))
            throw new InvalidOperationException(LocalizationService.T("尚未填写 AgentId（用于工作通知）。","AgentId is missing (needed for work notifications)."));
        var token=await GetAccessTokenAsync(settings,cancellationToken).ConfigureAwait(false);
        var mediaId=await UploadImageAsync(token,png,cancellationToken).ConfigureAwait(false);
        return await SendWorkNotificationAsync(settings,token,mediaId,cancellationToken).ConfigureAwait(false);
    }

    private static async Task<string> UploadImageAsync(string accessToken,byte[] png,CancellationToken cancellationToken)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        using var form=new MultipartFormDataContent();
        var image=new ByteArrayContent(png);
        image.Headers.ContentType=new MediaTypeHeaderValue("image/png");
        form.Add(image,"media","mewu_ai.png");
        using var client=NetworkHttpClientFactory.Create();
        using var response=await client.PostAsync($"https://oapi.dingtalk.com/media/upload?access_token={Uri.EscapeDataString(accessToken)}&type=image",form,timeout.Token).ConfigureAwait(false);
        var body=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
        var payload=JsonNode.Parse(body)?.AsObject();
        var errcode=payload?["errcode"]?.GetValue<int>()??-1;
        if(errcode!=0)
        {
            var errmsg=payload?["errmsg"]?.GetValue<string>()??body;
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("钉钉上传图片失败（{0}）。","DingTalk image upload failed ({0})."),errmsg));
        }
        return payload?["media_id"]?.GetValue<string>()??throw new InvalidOperationException(LocalizationService.T("钉钉响应缺少 media_id。","DingTalk response has no media_id."));
    }

    private static async Task<string> SendWorkNotificationAsync(AppSettings settings,string accessToken,string mediaId,CancellationToken cancellationToken)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        var users=settings.DingTalkTargetUsers.Split(',',';','，','；','|',' ')
            .Select(value=>value.Trim()).Where(value=>value.Length>0).Distinct().ToArray();
        if(users.Length==0)throw new InvalidOperationException(LocalizationService.T("尚未填写接收人 userid。","No recipient userids configured."));
        var payload=new JsonObject
        {
            ["agent_id"]=settings.DingTalkAgentId.Trim(),
            ["userid_list"]=string.Join("|",users),
            ["to_all_user"]=false,
            ["msg"]=new JsonObject
            {
                ["msgtype"]="image",
                ["image"]=new JsonObject{["media_id"]=mediaId}
            }
        };
        using var client=NetworkHttpClientFactory.Create();
        using var content=new StringContent(payload.ToJsonString(),Encoding.UTF8,"application/json");
        using var response=await client.PostAsync($"https://oapi.dingtalk.com/topapi/message/corp/asyncsend_v2?access_token={Uri.EscapeDataString(accessToken)}",content,timeout.Token).ConfigureAwait(false);
        var body=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
        var result=JsonNode.Parse(body)?.AsObject();
        var errcode=result?["errcode"]?.GetValue<int>()??-1;
        if(errcode!=0)
        {
            var errmsg=result?["errmsg"]?.GetValue<string>()??body;
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("钉钉发送失败（{0}）。","DingTalk send failed ({0})."),errmsg));
        }
        return LocalizationService.T($"已通过钉钉工作通知发送给 {users.Length} 位联系人。",$"Sent to {users.Length} DingTalk contact(s) via work notification.");
    }
}
