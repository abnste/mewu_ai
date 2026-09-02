using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class VideoAnnotationActionLabelFormatterTests
{
    [Theory]
    [InlineData("红色椭圆圈出的文字：'有且仅有一次'","圈出的文字")]
    [InlineData("红色下划线标出的文字：'第二次，请家长到…'","下划线文字")]
    [InlineData("街坊手中的手机（黑色）","黑色手机")]
    public void CondenseSubject_RemovesVerboseVisualDescription(string input,string expected)=>Assert.Equal(expected,VideoAnnotationActionLabelFormatter.CondenseSubject(input));

    [Fact]
    public void Create_UsesCompactChineseTimeBubbleLabel()
    {
        var annotation=new AiAnnotation(.1,.1,.2,.2,"街坊手中的手机（黑色）",0,66,66,[new VideoAnnotationKeyframe(66,.1,.1,.2,.2)]);Assert.Equal("1分6秒：黑色手机",VideoAnnotationActionLabelFormatter.Create(annotation,VideoAnnotationAnswerActionKind.JumpToFrame));
    }

    [Fact]
    public void FormatCompactTime_PreservesTenthsNeededForPreciseSeek()=>Assert.Equal("0.6秒",VideoAnnotationActionLabelFormatter.FormatCompactTime(TimeSpan.FromSeconds(.6)));
}
