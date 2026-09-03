using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal readonly record struct ConversationHistoryPair(string Prompt,string Answer);

internal static class ConversationHistoryPairing
{
    internal static IReadOnlyList<ConversationHistoryPair> Pair(IEnumerable<AiMessage> messages)
    {
        ArgumentNullException.ThrowIfNull(messages);
        var pairs=new List<ConversationHistoryPair>();
        AiMessage? pendingPrompt=null;
        foreach(var message in messages)
        {
            if(message is null)continue;
            if(string.Equals(message.Role,"user",StringComparison.OrdinalIgnoreCase))
            {
                pendingPrompt=message;
                continue;
            }
            if(!string.Equals(message.Role,"assistant",StringComparison.OrdinalIgnoreCase)||pendingPrompt is null)continue;
            pairs.Add(new ConversationHistoryPair(pendingPrompt.Text,message.Text));
            pendingPrompt=null;
        }
        return pairs;
    }
}
