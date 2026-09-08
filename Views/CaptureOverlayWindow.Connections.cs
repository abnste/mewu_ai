// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private bool _connectionRenderQueued;
    private bool HasAiAnnotations(SelectionItem item)=>item.AnnotationNotes.Count>0||GetCrossRegionConnections().Any(link=>link.Annotation.Destination?.ReferenceHandle==item.ReferenceHandle);

    private IReadOnlyList<CrossRegionConnection> GetCrossRegionConnections()
    {
        var items=_selections.Where(item=>!item.IsImplicit).ToArray();
        var targets=items.Select(item=>new AnnotationReferenceTarget(item.ReferenceHandle,item.VideoPath is not null)).ToArray();
        var connections=new List<CrossRegionConnection>();
        foreach(var item in items)
        foreach(var note in item.AnnotationNotes.Where(note=>note.Kind==AiAnnotationKind.Connection))
        {
            if(connections.Count>=CrossRegionConnectionService.MaximumConnections)return connections;
            if(item.ReferenceHandle!=note.ReferenceHandle||!CrossRegionConnectionService.TryResolve(note,targets,out var resolved,out var from,out var to))continue;
            var source=items[from];var destination=items[to];var end=resolved.Destination!;
            connections.Add(new(resolved,CrossRegionConnectionService.Project(source.Bounds,note.X,note.Y,note.Width,note.Height),
                CrossRegionConnectionService.Project(destination.Bounds,end.X,end.Y,end.Width,end.Height),GetReferenceLabel(source),GetReferenceLabel(destination)));
        }
        return connections;
    }

    private void QueueCrossRegionConnections()
    {
        if(_closed||_connectionRenderQueued)return;
        _connectionRenderQueued=true;
        _=Dispatcher.BeginInvoke(System.Windows.Threading.DispatcherPriority.Render,new Action(()=>
        {
            _connectionRenderQueued=false;if(_closed)return;
            var links=GetCrossRegionConnections();
            CrossRegionConnections.Source=links.Count==0?null:CrossRegionConnectionRenderer.CreateDrawing(Root.ActualWidth,Root.ActualHeight,links);
            CrossRegionConnections.Width=Math.Max(1,Root.ActualWidth);CrossRegionConnections.Height=Math.Max(1,Root.ActualHeight);
        }));
    }

    private void RemoveConnectionsTouching(SelectionItem target)
    {
        foreach(var item in _selections)
            item.AnnotationNotes.RemoveAll(note=>note.Kind==AiAnnotationKind.Connection&&(note.ReferenceHandle==target.ReferenceHandle||note.Destination?.ReferenceHandle==target.ReferenceHandle));
        QueueCrossRegionConnections();
    }
}
