// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;

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
    protected override int VideoSamplingFramesPerSecond=>VideoAnalysisTimebaseService.FramesPerSecond;

    public override async Task<AiResult> SendAsync(AiRequest request,CancellationToken token)
    {
        ArgumentNullException.ThrowIfNull(request);
        if(!Capabilities.SupportsVideo||request.Attachments is null||!request.Attachments.Any(item=>item?.Type==AiAttachmentType.Video))return await base.SendAsync(request,token).ConfigureAwait(false);
        var copies=new List<VideoAnalysisCopy>();
        try
        {
            // Reject oversized input before any file read, encoding or Base64.
            ValidateRequest(request);
            var attachments=new List<AiAttachment>();
            var metadata=new System.Text.StringBuilder();
            for(var index=0;index<request.Attachments.Count;index++)
            {
                token.ThrowIfCancellationRequested();var attachment=request.Attachments[index];
                if(attachment.Type!=AiAttachmentType.Video){attachments.Add(attachment);continue;}
                var copy=await VideoAnalysisTimebaseService.CreateAsync(attachment,token).ConfigureAwait(false);copies.Add(copy);
                attachments.Add(attachment with{Data=null,FilePath=copy.Path,Duration=copy.Duration,ProviderOwnsData=false});
                metadata.Append(System.Globalization.CultureInfo.InvariantCulture,$"\n视频附件 {index}：实际时长 {copy.Duration.TotalSeconds:0.###} 秒；编码与采样均为 {VideoAnalysisTimebaseService.FramesPerSecond} FPS，时间标签已经是实际秒数，不要再乘百分比或比例。事件时间精度不应超出采样间隔；不确定时写约或时间范围。");
            }
            var calibrated=new AiRequest{Prompt=request.Prompt+metadata,Attachments=attachments,History=request.History,StreamingProgress=request.StreamingProgress,AgentProgress=request.AgentProgress,InteractionHandler=request.InteractionHandler,StreamingCompletionPredicate=request.StreamingCompletionPredicate,ExpectStructuredResponse=request.ExpectStructuredResponse,DisableReasoning=request.DisableReasoning,MaxOutputTokens=request.MaxOutputTokens,UseModelMaximumOutputTokens=request.UseModelMaximumOutputTokens};
            return await base.SendAsync(calibrated,token).ConfigureAwait(false);
        }
        finally{foreach(var copy in copies)copy.Dispose();AiImageEncodingService.ClearAttachmentBuffers(request.Attachments.Where(item=>item is not null));}
    }

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
