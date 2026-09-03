using System.Globalization;
using System.Text;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Uses OCR text geometry as a semantic anchor for coarse model callouts.
/// A visual model is good at describing a target but can return a box that
/// spans neighbouring rows. OCR already provides the exact line bounds, so a
/// strong label-to-line match is safer than trying to shrink a large box from
/// image edges alone.
/// </summary>
internal static class OcrAnnotationRefinementService
{
    private const int MaximumTextLength=192;

    internal static IReadOnlyList<AiAnnotation> RefineAll(
        OcrDocument document,
        int imageWidth,
        int imageHeight,
        IReadOnlyList<AiAnnotation> annotations,
        out int refinedCount)
    {
        ArgumentNullException.ThrowIfNull(document);
        ArgumentNullException.ThrowIfNull(annotations);
        refinedCount=0;
        if(imageWidth<12||imageHeight<12||document.Lines.Count==0||annotations.Count==0)return annotations;

        var lines=document.Lines
            .Where(IsUsable)
            .Select((line,index)=>new CandidateLine(index,line,Normalize(line.Text)))
            .Where(candidate=>candidate.Normalized.Length>=2)
            .Take(256)
            .ToArray();
        if(lines.Length==0)return annotations;

        var result=new AiAnnotation[annotations.Count];
        var usedLines=new HashSet<int>();
        for(var index=0;index<annotations.Count;index++)
        {
            var annotation=annotations[index];
            if(annotation.IsVideoTimeline||annotation.Kind is not (AiAnnotationKind.Callout or AiAnnotationKind.Rectangle or AiAnnotationKind.Ellipse)||string.IsNullOrWhiteSpace(annotation.Text))
            {
                result[index]=annotation;
                continue;
            }

            var label=Normalize(annotation.Text);
            if(label.Length<2)
            {
                result[index]=annotation;
                continue;
            }

            var coarseCenterX=(annotation.X+annotation.Width/2)*imageWidth;
            var coarseCenterY=(annotation.Y+annotation.Height/2)*imageHeight;
            var ranked=lines
                .Select(line=>Score(label,line,coarseCenterX,coarseCenterY,imageWidth,imageHeight,usedLines.Contains(line.Index)))
                .Where(match=>match.SemanticStrength>0)
                .OrderByDescending(match=>match.TotalScore)
                .ThenBy(match=>match.Distance)
                .ToArray();
            if(ranked.Length==0||!IsConfident(ranked[0],ranked.Length>1?ranked[1]:null))
            {
                result[index]=annotation;
                continue;
            }

            var best=ranked[0];
            var paddingX=Math.Clamp(best.Line.Line.Height*.18,2,8);
            var paddingY=Math.Clamp(best.Line.Line.Height*.12,1,5);
            var left=Math.Clamp(best.Line.Line.X-paddingX,0,imageWidth-1);
            var top=Math.Clamp(best.Line.Line.Y-paddingY,0,imageHeight-1);
            var right=Math.Clamp(best.Line.Line.X+best.Line.Line.Width+paddingX,left+1,imageWidth);
            var bottom=Math.Clamp(best.Line.Line.Y+best.Line.Line.Height+paddingY,top+1,imageHeight);
            result[index]=annotation with
            {
                X=left/imageWidth,
                Y=top/imageHeight,
                Width=(right-left)/imageWidth,
                Height=(bottom-top)/imageHeight
            };
            usedLines.Add(best.Line.Index);
            refinedCount++;
        }
        return result;
    }

