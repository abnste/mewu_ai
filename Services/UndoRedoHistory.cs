namespace mewu_ai_Assistant.Services;

internal sealed class UndoRedoHistory<T>
{
    private readonly int _capacity;
    private readonly List<Entry> _undo=[];
    private readonly List<Entry> _redo=[];

    internal UndoRedoHistory(int capacity=50)
    {
        ArgumentOutOfRangeException.ThrowIfLessThan(capacity,1);
        _capacity=capacity;
    }

    internal int UndoCount=>_undo.Count;
    internal int RedoCount=>_redo.Count;

    internal void Record(T before,T after,string label)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(label);
        _undo.Add(new Entry(before,after,label));
        if(_undo.Count>_capacity)_undo.RemoveAt(0);
        _redo.Clear();
    }

    internal bool TryUndo(out T state,out string label)
    {
        if(_undo.Count==0){state=default!;label=string.Empty;return false;}
        var entry=_undo[^1];_undo.RemoveAt(_undo.Count-1);_redo.Add(entry);state=entry.Before;label=entry.Label;return true;
    }

    internal bool TryRedo(out T state,out string label)
    {
        if(_redo.Count==0){state=default!;label=string.Empty;return false;}
        var entry=_redo[^1];_redo.RemoveAt(_redo.Count-1);_undo.Add(entry);state=entry.After;label=entry.Label;return true;
    }

    internal void Clear(){_undo.Clear();_redo.Clear();}

    private sealed record Entry(T Before,T After,string Label);
}
