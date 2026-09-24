// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.Text.RegularExpressions;
using System.Windows;
using System.Windows.Media;

namespace mewu_ai_Assistant.Services;

/// <summary>Shared vector layout for live annotation cards and exported annotations.</summary>
internal static class AnnotationFormulaLayout
{
    private static readonly Regex Formula=new(@"\$\$[\s\S]+?\$\$|\$[^$\r\n]+?\$|\\\[[\s\S]+?\\\]|\\\([\s\S]+?\\\)",RegexOptions.CultureInvariant,TimeSpan.FromMilliseconds(100));

    internal static DrawingImage? TryCreate(string value,double width,double size,Brush brush,bool halo=false)
    {
        if(value.Length is 0 or >8192)return null;
        width=Math.Max(1,width);
        var matches=Formula.Matches(value).Cast<Match>().Take(33).ToArray();
        if(matches.Length>32)return null;
        var parts=new List<(string Text,DrawingImage? Formula)>();var start=0;
        foreach(var match in matches)
        {
            if(match.Index>start)parts.Add((value[start..match.Index],null));
            parts.Add((match.Value,MathFormulaRenderer.Create(match.Value,size,brush,false,halo)));
            start=match.Index+match.Length;
        }
        if(start<value.Length)parts.Add((value[start..],null));
        if(matches.Length==0)
        {
            // Bare LaTeX is common in annotation text, which is not Markdown.
            if(!value.TrimStart().StartsWith('\\'))return null;
            var bare=MathFormulaRenderer.Create(value,size,brush,false,halo);
            if(bare is null)return null;
            parts.Clear();parts.Add((value,bare));
        }
        if(!parts.Any(part=>part.Formula is not null))return null;
        var group=new DrawingGroup();var y=0d;
        using(var drawing=group.Open())
        {
            foreach(var part in parts)
            {
                if(part.Formula is {} formula)
                {
                    var scale=Math.Min(1,width/formula.Width);
                    drawing.DrawImage(formula,new Rect(0,y,formula.Width*scale,formula.Height*scale));
                    y+=formula.Height*scale+size*.25;
                }
                else if(!string.IsNullOrWhiteSpace(part.Text))
                {
                    var text=new FormattedText(part.Text.Trim('\r','\n'),CultureInfo.CurrentUICulture,FlowDirection.LeftToRight,
                        new Typeface("Microsoft YaHei UI"),size,brush,1){MaxTextWidth=width};
                    if(halo)drawing.DrawGeometry(Brushes.White,new Pen(Brushes.White,size*.14),text.BuildGeometry(new Point(0,y)));
                    drawing.DrawText(text,new Point(0,y));y+=text.Height+size*.2;
                }
            }
            drawing.DrawRectangle(Brushes.Transparent,null,new Rect(0,0,width,Math.Max(1,y)));
        }
        group.ClipGeometry=new RectangleGeometry(new Rect(0,0,width,Math.Max(1,y)));
        group.Freeze();var result=new DrawingImage(group);result.Freeze();return result;
    }
}
