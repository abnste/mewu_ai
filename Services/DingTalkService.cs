// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json.Nodes;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;

/// <summary>Internal DingTalk app: token → uploaded media → asynchronous work notification.</summary>
internal static class DingTalkService
{
    internal const string CredentialId="dingtalk-app-secret";
    private static readonly TimeSpan Timeout=TimeSpan.FromSeconds(40);
    private const string Service="DingTalk";
    internal static bool IsConfigured(AppSettings settings)
        =>settings.DingTalkEnabled && !string.IsNullOrWhiteSpace(settings.DingTalkAppKey)
          && !string.IsNullOrWhiteSpace(settings.DingTalkTargetUsers)
          && new CredentialService().Read(CredentialId) is {Length:>0};
    internal static void SaveSecret(string secret)
    {
        if(string.IsNullOrWhiteSpace(secret))throw new InvalidOperationException(LocalizationService.T("App Secret 不能为空。","App Secret cannot be empty."));
        new CredentialService().Save(CredentialId,secret.Trim());
    }
    internal static void ClearSecret()=>new CredentialService().Save(CredentialId,string.Empty);
    // No shared token state: each explicit operation obtains its own credential snapshot.
    private static async Task<string> GetAccessTokenAsync(AppSettings settings,HttpClient client,string secret,CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        if(string.IsNullOrWhiteSpace(settings.DingTalkAppKey)||string.IsNullOrWhiteSpace(secret))
            throw new InvalidOperationException(LocalizationService.T("请填写钉钉 AppKey 并保存 App Secret。","Enter the DingTalk AppKey and save the App Secret."));
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        var url=$"https://oapi.dingtalk.com/gettoken?appkey={Uri.EscapeDataString(settings.DingTalkAppKey.Trim())}&appsecret={Uri.EscapeDataString(secret)}";
        using var response=await client.GetAsync(url,timeout.Token).ConfigureAwait(false);
        var payload=await McpHttpResponse.ReadAsync(response,Service,"errcode",timeout.Token).ConfigureAwait(false);
        return McpHttpResponse.RequiredString(payload,"access_token",Service);
    }
    internal static Task TestConnectionAsync(AppSettings settings,CancellationToken cancellationToken)
        =>TestConnectionAsync(settings,NetworkHttpClientFactory.Create(),ReadSecret(cancellationToken),cancellationToken);
    internal static async Task TestConnectionAsync(AppSettings settings,HttpClient client,string secret,CancellationToken cancellationToken)
        =>_ = await GetAccessTokenAsync(settings,client,secret,cancellationToken).ConfigureAwait(false);
    internal static Task<string> SendImageAsync(AppSettings settings,byte[] png,CancellationToken cancellationToken)
        =>SendImageAsync(settings,png,NetworkHttpClientFactory.Create(),ReadSecret(cancellationToken),cancellationToken);
    internal static async Task<string> SendImageAsync(AppSettings settings,byte[] png,HttpClient client,string secret,CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        if(!settings.DingTalkEnabled)throw new InvalidOperationException(LocalizationService.T("钉钉未启用。","DingTalk is not enabled."));
        if(!long.TryParse(settings.DingTalkAgentId.Trim(),NumberStyles.None,CultureInfo.InvariantCulture,out var agentId)||agentId<=0)
            throw new InvalidOperationException(LocalizationService.T("AgentId 必须是正整数。","AgentId must be a positive integer."));
        var users=settings.DingTalkTargetUsers.Split([',',';','，','；','|',' ','\r','\n','\t'],StringSplitOptions.TrimEntries|StringSplitOptions.RemoveEmptyEntries).Distinct(StringComparer.Ordinal).ToArray();
        if(users.Length==0||users.Length>5000)throw new InvalidOperationException(LocalizationService.T("请填写 1 至 5000 位接收人 userid。","Specify between 1 and 5000 recipient userids."));
        if(png.Length==0)throw new InvalidOperationException(LocalizationService.T("图片不能为空。","The image cannot be empty."));
        var token=await GetAccessTokenAsync(settings,client,secret,cancellationToken).ConfigureAwait(false);
        var mediaId=await UploadImageAsync(client,token,png,cancellationToken).ConfigureAwait(false);
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        var payload=new JsonObject
        {
            ["agent_id"]=agentId,["userid_list"]=string.Join(",",users),["to_all_user"]=false,
            ["msg"]=new JsonObject{["msgtype"]="image",["image"]=new JsonObject{["media_id"]=mediaId}}
        };
        using var content=new StringContent(payload.ToJsonString(),Encoding.UTF8,"application/json");
        using var response=await client.PostAsync($"https://oapi.dingtalk.com/topapi/message/corpconversation/asyncsend_v2?access_token={Uri.EscapeDataString(token)}",content,timeout.Token).ConfigureAwait(false);
        var result=await McpHttpResponse.ReadAsync(response,Service,"errcode",timeout.Token).ConfigureAwait(false);
        if(result["task_id"] is not JsonValue task || !task.TryGetValue<long>(out var taskId) || taskId<=0)throw McpHttpResponse.Invalid(Service);
        return LocalizationService.T($"已提交钉钉工作通知，接收人 {users.Length} 位；最终投递结果由钉钉处理。",$"DingTalk accepted a work notification for {users.Length} contact(s); final delivery is handled by DingTalk.");
    }
    private static async Task<string> UploadImageAsync(HttpClient client,string accessToken,byte[] png,CancellationToken cancellationToken)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        using var form=new MultipartFormDataContent();
        var image=new ByteArrayContent(png);
        image.Headers.ContentType=new MediaTypeHeaderValue("image/png");
        form.Add(image,"media","mewu_ai.png");
        using var response=await client.PostAsync($"https://oapi.dingtalk.com/media/upload?access_token={Uri.EscapeDataString(accessToken)}&type=image",form,timeout.Token).ConfigureAwait(false);
        var payload=await McpHttpResponse.ReadAsync(response,Service,"errcode",timeout.Token).ConfigureAwait(false);
        return McpHttpResponse.RequiredString(payload,"media_id",Service);
    }
    private static string ReadSecret(CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        return new CredentialService().Read(CredentialId)??string.Empty;
    }
}
