using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class VolcengineModelPolicy
{
    internal const string StandardBaseUrl="https://ark.cn-beijing.volces.com/api/v3";
    internal static readonly IReadOnlyList<string> RecommendedModels=
    [
        "doubao-seed-2-1-pro-260628",
        "doubao-seed-2-1-turbo-260628",
        "doubao-seed-2-0-pro-260215",
        "doubao-seed-2-0-lite-260428",
        "doubao-seed-2-0-mini-260428",
        "deepseek-v4-pro-ga-260813",
        "deepseek-v4-flash-ga-260731",
        "glm-5-2-260617"
    ];

    internal static bool IsEndpoint(Uri baseUri)=>HostMatches(baseUri.Host,"volces.com");

    internal static AiProviderCapabilities GetCapabilities(string model)
    {
        var normalized=model.Trim();
        var seed2=normalized.StartsWith("doubao-seed-2-",StringComparison.OrdinalIgnoreCase);
        var glm53Flash=normalized.StartsWith("glm-5-3-flash",StringComparison.OrdinalIgnoreCase)||normalized.StartsWith("glm-5.3-flash",StringComparison.OrdinalIgnoreCase);
        var deepSeekVision=normalized.StartsWith("deepseek-v4",StringComparison.OrdinalIgnoreCase)&&normalized.Contains("vision",StringComparison.OrdinalIgnoreCase);
        var image=seed2||glm53Flash||deepSeekVision||
                  normalized.Contains("vision",StringComparison.OrdinalIgnoreCase)||
                  normalized.Contains("ui-tars",StringComparison.OrdinalIgnoreCase)||
                  normalized.StartsWith("doubao-seed-1-8-",StringComparison.OrdinalIgnoreCase);
        var video=seed2||glm53Flash||deepSeekVision;
        var accepted=video
            ?new HashSet<string>(["image/png","image/jpeg","image/webp","video/mp4","video/x-msvideo","video/quicktime"],StringComparer.OrdinalIgnoreCase)
            :image
                ?new HashSet<string>(["image/png","image/jpeg","image/webp"],StringComparer.OrdinalIgnoreCase)
                :new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        return new(image,video,true,image?10L*1024*1024:0,video?50L*1024*1024:0,TimeSpan.Zero,accepted);
    }

    internal static bool IsChatModel(string model)
    {
        if(string.IsNullOrWhiteSpace(model))return false;
        var value=model.Trim();
        if(value.Contains("embedding",StringComparison.OrdinalIgnoreCase)||value.Contains("seedream",StringComparison.OrdinalIgnoreCase)||value.Contains("seedance",StringComparison.OrdinalIgnoreCase)||value.Contains("seed3d",StringComparison.OrdinalIgnoreCase)||value.Contains("seededit",StringComparison.OrdinalIgnoreCase)||value.StartsWith("wan",StringComparison.OrdinalIgnoreCase)||value.Contains("hitem3d",StringComparison.OrdinalIgnoreCase)||value.Contains("hyper3d",StringComparison.OrdinalIgnoreCase))return false;
        return value.StartsWith("doubao",StringComparison.OrdinalIgnoreCase)||value.StartsWith("deepseek",StringComparison.OrdinalIgnoreCase)||value.StartsWith("glm",StringComparison.OrdinalIgnoreCase)||value.StartsWith("kimi",StringComparison.OrdinalIgnoreCase)||value.StartsWith("qwen",StringComparison.OrdinalIgnoreCase)||value.StartsWith("mistral",StringComparison.OrdinalIgnoreCase);
    }

    private static bool HostMatches(string host,string domain)=>host.Equals(domain,StringComparison.OrdinalIgnoreCase)||host.EndsWith("."+domain,StringComparison.OrdinalIgnoreCase);
}
