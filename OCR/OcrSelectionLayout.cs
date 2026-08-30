using System.Globalization;
using System.Windows;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.OCR;

internal sealed record OcrSelectionGlyph(Rect Bounds,int Utf16Start,int Utf16Length);
internal sealed record OcrSelectionToken(string Prefix,string Text,IReadOnlyList<OcrSelectionGlyph> Glyphs);
internal sealed record OcrSelectionLine(IReadOnlyList<OcrSelectionToken> Tokens);

internal static class OcrSelectionLayout
{
    internal const int MaxSelectableGlyphs=12_000;
    internal static IReadOnlyList<OcrSelectionLine> Build(IReadOnlyList<OcrLine> lines,double scaleX,double scaleY)
    {
        ArgumentNullException.ThrowIfNull(lines);
        if(!double.IsFinite(scaleX)||!double.IsFinite(scaleY)||scaleX<=0||scaleY<=0)return [];
        var result=new List<OcrSelectionLine>();var glyphCount=0;
        foreach(var line in lines.Where(IsUsableLine).OrderBy(line=>line.Y).ThenBy(line=>line.X))
        {
            var words=(line.Words??[]).Where(IsUsableWord).OrderBy(word=>word.X).ThenBy(word=>word.Y).ToList();
            if(words.Count==0)words.Add(new OcrWord(line.Text,line.X,line.Y,line.Width,line.Height));
            var tokens=new List<OcrSelectionToken>(words.Count);OcrWord? previous=null;
            foreach(var word in words)
            {
                var prefix=previous is not null&&ShouldInsertSpace(previous,word)?" ":string.Empty;
                var bounds=Scale(word.X,word.Y,word.Width,word.Height,scaleX,scaleY);var glyphs=CreateGlyphs(word.Text,bounds,MaxSelectableGlyphs-glyphCount);glyphCount+=glyphs.Count;
                if(glyphs.Count>0)tokens.Add(new OcrSelectionToken(prefix,word.Text,glyphs));
                previous=word;
                if(glyphCount>=MaxSelectableGlyphs)break;
            }
            if(tokens.Count>0)result.Add(new OcrSelectionLine(tokens));
            if(glyphCount>=MaxSelectableGlyphs)break;
        }
        return result;
    }

    private static IReadOnlyList<OcrSelectionGlyph> CreateGlyphs(string text,Rect bounds,int limit)
    {
        var starts=StringInfo.ParseCombiningCharacters(text);if(starts.Length==0||limit<=0)return [];
        var count=Math.Min(starts.Length,limit);var result=new List<OcrSelectionGlyph>(count);
        for(var index=0;index<count;index++)
        {
            var start=starts[index];var end=index+1<starts.Length?starts[index+1]:text.Length;var left=bounds.Left+bounds.Width*index/starts.Length;var right=bounds.Left+bounds.Width*(index+1)/starts.Length;
            result.Add(new OcrSelectionGlyph(new Rect(left,bounds.Top,Math.Max(.5,right-left),Math.Max(.5,bounds.Height)),start,end-start));
        }
        return result;
    }

    private static Rect Scale(double x,double y,double width,double height,double scaleX,double scaleY)=>
        new(Math.Max(0,x*scaleX),Math.Max(0,y*scaleY),Math.Max(.5,width*scaleX),Math.Max(.5,height*scaleY));

    private static bool ShouldInsertSpace(OcrWord previous,OcrWord current)
    {
        var gap=current.X-(previous.X+previous.Width);var reference=Math.Max(1,Math.Min(previous.Height,current.Height));
        return gap>reference*.22;
    }

    private static bool IsUsableLine(OcrLine line)=>line is not null&&!string.IsNullOrEmpty(line.Text)&&IsFiniteBox(line.X,line.Y,line.Width,line.Height);
    private static bool IsUsableWord(OcrWord word)=>word is not null&&!string.IsNullOrEmpty(word.Text)&&IsFiniteBox(word.X,word.Y,word.Width,word.Height);
    private static bool IsFiniteBox(double x,double y,double width,double height)=>double.IsFinite(x)&&double.IsFinite(y)&&double.IsFinite(width)&&double.IsFinite(height)&&width>0&&height>0;
}
