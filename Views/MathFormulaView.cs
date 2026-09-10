// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

internal sealed class MathFormulaView:Image
{
    internal string OriginalText { get; }
    internal MathFormulaView(string text,DrawingImage source,bool copyMenu=true)
    {
        OriginalText=text;Source=source;Width=source.Width;Height=source.Height;Stretch=Stretch.None;
        HorizontalAlignment=HorizontalAlignment.Left;Margin=new Thickness(0,3,0,3);ToolTip=text;
        System.Windows.Automation.AutomationProperties.SetName(this,text);
        if(copyMenu)ContextMenu=TextSelectionMenu.Create(()=>text,()=>text,value=>ClipboardService.TrySetText(value,out string? _));
    }
}
