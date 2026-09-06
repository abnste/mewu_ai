// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Data;
using System.Windows.Documents;
using System.Windows.Media;

namespace mewu_ai_Assistant.Views;

/// <summary>Shared layout for every AI channel; provider pages supply only their fields and actions.</summary>
internal class AiSettingsForm : StackPanel
{
    internal StackPanel Fields {get;}=new();
    internal WrapPanel Actions {get;}=new(){Margin=new Thickness(0,0,0,8)};
    internal AiSettingsForm(string title,string description,TextBlock status)
    {
        Margin=new Thickness(6,4,6,8);FontReset();
        Children.Add(new TextBlock{Text=title,FontSize=17,FontWeight=FontWeights.SemiBold,Foreground=BrushesForForm.Primary,Margin=new Thickness(0,0,0,6)});
        Children.Add(new TextBlock{Text=description,FontSize=12,FontWeight=FontWeights.Normal,Foreground=BrushesForForm.Secondary,TextWrapping=TextWrapping.Wrap,LineHeight=18,MinHeight=36,Margin=new Thickness(0,0,0,12)});
        Children.Add(Actions);
        status.FontSize=12;status.FontWeight=FontWeights.Normal;status.Foreground=BrushesForForm.Secondary;
        status.Margin=new Thickness(0,0,0,16);status.MinHeight=18;
        status.TextWrapping=TextWrapping.NoWrap;status.TextTrimming=TextTrimming.CharacterEllipsis;
        status.SetBinding(ToolTipProperty,new Binding(nameof(TextBlock.Text)){Source=status});Children.Add(status);
        Children.Add(Fields);
    }

    private void FontReset()
    {
        // A selected TabItem's semibold font must not leak into the form body.
        TextElement.SetFontWeight(this,FontWeights.Normal);TextElement.SetFontSize(this,13);
    }

    internal static FrameworkElement Field(string label,FrameworkElement editor)
    {
        var field=new StackPanel{Margin=new Thickness(0,0,0,14)};
        field.Children.Add(new TextBlock{Text=label,FontSize=12,FontWeight=FontWeights.Normal,Foreground=BrushesForForm.Secondary,Margin=new Thickness(0,0,0,6)});
        if(editor is Control control)PrepareEditor(control);
        field.Children.Add(editor);return field;
    }

    internal static void PrepareEditor(Control editor)
    {
        editor.MinWidth=0;editor.MinHeight=38;editor.Margin=new Thickness(0);
        editor.FontSize=13;editor.FontWeight=FontWeights.Normal;
        editor.HorizontalAlignment=HorizontalAlignment.Stretch;
    }

    internal void AddAction(Button button,string label)
    {
        button.Content=label;button.FontSize=12;button.FontWeight=FontWeights.Normal;
        button.MinHeight=34;button.Padding=new Thickness(13,7,13,7);button.Margin=new Thickness(0,0,8,4);
        button.HorizontalAlignment=HorizontalAlignment.Left;
        button.SetResourceReference(StyleProperty,"SecondaryButton");
        System.Windows.Automation.AutomationProperties.SetName(button,label);Actions.Children.Add(button);
    }

    private static class BrushesForForm
    {
        internal static readonly Brush Primary=new SolidColorBrush(Color.FromRgb(23,32,51));
        internal static readonly Brush Secondary=new SolidColorBrush(Color.FromRgb(99,112,137));
    }
}
