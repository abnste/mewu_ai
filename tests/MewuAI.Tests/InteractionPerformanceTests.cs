using System.Runtime.ExceptionServices;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Xunit;

namespace MewuAI.Tests;

public sealed class InteractionPerformanceTests
{
    [Fact]
    public void StreamBurstPostsOneWakeupAndPreservesEveryDelta()
    {
        RunSta(() =>
        {
            var dispatcher=Dispatcher.CurrentDispatcher;
            var posted=0;
            DispatcherHookEventHandler hook=(_,_)=>posted++;
            dispatcher.Hooks.OperationPosted+=hook;
            var batches=new List<AiStreamDelta>();
            using var progress=new BufferedAiStreamProgress(dispatcher,()=>true,batches.Add);
            try
            {
                var producer=Task.Run(()=>
                {
                    for(var i=0;i<10_000;i++)progress.Report(new("答","想"));
                });
                Assert.True(producer.Wait(TimeSpan.FromSeconds(20)));
                producer.GetAwaiter().GetResult();
                Assert.Equal(1,posted);
                progress.Flush();
                var batch=Assert.Single(batches);
                Assert.Equal(new string('答',10_000),batch.Content);
                Assert.Equal(new string('想',10_000),batch.ReasoningContent);
                progress.Flush();
                Assert.Single(batches);
            }
            finally { dispatcher.Hooks.OperationPosted-=hook; }
        });
    }

    [Fact]
    public void StreamRejectsCanceledBatchAndAllLateDeltas()
    {
        RunSta(() =>
        {
            var current=true;var batches=new List<AiStreamDelta>();
            using var progress=new BufferedAiStreamProgress(Dispatcher.CurrentDispatcher,()=>current,batches.Add);
            progress.Report(new("old","thinking"));
            current=false;progress.Flush();
            current=true;progress.Report(new("late","late"));progress.Flush();
            Assert.Empty(batches);
        });
    }

    [Fact]
    public void StreamRenderFailureIsReportedToRequestInsteadOfEscapingDispatcher()
    {
        RunSta(() =>
        {
            using var progress=new BufferedAiStreamProgress(Dispatcher.CurrentDispatcher,()=>true,_=>throw new InvalidDataException("render"));
            progress.Report(new("answer",""));
            progress.Flush();
            Assert.Throws<InvalidDataException>(progress.ThrowIfFaulted);
        });
    }

    [Fact]
    public void StreamDisposeDropsAlreadyQueuedWakeup()
    {
        RunSta(() =>
        {
            var dispatcher=Dispatcher.CurrentDispatcher;var batches=new List<AiStreamDelta>();
            var progress=new BufferedAiStreamProgress(dispatcher,()=>true,batches.Add);
            progress.Report(new("queued",""));progress.Dispose();
            var frame=new DispatcherFrame();
            dispatcher.BeginInvoke(DispatcherPriority.ContextIdle,new Action(()=>frame.Continue=false));
            Dispatcher.PushFrame(frame);
            Assert.Empty(batches);
        });
    }

    [Fact]
    public void SelectionCacheReusesCropButInvalidatesFrameBoundsAndReset()
    {
        RunSta(() =>
        {
            var first=CreateBitmap(11);var second=CreateBitmap(99);
            var cache=new SelectionImageCache();var bounds=new Int32Rect(1,1,3,3);
            var crop=cache.Get(first,bounds);
            for(var i=0;i<1_000;i++)Assert.Same(crop,cache.Get(first,bounds));
            var moved=cache.Get(first,new Int32Rect(2,2,3,3));Assert.NotSame(crop,moved);
            var refreshed=cache.Get(second,bounds);Assert.NotSame(moved,refreshed);
            var pixels=new byte[36];refreshed.CopyPixels(pixels,12,0);Assert.Equal(99,pixels[0]);
            Assert.Same(first,cache.Get(second,bounds,first));
            Assert.Same(refreshed,cache.Get(second,bounds));
            cache.Clear();Assert.NotSame(refreshed,cache.Get(second,bounds));
        });
    }

