using mewu_ai_Assistant.AI;
using Xunit;

namespace MewuAI.Tests;

public sealed class TranslationResponseParserTests
{
    [Fact] public void ParsesObjectAndPreservesLineOrder(){Assert.True(TranslationResponseParser.TryParse("{\"translations\":[\"第一行\",\"第二行\"]}",2,out var values));Assert.Equal(["第一行","第二行"],values);}
    [Fact] public void ParsesMarkdownFencedJson(){Assert.True(TranslationResponseParser.TryParse("```json\n{\"translations\":[\"译文\"]}\n```",1,out var values));Assert.Equal("译文",Assert.Single(values));}
    [Fact] public void RejectsWrongLineCount(){Assert.False(TranslationResponseParser.TryParse("{\"translations\":[\"只有一行\"]}",2,out _));}
}
