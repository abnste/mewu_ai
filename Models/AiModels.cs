using System.Text.Json.Serialization;
namespace mewu_ai_Assistant.Models;
public enum AiAttachmentType { Image,Video }
public sealed record AiAttachment(AiAttachmentType Type,string MimeType,byte[]? Data=null,string? FilePath=null,TimeSpan? Duration=null);
public sealed record AiMessage(string Role,string Text);
public sealed record AiStreamDelta(string Content,string ReasoningContent);
public sealed class AiRequest { public string Prompt { get; init; }=string.Empty; public List<AiMessage> History { get; init; }=[]; public List<AiAttachment> Attachments { get; init; }=[]; public IProgress<AiStreamDelta>? StreamingProgress { get; init; } }
public sealed record AiProviderCapabilities(bool SupportsText,bool SupportsImage,bool SupportsVideo,bool SupportsStreaming,bool SupportsStructuredOutput,long MaxAttachmentSize,TimeSpan MaxVideoDuration,IReadOnlySet<string> AcceptedMimeTypes);
public sealed record AiResult(string Answer,IReadOnlyList<AiAnnotation> Annotations,string Reasoning="");
public sealed record AiAnnotation(double X,double Y,double Width,double Height,string Text,string Type,int RegionIndex=0);
public sealed class StructuredAiResponse
{
    [JsonPropertyName("answer")] public string Answer { get; set; }=string.Empty;
    [JsonPropertyName("annotations")] public List<AiAnnotationDto> Annotations { get; set; }=[];
}
public sealed class AiAnnotationDto
{
    [JsonPropertyName("x")] public double X { get; set; } [JsonPropertyName("y")] public double Y { get; set; }
    [JsonPropertyName("width")] public double Width { get; set; } [JsonPropertyName("height")] public double Height { get; set; }
    [JsonPropertyName("text")] public string Text { get; set; }=string.Empty; [JsonPropertyName("type")] public string Type { get; set; }="note";
    [JsonPropertyName("regionIndex")] public int RegionIndex { get; set; }
}
