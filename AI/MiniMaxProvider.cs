using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.AI;

public sealed class MiniMaxProvider : OpenAiCompatibleProvider
{
    private const long MaxImageBytes=10L*1024*1024;
    private const long MaxVideoBytes=50L*1024*1024;
    // The 50 MB value is a per-video limit, not an inline request guarantee.
    // Base64 expands a 50 MiB video to roughly 66.7 MiB, which exceeds the
    // official 64 MB request-body limit before the JSON envelope is added.
    internal const long MaxRequestBodyBytes=64L*1024*1024;

    public override AiProviderCapabilities Capabilities { get; }
    protected override bool StreamingContentIsCumulative=>true;
    protected override long MaxRequestBodySize=>MaxRequestBodyBytes;

    public MiniMaxProvider(AiProviderSettings settings,string key):base(settings,key)
    {
        var isM3=settings.Model.Equals("MiniMax-M3",StringComparison.OrdinalIgnoreCase);
        Capabilities=isM3
            ?new(true,true,true,MaxImageBytes,MaxVideoBytes,TimeSpan.Zero,new HashSet<string>(StringComparer.OrdinalIgnoreCase)
            {
                "image/jpeg","image/png","image/gif","image/webp",
                "video/mp4","video/avi","video/x-msvideo","video/mov","video/quicktime","video/x-matroska"
            })
            :new(false,false,true,0,0,TimeSpan.Zero,new HashSet<string>());
    }

    protected override void ValidateAttachmentSize(AiAttachment attachment,long size)
    {
        if(attachment.Type==AiAttachmentType.Image&&size>MaxImageBytes)throw new InvalidOperationException("MiniMax M3 单张图片不能超过 10 MB");
        if(attachment.Type==AiAttachmentType.Video&&size>MaxVideoBytes)throw new InvalidOperationException("MiniMax M3 视频不能超过 50 MB");
        base.ValidateAttachmentSize(attachment,size);
    }

    protected override InvalidOperationException CreateRequestBodyTooLargeException(long bytes)=>new($"MiniMax 请求体预计为 {(bytes==long.MaxValue?"超大":(bytes/(1024d*1024d)).ToString("0.##",System.Globalization.CultureInfo.InvariantCulture))} MB，超过 64 MB 聚合限制；视频请压缩至约 47 MB，或改用 MiniMax Files API 的 mm_file:// 引用");
}
