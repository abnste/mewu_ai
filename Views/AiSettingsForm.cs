// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Data;
using System.Windows.Documents;
using System.Windows.Media;
using mewu_ai_Assistant.Services;

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

    internal static FrameworkElement Field(string label,FrameworkElement editor,TextBlock? inlineStatus=null)
    {
        var field=new StackPanel{Margin=new Thickness(0,0,0,14)};
        var heading=new TextBlock{FontSize=12,FontWeight=FontWeights.Normal,Foreground=BrushesForForm.Secondary,Margin=new Thickness(0,0,0,6)};
        if(inlineStatus is not null)
        {
            // The localization watcher writes TextBlock.Text, which would replace
            // these bound runs. Localize the title and status individually instead.
            LocalizationService.SetExcludeFromLocalization(heading,true);
            heading.TextWrapping=TextWrapping.NoWrap;
            heading.TextTrimming=TextTrimming.CharacterEllipsis;
            heading.Inlines.Add(new Run(LocalizationService.TranslateUiText(label)));
            var hint=new Run{FontSize=11};
            hint.SetBinding(Run.TextProperty,new Binding(nameof(TextBlock.Text)){Source=inlineStatus,Mode=BindingMode.OneWay,Converter=InlineStatusConverter.Instance});
            hint.SetBinding(TextElement.ForegroundProperty,new Binding(nameof(TextBlock.Foreground)){Source=inlineStatus,Mode=BindingMode.OneWay});
            heading.Inlines.Add(hint);
            var tooltip=new TextBlock{MaxWidth=440,TextWrapping=TextWrapping.Wrap};
            tooltip.SetBinding(TextBlock.TextProperty,new Binding(nameof(TextBlock.Text)){Source=inlineStatus,Mode=BindingMode.OneWay,Converter=InlineStatusConverter.Instance,ConverterParameter=false});
            heading.ToolTip=tooltip;
        }
        else heading.Text=label;
        field.Children.Add(heading);
        if(editor is Control control)PrepareEditor(control);
        field.Children.Add(editor);return field;
    }

    private sealed class InlineStatusConverter:IValueConverter
    {
        internal static readonly InlineStatusConverter Instance=new();
        public object Convert(object value,Type targetType,object parameter,System.Globalization.CultureInfo culture)
        {
            if(value is not string text||string.IsNullOrWhiteSpace(text))return string.Empty;
            var localized=LocalizationService.TranslateUiText(text);
            return parameter is false?localized:$" ({localized.Trim().ReplaceLineEndings(" ")})";
        }
        public object ConvertBack(object value,Type targetType,object parameter,System.Globalization.CultureInfo culture)
            =>Binding.DoNothing;
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
