// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;

internal static class ConversationArchivePolicy
{
    internal static (string Provider,string Model) Scope(ConversationChannel channel,AppSettings settings)
        =>channel.Kind switch
        {
            ConversationChannelKind.WorkBuddy=>("WorkBuddy",channel.Model),
            ConversationChannelKind.MiniMaxCode=>("MiniMax Code",channel.Model),
            ConversationChannelKind.Codex=>("ChatGPT Work · Codex",channel.Model),
            ConversationChannelKind.Hermes=>($"本机 Hermes · {settings.HermesProfile}",channel.Model),
            _=>(settings.Providers.FirstOrDefault(p=>p.Id==channel.ProviderId)?.Name??channel.ProviderId,channel.Model)
        };

    internal static bool Matches(ConversationSessionArchive session,ConversationChannel channel,AppSettings settings)
    {
        var scope=Scope(channel,settings);
        return scope.Provider==session.Provider&&scope.Model==session.Model;
    }

    internal static IReadOnlyList<ConversationHistoryEntry> Entries(ConversationSessionArchive session)
        =>session.Entries.Where(e=>e.Provider==session.Provider&&e.Model==session.Model&&
            string.Equals(e.SessionId?.Trim()??string.Empty,session.Id,StringComparison.Ordinal)).TakeLast(24).ToArray();
}
