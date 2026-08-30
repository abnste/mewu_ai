using mewu_ai_Assistant.AI;
using Xunit;
namespace MewuAI.Tests;
public sealed class StructuredResponseParserTests
{
    [Fact] public void Parse_ValidNormalizedAnnotation(){var r=StructuredResponseParser.Parse("```json\n{\"answer\":\"说明\",\"annotations\":[{\"x\":0.1,\"y\":0.2,\"width\":0.3,\"height\":0.2,\"text\":\"重点\",\"type\":\"note\"}]}\n```");Assert.Equal("说明",r.Answer);Assert.Single(r.Annotations);}
    [Fact] public void Parse_PreservesMultiRegionIndex(){var r=StructuredResponseParser.Parse("{\"answer\":\"说明\",\"annotations\":[{\"regionIndex\":2,\"x\":0.1,\"y\":0.2,\"width\":0.3,\"height\":0.2,\"text\":\"第三个区域\",\"type\":\"note\"}]}");Assert.Equal(2,Assert.Single(r.Annotations).RegionIndex);}
    [Fact] public void Parse_MalformedJsonFallsBack(){const string value="{not json";var r=StructuredResponseParser.Parse(value);Assert.Equal(value,r.Answer);Assert.Empty(r.Annotations);}
    [Fact] public void Parse_DropsOutOfBoundsAnnotation(){var r=StructuredResponseParser.Parse("{\"answer\":\"ok\",\"annotations\":[{\"x\":.9,\"y\":.2,\"width\":.5,\"height\":.2,\"text\":\"bad\",\"type\":\"note\"}]}");Assert.Empty(r.Annotations);}
    [Fact] public void Parse_SeparatesThinkTagsFromAnswer(){var r=StructuredResponseParser.Parse("<think>先识别主体，再比较区域</think>{\"answer\":\"两处内容不同\",\"annotations\":[]}");Assert.Equal("两处内容不同",r.Answer);Assert.Equal("先识别主体，再比较区域",r.Reasoning);Assert.DoesNotContain("think",r.Answer,StringComparison.OrdinalIgnoreCase);}
    [Fact] public void Parse_PreservesDedicatedReasoningField(){var r=StructuredResponseParser.Parse("{\"answer\":\"完成\",\"annotations\":[]}","独立思考内容");Assert.Equal("独立思考内容",r.Reasoning);}
}
