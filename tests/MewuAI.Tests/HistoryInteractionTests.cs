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

    private static void RunSta(Action action)
    {
        Exception? error=null;
        var thread=new Thread(()=>{try{action();}catch(Exception ex){error=ex;}});
        thread.SetApartmentState(ApartmentState.STA);thread.Start();
        Assert.True(thread.Join(TimeSpan.FromSeconds(20)));
        if(error is not null)ExceptionDispatchInfo.Capture(error).Throw();
    }
}
