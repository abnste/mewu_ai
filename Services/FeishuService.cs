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
/// 飞书图片分享：走自建应用（open.feishu.cn 开放平台）。
/// 用户创建应用并开通 im:message 相关权限后填入 AppId/AppSecret（DPAPI 保存）。
/// 流程：tenant_access_token 鉴权 → im/v1/images 上传得 image_id →
/// im/v1/messages 把图片发送到指定会话（chat_id 或 open_id）。
/// </summary>
internal static class FeishuService
{
    internal const string CredentialId="feishu-app-secret";
    private static readonly TimeSpan Timeout=TimeSpan.FromSeconds(40);
    private const string BaseUrl="https://open.feishu.cn";

    internal static bool IsConfigured(AppSettings settings)
        =>settings.FeishuEnabled
          &&!string.IsNullOrWhiteSpace(settings.FeishuAppId)
          &&!string.IsNullOrWhiteSpace(settings.FeishuTargetId)
          &&new CredentialService().Read(CredentialId) is {Length:>0};

    internal static void SaveSecret(string secret)
    {
        if(string.IsNullOrWhiteSpace(secret))throw new InvalidOperationException(LocalizationService.IsEnglish?"App Secret cannot be empty.":"App Secret 不能为空。");
        new CredentialService().Save(CredentialId,secret.Trim());
    }

    internal static void ClearSecret()=>new CredentialService().Save(CredentialId,string.Empty);

