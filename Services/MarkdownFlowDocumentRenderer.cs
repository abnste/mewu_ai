using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Windows;
using System.Windows.Documents;
using System.Windows.Media;
using Markdig;
using Markdig.Extensions.Tables;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;
using MdBlock=Markdig.Syntax.Block;
using WpfBlock=System.Windows.Documents.Block;
using WpfInline=System.Windows.Documents.Inline;

namespace mewu_ai_Assistant.Services;

/// <summary>Turns untrusted AI Markdown into native, selectable WPF content.</summary>
public static class MarkdownFlowDocumentRenderer
{
    private static readonly MarkdownPipeline Pipeline=new MarkdownPipelineBuilder()
        .UseAdvancedExtensions()
        .UseEmojiAndSmiley()
        .Build();
    private static readonly FontFamily BodyFont=new("Segoe UI");
    private static readonly FontFamily EmojiFont=new("Segoe UI Emoji");
    private static readonly FontFamily CodeFont=new("Cascadia Mono, Consolas");
    private static readonly Brush BodyBrush=new SolidColorBrush(Color.FromRgb(38,52,74));
    private static readonly Brush MutedBrush=new SolidColorBrush(Color.FromRgb(91,106,128));
    private static readonly Brush LinkBrush=new SolidColorBrush(Color.FromRgb(78,98,218));
    private static readonly Brush CodeBackground=new SolidColorBrush(Color.FromRgb(243,246,250));
    private static readonly Brush RuleBrush=new SolidColorBrush(Color.FromRgb(220,228,239));

    public static FlowDocument Render(string? markdown,double fontSize=13)
    {
        var document=new FlowDocument
        {
            PagePadding=new Thickness(0),ColumnGap=0,FontFamily=BodyFont,
            FontSize=fontSize,Foreground=BodyBrush,LineHeight=fontSize*1.54
        };
        var parsed=Markdown.Parse(markdown??string.Empty,Pipeline);
        foreach(var block in parsed)AddBlock(document.Blocks,block,fontSize);
        if(document.Blocks.Count==0)document.Blocks.Add(new Paragraph());
        return document;
    }

    public static string ToPlainText(FlowDocument document)=>
        new TextRange(document.ContentStart,document.ContentEnd).Text.TrimEnd('\r','\n');

    private static void AddBlock(BlockCollection target,MdBlock block,double fontSize)
    {
        switch(block)
        {
            case HeadingBlock heading:
                target.Add(CreateParagraph(heading.Inline,fontSize+Math.Max(1,7-heading.Level)*1.15,FontWeights.SemiBold,new Thickness(0,heading.Level==1?2:5,0,4)));
                break;
            case ParagraphBlock paragraph:
                target.Add(CreateParagraph(paragraph.Inline,fontSize,FontWeights.Normal,new Thickness(0,0,0,7)));
                break;
            case QuoteBlock quote:
            {
                var section=new Section{BorderBrush=LinkBrush,BorderThickness=new Thickness(3,0,0,0),Padding=new Thickness(10,2,0,1),Margin=new Thickness(0,2,0,8),Foreground=MutedBrush};
                foreach(var child in quote)AddBlock(section.Blocks,child,fontSize);
                target.Add(section);break;
            }
            case Markdig.Syntax.ListBlock list:
            {
                var result=new System.Windows.Documents.List
                {
                    MarkerStyle=list.IsOrdered?TextMarkerStyle.Decimal:TextMarkerStyle.Disc,
                    Margin=new Thickness(19,0,0,7),Padding=new Thickness(0)
                };
                foreach(var child in list)
                {
                    if(child is not ListItemBlock item)continue;
                    var listItem=new System.Windows.Documents.ListItem{Margin=new Thickness(0,0,0,2)};
                    foreach(var itemBlock in item)AddBlock(listItem.Blocks,itemBlock,fontSize);
                    result.ListItems.Add(listItem);
                }
                target.Add(result);break;
            }
            case FencedCodeBlock fenced:
                target.Add(CreateCodeBlock(fenced.Lines.ToString(),fontSize));break;
            case CodeBlock code:
                target.Add(CreateCodeBlock(code.Lines.ToString(),fontSize));break;
            case ThematicBreakBlock:
                target.Add(new Paragraph{BorderBrush=RuleBrush,BorderThickness=new Thickness(0,1,0,0),Margin=new Thickness(0,7,0,9),FontSize=1,LineHeight=1});break;
            case Markdig.Extensions.Tables.Table table:
                target.Add(CreateTable(table,fontSize));break;
            case HtmlBlock html:
                target.Add(CreateLiteralParagraph(html.Lines.ToString(),fontSize));break;
            case ContainerBlock container:
                foreach(var child in container)AddBlock(target,child,fontSize);
                break;
            case LeafBlock leaf when leaf.Inline is not null:
                target.Add(CreateParagraph(leaf.Inline,fontSize,FontWeights.Normal,new Thickness(0,0,0,7)));
                break;
        }
    }

    private static Paragraph CreateParagraph(ContainerInline? source,double fontSize,FontWeight weight,Thickness margin)
    {
        var paragraph=new Paragraph{FontSize=fontSize,FontWeight=weight,Margin=margin,LineHeight=fontSize*1.5};
        AddInlines(paragraph.Inlines,source?.FirstChild,fontSize);
        return paragraph;
    }

    private static Paragraph CreateLiteralParagraph(string text,double fontSize)
    {
        var paragraph=new Paragraph{Margin=new Thickness(0,0,0,7),FontSize=fontSize};
        AddTextRuns(paragraph.Inlines,text,fontSize);return paragraph;
    }

