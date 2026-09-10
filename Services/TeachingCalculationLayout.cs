// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text;

namespace mewu_ai_Assistant.Services;

/// <summary>Whitespace-only presentation of legacy one-line equality chains.</summary>
internal static class TeachingCalculationLayout
{
    internal static string Format(string text)
    {
        // Explicit model/teacher line breaks are authoritative. This is not a
        // mathematical parser, OCR reconstruction, or answer normalization.
        if(text.Length>600||text.IndexOfAny(['\r','\n','$','\\'])>=0)return text;
        var brackets=new Stack<char>();var equals=new List<int>();
        for(var index=0;index<text.Length;index++)
        {
            var c=text[index];
            if(c is '(' or '[' or '{'){brackets.Push(c);continue;}
            if(c is ')' or ']' or '}')
            {
                if(!brackets.TryPop(out var open)||c!=(open=='('?')':open=='['?']':'}'))return text;
                continue;
            }
            if(brackets.Count>0)continue;
            // Lists, prose and mixed relations are ambiguous; let the editor
            // wrap those normally instead of guessing their calculation steps.
            if(c is ',' or ';' or '，' or '；' or '→' or '⇒'||char.IsLetter(c)&&c>0x2E80&&c<0xA000)return text;
            if(c=='='&&(index==0||!"=<>!".Contains(text[index-1]))&&(index+1==text.Length||!"=<>".Contains(text[index+1])))equals.Add(index);
        }
        if(brackets.Count!=0||equals.Count<2)return text;
        var result=new StringBuilder(text.Length+equals.Count);var start=0;
        foreach(var boundary in equals.Skip(1))
        {
            var part=text[start..boundary].TrimEnd();
            if(start!=0)part=part.TrimStart();
            result.Append(part).Append('\n');start=boundary;
        }
        var formatted=result.Append(text[start..].TrimStart()).ToString();
        return formatted.Length<=600?formatted:text;
    }
}
