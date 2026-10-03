// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-FileCopyrightText: 2026 ima-mcp-server contributors (COS 签名流程参考, MIT)
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Net.Http;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json.Nodes;
using System.Globalization;
using System.Text.RegularExpressions;
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
    internal static Task<IReadOnlyList<ImaKnowledgeBase>> GetKnowledgeBasesAsync(AppSettings settings,CancellationToken cancellationToken)
        =>GetKnowledgeBasesAsync(settings,NetworkHttpClientFactory.Create(),ReadSecret(cancellationToken),cancellationToken);

    internal static async Task<IReadOnlyList<ImaKnowledgeBase>> GetKnowledgeBasesAsync(AppSettings settings,HttpClient client,string apiKey,CancellationToken cancellationToken)
    {
        settings=Snapshot(settings);
        var result=new List<ImaKnowledgeBase>();
        var ids=new HashSet<string>(StringComparer.Ordinal);
        var cursors=new HashSet<string>(StringComparer.Ordinal);
        var cursor=string.Empty;
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        for(var page=0;page<100;page++)
        {
            var payload=await PostAsync(settings,client,apiKey,"get_addable_knowledge_base_list",new JsonObject{["cursor"]=cursor,["limit"]=50},timeout.Token).ConfigureAwait(false);
            if(payload["addable_knowledge_base_list"] is not JsonArray list)throw McpHttpResponse.Invalid("ima");
            foreach(var entry in list)
            {
                if(entry is not JsonObject item)throw McpHttpResponse.Invalid("ima");
                var id=McpHttpResponse.RequiredString(item,"id","ima");
                var name=McpHttpResponse.RequiredString(item,"name","ima");
                if(ids.Add(id))result.Add(new ImaKnowledgeBase(id,name));
            }
            if(payload["is_end"] is not JsonValue value||!value.TryGetValue<bool>(out var isEnd))throw McpHttpResponse.Invalid("ima");
            if(isEnd)return result;
            cursor=McpHttpResponse.RequiredString(payload,"next_cursor","ima");
            if(!cursors.Add(cursor))throw McpHttpResponse.Invalid("ima");
        }
        throw new InvalidOperationException(LocalizationService.T("ima 知识库列表页数过多。","The ima knowledge-base list has too many pages."));
    }

    /// <summary>把 PNG 截图归档进 ima 知识库，返回可读结果文本。</summary>
    internal static Task<string> SaveImageAsync(AppSettings settings,byte[] png,CancellationToken cancellationToken)
        =>SaveImageAsync(settings,png,NetworkHttpClientFactory.Create(),ReadSecret(cancellationToken),cancellationToken);

    internal static async Task<string> SaveImageAsync(AppSettings settings,byte[] png,HttpClient client,string apiKey,CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        settings=Snapshot(settings);
        if(!settings.ImaEnabled||string.IsNullOrWhiteSpace(settings.ImaClientId)||string.IsNullOrWhiteSpace(apiKey))
            throw new InvalidOperationException(LocalizationService.T("ima 未启用或配置不完整。请到 设置 → MCP → ima 填写 Client ID 并保存 API Key。","ima is not enabled or incomplete. Fill in the Client ID and save the API Key under Settings → MCP → ima."));
        if(png.Length==0||png.Length>30*1024*1024)throw new InvalidOperationException(LocalizationService.T("ima 图片大小必须大于 0 且不超过 30 MB。","ima images must be nonempty and no larger than 30 MB."));
        var kbId=settings.ImaKnowledgeBaseId.Trim();
        var kbName=string.IsNullOrWhiteSpace(settings.ImaKnowledgeBaseName)?kbId:settings.ImaKnowledgeBaseName;
        if(kbId.Length==0)
        {
            var bases=await GetKnowledgeBasesAsync(settings,client,apiKey,cancellationToken).ConfigureAwait(false);
            if(bases.Count==0)throw new InvalidOperationException(LocalizationService.T("ima 账号下没有可添加内容的知识库。","No addable ima knowledge bases found."));
            kbId=bases[0].Id;
            kbName=bases[0].Name;
        }
        var stamp=DateTime.Now.ToString("yyyyMMdd-HHmmss",System.Globalization.CultureInfo.InvariantCulture)+"-"+Guid.NewGuid().ToString("N");
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
        var created=await PostAsync(settings,client,apiKey,"create_media",createPayload,cancellationToken).ConfigureAwait(false);
        var mediaId=McpHttpResponse.RequiredString(created,"media_id","ima");
        var cos=McpHttpResponse.RequiredObject(created,"cos_credential","ima");

        // Step 2：COS 直传。
        await UploadToCosAsync(client,cos,png,cancellationToken).ConfigureAwait(false);

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
        var added=await PostAsync(settings,client,apiKey,"add_knowledge",addPayload,cancellationToken).ConfigureAwait(false);
        McpHttpResponse.RequiredString(added,"media_id","ima");
        return LocalizationService.T($"已存入 ima 知识库「{kbName}」：{fileName}",$"Saved to ima knowledge base “{kbName}”: {fileName}");
    }

    private static async Task<JsonObject> PostAsync(AppSettings settings,HttpClient client,string apiKey,string endpoint,JsonObject payload,CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        if(string.IsNullOrWhiteSpace(apiKey))throw new InvalidOperationException(LocalizationService.T("尚未保存 ima API Key。","No ima API Key saved yet."));
        if(string.IsNullOrWhiteSpace(settings.ImaClientId)||settings.ImaClientId.Any(char.IsControl)||apiKey.Any(char.IsControl))throw new InvalidOperationException(LocalizationService.T("ima Client ID 或 API Key 格式无效。","The ima Client ID or API Key format is invalid."));
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        using var content=new StringContent(payload.ToJsonString(),Encoding.UTF8,"application/json");
        using var request=new HttpRequestMessage(HttpMethod.Post,$"{BaseUrl}/{endpoint}"){Content=content};
        request.Headers.TryAddWithoutValidation("ima-openapi-clientid",settings.ImaClientId.Trim());
        request.Headers.TryAddWithoutValidation("ima-openapi-apikey",apiKey);
        request.Headers.TryAddWithoutValidation("ima-openapi-ctx","client=mewu_ai");
        using var response=await client.SendAsync(request,timeout.Token).ConfigureAwait(false);
        var parsed=await McpHttpResponse.ReadAsync(response,"ima "+endpoint,"code",timeout.Token).ConfigureAwait(false);
        return McpHttpResponse.RequiredObject(parsed,"data","ima");
    }

    /// <summary>COS 简单上传（PUT）+ q-sign-algorithm=sha1 临时密钥签名。
    /// 与 ima 官方客户端一致：HttpString 末尾带换行、方法名小写 put、
    /// STS token 走 x-cos-security-token 头。</summary>
    private static async Task UploadToCosAsync(HttpClient client,JsonObject cos,byte[] png,CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        var secretId=McpHttpResponse.RequiredString(cos,"secret_id","ima COS");
        var secretKey=McpHttpResponse.RequiredString(cos,"secret_key","ima COS");
        var token=McpHttpResponse.RequiredString(cos,"token","ima COS");
        var bucket=McpHttpResponse.RequiredString(cos,"bucket_name","ima COS");
        var region=McpHttpResponse.RequiredString(cos,"region","ima COS");
        var cosKey=McpHttpResponse.RequiredString(cos,"cos_key","ima COS").TrimStart('/');
        if(!Regex.IsMatch(bucket,@"\A[a-z0-9][a-z0-9-]*-[0-9]+\z")||!Regex.IsMatch(region,@"\A[a-z0-9]+(?:-[a-z0-9]+)+\z")
            ||cosKey.Length==0||cosKey.Contains('\\')||cosKey.Any(char.IsControl)||cosKey.Split('/').Any(p=>p is "." or "..")
            ||secretId.Any(char.IsControl)||token.Any(char.IsControl))throw McpHttpResponse.Invalid("ima COS");
        var host=$"{bucket}.cos.{region}.myqcloud.com";
        var uri="/"+cosKey;
        var escapedUri="/"+string.Join("/",cosKey.Split('/').Select(Uri.EscapeDataString));
        var contentType="image/png";
        var start=RequiredTimestamp(cos,"start_time");
        var end=RequiredTimestamp(cos,"expired_time");
        var now=DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        if(start>now+60||end<=now||end<=start)throw McpHttpResponse.Invalid("ima COS");
        var keyTime=$"{start};{end}";
        var signKey=HmacSha1Hex(keyTime,secretKey!);
        var httpString=$"put\n{uri}\n\ncontent-type={Uri.EscapeDataString(contentType)}&host={Uri.EscapeDataString(host)}\n";
        var httpSha1=Sha1Hex(httpString);
        var stringToSign=$"sha1\n{keyTime}\n{httpSha1}\n";
        var signature=HmacSha1Hex(stringToSign,signKey);
        var authorization=$"q-sign-algorithm=sha1&q-ak={secretId}&q-sign-time={keyTime}&q-key-time={keyTime}&q-header-list=content-type;host&q-url-param-list=&q-signature={signature}";

        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        using var content=new ByteArrayContent(png);
        content.Headers.TryAddWithoutValidation("Content-Type",contentType);
        using var request=new HttpRequestMessage(HttpMethod.Put,$"https://{host}{escapedUri}"){Content=content};
        request.Headers.TryAddWithoutValidation("Authorization",authorization);
        request.Headers.TryAddWithoutValidation("x-cos-security-token",token??string.Empty);
        using var response=await client.SendAsync(request,timeout.Token).ConfigureAwait(false);
        if(!response.IsSuccessStatusCode)
        {
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("ima COS 上传失败（HTTP {0}）。","ima COS upload failed (HTTP {0})."),(int)response.StatusCode));
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

    private static long RequiredTimestamp(JsonObject cos,string name)
    {
        if(cos[name] is JsonValue value)
        {
            if(value.TryGetValue<long>(out var number))return number;
            if(value.TryGetValue<string>(out var text)&&long.TryParse(text,NumberStyles.None,CultureInfo.InvariantCulture,out number))return number;
        }
        throw McpHttpResponse.Invalid("ima COS");
    }

    private static string ReadSecret(CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        return new CredentialService().Read(CredentialId)??string.Empty;
    }

    private static AppSettings Snapshot(AppSettings settings)=>new()
    {
        ImaEnabled=settings.ImaEnabled,ImaClientId=settings.ImaClientId,
        ImaKnowledgeBaseId=settings.ImaKnowledgeBaseId,ImaKnowledgeBaseName=settings.ImaKnowledgeBaseName
    };
}
