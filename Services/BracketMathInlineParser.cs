// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using Markdig.Extensions.Mathematics;
using Markdig.Helpers;
using Markdig.Parsers;
using Markdig.Syntax;

namespace mewu_ai_Assistant.Services;

internal sealed class BracketMathInline:MathInline
{
    internal string SourceText { get; init; }="";
}
internal sealed class BracketMathInlineParser:InlineParser
{
    internal BracketMathInlineParser()=>OpeningCharacters=['\\'];
    public override bool Match(InlineProcessor processor,ref StringSlice slice)
    {
        var open=slice.PeekChar(1);if(open is not ('(' or '['))return false;
        var end=open=='('?')':']';var scan=slice;var start=scan.Start;
        scan.NextChar();scan.NextChar();var body=scan.Start;
        for(var count=0;count<2048&&scan.CurrentChar!='\0';count++,scan.NextChar())
        {
            if(scan.CurrentChar!='\\'||scan.PeekChar(1)!=end)continue;
            var close=scan.Start;scan.NextChar();scan.NextChar();
            var content=slice;content.Start=body;content.End=close-1;
            var source=slice;source.End=scan.Start-1;
            processor.Inline=new BracketMathInline{SourceText=source.ToString(),Content=content,
                Span=new SourceSpan(processor.GetSourcePosition(start,out var line,out var column),processor.GetSourcePosition(scan.Start-1)),Line=line,Column=column};
            slice=scan;return true;
        }
        return false;
    }
}
