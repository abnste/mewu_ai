// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Media;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private sealed record ManualDrawingSnapshot(Stroke[] Strokes,DrawingElementSpec[] Elements,
        RegionMark[] Marks,DrawingAction[] Order,DrawingAction[] Redo,int NextNumber)
    {
        public Guid NumberPreference {get;init;}
    }

    // A drawing can be referenced by its live canvas, an erase/clear action and
    // several geometry/style actions. One mapper owns the entire object graph,
    // so cloning or changing the selection origin never duplicates/translates a
    // shared stroke or region more than once.
    private sealed class ManualDrawingMapper(bool clone,Vector offset)
    {
        private readonly Dictionary<Stroke,Stroke> _strokes=new(ReferenceEqualityComparer.Instance);
        private readonly Dictionary<StrokeDrawingState,StrokeDrawingState> _states=new(ReferenceEqualityComparer.Instance);
        private readonly Dictionary<RegionMark,RegionMark> _marks=new(ReferenceEqualityComparer.Instance);
        private readonly Dictionary<DrawingElementSpec,DrawingElementSpec> _elements=new(ReferenceEqualityComparer.Instance);

        internal Stroke Stroke(Stroke original)
        {
            if(_strokes.TryGetValue(original,out var mapped))return mapped;
            mapped=clone?original.Clone():original;
            if(clone&&mapped is mewu_ai_Assistant.Services.BackgroundHighlightStroke snapshotHighlight)snapshotHighlight.ReleaseDerivedSource();
            _strokes.Add(original,mapped);
            // WPF translates the existing point collection and raises one
            // change notification; rebuilding it allocates several point arrays
            // per visible and history-only stroke on every pointer update.
            if(offset.X!=0||offset.Y!=0)mapped.Transform(new Matrix(1,0,0,1,offset.X,offset.Y),false);
            if(mapped is mewu_ai_Assistant.Services.BackgroundHighlightStroke highlight)highlight.RebaseSource(offset);
            return mapped;
        }

        internal RegionMark Mark(RegionMark original)
        {
            if(_marks.TryGetValue(original,out var mapped))return mapped;
            mapped=clone?CloneRegionMark(original):original;
            _marks.Add(original,mapped);
            if(offset.X!=0||offset.Y!=0)
            {
                var bounds=mapped.Bounds;bounds.Offset(offset);mapped.Bounds=bounds;
            }
            return mapped;
        }

        internal DrawingElementSpec Element(DrawingElementSpec original)
        {
            if(_elements.TryGetValue(original,out var mapped))return mapped;
            // Element records and frozen mosaic pixels are immutable and safe
            // to share between snapshots. Only their local geometry changes.
            mapped=offset.X==0&&offset.Y==0?original:MoveDrawingElement(original,offset);
            _elements.Add(original,mapped);
            return mapped;
        }

        private StrokeDrawingState State(StrokeDrawingState original)
        {
            if(_states.TryGetValue(original,out var mapped))return mapped;
            // Snapshots own their mutable point arrays. The live history graph
            // can rebase its own arrays in place, once per shared state object.
            mapped=clone||original.Points is not StylusPoint[]?new StrokeDrawingState(original.Points.ToArray()):original;
            _states.Add(original,mapped);
            var points=(StylusPoint[])mapped.Points;
            if(offset.X!=0||offset.Y!=0)
                for(var index=0;index<points.Length;index++)
                {
                    var point=points[index];point.X+=offset.X;point.Y+=offset.Y;points[index]=point;
                }
            return mapped;
        }

        internal DrawingAction Action(DrawingAction action)=>action switch
        {
            StrokeDrawingAction value=>new StrokeDrawingAction(Stroke(value.Stroke)),
            StrokeRemovalDrawingAction value=>new StrokeRemovalDrawingAction(Stroke(value.Stroke)),
            StrokeMoveDrawingAction value=>new StrokeMoveDrawingAction(Stroke(value.Stroke),State(value.Before),State(value.After)),
            StrokeStyleDrawingAction value=>new StrokeStyleDrawingAction(Stroke(value.Stroke),
                clone?value.Before.Clone():value.Before,clone?value.After.Clone():value.After),
            ElementDrawingAction value=>value with{Element=Element(value.Element)},
            LiftDrawingAction value=>new LiftDrawingAction((MosaicDrawingElement)Element(value.Background),(MosaicDrawingElement)Element(value.Content)),
            ElementRemovalDrawingAction value=>new ElementRemovalDrawingAction(Element(value.Element),value.Index),
            ElementMoveDrawingAction value=>new ElementMoveDrawingAction(Element(value.Before),Element(value.After)),
            ElementStyleDrawingAction value=>new ElementStyleDrawingAction(Element(value.Before),Element(value.After)),
            RegionMarkDrawingAction value=>new RegionMarkDrawingAction(Mark(value.Mark),value.Removed,value.Index),
            RegionMarkGeometryDrawingAction value=>new RegionMarkGeometryDrawingAction(Mark(value.Mark),Mark(value.Before),Mark(value.After)),
            RegionMarkColorDrawingAction value=>new RegionMarkColorDrawingAction(Mark(value.Mark),value.Before,value.After),
            ClearDrawingAction value=>new ClearDrawingAction(value.Strokes.Select(Stroke).ToArray(),
                value.Elements.Select(Element).ToArray(),value.Marks.Select(Mark).ToArray(),value.NextNumber){NumberPreference=value.NumberPreference},
            _=>throw new InvalidOperationException("Unsupported manual drawing history action: "+action.GetType().Name)
        };

        internal ManualDrawingSnapshot Snapshot(IEnumerable<Stroke> strokes,IEnumerable<DrawingElementSpec> elements,
            IEnumerable<RegionMark> marks,IEnumerable<DrawingAction> order,IEnumerable<DrawingAction> redo,int nextNumber,Guid numberPreference)=>new(
                strokes.Select(Stroke).ToArray(),elements.Select(Element).ToArray(),marks.Select(Mark).ToArray(),
                order.Select(Action).ToArray(),redo.Select(Action).ToArray(),nextNumber){NumberPreference=numberPreference};
    }

    private static ManualDrawingSnapshot CaptureManualDrawingSnapshot(SelectionItem item)=>
        new ManualDrawingMapper(true,default).Snapshot(item.Markup.Strokes,item.DrawingElements,item.RegionMarks,
            item.DrawingOrder,item.DrawingRedo,item.NextDrawingNumber,item.DrawingNumberPreference);

    private void ApplyManualDrawingSnapshot(SelectionItem item,ManualDrawingSnapshot snapshot)
    {
        InvalidateRegionMarkMatching(item);
        var restored=new ManualDrawingMapper(true,default).Snapshot(snapshot.Strokes,snapshot.Elements,snapshot.Marks,
            snapshot.Order,snapshot.Redo,snapshot.NextNumber,snapshot.NumberPreference);
        item.Markup.Strokes.Clear();
        foreach(var stroke in restored.Strokes)AddStrokeWithoutHistory(item,stroke);
        item.DrawingElements.Clear();item.DrawingElements.AddRange(restored.Elements);
        item.RegionMarks.Clear();item.RegionMarks.AddRange(restored.Marks);
        item.DrawingOrder.Clear();item.DrawingOrder.AddRange(restored.Order);
        item.DrawingRedo.Clear();foreach(var action in restored.Redo.Reverse())item.DrawingRedo.Push(action);
        item.NextDrawingNumber=restored.NextNumber;
        item.DrawingNumberPreference=restored.NumberPreference;
        RebuildDrawingElements(item);RenderRegionMarks(item);
    }

    private void SetSelectionBoundsPreservingManualContent(SelectionItem item,Rect next)
    {
        var previous=item.Bounds;
        if(item.VideoPath is null&&item.CapturedImageOverride is null)
            RebaseManualDrawing(item,new Vector(previous.X-next.X,previous.Y-next.Y));
        item.Bounds=next;
        RefreshBackgroundHighlightSources(item);
    }

    private void RebaseManualDrawing(SelectionItem item,Vector offset)
    {
        if(offset.X==0&&offset.Y==0)return;
        RebaseBackgroundHighlightSource(item,offset);
        InvalidateRegionMarkMatching(item);
        var mapped=new ManualDrawingMapper(false,offset).Snapshot(item.Markup.Strokes,item.DrawingElements,item.RegionMarks,
            item.DrawingOrder,item.DrawingRedo,item.NextDrawingNumber,item.DrawingNumberPreference);
        item.DrawingElements.Clear();item.DrawingElements.AddRange(mapped.Elements);
        item.DrawingOrder.Clear();item.DrawingOrder.AddRange(mapped.Order);
        item.DrawingRedo.Clear();foreach(var action in mapped.Redo.Reverse())item.DrawingRedo.Push(action);
        // Reposition existing editors and committed mosaic pixels instead of
        // rebuilding them from a newly cropped image or changing input focus.
        foreach(var element in item.DrawingElements)
        {
            if(FindDrawingElementVisual(item,element.Id) is not { } visual)continue;
            InkCanvas.SetLeft(visual,element.X);InkCanvas.SetTop(visual,element.Y);
        }
        RenderRegionMarks(item);
    }
}