    [Fact]
    public void MarkdownDoesNotRebuildUnchangedTextOrRetainStaleActions()
    {
        RunSta(() =>
        {
            var view=new MarkdownAnswerView{Markdown="answer"};var original=view.Document;
            view.SetMarkdownWithActions("answer",[]);Assert.Same(original,view.Document);
            view.SetMarkdownWithActions("answer",[new("marker","marker",()=>{})]);
            Assert.NotSame(original,view.Document);
            view.Markdown="answer";
            Assert.Single(view.Document.Blocks);
            Assert.False(view.IsUndoEnabled);
            view.Markdown="| A | B |\n| --- | --- |\n| 1 | 2 |";Assert.True(view.ContainsTable);
            view.Markdown="plain";Assert.False(view.ContainsTable);
        });
    }

    [Fact]
    public void HistoryRetainsRedoResourcesAndReleasesExpiredBranches()
    {
        var history=new UndoRedoHistory<string>(2);
        history.Record("A","B","one");history.Record("B","C","two");
        history.TryUndo(out _,out _);
        Assert.Contains("C",history.RetainedStates);
        history.Record("B","D","replacement");Assert.DoesNotContain("C",history.RetainedStates);
        history.Record("D","E","three");Assert.DoesNotContain("A",history.RetainedStates);
        Assert.Contains("B",history.RetainedStates);Assert.Contains("E",history.RetainedStates);
        history.Clear();Assert.Empty(history.RetainedStates);
    }

    [Fact]
    public async Task HistoryTailSkipsPartialUtf8AndPreservesRecentCompleteRecords()
    {
        var directory=Path.Combine(Path.GetTempPath(),"MewuAI-Tests",Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            var path=Path.Combine(directory,"history.jsonl");
            // The tail starts inside an oversized multibyte record. A decoder
            // must never see that partial character/record prefix.
            await File.WriteAllTextAsync(path,new string('中',ConversationHistoryService.MaxReadBytes/3+100)+"\n",new System.Text.UTF8Encoding(false),TestContext.Current.CancellationToken);
            var service=new ConversationHistoryService(path,(_,_)=>{});
            await service.AppendAsync("provider","model","问题一 🎉","回答一",TestContext.Current.CancellationToken);
            await File.AppendAllTextAsync(path,"broken JSON\n",TestContext.Current.CancellationToken);
            await service.AppendAsync("provider","model","问题二","回答二 👋",TestContext.Current.CancellationToken);
            var entries=await service.ReadRecentAsync(token:TestContext.Current.CancellationToken);
            Assert.Equal(2,entries.Count);
            Assert.Equal("问题一 🎉",entries[0].Prompt);
            Assert.Equal("回答二 👋",entries[1].Answer);
        }
        finally { Directory.Delete(directory,true); }
    }

    [Fact]
    public async Task HistoryTailWithNoCompleteLineReturnsEmptyWithoutDecodingOversizedRecord()
    {
        var directory=Path.Combine(Path.GetTempPath(),"MewuAI-Tests",Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            var path=Path.Combine(directory,"history.jsonl");
            await using(var stream=File.Create(path))stream.SetLength(ConversationHistoryService.MaxReadBytes+1L);
            var service=new ConversationHistoryService(path,(_,_)=>{});
            Assert.Empty(await service.ReadRecentAsync(token:TestContext.Current.CancellationToken));
        }
        finally { Directory.Delete(directory,true); }
    }

    private static BitmapSource CreateBitmap(byte value)
    {
        var image=BitmapSource.Create(8,8,96,96,PixelFormats.Bgr32,null,Enumerable.Repeat(value,256).ToArray(),32);
        image.Freeze();return image;
    }

    private static void RunSta(Action action)
    {
        Exception? failure=null;
        var thread=new Thread(()=>
        {
            try { action(); }
            catch(Exception error) { failure=error; }
            finally { Dispatcher.CurrentDispatcher.InvokeShutdown(); }
        }){IsBackground=true};
        thread.SetApartmentState(ApartmentState.STA);thread.Start();
        Assert.True(thread.Join(TimeSpan.FromSeconds(30)),"STA verification did not complete");
        if(failure is not null)ExceptionDispatchInfo.Capture(failure).Throw();
    }
}
