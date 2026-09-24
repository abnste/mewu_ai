// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-FileCopyrightText: 2026 ima-mcp-server contributors (COS 签名流程参考, MIT)
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Net.Http;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json.Nodes;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;

/// <summary>
/// 腾讯 ima 知识库截图归档：走 ima 官方 OpenAPI（ima.qq.com/openapi）。
/// 凭证（Client ID + API Key）在 ima.qq.com/agent-interface 生成；API Key 经
/// CredentialService（DPAPI）保存。流程（与 ima 官方 Web/客户端一致）：
///   1. POST /openapi/wiki/v1/get_addable_knowledge_base_list → 解析目标知识库
///   2. POST /openapi/wiki/v1/create_media                    → media_id + COS 上传凭证
///   3. PUT  {bucket}.cos.{region}.myqcloud.com/{cos_key}      → COS 直传（sha1 签名）
///   4. POST /openapi/wiki/v1/add_knowledge                   → 登记进知识库
/// 说明：ima 未向第三方开放扫码授权，官方接入方式即 Client ID/API Key。
/// </summary>
internal static class ImaVaultService
{
    internal const string CredentialId="ima-openapi-apikey";
    private const string BaseUrl="https://ima.qq.com/openapi/wiki/v1";
    private const int ImageMediaType=9; // ima media_type：9 = 图片
    private static readonly TimeSpan Timeout=TimeSpan.FromSeconds(60);

    internal static bool IsConfigured(AppSettings settings)
        =>settings.ImaEnabled
          &&!string.IsNullOrWhiteSpace(settings.ImaClientId)
          &&new CredentialService().Read(CredentialId) is {Length:>0};

    internal static void SaveSecret(string secret)
    {
        if(string.IsNullOrWhiteSpace(secret))throw new InvalidOperationException(LocalizationService.IsEnglish?"API Key cannot be empty.":"API Key 不能为空。");
        new CredentialService().Save(CredentialId,secret.Trim());
    }

    internal static void ClearSecret()=>new CredentialService().Save(CredentialId,string.Empty);

    internal sealed record ImaKnowledgeBase(string Id,string Name);

    /// <summary>列出当前账号可添加内容的知识库（用于设置页选择与连接测试）。</summary>
    internal static async Task<IReadOnlyList<ImaKnowledgeBase>> GetKnowledgeBasesAsync(AppSettings settings,CancellationToken cancellationToken)
    {
        var payload=await PostAsync(settings,"get_addable_knowledge_base_list",new JsonObject{["cursor"]=string.Empty,["limit"]=50},cancellationToken).ConfigureAwait(false);
        var list=payload?["knowledge_base_list"]?.AsArray()??payload?["data"]?["knowledge_base_list"]?.AsArray();
        var result=new List<ImaKnowledgeBase>();
        if(list is not null)
            foreach(var entry in list)
            {
                var id=entry?["knowledge_base_id"]?.GetValue<string>()??entry?["id"]?.GetValue<string>();
                var name=entry?["name"]?.GetValue<string>()??entry?["title"]?.GetValue<string>()??string.Empty;
                if(!string.IsNullOrWhiteSpace(id))result.Add(new ImaKnowledgeBase(id!,name));
            }
        return result;
    }

