using System.Windows;
using System.Windows.Controls;

namespace mewu_ai_Assistant.Views;

internal readonly record struct HistoryPreviewEntry(string Prompt,string Answer,bool IsCurrent);

public sealed class HistoryPreviewPanel : StackPanel
{
    private readonly List<(HistoryPreviewEntry Entry,UIElement View)> _rows=[];

    internal void UpdateRows(IReadOnlyList<HistoryPreviewEntry> entries,Func<HistoryPreviewEntry,UIElement> create)
    {
        var unchanged=entries.Count==_rows.Count;
        for(var i=0;unchanged&&i<entries.Count;i++)unchanged=entries[i]==_rows[i].Entry;
        if(unchanged)return;
        var used=new bool[_rows.Count];
        var next=new List<(HistoryPreviewEntry Entry,UIElement View)>(entries.Count);
        foreach(var entry in entries)
        {
            UIElement? view=null;
            for(var i=0;i<_rows.Count;i++)
            {
                if(used[i]||_rows[i].Entry!=entry)continue;
                used[i]=true;view=_rows[i].View;break;
            }
            next.Add((entry,view??create(entry)));
        }
        for(var i=0;i<next.Count;i++)
        {
            var view=next[i].View;
            if(i<Children.Count&&ReferenceEquals(Children[i],view))continue;
            if(Children.Contains(view))Children.Remove(view);
            Children.Insert(i,view);
        }
        while(Children.Count>next.Count)Children.RemoveAt(Children.Count-1);
        _rows.Clear();_rows.AddRange(next);
    }
}
