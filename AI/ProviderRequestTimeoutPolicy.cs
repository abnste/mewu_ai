using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.AI;

internal static class ProviderRequestTimeoutPolicy
{
    internal static readonly TimeSpan Standard=TimeSpan.FromMinutes(5);
    internal static readonly TimeSpan WithVideo=TimeSpan.FromMinutes(10);

    internal static TimeSpan For(AiRequest request)
    {
        ArgumentNullException.ThrowIfNull(request);
        return request.Attachments?.Any(attachment=>attachment is not null&&attachment.Type==AiAttachmentType.Video)==true
            ?WithVideo
            :Standard;
    }
}