    private static Paragraph CreateCodeBlock(string code,double fontSize)
    {
        var paragraph=new Paragraph
        {
            FontFamily=CodeFont,FontSize=Math.Max(11,fontSize-1),LineHeight=Math.Max(17,fontSize*1.45),
            Background=CodeBackground,Padding=new Thickness(10,8,10,8),Margin=new Thickness(0,2,0,9)
        };
        paragraph.Inlines.Add(new Run(code.TrimEnd('\r','\n')){FontFamily=CodeFont});
        return paragraph;
    }

    private static System.Windows.Documents.Table CreateTable(Markdig.Extensions.Tables.Table source,double fontSize)
    {
        var table=new System.Windows.Documents.Table{CellSpacing=0,Margin=new Thickness(0,3,0,9)};
        var group=new TableRowGroup();table.RowGroups.Add(group);
        foreach(var rowBlock in source)
        {
            if(rowBlock is not Markdig.Extensions.Tables.TableRow sourceRow)continue;
            var row=new System.Windows.Documents.TableRow();group.Rows.Add(row);
            foreach(var cellBlock in sourceRow)
            {
                if(cellBlock is not Markdig.Extensions.Tables.TableCell sourceCell)continue;
                var cell=new System.Windows.Documents.TableCell
                {
                    BorderBrush=RuleBrush,BorderThickness=new Thickness(.5),Padding=new Thickness(7,5,7,5),
                    Background=sourceRow.IsHeader?CodeBackground:Brushes.Transparent
                };
                foreach(var child in sourceCell)AddBlock(cell.Blocks,child,fontSize-1);
                row.Cells.Add(cell);
            }
        }
        return table;
    }

    private static void AddInlines(InlineCollection target,Markdig.Syntax.Inlines.Inline? first,double fontSize)
    {
        for(var current=first;current is not null;current=current.NextSibling)
        {
            switch(current)
            {
                case LiteralInline literal:AddTextRuns(target,literal.Content.ToString(),fontSize);break;
                case LineBreakInline:target.Add(new LineBreak());break;
                case CodeInline code:
                    target.Add(new Run(code.Content){FontFamily=CodeFont,FontSize=Math.Max(11,fontSize-1),Background=CodeBackground});break;
                case EmphasisInline emphasis:
                {
                    var span=new Span();
                    if(emphasis.DelimiterChar=='~')span.TextDecorations=TextDecorations.Strikethrough;
                    else if(emphasis.DelimiterCount>=2)span.FontWeight=FontWeights.SemiBold;
                    else span.FontStyle=FontStyles.Italic;
                    AddInlines(span.Inlines,emphasis.FirstChild,fontSize);target.Add(span);break;
                }
                case LinkInline link when link.IsImage:
                {
                    var alt=new Span{Foreground=MutedBrush};alt.Inlines.Add(new Run("[图片："));AddInlines(alt.Inlines,link.FirstChild,fontSize);alt.Inlines.Add(new Run("]"));target.Add(alt);break;
                }
                case LinkInline link:
                {
                    var hyperlink=new Hyperlink{Foreground=LinkBrush,TextDecorations=TextDecorations.Underline};
                    AddInlines(hyperlink.Inlines,link.FirstChild,fontSize);
                    if(TrySafeWebUri(link.Url,out var uri))
                    {
                        hyperlink.NavigateUri=uri;hyperlink.ToolTip=uri.AbsoluteUri;
                        hyperlink.RequestNavigate+=static (_,eventArgs)=>
                        {
                            if(TrySafeWebUri(eventArgs.Uri.AbsoluteUri,out var safe))
                                Process.Start(new ProcessStartInfo(safe.AbsoluteUri){UseShellExecute=true});
                            eventArgs.Handled=true;
                        };
                    }
                    target.Add(hyperlink);break;
                }
                case HtmlInline html:AddTextRuns(target,html.Tag,fontSize);break;
                case ContainerInline container:
                {
                    var span=new Span();AddInlines(span.Inlines,container.FirstChild,fontSize);target.Add(span);break;
                }
                default:
                    AddTextRuns(target,current.ToString()??string.Empty,fontSize);break;
            }
        }
    }

    private static void AddTextRuns(InlineCollection target,string text,double fontSize)
    {
        if(string.IsNullOrEmpty(text))return;
        var normal=new StringBuilder();
        var enumerator=StringInfo.GetTextElementEnumerator(text);
        while(enumerator.MoveNext())
        {
            var element=enumerator.GetTextElement();
            if(!IsEmoji(element)){normal.Append(element);continue;}
            FlushNormal();
            target.Add(new Run(element){FontFamily=EmojiFont,FontSize=fontSize*1.06,BaselineAlignment=BaselineAlignment.Center});
        }
        FlushNormal();
        void FlushNormal(){if(normal.Length==0)return;target.Add(new Run(normal.ToString()){FontFamily=BodyFont});normal.Clear();}
    }

    internal static bool IsEmoji(string textElement)
    {
        foreach(var rune in textElement.EnumerateRunes())
        {
            var value=rune.Value;
            if(value is 0xFE0F or 0x20E3||value is >=0x1F000 and <=0x1FAFF||value is >=0x2600 and <=0x27BF)return true;
        }
        return false;
    }

    private static bool TrySafeWebUri(string? value,out Uri uri)
    {
        if(Uri.TryCreate(value,UriKind.Absolute,out var candidate)&&candidate.Scheme is "http" or "https")
        {uri=candidate;return true;}
        uri=null!;return false;
    }
}
