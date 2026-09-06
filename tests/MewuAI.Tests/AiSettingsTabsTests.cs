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
    public void WorkBuddyUsesSingleSelectionAndRetainsDraftWhenSwitching()
    {
        RunSta(()=>
        {
            var workBuddy=new TextBox{Text="saved-workbuddy-model"};
            var view=new AiSettingsTabs(new TextBox(),new TextBox(),new TextBox(),AiSettingsTabs.WorkBuddyIndex,workBuddy);
            view.Measure(new Size(500,350));view.Arrange(new Rect(0,0,500,350));view.UpdateLayout();
            Assert.Equal(AiSettingsTabs.WorkBuddyIndex,view.SelectedBackendIndex);
            Assert.True(((TabItem)view.Tabs.Items[AiSettingsTabs.WorkBuddyIndex]).IsEnabled);
            Assert.False(((TabItem)view.Tabs.Items[4]).IsEnabled);
            Assert.False(((TabItem)view.Tabs.Items[5]).IsEnabled);
            workBuddy.Text="unsaved-workbuddy-model";
            view.Tabs.SelectedIndex=2;view.Tabs.SelectedIndex=0;view.Tabs.SelectedIndex=AiSettingsTabs.WorkBuddyIndex;
            Assert.Equal("unsaved-workbuddy-model",workBuddy.Text);
            Assert.Same(workBuddy,((ScrollViewer)((TabItem)view.Tabs.SelectedItem).Content).Content);
            Assert.Single(view.Tabs.Items.Cast<TabItem>(),item=>item.IsSelected);
        });
    }

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
            Assert.Equal(new[]{"API","Hermes","Codex","WorkBuddy","OpenClaw","Claude Code"},tabs.Items.Cast<TabItem>().Select(t=>t.Header));
            Assert.Equal(0,tabs.SelectedIndex);
            for(var i=1;i<3;i++)
            {
                tabs.SelectedIndex=i;
                view.Measure(new Size(500,350));view.Arrange(new Rect(0,0,500,350));view.UpdateLayout();
                Assert.Equal(i,view.SelectedBackendIndex);
                Assert.Single(tabs.Items.Cast<TabItem>(),item=>item.IsSelected);
            }
            Assert.All(tabs.Items.Cast<TabItem>().Skip(3),item=>{Assert.False(item.IsEnabled);Assert.NotNull(item.ToolTip);});
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

    [Theory]
    [InlineData(0)]
    [InlineData(1)]
    [InlineData(2)]
    public void OpensOnSavedBackendAndIgnoresNestedModelSelection(int savedBackend)
    {
        RunSta(()=>
        {
            var models=new ComboBox();models.Items.Add("first");models.Items.Add("second");
            var view=new AiSettingsTabs(models,new TextBox(),new TextBox(),savedBackend);
            view.Measure(new Size(500,350));view.Arrange(new Rect(0,0,500,350));view.UpdateLayout();
            Assert.Equal(savedBackend,view.SelectedBackendIndex);
            var changes=0;view.BackendChanged+=(_,_)=>changes++;
            view.Tabs.SelectedIndex=0;
            var beforeModelSelection=changes;models.SelectedIndex=1;
            Assert.Equal(beforeModelSelection,changes);
            view.Tabs.SelectedIndex=2;
            Assert.Equal(beforeModelSelection+1,changes);
            Assert.Equal(2,view.SelectedBackendIndex);
            view.Tabs.SelectedIndex=0;Assert.Equal("second",models.SelectedItem);
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
