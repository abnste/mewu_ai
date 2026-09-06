// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class AiSettingsTabs : UserControl
{
    internal const int ApiIndex=0;
    internal const int HermesIndex=1;
    internal const int CodexIndex=2;
    internal const int WorkBuddyIndex=3;
    internal const int MiniMaxCodeIndex=4;
    internal TabControl Tabs=>BackendTabs;
    internal int SelectedBackendIndex=>BackendTabs.SelectedIndex;
    internal event EventHandler? BackendChanged;

    public AiSettingsTabs(UIElement api,UIElement hermes,UIElement? codex=null,int selectedBackendIndex=0,UIElement? workBuddy=null,UIElement? miniMaxCode=null)
    {
        InitializeComponent();
        ChannelPrompt.Text=LocalizationService.T("请选择AI渠道：","Choose an AI channel:");
        System.Windows.Automation.AutomationProperties.SetName(BackendTabs,LocalizationService.T("AI 接入方式","AI integrations"));
        AddPage("API",api);
        AddPage("Hermes",hermes);
        AddPage("Codex",codex??ComingSoon("Codex"));
        AddPage("WorkBuddy",workBuddy??ComingSoon("WorkBuddy"),workBuddy is not null);
        if(miniMaxCode is not null)AddPage("MiniMax Code",miniMaxCode);
        foreach(var name in new[]{"OpenClaw","Claude Code"})AddPage(name,ComingSoon(name),false);
        BackendTabs.SelectedIndex=selectedBackendIndex is >=ApiIndex and <=CodexIndex||selectedBackendIndex==WorkBuddyIndex&&workBuddy is not null||selectedBackendIndex==MiniMaxCodeIndex&&miniMaxCode is not null?selectedBackendIndex:ApiIndex;
        BackendTabs.SelectionChanged+=(_,e)=>
        {
            if(ReferenceEquals(e.Source,BackendTabs))BackendChanged?.Invoke(this,EventArgs.Empty);
        };
    }

    private void AddPage(string name,UIElement content,bool available=true)
    {
        // Each page owns its existing controls and scroll position. Switching
        // backends must not rebuild editors or discard other backend drafts.
        var tab=new TabItem
        {
            Header=name,
            IsEnabled=available,
            ToolTip=available?null:LocalizationService.T("陆续适配中","Integration coming soon"),
            Content=new ScrollViewer
            {
                Content=content,VerticalScrollBarVisibility=ScrollBarVisibility.Auto,
                HorizontalScrollBarVisibility=ScrollBarVisibility.Disabled,Padding=new Thickness(2)
            }
        };
        ToolTipService.SetShowOnDisabled(tab,true);
        System.Windows.Automation.AutomationProperties.SetName(tab,name);
        BackendTabs.Items.Add(tab);
    }

    private static UIElement ComingSoon(string name)
    {
        var text=new StackPanel{HorizontalAlignment=HorizontalAlignment.Center,Margin=new Thickness(20,72,20,40)};
        text.Children.Add(new TextBlock{Text=name,FontSize=22,FontWeight=FontWeights.SemiBold,Foreground=new SolidColorBrush(Color.FromRgb(47,59,82)),HorizontalAlignment=HorizontalAlignment.Center});
        text.Children.Add(new TextBlock{Text=LocalizationService.T("陆续适配中","Integration coming soon"),FontSize=13,Foreground=new SolidColorBrush(Color.FromRgb(119,131,152)),Margin=new Thickness(0,12,0,0),HorizontalAlignment=HorizontalAlignment.Center});
        return text;
    }
}
