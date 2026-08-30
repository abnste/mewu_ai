using mewu_ai_Assistant.AI;
using Xunit;

namespace MewuAI.Tests;

public sealed class StreamingResponseParserTests
{
    [Fact] public void ParsesOpenAiCompatibleDelta(){Assert.True(StreamingResponseParser.TryParse("data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}",out var delta,out var done));Assert.Equal("你好",delta.Content);Assert.Empty(delta.ReasoningContent);Assert.False(done);}
    [Fact] public void ParsesReasoningSeparately(){Assert.True(StreamingResponseParser.TryParse("data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先识别图片\"}}]}",out var delta,out _));Assert.Empty(delta.Content);Assert.Equal("先识别图片",delta.ReasoningContent);}
    [Fact] public void RecognizesDoneSentinel(){Assert.True(StreamingResponseParser.TryParse("data: [DONE]",out var delta,out var done));Assert.Empty(delta.Content);Assert.Empty(delta.ReasoningContent);Assert.True(done);}
    [Fact] public void IgnoresMalformedEvent(){Assert.False(StreamingResponseParser.TryParse("data: not-json",out _,out _));}
}
