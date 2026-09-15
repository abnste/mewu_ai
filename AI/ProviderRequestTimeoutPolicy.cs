// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.AI;

internal static class ProviderRequestTimeoutPolicy
{
    // Large reasoning models can legitimately spend several minutes before
    // emitting the final structured answer. Keep a generous total deadline,
    // while the transport layer separately detects a stalled connection.
    internal static readonly TimeSpan Standard=TimeSpan.FromMinutes(10);
    internal static readonly TimeSpan WithVideo=TimeSpan.FromMinutes(15);
    internal static readonly TimeSpan FirstResponse=TimeSpan.FromSeconds(90);
    internal static readonly TimeSpan InterChunk=TimeSpan.FromSeconds(120);

    internal static TimeSpan For(AiRequest request)
    {
        ArgumentNullException.ThrowIfNull(request);
        return request.Attachments?.Any(attachment=>attachment is not null&&attachment.Type==AiAttachmentType.Video)==true
            ?WithVideo
            :Standard;
    }
}
