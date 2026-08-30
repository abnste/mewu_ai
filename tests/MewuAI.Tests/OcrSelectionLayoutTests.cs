using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.OCR;
using Xunit;

namespace MewuAI.Tests;

public sealed class OcrSelectionLayoutTests
{
    [Fact]
    public void UsesWordAndCharacterCoordinatesInReadingOrder()
    {
        var lines=new[]
        {
            new OcrLine("AB 中",10,20,90,20,[new OcrWord("中",80,20,20,20),new OcrWord("AB",10,20,40,20)])
        };
        var line=Assert.Single(OcrSelectionLayout.Build(lines,2,.5));
        Assert.Equal(new[]{"AB","中"},line.Tokens.Select(token=>token.Text));
        Assert.Equal(" ",line.Tokens[1].Prefix);
        Assert.Equal(2,line.Tokens[0].Glyphs.Count);
        Assert.Equal(20,line.Tokens[0].Glyphs[0].Bounds.Left);
        Assert.Equal(40,line.Tokens[0].Glyphs[0].Bounds.Width);
        Assert.Equal(10,line.Tokens[0].Glyphs[0].Bounds.Height);
    }

    [Fact]
    public void FallsBackToLineBoxAndKeepsSurrogatePairsAtomic()
    {
        var line=Assert.Single(OcrSelectionLayout.Build([new OcrLine("A😀B",0,0,40,10,[])],1,1));
        var token=Assert.Single(line.Tokens);
        Assert.Equal(3,token.Glyphs.Count);
        Assert.Equal(2,token.Glyphs[1].Utf16Length);
    }

    [Fact]
    public void EmptyInvalidOrZeroSizedResultsCreateNoHitLayout()
    {
        var lines=new[]{new OcrLine("",0,0,10,10,[]),new OcrLine("text",double.NaN,0,10,10,[]),new OcrLine("text",0,0,0,10,[])};
        Assert.Empty(OcrSelectionLayout.Build(lines,1,1));
        Assert.Empty(OcrSelectionLayout.Build([new OcrLine("text",0,0,10,10,[])],0,1));
    }
}
