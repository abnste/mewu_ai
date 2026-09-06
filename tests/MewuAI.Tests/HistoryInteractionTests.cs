// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Runtime.ExceptionServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Xunit;

namespace MewuAI.Tests;

public sealed class HistoryInteractionTests
{
    [Fact]
    public void HistorySupportsSelectionCopyAndFullUntruncatedCopy()
    {
        RunSta(()=>
        {
            var full="第一行中文\n第二行文字"+new string('长',1000)+"末尾";
            string? copied=null;var box=new HistoryTextBox(full,text=>copied=text);
            Assert.True(box.IsReadOnly);Assert.True(box.IsHitTestVisible);
            Assert.InRange(box.Text.Length,1,901);
            box.Select(2,7);
            Assert.True(ApplicationCommands.Copy.CanExecute(null,box));
            ApplicationCommands.Copy.Execute(null,box);
            Assert.Equal(box.Text.Substring(2,7),copied);
            ((MenuItem)box.ContextMenu.Items[1]).RaiseEvent(new RoutedEventArgs(MenuItem.ClickEvent));
            Assert.Equal(full,copied);
            box.Select(0,0);Assert.False(ApplicationCommands.Copy.CanExecute(null,box));
        });
    }

    [Fact]
    public void ForegroundConversationWinsOverToolbarAndItsTransitPadding()
    {
        var prompt=new Rect(300,400,574,240);var toolbar=new Rect(350,430,400,40);
        foreach(var point in new[]{new Point(400,450),new Point(400,423)})
        {
            Assert.True(CaptureOverlayPolicy.IsPointerInFloatingBarInteractionZone(point,toolbar,12));
            Assert.False(CaptureOverlayPolicy.IsPointerInFloatingBarInteractionZone(point,toolbar,12,prompt));
        }
        // A hidden prompt must not keep its old footprint as an obstacle.
        Assert.True(CaptureOverlayPolicy.IsPointerInFloatingBarInteractionZone(new Point(400,450),toolbar,12,null));
        Assert.True(CaptureOverlayPolicy.IsPointerInFloatingBarInteractionZone(new Point(295,450),new Rect(280,430,50,40),12,prompt));
    }

    [Fact]
    public void UnchangedHistoryReusesControlsAndKeepsSelectionWhenANewTurnArrives()
    {
        RunSta(()=>
        {
            var panel=new HistoryPreviewPanel();var created=0;
            UIElement Create(HistoryPreviewEntry entry){created++;return new HistoryTextBox(entry.Answer,_=>{});}
            var entries=Enumerable.Range(0,6).Select(i=>new HistoryPreviewEntry($"Q{i}",$"回答{i}内容",false)).ToArray();
            panel.UpdateRows(entries,Create);
            var selected=(HistoryTextBox)panel.Children[2];selected.Select(1,3);
            var originalSelection=selected.SelectedText;
            var allocatedBefore=GC.GetAllocatedBytesForCurrentThread();
            for(var i=0;i<1000;i++)panel.UpdateRows(entries,Create);
            var allocated=GC.GetAllocatedBytesForCurrentThread()-allocatedBefore;
            Assert.Equal(6,created);Assert.Same(selected,panel.Children[2]);Assert.Equal(originalSelection,selected.SelectedText);
            panel.UpdateRows(entries.Skip(1).Append(new HistoryPreviewEntry("Q6","回答6内容",true)).ToArray(),Create);
            Assert.Equal(7,created);Assert.Same(selected,panel.Children[1]);Assert.Equal(originalSelection,selected.SelectedText);
            TestContext.Current.TestOutputHelper!.WriteLine($"1000 unchanged refreshes: {allocated} allocated bytes, 0 new controls.");
        });
    }

    [Fact]
    public void DuplicateHistoryRowsHaveIndependentControlsAndEmptyHistoryRemovesOldContent()
    {
        RunSta(()=>
        {
            var panel=new HistoryPreviewPanel();var same=new HistoryPreviewEntry("same","same",false);
            panel.UpdateRows([same,same],entry=>new HistoryTextBox(entry.Answer,_=>{}));
            Assert.NotSame(panel.Children[0],panel.Children[1]);
            panel.UpdateRows([],entry=>throw new InvalidOperationException());Assert.Empty(panel.Children);
        });
    }

    private static void RunSta(Action action)
    {
        Exception? error=null;
        var thread=new Thread(()=>{try{action();}catch(Exception ex){error=ex;}});
        thread.SetApartmentState(ApartmentState.STA);thread.Start();
        Assert.True(thread.Join(TimeSpan.FromSeconds(20)));
        if(error is not null)ExceptionDispatchInfo.Capture(error).Throw();
    }
}
