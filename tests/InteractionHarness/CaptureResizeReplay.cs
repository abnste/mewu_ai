// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using KeyEventArgs=System.Windows.Input.KeyEventArgs;
using Application=System.Windows.Application;
using Point=System.Windows.Point;
using Color=System.Windows.Media.Color;
internal static class CaptureResizeReplay
{
    private const BindingFlags Flags=BindingFlags.Instance|BindingFlags.NonPublic|BindingFlags.Public|BindingFlags.DeclaredOnly;
    internal static void Run(Application app)
    {
        using var host=new AppHost(app,null,"MewuAI-IsolatedResize-"+Guid.NewGuid().ToString("N"));
        host.Settings.TeachingMode=false;host.Settings.EnableVoiceInput=false;host.Settings.AutomaticallyStartListening=false;
        var bytes=new byte[800*600*4];for(var i=0;i<bytes.Length;i+=4){bytes[i]=235;bytes[i+1]=242;bytes[i+2]=248;bytes[i+3]=255;}
        var bitmap=BitmapSource.Create(800,600,96,96,PixelFormats.Bgra32,null,bytes,800*4);bitmap.Freeze();Array.Clear(bytes);
        var frame=new CaptureFrame(0,0,bitmap);
        var overlay=new CaptureOverlayWindow(host,null,frame){Title="Mewu isolated resize replay",ShowActivated=false,IsHitTestVisible=false};
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        var checks=new List<string>();var movePreviewSamples=new List<object>();string? failure=null;double? resizePreviewP50Ms=null,resizePreviewP95Ms=null;
        var watchdog=new DispatcherTimer{Interval=TimeSpan.FromSeconds(30)};
        var finished=false;
        watchdog.Tick+=(_,_)=>{failure="Replay exceeded 30 seconds";Finish();};
        void Require(bool condition,string name){if(!condition)throw new InvalidOperationException(name);checks.Add(name);}
        object Get(string name)=>typeof(CaptureOverlayWindow).GetField(name,Flags)!.GetValue(overlay)!;
        object? Invoke(string name,params object?[] args)=>typeof(CaptureOverlayWindow).GetMethod(name,Flags)!.Invoke(overlay,args);
        void Finish()
        {
            if(finished)return;finished=true;watchdog.Stop();
            try
            {
                Directory.CreateDirectory(".codex-build/resize-replay");
                File.WriteAllText(".codex-build/resize-replay/result.json",JsonSerializer.Serialize(new{passed=failure is null,checks,failure,syntheticFrameOnly=true,globalInputInjected=false,realPointerCaptureTested=false,resizePreviewP50Ms,resizePreviewP95Ms,movePreviewSamples},new JsonSerializerOptions{WriteIndented=true}));
            }
            catch(Exception ex){failure??=ex.ToString();}
            finally {try {overlay.Close();}finally {app.Shutdown(failure is null?0:1);}}
        }
        overlay.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle,new Action(()=>
        {
            try
            {
                var logger=new PrivacyLogger();
                var logDirectory=(string)typeof(PrivacyLogger).GetField("_directory",Flags)!.GetValue(logger)!;
                Require(string.Equals(logDirectory,Path.GetFullPath(".codex-build/resize-replay/logs"),StringComparison.OrdinalIgnoreCase),"default diagnostics redirected to replay directory");
                logger.Error("ReplaySyntheticError",new InvalidOperationException("Synthetic replay diagnostic"));
                Require(Directory.EnumerateFiles(logDirectory,"*.log").Any(),"exception diagnostics use isolated logs");
                Require(ReferenceEquals(Get("_frame"),frame),"constructor uses supplied synthetic frame");
                Require(Get("_rightPassThrough") is null,"global right-button hook not installed");
                var root=(Canvas)overlay.FindName("Root");
                var item=Invoke("CreateSelection",false)!;var type=item.GetType();
                var boundsField=type.GetField("Bounds")!;var notes=(IList)type.GetProperty("AnnotationNotes")!.GetValue(item)!;
                ((IList)Get("_selections")).Add(item);boundsField.SetValue(item,new Rect(80,80,180,120));Invoke("Select",0);
                var history=Get("_overlayHistory");int Undo()=> (int)history.GetType().GetProperty("UndoCount",Flags)!.GetValue(history)!;
                foreach(var size in new[]{8d,9d,11d,12d})foreach(var handleName in new[]{"Nw","N","Ne","W","E","Sw","S","Se"})foreach(var delta in new[]{-10000d,10000d})
                {
                    var original=new Rect(80,80,size,size);boundsField.SetValue(item,original);notes.Clear();notes.Add(new AiAnnotation(.1,.1,.2,.2,"fixture",Kind:AiAnnotationKind.Rectangle));
                    var handle=(Thumb)overlay.FindName(handleName);var before=Undo();
                    handle.RaiseEvent(new DragDeltaEventArgs(delta,delta){RoutedEvent=Thumb.DragDeltaEvent});
                    var preview=(Rect)boundsField.GetValue(item)!;
                    Require(preview.Width>=8&&preview.Height>=8&&preview.Left>=0&&preview.Top>=0&&preview.Right<=root.ActualWidth+.01&&preview.Bottom<=root.ActualHeight+.01,"bounded resize "+size+handleName+delta);
                    Require(notes.Count==1,"preview retains annotations");
                    handle.RaiseEvent(new DragCompletedEventArgs(0,0,true){RoutedEvent=Thumb.DragCompletedEvent});
                    Require(((Rect)boundsField.GetValue(item)!).Equals(original)&&notes.Count==1&&Undo()==before,"cancel restores bounds/content/history");
                }
                var start=new Rect(80,80,180,120);boundsField.SetValue(item,start);var east=(Thumb)overlay.FindName("E");
                east.RaiseEvent(new DragDeltaEventArgs(20,0){RoutedEvent=Thumb.DragDeltaEvent});
                var escape=new KeyEventArgs(Keyboard.PrimaryDevice,PresentationSource.FromVisual(overlay)!,Environment.TickCount,Key.Escape){RoutedEvent=Keyboard.PreviewKeyDownEvent};
                Invoke("OnPreviewKeyDown",overlay,escape);
                Require(escape.Handled&&((Rect)boundsField.GetValue(item)!).Equals(start)&&notes.Count==1&&!((bool)Get("_closed")),"Escape cancels resize while retaining overlay and content");
                var count=Undo();east.RaiseEvent(new DragDeltaEventArgs(20,0){RoutedEvent=Thumb.DragDeltaEvent});east.RaiseEvent(new DragCompletedEventArgs(0,0,false){RoutedEvent=Thumb.DragCompletedEvent});
                Require(notes.Count==0&&Undo()==count+1,"commit clears derived layer and records exactly one undo");
                count=Undo();east.RaiseEvent(new DragDeltaEventArgs(0,0){RoutedEvent=Thumb.DragDeltaEvent});east.RaiseEvent(new DragCompletedEventArgs(0,0,false){RoutedEvent=Thumb.DragCompletedEvent});
                Require(Undo()==count,"unchanged commit adds no undo");
                Invoke("UndoOverlayOperation");
                Require(((Rect)boundsField.GetValue(item)!).Equals(start)&&notes.Count==1,"undo restores original bounds and annotations");
                Invoke("RedoOverlayOperation");
                Require(((Rect)boundsField.GetValue(item)!).Width==start.Width+20&&notes.Count==0,"redo restores committed resize");
                // Regression cases use the actual original controls/history, with no desktop capture or input injection.
                var marks=(IList)type.GetField("RegionMarks")!.GetValue(item)!;
                var order=(IList)type.GetProperty("DrawingOrder")!.GetValue(item)!;
                var elements=(IList)type.GetProperty("DrawingElements")!.GetValue(item)!;
                Invoke("AddRegionMark",item,new Rect(10,10,30,20));
                Require(marks.Count==1,"region creation adds one mark");
                Invoke("DrawUndo",overlay,new RoutedEventArgs());Require(marks.Count==0,"undo region creation");
                Invoke("DrawRedo",overlay,new RoutedEventArgs());Require(marks.Count==1,"redo region creation");
                Invoke("EraseDrawingObjectsAt",item,new Point(20,20));Require(marks.Count==0,"eraser removes region");
                Invoke("DrawUndo",overlay,new RoutedEventArgs());Require(marks.Count==1,"undo region erasure");
                var lowerMark=marks[0];Invoke("AddRegionMark",item,new Rect(20,15,20,10));var upperMark=marks[1];
                Invoke("EraseDrawingObjectsAt",item,new Point(12,12));Require(marks.Count==1&&ReferenceEquals(marks[0],upperMark),"erase lower overlapping mark");
                Invoke("DrawUndo",overlay,new RoutedEventArgs());Require(marks.Count==2&&ReferenceEquals(marks[0],lowerMark)&&ReferenceEquals(marks[1],upperMark),"undo erasure preserves overlap layer order");
                Invoke("DrawRedo",overlay,new RoutedEventArgs());Require(marks.Count==1&&ReferenceEquals(marks[0],upperMark),"redo lower mark erasure preserves upper mark");
                Invoke("DrawUndo",overlay,new RoutedEventArgs());Invoke("DrawUndo",overlay,new RoutedEventArgs());Require(marks.Count==1&&ReferenceEquals(marks[0],lowerMark),"undo second creation restores original mark");
                var snapshot=Invoke("CaptureOverlaySnapshot")!;
                var mark=marks[0]!;var markType=mark.GetType();
                markType.GetProperty("Bounds")!.SetValue(mark,new Rect(60,60,10,10));
                var template=(byte[]?)markType.GetProperty("Template")!.GetValue(mark);var firstPixel=template?[0];
                if(template is {Length:>0})template[0]^=255;
                Invoke("ApplyOverlaySnapshot",snapshot);
                Require((Rect)markType.GetProperty("Bounds")!.GetValue(marks[0])! ==new Rect(10,10,30,20),"overlay history freezes region geometry");
                Require(firstPixel is null||((byte[])markType.GetProperty("Template")!.GetValue(marks[0])!)[0]==firstPixel,"overlay history owns region template bytes");
                Invoke("AddNumberDrawingElement",item,new Point(70,40));
                var orderBeforeClear=order.Count;
                Invoke("DrawClear",overlay,new RoutedEventArgs());Require(marks.Count==0&&elements.Count==0&&order.Count==orderBeforeClear+1,"clear is one reversible history operation");
                Invoke("DrawUndo",overlay,new RoutedEventArgs());Require(marks.Count==1&&elements.Count==1,"undo clear restores marks and elements");
                Invoke("DrawRedo",overlay,new RoutedEventArgs());Require(marks.Count==0&&elements.Count==0,"redo clear removes restored content");
                Invoke("DrawUndo",overlay,new RoutedEventArgs());Invoke("DrawUndo",overlay,new RoutedEventArgs());
                Require(marks.Count==1&&elements.Count==0,"earlier history remains usable after undo clear");
                var resizeBefore=Invoke("CaptureOverlaySnapshot")!;
                Invoke("ClearImageOnlyLayers",item);Require(marks.Count==0,"source replacement clears colored regions");
                Invoke("ApplyOverlaySnapshot",resizeBefore);Require(marks.Count==1,"overlay history restores invalidated regions");
                // Moving away and back must not destroy content or manufacture history.
                var moveOrigin=(Rect)boundsField.GetValue(item)!;
                typeof(CaptureOverlayWindow).GetField("_pointerOperationBefore",Flags)!.SetValue(overlay,Invoke("CaptureOverlaySnapshot"));
                Invoke("CommitMovedSelectionContent",item);Require(marks.Count==1,"net-zero move retains region content");
                Invoke("SetSelectionBoundsPreservingManualContent",item,new Rect(moveOrigin.X+3,moveOrigin.Y,moveOrigin.Width,moveOrigin.Height));
                typeof(CaptureOverlayWindow).GetField("_moving",Flags)!.SetValue(overlay,true);
                foreach(var key in new[]{Key.Enter,Key.C,Key.S,Key.P,Key.Delete,Key.Z,Key.Y,Key.Left,Key.O,Key.T})
                {
                    var heldMoveKey=new KeyEventArgs(Keyboard.PrimaryDevice,PresentationSource.FromVisual(overlay)!,Environment.TickCount,key){RoutedEvent=Keyboard.PreviewKeyDownEvent};
                    Invoke("OnPreviewKeyDown",overlay,heldMoveKey);
                    Require(heldMoveKey.Handled&&marks.Count==1&&!((bool)Get("_closed")),"held move blocks uncommitted export/edit "+key);
                }
                typeof(CaptureOverlayWindow).GetField("_moving",Flags)!.SetValue(overlay,false);
                Invoke("CommitMovedSelectionContent",item);Require(marks.Count==1,"committed changed move retains anchored manual content");
                Invoke("ApplyOverlaySnapshot",resizeBefore);
                VerifyManualDrawingReframing(overlay,Require,movePreviewSamples);
                // Undo of geometry must preserve wording edited after that gesture.
                var textType=typeof(CaptureOverlayWindow).GetNestedType("TextDrawingElement",BindingFlags.NonPublic)!;
                var id=Guid.NewGuid();
                object Text(double x,double y,string words,Color color)=>Activator.CreateInstance(textType,BindingFlags.Instance|BindingFlags.Public|BindingFlags.NonPublic,null,new object[]{id,x,y,100d,words,"Segoe UI",18d,color,false},null)!;
                var originalText=Text(10,10,"A",Colors.Red);var movedText=Text(30,20,"A",Colors.Red);var editedText=Text(30,20,"B",Colors.Blue);
                elements.Add(editedText);
                var geometry=typeof(CaptureOverlayWindow).GetMethod("ApplyDrawingElementGeometry",Flags|BindingFlags.Static)!;
                Require((bool)geometry.Invoke(null,new[]{item,originalText,movedText})!,"geometry undo finds text element");
                var restoredText=elements[elements.Count-1]!;
                Require((string)textType.GetProperty("Text")!.GetValue(restoredText)! =="B"&&(Color)textType.GetProperty("Color")!.GetValue(restoredText)! ==Colors.Blue,"move undo preserves later wording and color");
                Require((double)textType.GetProperty("X")!.GetValue(restoredText)! ==10,"move undo restores only geometry");
                elements.Remove(restoredText);
                var ink=(InkCanvas)type.GetProperty("Markup")!.GetValue(item)!;
                var darkStroke=new System.Windows.Ink.Stroke(new StylusPointCollection{new StylusPoint(5,20),new StylusPoint(45,20)}){DrawingAttributes=new(){Color=Colors.Black,Width=8,Height=8}};
                ink.Strokes.Add(darkStroke);
                var exportBounds=(Rect)boundsField.GetValue(item)!;
                var exported=(BitmapSource)Invoke("RenderManualOverlay",item,(int)exportBounds.Width,(int)exportBounds.Height)!;
                var exportedPixel=new byte[4];exported.CopyPixels(new Int32Rect(20,20,1,1),exportedPixel,4,0);
                Require(exportedPixel[0]<5&&exportedPixel[1]<5&&exportedPixel[2]<5&&exportedPixel[3]>250,"export keeps colored regions beneath opaque ink");
                ink.Strokes.Remove(darkStroke);
                // Resize preview keeps the same frozen texture brush; release materializes one crop.
                east.RaiseEvent(new DragDeltaEventArgs(1,0){RoutedEvent=Thumb.DragDeltaEvent});
                var hostPanel=(Grid)type.GetProperty("Host")!.GetValue(item)!;
                var previewBrush=hostPanel.Background;
                east.RaiseEvent(new DragDeltaEventArgs(1,0){RoutedEvent=Thumb.DragDeltaEvent});
                Require(previewBrush is ImageBrush&&ReferenceEquals(previewBrush,hostPanel.Background),"resize reuses crop-free frozen texture preview");
                var samples=new double[300];
                for(var sample=0;sample<samples.Length;sample++)
                {
                    var timer=System.Diagnostics.Stopwatch.StartNew();
                    east.RaiseEvent(new DragDeltaEventArgs(sample%2==0?1:-1,0){RoutedEvent=Thumb.DragDeltaEvent});
                    samples[sample]=timer.Elapsed.TotalMilliseconds;
                }
                Array.Sort(samples);resizePreviewP50Ms=samples[149];resizePreviewP95Ms=samples[284];
                Require(ReferenceEquals(previewBrush,hostPanel.Background),"300 resize updates preserve one texture brush");
                east.RaiseEvent(new DragCompletedEventArgs(0,0,true){RoutedEvent=Thumb.DragCompletedEvent});
                Require(hostPanel.Background is null&&marks.Count==1,"cancel resize finalizes crop and preserves marks");
                for(var entry=0;entry<150;entry++){Invoke("AddNumberDrawingElement",item,new Point(70,40));Invoke("DrawClear",overlay,new RoutedEventArgs());}
                Require(order.Count<=128,"drawing history bounds repeated clear retention");
                Invoke("DrawUndo",overlay,new RoutedEventArgs());Require(elements.Count==1,"bounded history still restores the latest clear");
                var matching=typeof(CaptureOverlayWindow).GetMethod("ReanchorMarksCore",Flags|BindingFlags.Static)!;
                var inputType=typeof(CaptureOverlayWindow).GetNestedType("RegionMarkMatchInput",BindingFlags.NonPublic)!;
                var canceled=false;
                try{matching.Invoke(null,new object[]{bitmap,1d,1d,Array.CreateInstance(inputType,0),new CancellationToken(true)});}
                catch(TargetInvocationException error) when(error.InnerException is OperationCanceledException){canceled=true;}
                Require(canceled,"closed-capture cancellation stops matching before pixel conversion");
                Invoke("RefreshDesktopFrameIncludingPinnedWindows");Require(ReferenceEquals(Get("_frame"),frame),"refresh stays synthetic");
            }
            catch(Exception ex){failure=(ex is TargetInvocationException{InnerException:{} inner}?inner:ex).ToString();}
            finally {Finish();}
        }));
        watchdog.Start();overlay.Show();app.Run();
    }

    private static void VerifyManualDrawingReframing(CaptureOverlayWindow overlay,Action<bool,string> require,List<object> performance)
    {
        object Get(string name)=>typeof(CaptureOverlayWindow).GetField(name,Flags)!.GetValue(overlay)!;
        void Set(string name,object? value)=>typeof(CaptureOverlayWindow).GetField(name,Flags)!.SetValue(overlay,value);
        object? Invoke(string name,params object?[] args)=>typeof(CaptureOverlayWindow).GetMethod(name,Flags)!.Invoke(overlay,args);
        object Make(string name,params object?[] args)=>Activator.CreateInstance(typeof(CaptureOverlayWindow).GetNestedType(name,BindingFlags.NonPublic)!,
            BindingFlags.Instance|BindingFlags.Public|BindingFlags.NonPublic,null,args,null)!;
        var original=Invoke("CaptureOverlaySnapshot")!;
        var selections=(IList)Get("_selections");
        var item=Invoke("CreateSelection",false)!;var type=item.GetType();
        var boundsField=type.GetField("Bounds")!;
        var ink=(InkCanvas)type.GetProperty("Markup")!.GetValue(item)!;
        var elements=(IList)type.GetProperty("DrawingElements")!.GetValue(item)!;
        var order=(IList)type.GetProperty("DrawingOrder")!.GetValue(item)!;
        var redo=type.GetProperty("DrawingRedo")!.GetValue(item)!;
        var marks=(IList)type.GetField("RegionMarks")!.GetValue(item)!;
        Rect Bounds()=>(Rect)boundsField.GetValue(item)!;
        int RedoCount()=>(int)redo.GetType().GetProperty("Count")!.GetValue(redo)!;
        Point MarkPoint()
        {
            var r=(Rect)marks[0]!.GetType().GetProperty("Bounds")!.GetValue(marks[0])!;
            return new Point(r.X+Bounds().X,r.Y+Bounds().Y);
        }
        Point ElementPoint()
        {
            var element=elements[0]!;var elementType=element.GetType();
            return new Point((double)elementType.GetProperty("X")!.GetValue(element)!+Bounds().X,
                (double)elementType.GetProperty("Y")!.GetValue(element)!+Bounds().Y);
        }
        Point StrokePoint()=>new(ink.Strokes[0].StylusPoints[0].X+Bounds().X,ink.Strokes[0].StylusPoints[0].Y+Bounds().Y);
        void Move(double x,double y)
        {
            var start=Bounds();Set("_pointerOperationBefore",Invoke("CaptureOverlaySnapshot"));Set("_pointerOperationLabel","fixture move");
            Set("_moveOrigin",start);Set("_moveStart",new Point());Set("_moving",true);
            Invoke("UpdatePointerInteraction",new Point(x-start.X,y-start.Y));Invoke("FinishInterruptedPointerInteraction");
        }
        void Resize(string handle,double x,double y,bool canceled=false)
        {
            var thumb=(Thumb)overlay.FindName(handle);
            thumb.RaiseEvent(new DragDeltaEventArgs(x,y){RoutedEvent=Thumb.DragDeltaEvent});
            thumb.RaiseEvent(new DragCompletedEventArgs(0,0,canceled){RoutedEvent=Thumb.DragCompletedEvent});
        }
        byte[] Pixels()
        {
            var bitmap=(BitmapSource)Invoke("RenderManualOverlay",item,(int)Bounds().Width,(int)Bounds().Height)!;
            var pixels=new byte[bitmap.PixelWidth*bitmap.PixelHeight*4];bitmap.CopyPixels(pixels,bitmap.PixelWidth*4,0);return pixels;
        }
        void MeasureMovePreview(string name)
        {
            var start=Bounds();Set("_moveOrigin",start);Set("_moveStart",new Point());Set("_moving",true);
            var samples=new double[300];var allocated=GC.GetAllocatedBytesForCurrentThread();
            for(var sample=0;sample<samples.Length;sample++)
            {
                var timer=System.Diagnostics.Stopwatch.StartNew();
                Invoke("UpdatePointerInteraction",sample%2==0?new Point(1,1):new Point());
                samples[sample]=timer.Elapsed.TotalMilliseconds;
            }
            allocated=GC.GetAllocatedBytesForCurrentThread()-allocated;Set("_moving",false);Invoke("UpdateSelection",item);
            Array.Sort(samples);
            performance.Add(new{name,samples=samples.Length,strokes=ink.Strokes.Count,points=ink.Strokes.Sum(stroke=>stroke.StylusPoints.Count),
                history=order.Count,redo=RedoCount(),p50Ms=samples[149],p95Ms=samples[284],maxMs=samples[^1],allocatedBytes=allocated});
            require(Bounds()==start,"300 move previews return to original geometry: "+name);
        }
        try
        {
            selections.Add(item);boundsField.SetValue(item,new Rect(100,100,240,160));Invoke("Select",selections.Count-1);
            MeasureMovePreview("empty");
            Invoke("AddRegionMark",item,new Rect(12,12,30,20));
            var text=Make("TextDrawingElement",Guid.NewGuid(),55d,65d,110d,"anchor","Segoe UI",18d,Colors.Blue,false);
            elements.Add(text);order.Add(Make("ElementDrawingAction",text));Invoke("RebuildDrawingElements",item);
            var stroke=new System.Windows.Ink.Stroke(new StylusPointCollection{new StylusPoint(8,45),new StylusPoint(70,45)})
                {DrawingAttributes=new(){Color=Colors.Black,Width=5,Height=5}};
            ink.Strokes.Add(stroke);order.Add(Make("StrokeDrawingAction",stroke));
            var strokeBefore=Make("StrokeDrawingState",stroke.StylusPoints.ToArray());
            stroke.StylusPoints=new StylusPointCollection(stroke.StylusPoints.Select(point=>{point.X+=5;return point;}));
            var strokeAfter=Make("StrokeDrawingState",stroke.StylusPoints.ToArray());
            order.Add(Make("StrokeMoveDrawingAction",stroke,strokeBefore,strokeAfter));
            var markWorld=MarkPoint();var elementWorld=ElementPoint();var strokeWorld=StrokePoint();var baseline=Pixels();
            MeasureMovePreview("line-text-block");require(Pixels().SequenceEqual(baseline),"300 move previews preserve rendered manual pixels");
            Invoke("DrawClear",overlay,new RoutedEventArgs());Invoke("DrawUndo",overlay,new RoutedEventArgs());
            require(RedoCount()==1,"clear redo retained before reframing");
            var originalOrderCount=order.Count;
            Move(430,330);
            require(marks.Count==1&&elements.Count==1&&ink.Strokes.Count==1,"move outside retains all manual objects");
            require(MarkPoint()==markWorld&&ElementPoint()==elementWorld&&StrokePoint()==strokeWorld,"moved crop keeps manual objects on original desktop pixels");
            require(Pixels().Where((value,index)=>index%4==3).All(alpha=>alpha==0),"objects outside moved crop are clipped rather than relocated");
            require(order.Count==originalOrderCount&&RedoCount()==1,"move preserves drawing undo and redo histories");
            Move(100,100);
            require(Pixels().SequenceEqual(baseline),"move away and back restores exact rendered manual pixels");
            Invoke("DrawRedo",overlay,new RoutedEventArgs());require(marks.Count==0&&elements.Count==0&&ink.Strokes.Count==0,"clear redo still removes all reframed objects");
            Invoke("DrawUndo",overlay,new RoutedEventArgs());require(Pixels().SequenceEqual(baseline),"clear undo after reframing restores exact pixels");
            Invoke("DrawUndo",overlay,new RoutedEventArgs());require(StrokePoint()==new Point(strokeWorld.X-5,strokeWorld.Y),"earlier stroke move undo survives reframing");
            Invoke("DrawRedo",overlay,new RoutedEventArgs());require(StrokePoint()==strokeWorld,"earlier stroke move redo preserves shared stroke identity");
            var beforeResize=Bounds();Resize("Nw",-30,-20);
            require(Bounds().Width>beforeResize.Width&&Bounds().Height>beforeResize.Height,"upper-left resize enlarges crop");
            require(MarkPoint()==markWorld&&ElementPoint()==elementWorld&&StrokePoint()==strokeWorld,"enlargement preserves desktop anchoring for line text and block");
            Invoke("UndoOverlayOperation");require(Bounds()==beforeResize&&Pixels().SequenceEqual(baseline),"outer undo restores resize and complete manual content");
            Invoke("DrawUndo",overlay,new RoutedEventArgs());require(StrokePoint()==new Point(strokeWorld.X-5,strokeWorld.Y),"drawing history remains usable after outer resize undo");
            Invoke("DrawRedo",overlay,new RoutedEventArgs());
            Invoke("RedoOverlayOperation");require(MarkPoint()==markWorld&&ElementPoint()==elementWorld&&StrokePoint()==strokeWorld,"outer redo restores enlarged crop with anchored annotations");
            Resize("Nw",30,20);require(Bounds()==beforeResize&&Pixels().SequenceEqual(baseline),"enlarge then shrink back retains exact manual pixels");
            var originalStroke=ink.Strokes[0];var originalMark=marks[0];var beforeCancel=Bounds();
            Resize("Nw",60,55,true);
            require(Bounds()==beforeCancel&&ReferenceEquals(originalStroke,ink.Strokes[0])&&ReferenceEquals(originalMark,marks[0])&&Pixels().SequenceEqual(baseline),"cancel reframing restores geometry without replacing live manual objects");
            Resize("Nw",60,55);
            require(marks.Count==1&&elements.Count==1&&ink.Strokes.Count==1,"shrink clips out-of-bounds annotations without deleting them");
            Resize("Nw",-60,-55);require(Pixels().SequenceEqual(baseline),"shrink then enlarge back restores exact manual pixels");
            Invoke("EraseDrawingObjectsAt",item,new Point(20,20));require(marks.Count==0,"erase mark before reframing");
            Move(430,330);Move(100,100);Invoke("DrawUndo",overlay,new RoutedEventArgs());
            require(marks.Count==1&&MarkPoint()==markWorld,"undo erased hidden mark after reframing restores its original desktop anchor");
            var beforeSnapshot=Invoke("CaptureOverlaySnapshot")!;
            Invoke("DrawClear",overlay,new RoutedEventArgs());var cleared=Invoke("CaptureOverlaySnapshot")!;
            Invoke("ApplyOverlaySnapshot",beforeSnapshot);Invoke("ApplyOverlaySnapshot",cleared);Invoke("DrawUndo",overlay,new RoutedEventArgs());
            require(marks.Count==1&&elements.Count==1&&ink.Strokes.Count==1,"snapshot owns hidden clear history and restores its shared objects");
            require(Pixels().SequenceEqual(baseline),"snapshot clear undo restores original rendered pixels");
            var ordinarySnapshot=Invoke("CaptureOverlaySnapshot")!;
            Invoke("ClearManualDrawing",item);order.Clear();redo.GetType().GetMethod("Clear")!.Invoke(redo,null);
            for(var index=0;index<64;index++)
            {
                var retainedStroke=new System.Windows.Ink.Stroke(new StylusPointCollection(Enumerable.Range(0,128).Select(point=>new StylusPoint(5+point,5+index))));
                ink.Strokes.Add(retainedStroke);order.Add(Make("StrokeDrawingAction",retainedStroke));
                var state=Make("StrokeDrawingState",retainedStroke.StylusPoints.ToArray());
                order.Add(Make("StrokeMoveDrawingAction",retainedStroke,state,state));
            }
            MeasureMovePreview("64-strokes-8192-points-128-actions");Invoke("ApplyOverlaySnapshot",ordinarySnapshot);
            var mosaicPixels=BitmapSource.Create(2,2,96,96,PixelFormats.Bgra32,null,new byte[]{0,0,255,255,0,255,0,255,255,0,0,255,0,255,255,255},8);mosaicPixels.Freeze();
            var mosaic=Make("MosaicDrawingElement",Guid.NewGuid(),180d,90d,30d,30d);mosaic.GetType().GetProperty("Pixels")!.SetValue(mosaic,mosaicPixels);
            elements.Add(mosaic);order.Add(Make("ElementDrawingAction",mosaic));Invoke("RebuildDrawingElements",item);
            var mosaicBaseline=Pixels();Move(430,330);Move(100,100);
            require(Pixels().SequenceEqual(mosaicBaseline),"mosaic retains original rendered pixels after reframing");
            var mosaicSnapshot=Invoke("CaptureOverlaySnapshot")!;Invoke("ApplyOverlaySnapshot",mosaicSnapshot);
            var restoredMosaic=elements.Cast<object>().Single(element=>element.GetType().Name=="MosaicDrawingElement");
            require(ReferenceEquals(restoredMosaic.GetType().GetProperty("Pixels")!.GetValue(restoredMosaic),mosaicPixels),"snapshot shares immutable mosaic pixel cache");
            Invoke("DrawClear",overlay,new RoutedEventArgs());Invoke("DrawUndo",overlay,new RoutedEventArgs());
            require(Pixels().SequenceEqual(mosaicBaseline),"clear undo restores reframed mosaic cached pixels");
            Invoke("ApplyOverlaySnapshot",ordinarySnapshot);
            var ownImage=BitmapSource.Create(2,2,96,96,PixelFormats.Bgra32,null,new byte[16],8);ownImage.Freeze();
            type.GetField("CapturedImageOverride")!.SetValue(item,ownImage);
            var localMark=(Rect)marks[0]!.GetType().GetProperty("Bounds")!.GetValue(marks[0])!;
            Invoke("SetSelectionBoundsPreservingManualContent",item,new Rect(125,115,240,160));
            require((Rect)marks[0]!.GetType().GetProperty("Bounds")!.GetValue(marks[0])! ==localMark,"replacement-image annotations keep independent local-image coordinates");
            type.GetField("CapturedImageOverride")!.SetValue(item,null);type.GetField("VideoPath")!.SetValue(item,"synthetic-video-not-opened");
            Invoke("SetSelectionBoundsPreservingManualContent",item,new Rect(150,130,240,160));
            require((Rect)marks[0]!.GetType().GetProperty("Bounds")!.GetValue(marks[0])! ==localMark,"video annotations keep independent local-image coordinates");
            type.GetField("VideoPath")!.SetValue(item,null);
        }
        finally
        {
            Set("_moving",false);Set("_pointerOperationBefore",null);Set("_pointerOperationLabel","");
            Invoke("ApplyOverlaySnapshot",original);
        }
    }
}
