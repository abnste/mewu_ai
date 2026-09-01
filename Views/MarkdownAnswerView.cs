using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Input;
using System.Windows.Media;
using mewu_ai_Assistant.Services;
using EmojiRichTextBox=Emoji.Wpf.RichTextBox;

namespace mewu_ai_Assistant.Views;

public sealed class MarkdownAnswerView:EmojiRichTextBox
{
    private string _markdown=string.Empty;

    public MarkdownAnswerView()
    {
        IsReadOnly=true;IsReadOnlyCaretVisible=true;IsTabStop=false;IsDocumentEnabled=true;
        Background=Brushes.Transparent;BorderThickness=new Thickness(0);Padding=new Thickness(0);
        VerticalScrollBarVisibility=ScrollBarVisibility.Disabled;HorizontalScrollBarVisibility=ScrollBarVisibility.Disabled;
    }

    public string Markdown
    {
        get=>_markdown;
        set
        {
            var normalized=value??string.Empty;
            if(string.Equals(_markdown,normalized,StringComparison.Ordinal))return;
            _markdown=normalized;Document=MarkdownFlowDocumentRenderer.Render(normalized,FontSize>0?FontSize:13);
        }
    }

    public string PlainText=>Text.TrimEnd('\r','\n');

    public void SetMarkdownWithActions(string? markdown,IReadOnlyList<MarkdownAnswerAction> actions)
    {
        Markdown=markdown??string.Empty;
        if(actions.Count==0)return;
        var paragraph=new Paragraph{Margin=new Thickness(0,5,0,2)};
        paragraph.Inlines.Add(new Run("视频定位：") {FontWeight=FontWeights.SemiBold,Foreground=new SolidColorBrush(Color.FromRgb(58,72,96))});
        for(var index=0;index<actions.Count;index++)
        {
            if(index>0)paragraph.Inlines.Add(new Run("  "));
            var action=actions[index];
            var link=new Hyperlink(new Run(action.Label))
            {
                Foreground=new SolidColorBrush(Color.FromRgb(78,98,218)),
                TextDecorations=TextDecorations.Underline,
                Cursor=Cursors.Hand,
                ToolTip=action.ToolTip
            };
            link.Click+=(_,_)=>action.Invoke();
            paragraph.Inlines.Add(link);
        }
        Document.Blocks.Add(paragraph);
    }
}

public sealed record MarkdownAnswerAction(string Label,string ToolTip,Action Invoke);
