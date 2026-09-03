using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ConversationHistoryPairingTests
{
    [Fact]
    public void PairsEachAdjacentUserAndAssistantTurn()
    {
        var result=ConversationHistoryPairing.Pair([
            new AiMessage("system","instructions"),
            new AiMessage("user","问题一"),
            new AiMessage("assistant","回答一"),
            new AiMessage("user","问题二"),
            new AiMessage("assistant","回答二")
        ]);

        Assert.Equal(2,result.Count);
        Assert.Equal("问题一",result[0].Prompt);
        Assert.Equal("回答一",result[0].Answer);
        Assert.Equal("问题二",result[1].Prompt);
        Assert.Equal("回答二",result[1].Answer);
    }

    [Fact]
    public void IgnoresOrphanMessagesInsteadOfRenderingSeparateCards()
    {
        var result=ConversationHistoryPairing.Pair([
            new AiMessage("assistant","没有对应问题"),
            new AiMessage("user","未完成的问题"),
            new AiMessage("user","问题"),
            new AiMessage("assistant","回答")
        ]);

        var pair=Assert.Single(result);
        Assert.Equal("问题",pair.Prompt);
        Assert.Equal("回答",pair.Answer);
    }
}
