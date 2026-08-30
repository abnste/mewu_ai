using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class ConversationContextPolicy
{
    internal const int MaxHistoryCharacters=24_000;
    internal const int MaxConversationMessages=20;
    private const int MaxSystemCharacters=4_096;
    private const int MaxMessageCharacters=8_192;

    internal static IReadOnlyList<AiMessage> CreateBoundedHistory(IReadOnlyList<AiMessage> history)
    {
        ArgumentNullException.ThrowIfNull(history);
        var system=history
            .TakeWhile(message=>HasRole(message,"system"))
            .Take(1)
            .Where(message=>message is not null)
            .Select(message=>new AiMessage(message!.Role,TruncateMiddle(message.Text,MaxSystemCharacters)))
            .ToList();
        var firstConversationIndex=system.Count==0?0:history.TakeWhile(message=>HasRole(message,"system")).Count();
        var pairs=new List<(AiMessage User,AiMessage Assistant)>();
        for(var index=firstConversationIndex;index+1<history.Count;index++)
        {
            var user=history[index];var assistant=history[index+1];
            if(!HasRole(user,"user")||!HasRole(assistant,"assistant"))continue;
            pairs.Add((new AiMessage(user!.Role,TruncateMiddle(user.Text,MaxMessageCharacters)),new AiMessage(assistant!.Role,TruncateMiddle(assistant.Text,MaxMessageCharacters))));
            index++;
        }

        var usedCharacters=system.Sum(message=>message.Text.Length);
        var availableMessages=MaxConversationMessages-system.Count;
        var selected=new List<(AiMessage User,AiMessage Assistant)>();
        for(var index=pairs.Count-1;index>=0&&selected.Count*2+2<=availableMessages;index--)
        {
            var pair=pairs[index];var pairCharacters=pair.User.Text.Length+pair.Assistant.Text.Length;
            if(usedCharacters+pairCharacters>MaxHistoryCharacters)continue;
            selected.Add(pair);usedCharacters+=pairCharacters;
        }
        selected.Reverse();

        var result=new List<AiMessage>(system.Count+selected.Count*2);result.AddRange(system);
        foreach(var pair in selected){result.Add(pair.User);result.Add(pair.Assistant);}
        return result;
    }

    internal static void EnsureValidForProvider(IReadOnlyList<AiMessage> history)
    {
        ArgumentNullException.ThrowIfNull(history);
        if(history.Count>MaxConversationMessages)
            throw new InvalidOperationException($"发送给 Provider 的历史消息不能超过 {MaxConversationMessages} 条");

        long characters=0;
        foreach(var message in history)
        {
            if(message is null)throw new InvalidOperationException("对话历史包含空项");
            if(message.Role is null)throw new InvalidOperationException("对话历史角色不能为空");
            if(message.Text is null)throw new InvalidOperationException("对话历史正文不能为空");
            characters+=message.Text.Length;
            if(characters>MaxHistoryCharacters)
                throw new InvalidOperationException($"发送给 Provider 的历史正文不能超过 {MaxHistoryCharacters:N0} 个 UTF-16 字符");
        }

        var index=history.Count>0&&HasRole(history[0],"system")?1:0;
        for(;index<history.Count;index+=2)
        {
            if(index+1>=history.Count||!HasRole(history[index],"user")||!HasRole(history[index+1],"assistant"))
                throw new InvalidOperationException("Provider 历史只能包含至多一条开头 system 消息，以及完整的 user—assistant 问答对");
        }
    }

    internal static void TrimInPlace(List<AiMessage> history)
    {
        var bounded=CreateBoundedHistory(history);history.Clear();history.AddRange(bounded);
    }

    private static string TruncateMiddle(string? value,int limit)
    {
        value??=string.Empty;if(value.Length<=limit)return value;
        const string marker="\n…[较早内容已裁剪]…\n";var remaining=limit-marker.Length;var head=remaining/2;var tail=remaining-head;
        return string.Concat(value.AsSpan(0,head),marker,value.AsSpan(value.Length-tail));
    }

    private static bool HasRole(AiMessage? message,string role)=>
        message is not null&&string.Equals(message.Role,role,StringComparison.OrdinalIgnoreCase);
}