    internal static async Task<string> GetTenantTokenAsync(AppSettings settings,CancellationToken cancellationToken)
    {
        var secret=new CredentialService().Read(CredentialId);
        if(string.IsNullOrWhiteSpace(secret))throw new InvalidOperationException(LocalizationService.T("尚未保存飞书 App Secret。","No Feishu App Secret saved yet."));
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        var payload=new JsonObject{["app_id"]=settings.FeishuAppId.Trim(),["app_secret"]=secret};
        using var client=NetworkHttpClientFactory.Create();
        using var content=new StringContent(payload.ToJsonString(),Encoding.UTF8,"application/json");
        using var response=await client.PostAsync($"{BaseUrl}/open-apis/auth/v3/tenant_access_token/internal",content,timeout.Token).ConfigureAwait(false);
        var body=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
        var result=JsonNode.Parse(body)?.AsObject();
        var code=result?["code"]?.GetValue<int>()??-1;
        if(code!=0)
        {
            var message=result?["msg"]?.GetValue<string>()??body;
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("飞书获取 tenant_access_token 失败（{0}）。请核对 AppId/AppSecret。","Feishu tenant_access_token request failed ({0}). Check the AppId/AppSecret."),
                message));
        }
        return result?["tenant_access_token"]?.GetValue<string>()??throw new InvalidOperationException(LocalizationService.T("飞书响应缺少 tenant_access_token。","Feishu response has no tenant_access_token."));
    }

    /// <summary>列出当前身份可见的群（用于在设置页挑选发送目标）。</summary>
    internal static async Task<IReadOnlyList<(string ChatId,string Name)>> ListChatsAsync(AppSettings settings,CancellationToken cancellationToken)
    {
        var token=await GetTenantTokenAsync(settings,cancellationToken).ConfigureAwait(false);
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        using var client=NetworkHttpClientFactory.Create();
        using var request=new HttpRequestMessage(HttpMethod.Get,$"{BaseUrl}/open-apis/im/v1/chats?page_size=50");
        request.Headers.Authorization=new AuthenticationHeaderValue("Bearer",token);
        using var response=await client.SendAsync(request,timeout.Token).ConfigureAwait(false);
        var body=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
        var result=JsonNode.Parse(body)?.AsObject();
        var code=result?["code"]?.GetValue<int>()??-1;
        if(code!=0)
        {
            var message=result?["msg"]?.GetValue<string>()??body;
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("飞书获取群列表失败（{0}）。请确认应用已开通 im:chat 权限。","Feishu chat list request failed ({0}). Grant the im:chat scope to the app."),
                message));
        }
        var items=result?["data"]?["items"]?.AsArray();
        var chats=new List<(string,string)>();
        if(items is not null)
            foreach(var item in items)
            {
                var chatId=item?["chat_id"]?.GetValue<string>();
                var name=item?["name"]?.GetValue<string>();
                if(!string.IsNullOrEmpty(chatId))chats.Add((chatId!,name??chatId!));
            }
        return chats;
    }

    /// <summary>把 PNG 截图发送到设置中指定的会话/联系人。</summary>
    internal static async Task<string> SendImageAsync(AppSettings settings,byte[] png,CancellationToken cancellationToken)
    {
        if(!IsConfigured(settings))
            throw new InvalidOperationException(LocalizationService.T("飞书未启用或配置不完整。请到 设置 → MCP → 飞书 填写应用信息并保存 App Secret。","Feishu is not enabled or incomplete. Fill in the app details under Settings → MCP → Feishu."));
        var token=await GetTenantTokenAsync(settings,cancellationToken).ConfigureAwait(false);
        var imageId=await UploadImageAsync(token,png,cancellationToken).ConfigureAwait(false);
        var targetType=string.Equals(settings.FeishuTargetType,"open_id",StringComparison.OrdinalIgnoreCase)?"open_id":"chat_id";
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        var payload=new JsonObject
        {
            ["receive_id"]=settings.FeishuTargetId.Trim(),
            ["msg_type"]="image",
            ["content"]=new JsonObject{["image_id"]=imageId}.ToJsonString()
        };
        using var client=NetworkHttpClientFactory.Create();
        using var request=new HttpRequestMessage(HttpMethod.Post,$"{BaseUrl}/open-apis/im/v1/messages?receive_id_type={targetType}")
        {
            Content=new StringContent(payload.ToJsonString(),Encoding.UTF8,"application/json")
        };
        request.Headers.Authorization=new AuthenticationHeaderValue("Bearer",token);
        using var response=await client.SendAsync(request,timeout.Token).ConfigureAwait(false);
        var body=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
        var result=JsonNode.Parse(body)?.AsObject();
        var code=result?["code"]?.GetValue<int>()??-1;
        if(code!=0)
        {
            var message=result?["msg"]?.GetValue<string>()??body;
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("飞书发送失败（{0}）。请核对接收人 ID 与应用权限（im:message）。","Feishu send failed ({0}). Check the receiver ID and the im:message scope."),
                message));
        }
        return LocalizationService.T("已通过飞书发送图片。","Image sent via Feishu.");
    }

    private static async Task<string> UploadImageAsync(string tenantToken,byte[] png,CancellationToken cancellationToken)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        using var form=new MultipartFormDataContent();
        var image=new ByteArrayContent(png);
        image.Headers.ContentType=new MediaTypeHeaderValue("image/png");
        form.Add(image,"image","mewu_ai.png");
        using var client=NetworkHttpClientFactory.Create();
        using var request=new HttpRequestMessage(HttpMethod.Post,$"{BaseUrl}/open-apis/im/v1/images?image_type=message"){Content=form};
        request.Headers.Authorization=new AuthenticationHeaderValue("Bearer",tenantToken);
        using var response=await client.SendAsync(request,timeout.Token).ConfigureAwait(false);
        var body=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
        var result=JsonNode.Parse(body)?.AsObject();
        var code=result?["code"]?.GetValue<int>()??-1;
        if(code!=0)
        {
            var message=result?["msg"]?.GetValue<string>()??body;
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("飞书上传图片失败（{0}）。","Feishu image upload failed ({0})."),message));
        }
        return result?["data"]?["image_id"]?.GetValue<string>()??throw new InvalidOperationException(LocalizationService.T("飞书响应缺少 image_id。","Feishu response has no image_id."));
    }
}
