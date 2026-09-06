// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class AiSettingsTabs : UserControl
{
    internal TabControl Tabs=>BackendTabs;

    public AiSettingsTabs(UIElement api,UIElement hermes)
    {
        InitializeComponent();
        System.Windows.Automation.AutomationProperties.SetName(BackendTabs,LocalizationService.T("AI 接入方式","AI integrations"));
        AddPage("API",api);
        AddPage("Hermes",hermes);
        foreach(var name in new[]{"Codex","OpenClaw","Claude Code","WorkBuddy"})AddPage(name,ComingSoon(name));
        BackendTabs.SelectedIndex=0;
    }

    private void AddPage(string name,UIElement content)
    {
        // Each page owns its existing controls and scroll position. Switching
        // tabs must not rebuild editors, discard drafts, or activate a backend.
        var tab=new TabItem
        {
            Header=name,
            Content=new ScrollViewer
            {
                Content=content,VerticalScrollBarVisibility=ScrollBarVisibility.Auto,
                HorizontalScrollBarVisibility=ScrollBarVisibility.Disabled,Padding=new Thickness(2)
            }
        };
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
