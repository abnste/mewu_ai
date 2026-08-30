using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.AI;
public sealed class MiniMaxProvider : OpenAiCompatibleProvider
{
    private const long MaxImageBytes=10L*1024*1024;
    private const long MaxVideoBytes=50L*1024*1024;
    public override AiProviderCapabilities Capabilities { get; }
    public MiniMaxProvider(AiProviderSettings settings,string key):base(settings,key)
    {
        var isM3=settings.Model.Equals("MiniMax-M3",StringComparison.OrdinalIgnoreCase);
        Capabilities=isM3
            ?new(true,true,true,true,true,MaxVideoBytes,TimeSpan.Zero,new HashSet<string>(StringComparer.OrdinalIgnoreCase)
            {
                "image/jpeg","image/png","image/gif","image/webp",
                "video/mp4","video/avi","video/x-msvideo","video/mov","video/quicktime","video/x-matroska"
            })
            :new(true,false,false,true,false,0,TimeSpan.Zero,new HashSet<string>());
    }

    public override Task<AiResult> SendAsync(AiRequest request,CancellationToken token)
    {
        foreach(var attachment in request.Attachments)
        {
            var size=attachment.Data?.LongLength??(attachment.FilePath is null?0:new FileInfo(attachment.FilePath).Length);
            if(attachment.Type==AiAttachmentType.Image&&size>MaxImageBytes)throw new InvalidOperationException("MiniMax M3 单张图片不能超过 10 MB");
            if(attachment.Type==AiAttachmentType.Video&&size>MaxVideoBytes)throw new InvalidOperationException("MiniMax M3 视频不能超过 50 MB");
        }
        return base.SendAsync(request,token);
    }
}
