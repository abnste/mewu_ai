using mewu_ai_Assistant.AI;
using Xunit;

namespace MewuAI.Tests;

public sealed class StreamingResponseParserTests
{
    [Fact] public void ParsesOpenAiCompatibleDelta(){Assert.True(StreamingResponseParser.TryParse("data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}",out var delta,out var done));Assert.Equal("你好",delta);Assert.False(done);}
    [Fact] public void RecognizesDoneSentinel(){Assert.True(StreamingResponseParser.TryParse("data: [DONE]",out var delta,out var done));Assert.Empty(delta);Assert.True(done);}
    [Fact] public void IgnoresMalformedEvent(){Assert.False(StreamingResponseParser.TryParse("data: not-json",out _,out _));}
}
