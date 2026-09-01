using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class UndoRedoHistoryTests
{
    [Fact]
    public void RecordUndoRedo_RestoresBothStates()
    {
        var history=new UndoRedoHistory<int>();
        history.Record(1,2,"新建截图区域");

        Assert.True(history.TryUndo(out var undone,out var undoLabel));
        Assert.Equal(1,undone);
        Assert.Equal("新建截图区域",undoLabel);
        Assert.True(history.TryRedo(out var redone,out var redoLabel));
        Assert.Equal(2,redone);
        Assert.Equal(undoLabel,redoLabel);
    }

    [Fact]
    public void NewRecord_ClearsRedoBranch()
    {
        var history=new UndoRedoHistory<int>();
        history.Record(1,2,"A");
        Assert.True(history.TryUndo(out _,out _));

        history.Record(1,3,"B");

        Assert.False(history.TryRedo(out _,out _));
    }

    [Fact]
    public void Capacity_DropsOldestEntry()
    {
        var history=new UndoRedoHistory<int>(2);
        history.Record(0,1,"A");
        history.Record(1,2,"B");
        history.Record(2,3,"C");

        Assert.True(history.TryUndo(out var second,out _));
        Assert.Equal(2,second);
        Assert.True(history.TryUndo(out var first,out _));
        Assert.Equal(1,first);
        Assert.False(history.TryUndo(out _,out _));
    }
}
