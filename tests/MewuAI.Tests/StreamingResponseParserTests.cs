using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using Xunit;

namespace MewuAI.Tests;

public sealed class StreamingResponseParserTests
{
    [Fact] public void ParsesOpenAiCompatibleDelta(){Assert.True(StreamingResponseParser.TryParse("data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}",out var delta,out var done));Assert.Equal("你好",delta.Content);Assert.Empty(delta.ReasoningContent);Assert.False(done);}
    [Fact] public void ParsesReasoningSeparately(){Assert.True(StreamingResponseParser.TryParse("data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先识别图片\"}}]}",out var delta,out _));Assert.Empty(delta.Content);Assert.Equal("先识别图片",delta.ReasoningContent);}
    [Fact] public void ParsesCumulativeReasoningDetails(){Assert.True(StreamingResponseParser.TryParse("data: {\"choices\":[{\"delta\":{\"reasoning_details\":[{\"type\":\"reasoning.text\",\"text\":\"先看\"},{\"text\":\"图片\"}]}}]}",out var delta,out _));Assert.Equal("先看图片",delta.ReasoningContent);Assert.True(delta.ReasoningIsCumulative);}
    [Fact] public void ExplicitReasoningContentTakesPriorityOverReasoningDetails(){Assert.True(StreamingResponseParser.TryParse("data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"增量\",\"reasoning_details\":[{\"text\":\"累计文本\"}]}}]}",out var delta,out _));Assert.Equal("增量",delta.ReasoningContent);Assert.False(delta.ReasoningIsCumulative);}
    [Fact] public void RecognizesDoneSentinel(){Assert.True(StreamingResponseParser.TryParse("data: [DONE]",out var delta,out var done));Assert.Empty(delta.Content);Assert.Empty(delta.ReasoningContent);Assert.True(done);}
    [Fact] public void RecognizesFinishReasonWithoutDoneSentinel(){Assert.True(StreamingResponseParser.TryParse("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}",out _,out var done));Assert.True(done);}
    [Fact] public void RecognizesFinishReasonWhenProviderOmitsFinalDelta(){Assert.True(StreamingResponseParser.TryParse("data: {\"choices\":[{\"finish_reason\":\"stop\"}]}",out var delta,out var done));Assert.True(done);Assert.Empty(delta.Content);Assert.Empty(delta.ReasoningContent);}
    [Theory]
    [InlineData("\"\"")]
    [InlineData("\"   \"")]
    [InlineData("0")]
    [InlineData("{}")]
    [InlineData("[]")]
    public void InvalidFinishReasonShapesDoNotCompleteTheStream(string finishReason)
    {
        Assert.True(StreamingResponseParser.TryParse($"data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":{finishReason}}}]}}",out _,out var done));
        Assert.False(done);
    }
    [Fact] public void IgnoresMalformedEvent(){Assert.False(StreamingResponseParser.TryParse("data: not-json",out _,out _));}

    [Fact]
    public void Accumulator_DeduplicatesMiniMaxCumulativeContentAndReportsOnlyNewText()
    {
        var streamed=new List<string>();
        var progress=new RecordingProgress(streamed);
        var accumulator=new StreamingResponseAccumulator(contentIsCumulative:true);

        accumulator.Accept(new AiStreamDelta("你",string.Empty),false,progress,null);
        accumulator.Accept(new AiStreamDelta("你好",string.Empty),false,progress,null);
        accumulator.Accept(new AiStreamDelta("你好！",string.Empty),true,progress,null);

        Assert.Equal("你好！",accumulator.BuildResult().Answer);
        Assert.Equal(new[]{"你","好","！"},streamed);
    }

    [Fact]
    public void Accumulator_PreservesOrdinaryIncrementalContent()
    {
        var accumulator=new StreamingResponseAccumulator();
        accumulator.Accept(new AiStreamDelta("你",string.Empty),false,null,null);
        accumulator.Accept(new AiStreamDelta("好",string.Empty),true,null,null);
        Assert.Equal("你好",accumulator.BuildResult().Answer);
    }

    [Fact]
    public void AccumulatorPreservesEarlierTextWhenCumulativeStreamRestartsAtSentenceBoundary()
    {
        var streamed=new List<string>();
        var progress=new RecordingProgress(streamed);
        var accumulator=new StreamingResponseAccumulator(contentIsCumulative:true);
        accumulator.Accept(new AiStreamDelta("一只兔子躺在草地上",string.Empty),false,progress,null);
        accumulator.Accept(new AiStreamDelta("，看着粉色的蝴蝶",string.Empty),false,progress,null);
        accumulator.Accept(new AiStreamDelta("，看着粉色的蝴蝶飞到头上。",string.Empty),true,progress,null);

        Assert.Equal("一只兔子躺在草地上，看着粉色的蝴蝶飞到头上。",accumulator.BuildResult().Answer);
        Assert.Equal(new[]{"一只兔子躺在草地上","，看着粉色的蝴蝶","飞到头上。"},streamed);
    }

    [Fact]
    public void AccumulatorRemovesSuffixPrefixOverlapWhenCumulativeStreamRestarts()
    {
        var streamed=new List<string>();
        var accumulator=new StreamingResponseAccumulator(contentIsCumulative:true);
        accumulator.Accept(new AiStreamDelta("你好世界",string.Empty),false,new RecordingProgress(streamed),null);
        accumulator.Accept(new AiStreamDelta("世界很美",string.Empty),true,new RecordingProgress(streamed),null);

        Assert.Equal("你好世界很美",accumulator.BuildResult().Answer);
        Assert.Equal(new[]{"你好世界","很美"},streamed);
    }

    private sealed class RecordingProgress(List<string> values):IProgress<AiStreamDelta>
    {
        public void Report(AiStreamDelta value)=>values.Add(value.Content);
    }
}
