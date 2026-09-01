using mewu_ai_Assistant.AI;
using Xunit;
namespace MewuAI.Tests;
public sealed class StructuredResponseParserTests
{
    [Fact]
    public void StreamingPreviewOnlyShowsDecodedRootAnswerAcrossEveryChunkBoundary()
    {
        const string json="{\"meta\":{\"answer\":\"不能显示\"},\"answer\":\"第一行\\n引号：\\\"好\\\"，表情：\\uD83D\\uDE00\",\"annotations\":[]}";
        const string expected="第一行\n引号：\"好\"，表情：😀";
        for(var length=1;length<=json.Length;length++)
        {
            var preview=StructuredResponseParser.GetStreamingAnswerPreview(json[..length]);
            Assert.StartsWith(preview,expected,StringComparison.Ordinal);
            Assert.DoesNotContain("不能显示",preview,StringComparison.Ordinal);
            Assert.DoesNotContain("annotations",preview,StringComparison.Ordinal);
        }
        Assert.Equal(expected,StructuredResponseParser.GetStreamingAnswerPreview(json));
    }

    [Fact]
    public void StreamingPreviewSuppressesThinkBlocksAndMarkdownProtocolShell()
    {
        const string stream="```json\n{\"answer\":\"<think>内部推理不能显示</think>可见答案\",\"annotations\":[]}```";
        for(var length=1;length<=stream.Length;length++)
        {
            var preview=StructuredResponseParser.GetStreamingAnswerPreview(stream[..length]);
            Assert.DoesNotContain("内部推理",preview,StringComparison.Ordinal);
            Assert.DoesNotContain("<think",preview,StringComparison.OrdinalIgnoreCase);
            Assert.DoesNotContain("answer",preview,StringComparison.Ordinal);
        }
        Assert.Equal("可见答案",StructuredResponseParser.GetStreamingAnswerPreview(stream));
    }
    [Fact] public void StreamingTextPreviewShowsPlainTextWithoutThinkContent(){Assert.Equal("可见回答",StructuredResponseParser.GetStreamingTextPreview("<think>内部推理</think>可见回答"));Assert.Empty(StructuredResponseParser.GetStreamingTextPreview("<think>仍在推理"));}
    [Fact] public void StreamingTextPreviewKeepsStructuredProtocolHidden(){Assert.Equal("尚未结束",StructuredResponseParser.GetStreamingTextPreview("{\"answer\":\"尚未结束"));Assert.Equal("回答",StructuredResponseParser.GetStreamingTextPreview("{\"answer\":\"回答\",\"annotations\":[]}"));}
    [Fact] public void StreamingTextPreviewPreservesNonJsonCodeFence(){const string value="```csharp\nvar answer = 42;";Assert.Equal(value,StructuredResponseParser.GetStreamingTextPreview(value));}
    [Fact] public void Parse_ValidNormalizedAnnotation(){var r=StructuredResponseParser.Parse("```json\n{\"answer\":\"说明\",\"annotations\":[{\"x\":0.1,\"y\":0.2,\"width\":0.3,\"height\":0.2,\"text\":\"重点\",\"type\":\"note\"}]}\n```");Assert.Equal("说明",r.Answer);Assert.Single(r.Annotations);}
    [Theory]
    [InlineData("```json{\"answer\":\"说明\",\"annotations\":[{\"x\":0.1,\"y\":0.2,\"width\":0.3,\"height\":0.2,\"text\":\"重点\"}]}```")]
    [InlineData("```JSON {\"answer\":\"说明\",\"annotations\":[{\"x\":0.1,\"y\":0.2,\"width\":0.3,\"height\":0.2,\"text\":\"重点\"}]} ```")]
    [InlineData("``` {\"answer\":\"说明\",\"annotations\":[{\"x\":0.1,\"y\":0.2,\"width\":0.3,\"height\":0.2,\"text\":\"重点\"}]} ```")]
    public void Parse_AcceptsSameLineJsonFencesWithoutLeakingProtocol(string value)
    {
        var result=StructuredResponseParser.Parse(value);
        Assert.Equal("说明",result.Answer);
        Assert.Single(result.Annotations);
        Assert.DoesNotContain("```",result.Answer,StringComparison.Ordinal);
    }

