// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json.Nodes;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;

/// <summary>Self-built Feishu app: tenant token → image_key → image message.</summary>
internal static class FeishuService
{
    internal const string CredentialId="feishu-app-secret";
    private static readonly TimeSpan Timeout=TimeSpan.FromSeconds(40);
    private const string BaseUrl="https://open.feishu.cn";
    private const string Service="Feishu";
    internal static bool IsConfigured(AppSettings settings)
        =>settings.FeishuEnabled && !string.IsNullOrWhiteSpace(settings.FeishuAppId)
          && !string.IsNullOrWhiteSpace(settings.FeishuTargetId)
          && new CredentialService().Read(CredentialId) is {Length:>0};
    internal static void SaveSecret(string secret)
    {
        if(string.IsNullOrWhiteSpace(secret))throw new InvalidOperationException(LocalizationService.T("App Secret 不能为空。","App Secret cannot be empty."));
        new CredentialService().Save(CredentialId,secret.Trim());
    }
    internal static void ClearSecret()=>new CredentialService().Save(CredentialId,string.Empty);
    internal static Task<string> GetTenantTokenAsync(AppSettings settings,CancellationToken cancellationToken)
        =>GetTenantTokenAsync(settings,NetworkHttpClientFactory.Create(),ReadSecret(cancellationToken),cancellationToken);
    // Borrowed client, request-scoped credentials: concurrent accounts never share token state.
    internal static async Task<string> GetTenantTokenAsync(AppSettings settings,HttpClient client,string secret,CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        if(string.IsNullOrWhiteSpace(settings.FeishuAppId)||string.IsNullOrWhiteSpace(secret))
            throw new InvalidOperationException(LocalizationService.T("请填写飞书 AppId 并保存 App Secret。","Enter the Feishu AppId and save the App Secret."));
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        var payload=new JsonObject{["app_id"]=settings.FeishuAppId.Trim(),["app_secret"]=secret};
        using var content=new StringContent(payload.ToJsonString(),Encoding.UTF8,"application/json");
        using var response=await client.PostAsync($"{BaseUrl}/open-apis/auth/v3/tenant_access_token/internal",content,timeout.Token).ConfigureAwait(false);
        var result=await McpHttpResponse.ReadAsync(response,Service,"code",timeout.Token).ConfigureAwait(false);
        return McpHttpResponse.RequiredString(result,"tenant_access_token",Service);
    }
    internal static Task<IReadOnlyList<(string ChatId,string Name)>> ListChatsAsync(AppSettings settings,CancellationToken cancellationToken)
        =>ListChatsAsync(settings,NetworkHttpClientFactory.Create(),ReadSecret(cancellationToken),cancellationToken);
    internal static async Task<IReadOnlyList<(string ChatId,string Name)>> ListChatsAsync(AppSettings settings,HttpClient client,string secret,CancellationToken cancellationToken)
    {
        var token=await GetTenantTokenAsync(settings,client,secret,cancellationToken).ConfigureAwait(false);
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        var chats=new List<(string,string)>();
        var ids=new HashSet<string>(StringComparer.Ordinal);
        var seenPages=new HashSet<string>(StringComparer.Ordinal);
        var pageToken=string.Empty;
        for(var page=0;page<100;page++)
        {
            using var request=new HttpRequestMessage(HttpMethod.Get,$"{BaseUrl}/open-apis/im/v1/chats?page_size=100"+(pageToken.Length==0?string.Empty:$"&page_token={Uri.EscapeDataString(pageToken)}"));
            request.Headers.Authorization=new AuthenticationHeaderValue("Bearer",token);
            using var response=await client.SendAsync(request,timeout.Token).ConfigureAwait(false);
            var result=await McpHttpResponse.ReadAsync(response,Service,"code",timeout.Token).ConfigureAwait(false);
            var data=McpHttpResponse.RequiredObject(result,"data",Service);
            if(data["items"] is not JsonArray items)throw McpHttpResponse.Invalid(Service);
            foreach(var item in items)
            {
                if(item is not JsonObject entry)throw McpHttpResponse.Invalid(Service);
                var id=McpHttpResponse.RequiredString(entry,"chat_id",Service);
                var name=entry["name"] is JsonValue v && v.TryGetValue<string>(out var n)?n:id;
                if(ids.Add(id))chats.Add((id,name));
            }
            if(data["has_more"] is not JsonValue more || !more.TryGetValue<bool>(out var hasMore))throw McpHttpResponse.Invalid(Service);
            if(!hasMore)return chats;
            pageToken=McpHttpResponse.RequiredString(data,"page_token",Service);
            if(!seenPages.Add(pageToken))throw McpHttpResponse.Invalid(Service);
        }
        throw new InvalidOperationException(LocalizationService.T("飞书群列表页数过多，请缩小应用可见范围。","The Feishu chat list has too many pages. Narrow the app's visibility scope."));
    }
    internal static Task<string> SendImageAsync(AppSettings settings,byte[] png,CancellationToken cancellationToken)
        =>SendImageAsync(settings,png,NetworkHttpClientFactory.Create(),ReadSecret(cancellationToken),cancellationToken);
    internal static async Task<string> SendImageAsync(AppSettings settings,byte[] png,HttpClient client,string secret,CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        if(!settings.FeishuEnabled||string.IsNullOrWhiteSpace(settings.FeishuTargetId))
            throw new InvalidOperationException(LocalizationService.T("飞书未启用或未指定接收人。","Feishu is not enabled or no recipient is selected."));
        var targetType=settings.FeishuTargetType.Trim().ToLowerInvariant();
        var targetId=settings.FeishuTargetId.Trim();
        if(targetType is not ("open_id" or "chat_id"))throw new InvalidOperationException(LocalizationService.T("飞书接收人类型必须是 chat_id 或 open_id。","Feishu recipient type must be chat_id or open_id."));
        if(png.Length==0||png.Length>10*1024*1024)throw new InvalidOperationException(LocalizationService.T("飞书图片大小必须大于 0 且不超过 10 MB。","Feishu images must be nonempty and no larger than 10 MB."));
        var token=await GetTenantTokenAsync(settings,client,secret,cancellationToken).ConfigureAwait(false);
        var imageKey=await UploadImageAsync(client,token,png,cancellationToken).ConfigureAwait(false);
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        var payload=new JsonObject{["receive_id"]=targetId,["msg_type"]="image",["content"]=new JsonObject{["image_key"]=imageKey}.ToJsonString()};
        using var request=new HttpRequestMessage(HttpMethod.Post,$"{BaseUrl}/open-apis/im/v1/messages?receive_id_type={targetType}")
        {Content=new StringContent(payload.ToJsonString(),Encoding.UTF8,"application/json")};
        request.Headers.Authorization=new AuthenticationHeaderValue("Bearer",token);
        using var response=await client.SendAsync(request,timeout.Token).ConfigureAwait(false);
        var result=await McpHttpResponse.ReadAsync(response,Service,"code",timeout.Token).ConfigureAwait(false);
        McpHttpResponse.RequiredString(McpHttpResponse.RequiredObject(result,"data",Service),"message_id",Service);
        return LocalizationService.T("已通过飞书发送图片。","Image sent via Feishu.");
    }
    private static async Task<string> UploadImageAsync(HttpClient client,string tenantToken,byte[] png,CancellationToken cancellationToken)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        using var form=new MultipartFormDataContent();
        form.Add(new StringContent("message"),"image_type");
        var image=new ByteArrayContent(png);
        image.Headers.ContentType=new MediaTypeHeaderValue("image/png");
        form.Add(image,"image","mewu_ai.png");
        using var request=new HttpRequestMessage(HttpMethod.Post,$"{BaseUrl}/open-apis/im/v1/images"){Content=form};
        request.Headers.Authorization=new AuthenticationHeaderValue("Bearer",tenantToken);
        using var response=await client.SendAsync(request,timeout.Token).ConfigureAwait(false);
        var result=await McpHttpResponse.ReadAsync(response,Service,"code",timeout.Token).ConfigureAwait(false);
        return McpHttpResponse.RequiredString(McpHttpResponse.RequiredObject(result,"data",Service),"image_key",Service);
    }
    private static string ReadSecret(CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        return new CredentialService().Read(CredentialId)??string.Empty;
    }
}
