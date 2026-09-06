// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Runtime.ExceptionServices;
using System.Windows;
using System.Windows.Controls;
using mewu_ai_Assistant.Views;
using Xunit;

namespace MewuAI.Tests;

public sealed class AiSettingsTabsTests
{
    [Fact]
    public void SwitchingBackendsAndPlaceholdersPreservesUnsavedEditorInstancesAndValues()
    {
        RunSta(()=>
        {
            var api=new TextBox{Text="unsaved-model-id"};api.Select(2,5);
            var hermes=new TextBox{Text="unsaved-profile"};
            var codex=new TextBox{Text="unsaved-codex-model"};
            var view=new AiSettingsTabs(api,hermes,codex);
            var tabs=view.Tabs;
            Assert.Equal(new[]{"API","Hermes","Codex","OpenClaw","Claude Code","WorkBuddy"},tabs.Items.Cast<TabItem>().Select(t=>t.Header));
            Assert.Equal(0,tabs.SelectedIndex);
            for(var i=1;i<tabs.Items.Count;i++)
            {
                tabs.SelectedIndex=i;
                view.Measure(new Size(500,350));view.Arrange(new Rect(0,0,500,350));view.UpdateLayout();
                if(i>=3)
                {
                    var placeholder=Assert.IsType<StackPanel>(Assert.IsType<ScrollViewer>(((TabItem)tabs.SelectedItem).Content).Content);
                    Assert.All(placeholder.Children.Cast<UIElement>(),element=>Assert.IsType<TextBlock>(element));
                }
            }
            tabs.SelectedIndex=0;
            Assert.Same(api,((ScrollViewer)((TabItem)tabs.SelectedItem).Content).Content);
            Assert.Equal("unsaved-model-id",api.Text);Assert.Equal(2,api.SelectionStart);Assert.Equal(5,api.SelectionLength);
            tabs.SelectedIndex=1;
            Assert.Same(hermes,((ScrollViewer)((TabItem)tabs.SelectedItem).Content).Content);
            Assert.Equal("unsaved-profile",hermes.Text);
            tabs.SelectedIndex=2;
            Assert.Same(codex,((ScrollViewer)((TabItem)tabs.SelectedItem).Content).Content);
            Assert.Equal("unsaved-codex-model",codex.Text);
        });
    }

    private static void RunSta(Action action)
    {
        Exception? error=null;
        var thread=new Thread(()=>{try{action();}catch(Exception ex){error=ex;}}){IsBackground=true};
        thread.SetApartmentState(ApartmentState.STA);thread.Start();
        Assert.True(thread.Join(TimeSpan.FromSeconds(20)),"WPF tab test timed out");
        if(error is not null)ExceptionDispatchInfo.Capture(error).Throw();
    }
}