    /// <summary>把 PNG 截图归档进 ima 知识库，返回可读结果文本。</summary>
    internal static async Task<string> SaveImageAsync(AppSettings settings,byte[] png,CancellationToken cancellationToken)
    {
        if(!IsConfigured(settings))
            throw new InvalidOperationException(LocalizationService.T("ima 未启用或配置不完整。请到 设置 → MCP → ima 填写 Client ID 并保存 API Key。","ima is not enabled or incomplete. Fill in the Client ID and save the API Key under Settings → MCP → ima."));
        var (kbId,kbName)=await ResolveKnowledgeBaseAsync(settings,cancellationToken).ConfigureAwait(false);
        var stamp=DateTime.Now.ToString("yyyyMMdd-HHmmss",System.Globalization.CultureInfo.InvariantCulture);
        var fileName=$"MewuAI-{stamp}.png";

        // Step 1：创建媒体，取得 COS 直传凭证。
        var createPayload=new JsonObject
        {
            ["file_name"]=fileName,
            ["file_size"]=png.Length,
            ["content_type"]="image/png",
            ["knowledge_base_id"]=kbId,
            ["file_ext"]="png"
        };
        var created=await PostAsync(settings,"create_media",createPayload,cancellationToken).ConfigureAwait(false)
            ??throw new InvalidOperationException(LocalizationService.T("ima create_media 响应无效。","ima create_media returned an invalid response."));
        var mediaId=created["media_id"]?.GetValue<string>()
            ??throw new InvalidOperationException(LocalizationService.T("ima 响应缺少 media_id。","ima response has no media_id."));
        var cos=created["cos_credential"]?.AsObject()
            ??throw new InvalidOperationException(LocalizationService.T("ima 响应缺少 COS 上传凭证。","ima response has no COS upload credential."));

        // Step 2：COS 直传。
        await UploadToCosAsync(cos,png,cancellationToken).ConfigureAwait(false);

        // Step 3：登记进知识库。
        var addPayload=new JsonObject
        {
            ["media_type"]=ImageMediaType,
            ["media_id"]=mediaId,
            ["title"]=fileName,
            ["knowledge_base_id"]=kbId,
            ["file_info"]=new JsonObject
            {
                ["cos_key"]=cos["cos_key"]?.GetValue<string>()??string.Empty,
                ["file_size"]=png.Length,
                ["file_name"]=fileName
            }
        };
        await PostAsync(settings,"add_knowledge",addPayload,cancellationToken).ConfigureAwait(false);
        return LocalizationService.T($"已存入 ima 知识库「{kbName}」：{fileName}",$"Saved to ima knowledge base “{kbName}”: {fileName}");
    }

    private static async Task<(string Id,string Name)> ResolveKnowledgeBaseAsync(AppSettings settings,CancellationToken cancellationToken)
    {
        if(!string.IsNullOrWhiteSpace(settings.ImaKnowledgeBaseId))
            return (settings.ImaKnowledgeBaseId.Trim(),string.IsNullOrWhiteSpace(settings.ImaKnowledgeBaseName)?settings.ImaKnowledgeBaseId.Trim():settings.ImaKnowledgeBaseName);
        var list=await GetKnowledgeBasesAsync(settings,cancellationToken).ConfigureAwait(false);
        if(list.Count==0)
            throw new InvalidOperationException(LocalizationService.T("ima 账号下没有可添加内容的知识库，请先在 ima 中创建。","No addable ima knowledge bases found. Create one in ima first."));
        return (list[0].Id,list[0].Name);
    }

