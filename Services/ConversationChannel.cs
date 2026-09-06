// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
namespace mewu_ai_Assistant.Services;

internal enum ConversationChannelKind
{
    Api,
    Hermes,
    Codex,
    WorkBuddy
}

/// <summary>A concrete, currently usable route for one screen-assistant turn.</summary>
internal sealed record ConversationChannel(
    string Id,
    string DisplayName,
    string ProviderId,
    string Model,
    ConversationChannelKind Kind,
    bool SupportsImage,
    bool SupportsVideo)
{
    public override string ToString() => DisplayName;
}
