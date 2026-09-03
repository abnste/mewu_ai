using System.Text;
using System.Text.RegularExpressions;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class TableClipboardServiceTests
{
    [Fact]
    public void ParsesMarkdownTableCellsWithoutFormattingSyntax()
    {
        const string markdown="""
        | 姓名 | 分数 | 备注 |
        | --- | ---: | --- |
        | 张三 | 98.5% | **优秀** |
        | A\|B | -2 | |
        """;

        var table=Assert.Single(TableClipboardService.Parse(markdown));
        Assert.Equal(3,table.ColumnCount);Assert.Equal(3,table.Rows.Count);
        Assert.Equal(["姓名","分数","备注"],table.Rows[0]);
        Assert.Equal(["张三","98.5%","优秀"],table.Rows[1]);
        Assert.Equal(["A|B","-2",string.Empty],table.Rows[2]);
    }

    [Fact]
    public void ProducesMarkdownAndCsvThatPreserveTableShape()
    {
        var table=new MarkdownTableData([
            ["列|一","列二"],
            ["第一行\n续行","10"],
            [string.Empty,"20"]]);

        var markdown=TableClipboardService.BuildMarkdown([table]);
        var csv=TableClipboardService.BuildCsv([table]);

        Assert.Contains("| 列\\|一 | 列二 |",markdown);
        Assert.Contains("| 第一行<br>续行 | 10 |",markdown);
        Assert.Contains("\"列|一\",\"列二\"",csv);
        Assert.Contains("\"第一行\n续行\",\"10\"",csv);
    }

    [Fact]
    public void HtmlClipboardOffsetsAreUtf8ByteOffsets()
    {
        var html=TableClipboardService.BuildHtmlClipboard([new MarkdownTableData([["名称","值"],["温度","23℃"]])]);
        var bytes=Encoding.UTF8.GetBytes(html);var startHtml=Offset(html,"StartHTML");var endHtml=Offset(html,"EndHTML");var startFragment=Offset(html,"StartFragment");var endFragment=Offset(html,"EndFragment");

        Assert.Equal(bytes.Length,endHtml);Assert.True(startHtml<startFragment);Assert.True(startFragment<endFragment);
        Assert.StartsWith("<html><body>",Encoding.UTF8.GetString(bytes[startHtml..]));
        Assert.StartsWith("<div",Encoding.UTF8.GetString(bytes[startFragment..endFragment]));
        Assert.Contains("23℃",Encoding.UTF8.GetString(bytes[startFragment..endFragment]));
    }

    private static int Offset(string value,string name)
    {
        var match=Regex.Match(value,$"{name}:(\\d{{10}})",RegexOptions.CultureInvariant);Assert.True(match.Success);return int.Parse(match.Groups[1].Value,System.Globalization.CultureInfo.InvariantCulture);
    }
}