    private static Match Score(string label,CandidateLine line,double centerX,double centerY,int width,int height,bool used)
    {
        var (commonLength,_,_)=LongestCommonSubstring(label,line.Normalized);
        if(commonLength<2)return new Match(line,0,double.MaxValue,double.MinValue,false);
        var shorter=Math.Max(1,Math.Min(label.Length,line.Normalized.Length));
        var coverage=(double)commonLength/shorter;
        var commonBigrams=CommonBigramCount(label,line.Normalized);
        var bigramCoverage=(double)commonBigrams/Math.Max(1,Math.Min(label.Length-1,line.Normalized.Length-1));
        var exact=label.Contains(line.Normalized,StringComparison.Ordinal)||line.Normalized.Contains(label,StringComparison.Ordinal);
        var lineCenterX=line.Line.X+line.Line.Width/2;var lineCenterY=line.Line.Y+line.Line.Height/2;
        var dx=(lineCenterX-centerX)/Math.Max(1,width);var dy=(lineCenterY-centerY)/Math.Max(1,height);
        // A bad model box is commonly far too wide while still landing on the
        // correct row. Keep horizontal proximity as a weak tie-breaker and let
        // the text match plus vertical row position drive the correction.
        var distance=Math.Sqrt(dx*dx*.1225+dy*dy);
        var semantic=commonLength*5+coverage*35+commonBigrams*4+bigramCoverage*15+(exact?22:0);
        // Two-character matches such as “蓝色” are useful only when the model
        // already pointed near that line. Longer text matches may safely fix a
        // much larger model error.
        if(commonLength==2&&Math.Abs(dy)>.14)semantic=0;
        var score=semantic-distance*24-(used?8:0);
        return new Match(line,semantic,distance,score,exact);
    }

    private static int CommonBigramCount(string first,string second)
    {
        if(first.Length<2||second.Length<2)return 0;
        var available=new HashSet<string>(StringComparer.Ordinal);
        for(var index=0;index<second.Length-1;index++)available.Add(second.Substring(index,2));
        var common=new HashSet<string>(StringComparer.Ordinal);
        for(var index=0;index<first.Length-1;index++)
        {
            var value=first.Substring(index,2);
            if(available.Contains(value))common.Add(value);
        }
        return common.Count;
    }

    private static bool IsConfident(Match best,Match? runnerUp)
    {
        if(best.SemanticStrength<=0)return false;
        if(best.Exact&&best.SemanticStrength>=32)return true;
        if(best.SemanticStrength<22)return false;
        if(runnerUp is null)return true;
        return best.TotalScore-runnerUp.TotalScore>=3||best.Distance+0.08<runnerUp.Distance;
    }

    private static (int Length,int FirstStart,int SecondStart) LongestCommonSubstring(string first,string second)
    {
        if(first.Length==0||second.Length==0)return(0,0,0);
        var previous=new int[second.Length+1];var current=new int[second.Length+1];var best=0;var firstStart=0;var secondStart=0;
        for(var i=1;i<=first.Length;i++)
        {
            Array.Clear(current);
            for(var j=1;j<=second.Length;j++)
            {
                if(first[i-1]!=second[j-1])continue;
                current[j]=previous[j-1]+1;
                if(current[j]<=best)continue;
                best=current[j];firstStart=i-best;secondStart=j-best;
            }
            (previous,current)=(current,previous);
        }
        return(best,firstStart,secondStart);
    }

    private static string Normalize(string value)
    {
        var builder=new StringBuilder(Math.Min(value.Length,MaximumTextLength));
        foreach(var rune in value.EnumerateRunes())
        {
            if(builder.Length>=MaximumTextLength)break;
            var category=Rune.GetUnicodeCategory(rune);
            if(category is not (UnicodeCategory.UppercaseLetter or UnicodeCategory.LowercaseLetter or UnicodeCategory.TitlecaseLetter or UnicodeCategory.ModifierLetter or UnicodeCategory.OtherLetter or UnicodeCategory.DecimalDigitNumber))continue;
            builder.Append(rune.ToString().ToUpperInvariant());
        }
        return builder.ToString();
    }

    private static bool IsUsable(OcrLine line)=>line is not null&&!string.IsNullOrWhiteSpace(line.Text)&&double.IsFinite(line.X)&&double.IsFinite(line.Y)&&double.IsFinite(line.Width)&&double.IsFinite(line.Height)&&line.Width>1&&line.Height>1;

    private sealed record CandidateLine(int Index,OcrLine Line,string Normalized);
    private sealed record Match(CandidateLine Line,double SemanticStrength,double Distance,double TotalScore,bool Exact);
}
