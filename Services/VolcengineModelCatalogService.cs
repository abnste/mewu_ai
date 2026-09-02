using System.Net.Http;
using System.Net.Http.Headers;
using System.Text.Json;

namespace mewu_ai_Assistant.Services;

internal sealed class VolcengineModelCatalogService
{
    private const int MaximumResponseBytes=2*1024*1024;
    private static readonly HttpClient Client=new(new HttpClientHandler{AllowAutoRedirect=false,UseCookies=false}){Timeout=Timeout.InfiniteTimeSpan};

    internal async Task<IReadOnlyList<string>> GetChatModelsAsync(string baseUrl,string apiKey,IReadOnlyDictionary<string,string> customHeaders,CancellationToken token)
    {
        var baseUri=ProviderEndpointPolicy.NormalizeBaseUri(baseUrl);
        if(!VolcengineModelPolicy.IsEndpoint(baseUri))throw new InvalidOperationException("只有火山方舟地址可以获取方舟模型列表");
        if(string.IsNullOrWhiteSpace(apiKey)&&!customHeaders.Keys.Any(ProviderHeaderCredentialService.IsAuthentication))throw new InvalidOperationException("请先输入火山方舟 API Key");
        if(!string.IsNullOrWhiteSpace(apiKey)&&customHeaders.Keys.Any(ProviderHeaderCredentialService.IsAuthentication))throw new InvalidOperationException("API Key 与认证 Custom Header 不能同时发送");
        ProviderHeaderPolicy.EnsureValid(customHeaders);
        using var request=new HttpRequestMessage(HttpMethod.Get,new Uri(baseUri,"models"));
        if(!string.IsNullOrWhiteSpace(apiKey))request.Headers.Authorization=new AuthenticationHeaderValue("Bearer",apiKey);
        foreach(var header in customHeaders)if(!request.Headers.TryAddWithoutValidation(header.Key,header.Value))throw new InvalidOperationException($"无法添加 Provider 请求头：{header.Key}");
        using var response=await Client.SendAsync(request,HttpCompletionOption.ResponseHeadersRead,token).ConfigureAwait(false);
        if(!response.IsSuccessStatusCode)throw new InvalidOperationException($"获取火山模型列表失败（HTTP {(int)response.StatusCode}）");
        if(response.Content.Headers.ContentLength is >MaximumResponseBytes)throw new InvalidDataException("火山模型列表响应超过安全上限");
        var bytes=await response.Content.ReadAsByteArrayAsync(token).ConfigureAwait(false);
        if(bytes.Length>MaximumResponseBytes)throw new InvalidDataException("火山模型列表响应超过安全上限");
        using var document=JsonDocument.Parse(bytes);
        if(!document.RootElement.TryGetProperty("data",out var data)||data.ValueKind!=JsonValueKind.Array)throw new InvalidDataException("火山模型列表响应格式无效");
        var models=data.EnumerateArray().Select(item=>item.TryGetProperty("id",out var id)&&id.ValueKind==JsonValueKind.String?id.GetString():null).Where(id=>id is not null&&id.Length is >0 and <=200&&VolcengineModelPolicy.IsChatModel(id)).Select(id=>id!).Distinct(StringComparer.OrdinalIgnoreCase).ToList();
        var recommendedOrder=VolcengineModelPolicy.RecommendedModels.Select((model,index)=>(model,index)).ToDictionary(entry=>entry.model,entry=>entry.index,StringComparer.OrdinalIgnoreCase);
        return models.OrderBy(model=>recommendedOrder.GetValueOrDefault(model,int.MaxValue)).ThenByDescending(model=>model,StringComparer.OrdinalIgnoreCase).ToArray();
    }
}
