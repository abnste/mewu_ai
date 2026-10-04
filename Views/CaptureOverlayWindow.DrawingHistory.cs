// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows.Ink;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;
public partial class CaptureOverlayWindow
{
    private sealed record RegionMarkDrawingAction(RegionMark Mark,bool Removed,int Index):DrawingAction;
    private sealed record ClearDrawingAction(Stroke[] Strokes,DrawingElementSpec[] Elements,RegionMark[] Marks,int NextNumber):DrawingAction
    {
        public Guid NumberPreference {get;init;}
        public long RetainedBytes {get;}=Strokes.Sum(stroke=>64L+stroke.StylusPoints.Count*40L)
            +Elements.Sum(element=>128L+(element is TextDrawingElement text?text.Text.Length*2L:0)
                +(element is MosaicDrawingElement{Pixels:{} pixels}?((long)pixels.PixelWidth*pixels.Format.BitsPerPixel+7)/8*pixels.PixelHeight:0))
            +Marks.Sum(mark=>128L+(mark.Template?.LongLength??0));
    }
    // Keep ordinary editing responsive without retaining every cleared canvas or lift.
    // One oversized most-recent heavy action remains undoable; earlier ones are evicted.
    private static void TrimDrawingHistory(SelectionItem item)
    {
        const int maximumActions=128;const long heavyActionBudget=32L*1024*1024;
        if(item.DrawingOrder.Count>maximumActions)item.DrawingOrder.RemoveRange(0,item.DrawingOrder.Count-maximumActions);
        var heavyActions=item.DrawingOrder.Select((action,index)=>(action,index))
            .Where(entry=>entry.action is ClearDrawingAction or LiftDrawingAction or ElementDrawingAction{Element:MosaicDrawingElement{SeamlessErase:true}}).ToArray();
        long bytes=0;var retained=0;var firstRetained=0;
        for(var i=heavyActions.Length-1;i>=0;i--)
        {
            var cost=heavyActions[i].action switch
            {
                ClearDrawingAction clear=>clear.RetainedBytes,
                LiftDrawingAction lift=>LiftElementBytes(lift.Background)+LiftElementBytes(lift.Content),
                ElementDrawingAction{Element:MosaicDrawingElement patch}=>LiftElementBytes(patch),
                _=>0L
            };
            if(retained>0&&bytes+cost>heavyActionBudget){firstRetained=heavyActions[i].index+1;break;}
            bytes+=cost;retained++;
        }
        if(firstRetained>0)item.DrawingOrder.RemoveRange(0,firstRetained);

        static long LiftElementBytes(MosaicDrawingElement element)=>128L+
            (element.Pixels is {} pixels?((long)pixels.PixelWidth*pixels.Format.BitsPerPixel+7)/8*pixels.PixelHeight:0);
    }
    private static RegionMark CloneRegionMark(RegionMark mark)=>new()
    {
        Id=mark.Id,Bounds=mark.Bounds,Color=mark.Color,OnScreen=mark.OnScreen,
        Template=mark.Template?.ToArray(),TemplateWidth=mark.TemplateWidth,TemplateHeight=mark.TemplateHeight,
        TemplateStepX=mark.TemplateStepX,TemplateStepY=mark.TemplateStepY,
        TemplateOffsetX=mark.TemplateOffsetX,TemplateOffsetY=mark.TemplateOffsetY
    };
    private static bool ApplyDrawingElementGeometry(SelectionItem item,DrawingElementSpec geometry,DrawingElementSpec opposite)
    {
        var current=item.DrawingElements.FirstOrDefault(element=>element.Id==geometry.Id);
        DrawingElementSpec? updated=(current,geometry,opposite) switch
        {
            (TextDrawingElement text,TextDrawingElement target,TextDrawingElement previous)=>text with
            {X=target.X,Y=target.Y,Width=target.Width,FontSize=target.FontSize!=previous.FontSize?target.FontSize:text.FontSize},
            (NumberDrawingElement number,NumberDrawingElement target,_)=>number with{X=target.X,Y=target.Y,Diameter=target.Diameter},
            (MosaicDrawingElement mosaic,MosaicDrawingElement target,_)=>mosaic with{X=target.X,Y=target.Y,Width=target.Width,Height=target.Height,Pixels=target.Pixels},
            _=>null
        };
        return updated is not null&&ReplaceDrawingElement(item,updated);
    }
    private void CommitMovedSelectionContent(SelectionItem item)
    {
        var previous=_pointerOperationBefore?.Selections.FirstOrDefault(state=>ReferenceEquals(state.Item,item));
        if(previous is not null&&CaptureOverlayPolicy.HasContentGeometryChanged(previous.Bounds,item.Bounds))InvalidateImageDerivedLayers(item);
        RefreshBackgroundHighlightSources(item);
    }
    private void ClearManualDrawing(SelectionItem item)
    {
        InvalidateRegionMarkMatching(item);
        item.Markup.Strokes.Clear();item.Markup.Children.Clear();item.RasterLayer.Children.Clear();item.DrawingElements.Clear();
        item.NextDrawingNumber=1;item.DrawingNumberPreference=Guid.NewGuid();item.RegionMarks.Clear();RenderRegionMarks(item);
    }
    private bool ApplyRegionHistory(SelectionItem item,DrawingAction action,bool redo)
    {
        if(action is RegionMarkGeometryDrawingAction geometry)
        {
            if(!item.RegionMarks.Contains(geometry.Mark))return false;
            InvalidateRegionMarkMatching(item);ApplyRegionMarkGeometry(geometry.Mark,redo?geometry.After:geometry.Before);
            RenderRegionMarks(item);return true;
        }
        if(action is RegionMarkColorDrawingAction color)
        {
            if(!item.RegionMarks.Contains(color.Mark))return false;
            InvalidateRegionMarkMatching(item);color.Mark.Color=redo?color.After:color.Before;
            RenderRegionMarks(item);return true;
        }
        if(action is RegionMarkDrawingAction mark)
        {
            InvalidateRegionMarkMatching(item);
            if(redo!=mark.Removed){if(!item.RegionMarks.Contains(mark.Mark))item.RegionMarks.Insert(Math.Clamp(mark.Index,0,item.RegionMarks.Count),mark.Mark);}
            else item.RegionMarks.Remove(mark.Mark);
            RenderRegionMarks(item);return true;
        }
        if(action is not ClearDrawingAction clear)return false;
        ClearManualDrawing(item);
        if(!redo)
        {
            // Preserve stroke identity so earlier move/style actions still address it.
            foreach(var stroke in clear.Strokes)AddStrokeWithoutHistory(item,stroke);
            item.DrawingElements.AddRange(clear.Elements);item.RegionMarks.AddRange(clear.Marks);
            item.NextDrawingNumber=clear.NextNumber;item.DrawingNumberPreference=clear.NumberPreference;RebuildDrawingElements(item);RenderRegionMarks(item);
        }
        return true;
    }
}
