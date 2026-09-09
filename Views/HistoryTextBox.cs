// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

namespace mewu_ai_Assistant.Views;

internal sealed class HistoryTextBox : TextBox
{
    internal HistoryTextBox(string text,Action<string> copyText)
    {
        var fullText=text??string.Empty;
        var preview=fullText.Trim();
        Text=preview.Length<=900?preview:preview[..900]+"…";
        IsReadOnly=true;IsReadOnlyCaretVisible=false;IsUndoEnabled=false;
        Style=null;Background=Brushes.Transparent;BorderThickness=new Thickness(0);
        Padding=new Thickness(0);FontSize=12;TextWrapping=TextWrapping.Wrap;
        Foreground=new SolidColorBrush(Color.FromRgb(47,61,82));Cursor=Cursors.IBeam;
        HorizontalScrollBarVisibility=VerticalScrollBarVisibility=ScrollBarVisibility.Disabled;
        TextBlock.SetLineHeight(this,18);
        ContextMenu=TextSelectionMenu.Create(()=>SelectedText,()=>fullText,copyText);
        CommandBindings.Add(new CommandBinding(ApplicationCommands.Copy,
            (_,e)=>{if(SelectionLength>0)copyText(SelectedText);e.Handled=true;},
            (_,e)=>{e.CanExecute=SelectionLength>0;e.Handled=true;}));
    }
}
