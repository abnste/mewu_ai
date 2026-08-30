using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ConversationContextPolicyTests
{
    [Fact]
    public void LongConversationKeepsSystemAndRecentCompletePairsWithinBudget()
    {
        var history=new List<AiMessage>{new("system","固定系统规则")};
        for(var index=0;index<50;index++)
        {
            history.Add(new("user",$"问题-{index}-"+new string('问',1800)));
            history.Add(new("assistant",$"回答-{index}-"+new string('答',1800)));
        }

        var bounded=ConversationContextPolicy.CreateBoundedHistory(history);

        Assert.Equal("system",bounded[0].Role);
        Assert.InRange(bounded.Count,3,ConversationContextPolicy.MaxConversationMessages);
        Assert.True(bounded.Sum(message=>message.Text.Length)<=ConversationContextPolicy.MaxHistoryCharacters);
        Assert.Contains("问题-49-",bounded[^2].Text,StringComparison.Ordinal);
        Assert.Contains("回答-49-",bounded[^1].Text,StringComparison.Ordinal);
        for(var index=1;index<bounded.Count;index+=2)
        {
            Assert.Equal("user",bounded[index].Role);
            Assert.Equal("assistant",bounded[index+1].Role);
        }
    }

    [Fact]
    public void MalformedOrphanMessagesAreNotForwarded()
    {
        var history=new[]{new AiMessage("assistant","orphan"),new AiMessage("user","kept"),new AiMessage("assistant","reply"),new AiMessage("user","unfinished")};
        var bounded=ConversationContextPolicy.CreateBoundedHistory(history);
        Assert.Equal(new[]{"kept","reply"},bounded.Select(message=>message.Text));
    }

    [Fact]
    public void NullAndMalformedMessagesAreSkippedWithoutBreakingLaterPairs()
    {
        AiMessage[] history=[null!,new("user","kept"),new("assistant","reply"),null!];
        var bounded=ConversationContextPolicy.CreateBoundedHistory(history);
        Assert.Equal(new[]{"kept","reply"},bounded.Select(message=>message.Text));
    }

    [Fact]
    public void ProviderValidationRejectsOrphanedOrMisorderedHistory()
    {
        var orphaned=new[]{new AiMessage("user","unfinished")};
        var misplacedSystem=new[]{new AiMessage("user","question"),new AiMessage("assistant","answer"),new AiMessage("system","late")};

        Assert.Throws<InvalidOperationException>(()=>ConversationContextPolicy.EnsureValidForProvider(orphaned));
        Assert.Throws<InvalidOperationException>(()=>ConversationContextPolicy.EnsureValidForProvider(misplacedSystem));
    }

    [Fact]
    public void ProviderValidationRejectsMessageAndCharacterBudgets()
    {
        var tooMany=Enumerable.Range(0,ConversationContextPolicy.MaxConversationMessages/2+1)
            .SelectMany(index=>new[]{new AiMessage("user",$"question-{index}"),new AiMessage("assistant",$"answer-{index}")})
            .ToArray();
        var tooLong=new[]{
            new AiMessage("user",new string('问',ConversationContextPolicy.MaxHistoryCharacters)),
            new AiMessage("assistant","超出一个字符")
        };

        Assert.Throws<InvalidOperationException>(()=>ConversationContextPolicy.EnsureValidForProvider(tooMany));
        Assert.Throws<InvalidOperationException>(()=>ConversationContextPolicy.EnsureValidForProvider(tooLong));
    }

    [Fact]
    public void ProviderValidationAcceptsOneSystemAndCompletePairsAtExactLimits()
    {
        var history=new List<AiMessage>{new("system",string.Empty)};
        for(var index=0;index<9;index++)
        {
            history.Add(new("user",index==0?new string('问',ConversationContextPolicy.MaxHistoryCharacters):string.Empty));
            history.Add(new("assistant",string.Empty));
        }

        ConversationContextPolicy.EnsureValidForProvider(history);
    }
}
