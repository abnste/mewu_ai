// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.AI;
public interface IAiProvider
{
    string Id { get; } AiProviderCapabilities Capabilities { get; }
    Task<AiResult> SendAsync(AiRequest request,CancellationToken cancellationToken);
    Task<bool> TestConnectionAsync(CancellationToken cancellationToken);
}

/// <summary>Optional contract for providers which keep conversation state outside the
/// request payload (for example a persistent local agent session). The overlay's
/// explicit new-conversation action uses it to open a real context boundary
/// instead of only clearing the visible prompt history.</summary>
public interface IConversationSessionReset
{
    /// <summary>Drops the remembered conversation state so the next turn starts a
    /// blank session. Returns false when a turn is still executing and the state
    /// could not be reset; callers must keep the current conversation in that case.</summary>
    bool TryResetSession();
}
