// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows.Controls;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

internal static class TextSelectionMenu
{
    internal static ContextMenu Create(Func<string> selectedText,Func<string> fullText,Action<string> copyText)
    {
        var menu=new ContextMenu();menu.SetResourceReference(ContextMenu.StyleProperty,"TextSelectionContextMenu");
        var selected=new MenuItem{Header=LocalizationService.T("复制所选文字","Copy selected text")};
        var all=new MenuItem{Header=LocalizationService.T("复制完整内容","Copy full text")};
        selected.SetResourceReference(MenuItem.StyleProperty,"TextSelectionMenuItem");
        all.SetResourceReference(MenuItem.StyleProperty,"TextSelectionMenuItem");
        selected.Click+=(_,_)=>{var text=selectedText();if(text.Length>0)copyText(text);};
        all.Click+=(_,_)=>copyText(fullText());
        menu.Items.Add(selected);menu.Items.Add(all);
        menu.Opened+=(_,_)=>{selected.IsEnabled=selectedText().Length>0;all.IsEnabled=fullText().Length>0;};
        return menu;
    }
}
