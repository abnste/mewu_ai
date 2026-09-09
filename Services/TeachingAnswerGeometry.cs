// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text;
using System.Text.RegularExpressions;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class TeachingAnswerGeometry
{
    // This normalization is ONLY a location hint, never a math equivalence
    // test or a grading decision. Preserve signs and decimal points.
    internal static string Normalize(string text)
    {
        var value=string.Concat(text.Normalize(NormalizationForm.FormKC).Where(c=>!char.IsWhiteSpace(c))).Replace('−','-').Replace('×','*').Replace('·','*').Replace("^","").Replace("(","").Replace(")","");
        return Regex.Replace(value,"(?<=[0-9])[xX](?=[0-9])","*",RegexOptions.CultureInvariant,TimeSpan.FromMilliseconds(50));
    }
    internal static IReadOnlyList<GradingItem> Refine(OcrDocument document,int width,int height,IReadOnlyList<GradingItem> rows)
    {
        return rows.Select(row=>
        {
            if(row.Observed.Length is 0 or >80)return row;
            var target=Normalize(row.Observed);
            var exact=document.Lines.Where(line=>line.Text.Length<=160&&Normalize(line.Text)==target&&double.IsFinite(line.X)&&double.IsFinite(line.Y)&&double.IsFinite(line.Width)&&double.IsFinite(line.Height)&&line.X>=0&&line.Y>=0&&line.Width>0&&line.Height>0&&line.X+line.Width<=width&&line.Y+line.Height<=height).Take(2).ToArray();
            if(exact.Length==1)
            {
                var hit=exact[0];return row with{X=hit.X/width,Y=hit.Y/height,Width=hit.Width/width,Height=hit.Height/height};
            }
            var note=new AiAnnotation(row.X,row.Y,row.Width,row.Height,"「"+row.Observed+"」");
            var refined=OcrAnnotationRefinementService.RefineAll(document,width,height,[note],out _)[0];
            return row with{X=refined.X,Y=refined.Y,Width=refined.Width,Height=refined.Height};
        }).ToArray();
    }
}