    [Fact]
    public void StreamingPreviewAcceptsSameLineJsonFence()
    {
        const string value="```json{\"answer\":\"实时说明\",\"annotations\":[]}```";
        Assert.Equal("实时说明",StructuredResponseParser.GetStreamingAnswerPreview(value));
    }
    [Fact] public void Parse_PreservesMultiRegionIndex(){var r=StructuredResponseParser.Parse("{\"answer\":\"说明\",\"annotations\":[{\"regionIndex\":2,\"x\":0.1,\"y\":0.2,\"width\":0.3,\"height\":0.2,\"text\":\"第三个区域\",\"type\":\"note\"}]}");Assert.Equal(2,Assert.Single(r.Annotations).RegionIndex);}
    [Fact] public void Parse_MalformedJsonFallsBack(){const string value="{not json";var r=StructuredResponseParser.Parse(value);Assert.Equal(value,r.Answer);Assert.Empty(r.Annotations);}
    [Fact] public void Parse_DropsOutOfBoundsAnnotation(){var r=StructuredResponseParser.Parse("{\"answer\":\"ok\",\"annotations\":[{\"x\":.9,\"y\":.2,\"width\":.5,\"height\":.2,\"text\":\"bad\",\"type\":\"note\"}]}");Assert.Empty(r.Annotations);}
    [Fact] public void Parse_SeparatesThinkTagsFromAnswer(){var r=StructuredResponseParser.Parse("<think>先识别主体，再比较区域</think>{\"answer\":\"两处内容不同\",\"annotations\":[]}");Assert.Equal("两处内容不同",r.Answer);Assert.Equal("先识别主体，再比较区域",r.Reasoning);Assert.DoesNotContain("think",r.Answer,StringComparison.OrdinalIgnoreCase);}
    [Fact] public void Parse_PreservesDedicatedReasoningField(){var r=StructuredResponseParser.Parse("{\"answer\":\"完成\",\"annotations\":[]}","独立思考内容");Assert.Equal("独立思考内容",r.Reasoning);}
    [Fact] public void Parse_ValidStructuredResponseWithEmptyAnswerStaysEmpty(){var r=StructuredResponseParser.Parse("{\"answer\":\"\",\"annotations\":[]}","只有思考");Assert.Empty(r.Answer);Assert.Equal("只有思考",r.Reasoning);}
    [Fact] public void Parse_PreservesOrdinaryAnswerContainingJson(){const string value="依赖如下：{\"dependencies\":{\"x\":\"1\"}}";Assert.Equal(value,StructuredResponseParser.Parse(value).Answer);}
    [Fact] public void Parse_AllowsNullAnnotations(){var r=StructuredResponseParser.Parse("{\"answer\":\"正文\",\"annotations\":null}");Assert.Equal("正文",r.Answer);Assert.Empty(r.Annotations);}
    [Fact] public void Parse_IgnoresNullAnnotationItems(){var r=StructuredResponseParser.Parse("{\"answer\":\"正文\",\"annotations\":[null]}");Assert.Equal("正文",r.Answer);Assert.Empty(r.Annotations);}
    [Theory]
    [InlineData("null")]
    [InlineData("{}")]
    [InlineData("\"错误类型\"")]
    [InlineData("42")]
    [InlineData("true")]
    public void Parse_IgnoresAnnotationsWithWrongRootType(string annotations)
    {
        var result=StructuredResponseParser.Parse($"{{\"answer\":\"正文\",\"annotations\":{annotations}}}");
        Assert.Equal("正文",result.Answer);
        Assert.Empty(result.Annotations);
    }
    [Fact]
    public void Parse_IgnoresBadAnnotationItemsButKeepsValidSiblings()
    {
        const string value="""
            {"answer":"正文","annotations":[
              null,
              "错误类型",
              {"x":"0.1","y":0.2,"width":0.3,"height":0.2,"text":"错误坐标"},
              {"x":0.1,"y":0.2,"width":0.3,"height":0.2,"text":"有效","regionIndex":2},
              {"x":0.1,"y":0.2,"width":0.3,"height":0.2,"text":"错误区域","regionIndex":"2"}
            ]}
            """;
        var result=StructuredResponseParser.Parse(value);
        var annotation=Assert.Single(result.Annotations);
        Assert.Equal("正文",result.Answer);
        Assert.Equal("有效",annotation.Text);
        Assert.Equal(2,annotation.RegionIndex);
    }
    [Fact]
    public void Parse_PreservesCompleteRootJsonWithoutAnswerAsOrdinaryText()
    {
        const string value="{\"message\":\"这是普通 JSON\",\"annotations\":[]}";
        var result=StructuredResponseParser.Parse(value);
        Assert.Equal(value,result.Answer);
        Assert.Empty(result.Annotations);
    }
    [Theory]
    [InlineData("{\"message\":\"协议字段错误\",\"annotations\":[]}")]
    [InlineData("[ {\"answer\":\"错误根类型\"} ]")]
    [InlineData("```json\n{not valid}\n```")]
    [InlineData("json\n{\"result\":\"错误字段\"}")]
    public void Parse_ExpectedStructuredResponseNeverLeaksBrokenProtocol(string value)
    {
        var result=StructuredResponseParser.Parse(value,expectStructuredResponse:true);
        Assert.Empty(result.Answer);
        Assert.Empty(result.Annotations);
    }
    [Fact]
    public void Parse_ExpectedStructuredResponseStillAllowsPlainProseFallback()
    {
        const string value="模型直接返回了普通说明";
        Assert.Equal(value,StructuredResponseParser.Parse(value,expectStructuredResponse:true).Answer);
    }
    [Theory]
    [InlineData("null")]
    [InlineData("42")]
    [InlineData("true")]
    [InlineData("{}")]
    [InlineData("[]")]
    public void Parse_CompleteRootJsonWithNonStringAnswerHasEmptyBody(string answer)
    {
        var result=StructuredResponseParser.Parse($"{{\"answer\":{answer},\"annotations\":[{{\"x\":0.1,\"y\":0.2,\"width\":0.3,\"height\":0.2,\"text\":\"不应渲染\"}}]}}");
        Assert.Empty(result.Answer);
        Assert.Empty(result.Annotations);
    }
    [Fact] public void Parse_DropsBlankOrNegativeRegionAnnotations(){var r=StructuredResponseParser.Parse("{\"answer\":\"正文\",\"annotations\":[{\"regionIndex\":0,\"x\":0.1,\"y\":0.1,\"width\":0.2,\"height\":0.2,\"text\":\" \"},{\"regionIndex\":-1,\"x\":0.1,\"y\":0.1,\"width\":0.2,\"height\":0.2,\"text\":\"无效\"}]}");Assert.Empty(r.Annotations);}
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
