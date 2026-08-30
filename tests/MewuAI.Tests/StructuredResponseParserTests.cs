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
    [Fact] public void Parse_ValidStructuredResponseWithEmptyAnswerStaysEmpty(){var r=StructuredResponseParser.Parse("{\"answer\":\"\",\"annotations\":[]}","只有思考");Assert.Empty(r.Answer);Assert.Equal("只有思考",r.Reasoning);}
    [Fact] public void Parse_PreservesOrdinaryAnswerContainingJson(){const string value="依赖如下：{\"dependencies\":{\"x\":\"1\"}}";Assert.Equal(value,StructuredResponseParser.Parse(value).Answer);}
    [Fact] public void Parse_AllowsNullAnnotations(){var r=StructuredResponseParser.Parse("{\"answer\":\"正文\",\"annotations\":null}");Assert.Equal("正文",r.Answer);Assert.Empty(r.Annotations);}
    [Fact] public void Parse_IgnoresNullAnnotationItems(){var r=StructuredResponseParser.Parse("{\"answer\":\"正文\",\"annotations\":[null]}");Assert.Equal("正文",r.Answer);Assert.Empty(r.Annotations);}
    [Fact] public void Parse_RecoversAnswerFromTruncatedFencedResponseWithUnescapedQuotes()
    {
        const string value = """
            ```json
            {
              "answer": "视频先出现一只名为"大白兔"的白兔。\n随后出现\u8774\u8776，路径是 C:\\Temp\\clip.mp4，并标注为 \"结束\"。",
              "annotations": [
                {"x":0.1,"y":0.2,"width":0.3
            ```
            """;
        var result = StructuredResponseParser.Parse(value);
        Assert.Equal("视频先出现一只名为\"大白兔\"的白兔。\n随后出现蝴蝶，路径是 C:\\Temp\\clip.mp4，并标注为 \"结束\"。", result.Answer);
        Assert.Empty(result.Annotations);
    }
    [Fact] public void Parse_RecoversAnswerFromTruncatedPlainRootObject(){var r=StructuredResponseParser.Parse("{\"answer\":\"一只叫\"雪球\"的白兔\",\"annotations\":[{\"x\":0.1");Assert.Equal("一只叫\"雪球\"的白兔",r.Answer);Assert.Empty(r.Annotations);}
    [Fact] public void Parse_PreservesProseContainingStructuredJson(){const string value="这是格式示例：{\"answer\":\"示例回答\",\"annotations\":[]}";Assert.Equal(value,StructuredResponseParser.Parse(value).Answer);}
    [Fact] public void Parse_PreservesCompleteStructuredJsonFollowedByProse(){const string value="{\"answer\":\"示例回答\",\"annotations\":[]} 这只是正文里的示例";Assert.Equal(value,StructuredResponseParser.Parse(value).Answer);}
    [Fact] public void Parse_DoesNotRecoverNestedAnswerField(){const string value="{\"payload\":{\"answer\":\"内层回答\"},\"annotations\":[";Assert.Equal(value,StructuredResponseParser.Parse(value).Answer);}
    [Fact] public void Parse_DoesNotRecoverMalformedRootWithoutAnnotationsField(){const string value="{\"answer\":\"一只叫\"雪球\"的白兔\"";Assert.Equal(value,StructuredResponseParser.Parse(value).Answer);}
    [Fact] public void EmptyAnswerValidation_DistinguishesReasoningOnly(){Assert.Equal("AI 未返回有效正文，请重试",AiResultValidation.GetEmptyAnswerMessage(new(" ",[])));Assert.Equal("模型只返回了思考内容，未返回最终回答，请重试",AiResultValidation.GetEmptyAnswerMessage(new("",[],"推理")));Assert.Null(AiResultValidation.GetEmptyAnswerMessage(new("正文",[])));}
}
