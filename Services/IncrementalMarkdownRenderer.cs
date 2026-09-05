using System.Windows.Documents;

namespace mewu_ai_Assistant.Services;

// Keep settled document blocks alive as a stream grows. Replacing the entire
// RichTextBox document invalidates text selection, layout and emoji processing.
internal sealed class IncrementalMarkdownRenderer
{
    private readonly List<(Type Type,string Source,int BlockCount)> _blocks=[];
    private string _markdown=string.Empty;
    private double _fontSize;
    internal FlowDocument? Document { get; private set; }

    internal FlowDocument Update(string markdown,double fontSize,out bool containsTable,bool reset=false)
    {
        var parsed=MarkdownFlowDocumentRenderer.Parse(markdown);
        containsTable=MarkdownFlowDocumentRenderer.ContainsTable(parsed);
        var keys=parsed.Select(block=>(Type:block.GetType(),Source:block.Span.Start>=0&&block.Span.End<markdown.Length
            ?markdown.Substring(block.Span.Start,block.Span.Length):string.Empty)).ToArray();
        // References, footnotes and abbreviations can change the meaning of an
        // earlier block. Conservatively rebuild when either text contains '['.
        var keep=0;
        if(!reset&&Document is not null&&fontSize==_fontSize&&!markdown.Contains('[')&&!_markdown.Contains('['))
            while(keep<keys.Length&&keep<_blocks.Count&&keys[keep].Type==_blocks[keep].Type&&keys[keep].Source==_blocks[keep].Source)keep++;
        if(Document is null||fontSize!=_fontSize)
        {
            Document=MarkdownFlowDocumentRenderer.Render(string.Empty,fontSize);
            Document.Blocks.Clear();_blocks.Clear();keep=0;
        }
        var retainedCount=_blocks.Take(keep).Sum(block=>block.BlockCount);
        while(Document.Blocks.Count>retainedCount)Document.Blocks.Remove(Document.Blocks.LastBlock);
        if(_blocks.Count>keep)_blocks.RemoveRange(keep,_blocks.Count-keep);
        for(var index=keep;index<parsed.Count;index++)
        {
            var before=Document.Blocks.Count;var last=Document.Blocks.LastBlock;
            MarkdownFlowDocumentRenderer.AddBlock(Document.Blocks,parsed[index],fontSize);
            _blocks.Add((keys[index].Type,keys[index].Source,Document.Blocks.Count-before));
            // Emoji substitution is limited to newly rendered runs. Ordinary
            // Chinese/Latin text never enters the per-character glyph walker.
            for(var block=last is null?Document.Blocks.FirstBlock:last.NextBlock;block is not null;block=block.NextBlock)
            {
                var runs=new List<Run>();
                for(var pointer=block.ContentStart;pointer is not null&&pointer.CompareTo(block.ContentEnd)<0;pointer=pointer.GetNextContextPosition(System.Windows.Documents.LogicalDirection.Forward))
                    if(pointer.GetPointerContext(LogicalDirection.Forward)==TextPointerContext.ElementStart&&pointer.GetAdjacentElement(LogicalDirection.Forward) is Run run&&MarkdownFlowDocumentRenderer.IsEmoji(run.Text))runs.Add(run);
                foreach(var run in runs)Emoji.Wpf.FlowDocumentExtensions.SubstituteGlyphs(run);
            }
        }
        _fontSize=fontSize;_markdown=markdown;
        return Document;
    }
}
