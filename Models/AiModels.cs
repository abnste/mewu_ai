namespace mewu_ai_Assistant.Models;
public enum AiAttachmentType { Image,Video }
public sealed record AiAttachment(
    AiAttachmentType Type,
    string MimeType,
    byte[]? Data=null,
    string? FilePath=null,
    TimeSpan? Duration=null,
    bool ProviderOwnsData=true);
public sealed record AiMessage(string Role,string Text);
public sealed record AiStreamDelta(string Content,string ReasoningContent,bool ReasoningIsCumulative=false);
public enum AiAgentEventKind { Status,ToolStarted,ToolProgress,ToolCompleted }
public sealed record AiAgentEvent(AiAgentEventKind Kind,string Title,string Detail="",bool IsError=false);
public enum AiInteractionKind { Approval,Clarification,SudoPassword,Secret }
public sealed record AiInteractionRequest(
    AiInteractionKind Kind,
    string RequestId,
    string Title,
    string Message,
    IReadOnlyList<string> Choices,
    bool IsSensitive=false,
    bool MultiSelect=false,
    string QuestionId="");
public sealed record AiInteractionResponse(string Value,string Choice="",IReadOnlyList<string>? Values=null);
public sealed class AiRequest
{
    public string Prompt { get; init; }=string.Empty;
    public List<AiMessage> History { get; init; }=[];
    public List<AiAttachment> Attachments { get; init; }=[];
    public IProgress<AiStreamDelta>? StreamingProgress { get; init; }
    public IProgress<AiAgentEvent>? AgentProgress { get; init; }
    public Func<AiInteractionRequest,CancellationToken,Task<AiInteractionResponse>>? InteractionHandler { get; init; }
    public Func<string,bool>? StreamingCompletionPredicate { get; init; }
    public bool ExpectStructuredResponse { get; init; }
    public bool DisableReasoning { get; init; }
    public int? MaxOutputTokens { get; init; }
}
public sealed record AiProviderCapabilities(bool SupportsImage,bool SupportsVideo,bool SupportsStreaming,long MaxImageSize,long MaxVideoSize,TimeSpan MaxVideoDuration,IReadOnlySet<string> AcceptedMimeTypes)
{
    public long MaxAttachmentSize=>Math.Max(MaxImageSize,MaxVideoSize);
    public long MaxSizeFor(AiAttachmentType type)=>type switch
    {
        AiAttachmentType.Image=>MaxImageSize,
        AiAttachmentType.Video=>MaxVideoSize,
        _=>0
    };
}
public sealed record AiResult(string Answer,IReadOnlyList<AiAnnotation> Annotations,string Reasoning="");
public sealed record VideoAnnotationKeyframe(double Time,double X,double Y,double Width,double Height);
public sealed record AiAnnotation(
    double X,
    double Y,
    double Width,
    double Height,
    string Text,
    int RegionIndex=0,
    double? StartTime=null,
    double? EndTime=null,
    IReadOnlyList<VideoAnnotationKeyframe>? Keyframes=null)
{
    public bool IsVideoTimeline=>StartTime.HasValue&&EndTime.HasValue&&Keyframes is {Count:>0};
}
