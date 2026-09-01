using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public sealed class MarkdownAnswerView:RichTextBox
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

    public string PlainText=>MarkdownFlowDocumentRenderer.ToPlainText(Document);
}
