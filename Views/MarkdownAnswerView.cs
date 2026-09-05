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
    private bool _hasActions;
    public bool ContainsTable { get; private set; }
    public event EventHandler? MarkdownChanged;

    public MarkdownAnswerView()
    {
        IsReadOnly=true;IsReadOnlyCaretVisible=true;IsTabStop=false;IsDocumentEnabled=true;IsUndoEnabled=false;
        Background=Brushes.Transparent;BorderThickness=new Thickness(0);Padding=new Thickness(0);
        VerticalScrollBarVisibility=ScrollBarVisibility.Disabled;HorizontalScrollBarVisibility=ScrollBarVisibility.Disabled;
    }

    public string Markdown
    {
        get=>_markdown;
        set
        {
            var normalized=value??string.Empty;
            if(!_hasActions&&string.Equals(_markdown,normalized,StringComparison.Ordinal))return;
            RenderMarkdown(normalized);
        }
    }

    public string PlainText=>Text.TrimEnd('\r','\n');

    private void RenderMarkdown(string markdown)
    {
        var document=MarkdownFlowDocumentRenderer.Render(markdown,FontSize>0?FontSize:13,out var containsTable);
        _markdown=markdown;ContainsTable=containsTable;_hasActions=false;Document=document;
        MarkdownChanged?.Invoke(this,EventArgs.Empty);
    }

    public void SetMarkdownWithActions(string? markdown,IReadOnlyList<MarkdownAnswerAction> actions)
    {
        if(actions.Count==0&&!_hasActions){Markdown=markdown??string.Empty;return;}
        RenderMarkdown(markdown??string.Empty);
        if(actions.Count==0)return;
        _hasActions=true;
        var chips=new WrapPanel{Margin=new Thickness(0,7,0,2)};
        foreach(var action in actions)
        {
            var currentAction=action;var button=new Button{Content=currentAction.Label,ToolTip=currentAction.ToolTip,Foreground=new SolidColorBrush(Color.FromRgb(71,87,188)),Background=new SolidColorBrush(Color.FromRgb(239,242,255)),BorderBrush=new SolidColorBrush(Color.FromRgb(205,213,250)),BorderThickness=new Thickness(1),Padding=new Thickness(11,6,11,6),Margin=new Thickness(0,0,7,7),FontSize=Math.Max(12,FontSize-1),FontWeight=FontWeights.SemiBold,Cursor=Cursors.Hand,FocusVisualStyle=null};button.Template=CreateChipTemplate();button.Click+=(_,_)=>currentAction.Invoke();chips.Children.Add(button);
        }
        Document.Blocks.Add(new BlockUIContainer(chips){Margin=new Thickness(0)});
    }

    private static ControlTemplate CreateChipTemplate()
    {
        var border=new FrameworkElementFactory(typeof(Border));border.Name="Bubble";border.SetValue(Border.BackgroundProperty,new TemplateBindingExtension(Button.BackgroundProperty));border.SetValue(Border.BorderBrushProperty,new TemplateBindingExtension(Button.BorderBrushProperty));border.SetValue(Border.BorderThicknessProperty,new TemplateBindingExtension(Button.BorderThicknessProperty));border.SetValue(Border.CornerRadiusProperty,new CornerRadius(13));border.SetValue(Border.PaddingProperty,new TemplateBindingExtension(Button.PaddingProperty));var presenter=new FrameworkElementFactory(typeof(ContentPresenter));presenter.SetValue(HorizontalAlignmentProperty,HorizontalAlignment.Center);presenter.SetValue(VerticalAlignmentProperty,VerticalAlignment.Center);border.AppendChild(presenter);var template=new ControlTemplate(typeof(Button)){VisualTree=border};var hover=new Trigger{Property=IsMouseOverProperty,Value=true};hover.Setters.Add(new Setter(Button.BackgroundProperty,new SolidColorBrush(Color.FromRgb(226,232,255)),"Bubble"));template.Triggers.Add(hover);var pressed=new Trigger{Property=Button.IsPressedProperty,Value=true};pressed.Setters.Add(new Setter(OpacityProperty,.72,"Bubble"));template.Triggers.Add(pressed);return template;
    }
}

public sealed record MarkdownAnswerAction(string Label,string ToolTip,Action Invoke);