    private static async Task<JsonObject?> PostAsync(AppSettings settings,string endpoint,JsonObject payload,CancellationToken cancellationToken)
    {
        var apiKey=new CredentialService().Read(CredentialId);
        if(string.IsNullOrWhiteSpace(apiKey))throw new InvalidOperationException(LocalizationService.T("尚未保存 ima API Key。","No ima API Key saved yet."));
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        using var client=NetworkHttpClientFactory.Create();
        using var content=new StringContent(payload.ToJsonString(),Encoding.UTF8,"application/json");
        using var request=new HttpRequestMessage(HttpMethod.Post,$"{BaseUrl}/{endpoint}"){Content=content};
        request.Headers.TryAddWithoutValidation("ima-openapi-clientid",settings.ImaClientId.Trim());
        request.Headers.TryAddWithoutValidation("ima-openapi-apikey",apiKey);
        request.Headers.TryAddWithoutValidation("ima-openapi-ctx","client=mewu_ai");
        using var response=await client.SendAsync(request,timeout.Token).ConfigureAwait(false);
        var body=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
        var parsed=JsonNode.Parse(body)?.AsObject()
            ??throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("ima 返回非 JSON 响应（HTTP {0}）。","ima returned a non-JSON response (HTTP {0})."),(int)response.StatusCode));
        var code=parsed["code"]?.GetValue<int>()??-1;
        if(code!=0)
        {
            var message=parsed["msg"]?.GetValue<string>()??body;
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("ima 接口 {0} 失败（{1}）。","ima endpoint {0} failed ({1})."),endpoint,message));
        }
        return parsed["data"]?.AsObject()??parsed;
    }

    /// <summary>COS 简单上传（PUT）+ q-sign-algorithm=sha1 临时密钥签名。
    /// 与 ima 官方客户端一致：HttpString 末尾带换行、方法名小写 put、
    /// STS token 走 x-cos-security-token 头。</summary>
    private static async Task UploadToCosAsync(JsonObject cos,byte[] png,CancellationToken cancellationToken)
    {
        var secretId=cos["secret_id"]?.GetValue<string>();
        var secretKey=cos["secret_key"]?.GetValue<string>();
        var token=cos["token"]?.GetValue<string>();
        var bucket=cos["bucket"]?.GetValue<string>()??cos["bucket_name"]?.GetValue<string>();
        var region=cos["region"]?.GetValue<string>();
        var cosKey=cos["cos_key"]?.GetValue<string>();
        if(string.IsNullOrWhiteSpace(secretId)||string.IsNullOrWhiteSpace(secretKey)||string.IsNullOrWhiteSpace(bucket)||string.IsNullOrWhiteSpace(region)||string.IsNullOrWhiteSpace(cosKey))
            throw new InvalidOperationException(LocalizationService.T("ima COS 上传凭证字段不完整。","ima COS upload credential is incomplete."));
        var host=$"{bucket}.cos.{region}.myqcloud.com";
        var uri="/"+cosKey.TrimStart('/');
        var contentType="image/png";
        var start=DateTimeOffset.UtcNow.ToUnixTimeSeconds()-60;
        var end=start+3600;
        var keyTime=$"{start};{end}";
        var signKey=HmacSha1Hex(keyTime,secretKey!);
        var httpString=$"put\n{uri}\n\ncontent-type={Uri.EscapeDataString(contentType)}&host={Uri.EscapeDataString(host)}\n";
        var httpSha1=Sha1Hex(httpString);
        var stringToSign=$"sha1\n{keyTime}\n{httpSha1}\n";
        var signature=HmacSha1Hex(stringToSign,signKey);
        var authorization=$"q-sign-algorithm=sha1&q-ak={secretId}&q-sign-time={keyTime}&q-key-time={keyTime}&q-header-list=content-type;host&q-url-param-list=&q-signature={signature}";

        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        using var client=NetworkHttpClientFactory.Create();
        using var content=new ByteArrayContent(png);
        content.Headers.TryAddWithoutValidation("Content-Type",contentType);
        using var request=new HttpRequestMessage(HttpMethod.Put,$"https://{host}{uri}"){Content=content};
        request.Headers.TryAddWithoutValidation("Authorization",authorization);
        request.Headers.TryAddWithoutValidation("x-cos-security-token",token??string.Empty);
        using var response=await client.SendAsync(request,timeout.Token).ConfigureAwait(false);
        if(!response.IsSuccessStatusCode)
        {
            var body=await response.Content.ReadAsStringAsync(timeout.Token).ConfigureAwait(false);
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("ima COS 上传失败（HTTP {0}）。","ima COS upload failed (HTTP {0})."),$"{(int)response.StatusCode} {Truncate(body)}"));
        }
    }

    private static string HmacSha1Hex(string data,string key)
    {
        using var hmac=new HMACSHA1(Encoding.UTF8.GetBytes(key));
        return Convert.ToHexString(hmac.ComputeHash(Encoding.UTF8.GetBytes(data))).ToLowerInvariant();
    }

    private static string Sha1Hex(string data)
    {
        return Convert.ToHexString(SHA1.HashData(Encoding.UTF8.GetBytes(data))).ToLowerInvariant();
    }

    private static string Truncate(string value)=>value.Length<=200?value:value[..200];
}
