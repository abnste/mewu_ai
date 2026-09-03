using System.Globalization;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Recording;

internal static class AnnotationOverlayRenderer
{
    private static readonly Brush Cyan=new SolidColorBrush(Color.FromRgb(42,174,255));

    internal static BitmapSource RenderAiOverlay(int width,int height,IReadOnlyList<AiAnnotation> annotations,double? videoTime=null)
    {
        ArgumentOutOfRangeException.ThrowIfLessThan(width,1);
        ArgumentOutOfRangeException.ThrowIfLessThan(height,1);
        var visual=new DrawingVisual();
        using(var drawing=visual.RenderOpen())
        {
            var cardWidth=Math.Clamp(width*.3,145,360);var font=Math.Clamp(width/70d,11,22);var slots=new List<double>();var calloutCount=0;
            var callouts=annotations.Where(annotation=>annotation.Kind==AiAnnotationKind.Callout).ToArray();
            foreach(var annotation in annotations.Take(48))
            {
                var frame=new VideoAnnotationKeyframe(videoTime??0,annotation.X,annotation.Y,annotation.Width,annotation.Height);
                if(annotation.IsVideoTimeline&&(!videoTime.HasValue||videoTime<annotation.StartTime||videoTime>annotation.EndTime||!VideoAnnotationTimeline.TryInterpolate(annotation,videoTime.Value,out frame)))continue;
                if(AnnotationLayoutService.IsDuplicateTargetMarker(annotation,callouts))continue;
                var x=Math.Clamp(frame.X,0,1)*width;var y=Math.Clamp(frame.Y,0,1)*height;var boxWidth=Math.Max(14,Math.Clamp(frame.Width,0,1)*width);var boxHeight=Math.Max(14,Math.Clamp(frame.Height,0,1)*height);
                if(annotation.Kind==AiAnnotationKind.Mosaic)continue;
                var style=annotation.EffectiveStyle;var colorName=annotation.Kind is AiAnnotationKind.Rectangle or AiAnnotationKind.Ellipse&&string.Equals(style.Color,"#2AAEFF",StringComparison.OrdinalIgnoreCase)?"#FF0000":style.Color;var color=ParseColor(colorName,style.Opacity);var brush=new SolidColorBrush(color);var stroke=Math.Clamp(style.StrokeWidth*Math.Min(width,height),1,48);var pen=new Pen(brush,annotation.Kind==AiAnnotationKind.Highlighter?Math.Max(5,stroke):stroke){StartLineCap=PenLineCap.Round,EndLineCap=PenLineCap.Round,LineJoin=PenLineJoin.Round};
                var points=(frame.Points??annotation.Points)?.Select(point=>new Point(point.X*width,point.Y*height)).ToArray();
                switch(annotation.Kind)
                {
                    case AiAnnotationKind.Pen:
                    case AiAnnotationKind.Highlighter:
                        if(points is {Length:>=2})drawing.DrawGeometry(null,pen,CreatePolyline(points));
                        break;
                    case AiAnnotationKind.Rectangle:
                        drawing.DrawRectangle(style.Filled?WithOpacity(brush,.18):null,pen,new Rect(x,y,boxWidth,boxHeight));break;
                    case AiAnnotationKind.Ellipse:
                        drawing.DrawEllipse(style.Filled?WithOpacity(brush,.18):null,pen,new Point(x+boxWidth/2,y+boxHeight/2),boxWidth/2,boxHeight/2);break;
                    case AiAnnotationKind.Arrow:
                        if(points is {Length:>=2})DrawArrow(drawing,pen,points[0],points[^1],Math.Clamp(stroke*4,10,42));break;
                    case AiAnnotationKind.Text:
                        DrawText(drawing,annotation.Text,new Rect(x,y,boxWidth,boxHeight),Math.Clamp(style.FontSize*height,10,96),brush);break;
                    case AiAnnotationKind.Number:
                        var diameter=Math.Min(boxWidth,boxHeight);drawing.DrawEllipse(brush,null,new Point(x+diameter/2,y+diameter/2),diameter/2,diameter/2);DrawCenteredText(drawing,(annotation.Number??1).ToString(CultureInfo.InvariantCulture),new Rect(x,y,diameter,diameter),Math.Clamp(diameter*.48,12,52),Contrast(color));break;
                    default:
                        if(calloutCount++>=6)break;
                        var target=new Rect(x,y,boxWidth,boxHeight);var targetColor=string.Equals(style.Color,"#2AAEFF",StringComparison.OrdinalIgnoreCase)?Color.FromRgb(255,0,0):color;drawing.DrawRoundedRectangle(null,new Pen(new SolidColorBrush(targetColor),stroke),target,3,3);
                        var cardHeight=Math.Max(font*3.2,font*1.8);var occupied=slots.Select(top=>new Rect(5,top,Math.Max(1,width-10),cardHeight)).ToArray();var placement=AnnotationLayoutService.FindCalloutPlacement(target,new Size(cardWidth,cardHeight),new Size(width,height),occupied);var cardX=placement.CardBounds.Left;var cardY=placement.CardBounds.Top;slots.Add(cardY);var connector=placement.ConnectorPoint;var targetPoint=new Point(connector.X<=target.Left?target.Left:connector.X>=target.Right?target.Right:Math.Clamp(connector.X,target.Left,target.Right),connector.Y<=target.Top?target.Top:connector.Y>=target.Bottom?target.Bottom:Math.Clamp(connector.Y,target.Top,target.Bottom));
                        var linePen=new Pen(Cyan,Math.Max(1,width/1200d));linePen.Freeze();drawing.DrawLine(linePen,targetPoint,connector);drawing.DrawEllipse(Cyan,null,connector,2.5,2.5);
                        drawing.DrawRoundedRectangle(new SolidColorBrush(Color.FromArgb(248,255,255,255)),new Pen(new SolidColorBrush(Color.FromArgb(145,61,174,242)),1),new Rect(cardX,cardY,cardWidth,cardHeight),8,8);
                        DrawText(drawing,annotation.Text,new Rect(cardX+font*.65,cardY+font*.45,cardWidth-font*1.3,cardHeight-font),font,new SolidColorBrush(Color.FromRgb(35,48,70)));break;
                }
            }
        }
        var bitmap=new RenderTargetBitmap(width,height,96,96,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();return bitmap;
    }

    internal static BitmapSource ApplyAiAnnotations(BitmapSource source,IReadOnlyList<AiAnnotation> annotations,double? videoTime=null)
    {
        var result=ApplyAiMosaics(source,annotations,videoTime);
        return Composite(result,RenderAiOverlay(source.PixelWidth,source.PixelHeight,annotations,videoTime));
    }

    internal static BitmapSource ApplyAiMosaics(BitmapSource source,IReadOnlyList<AiAnnotation> annotations,double? videoTime=null)
    {
        BitmapSource result=source;
        foreach(var annotation in annotations.Where(item=>item.Kind==AiAnnotationKind.Mosaic).Take(16))
        {
            var frame=new VideoAnnotationKeyframe(videoTime??0,annotation.X,annotation.Y,annotation.Width,annotation.Height);
            if(annotation.IsVideoTimeline&&(!videoTime.HasValue||videoTime<annotation.StartTime||videoTime>annotation.EndTime||!VideoAnnotationTimeline.TryInterpolate(annotation,videoTime.Value,out frame)))continue;
            var x=Math.Clamp((int)Math.Floor(frame.X*source.PixelWidth),0,source.PixelWidth-1);var y=Math.Clamp((int)Math.Floor(frame.Y*source.PixelHeight),0,source.PixelHeight-1);var right=Math.Clamp((int)Math.Ceiling((frame.X+frame.Width)*source.PixelWidth),x+1,source.PixelWidth);var bottom=Math.Clamp((int)Math.Ceiling((frame.Y+frame.Height)*source.PixelHeight),y+1,source.PixelHeight);
            // Always sample the clean source, then composite only this region.
            // Reading from result would feed an earlier mosaic back into the
            // next block and progressively destroy detail in overlaps.
            var pixelated=ImagePixelationService.Pixelate(source,new Int32Rect(x,y,right-x,bottom-y),Math.Clamp((int)Math.Round(12*Math.Max(source.PixelWidth/1280d,source.PixelHeight/720d)),8,40));
            result=CompositeRegion(result,pixelated,new Int32Rect(x,y,right-x,bottom-y));
        }
        return result;
    }

    private static BitmapSource CompositeRegion(BitmapSource baseImage,BitmapSource overlay,Int32Rect region)
    {
        var visual=new DrawingVisual();using(var drawing=visual.RenderOpen())
        {
            var bounds=new Rect(0,0,baseImage.PixelWidth,baseImage.PixelHeight);drawing.DrawImage(baseImage,bounds);drawing.PushClip(new RectangleGeometry(new Rect(region.X,region.Y,region.Width,region.Height)));drawing.DrawImage(overlay,bounds);drawing.Pop();
        }
        var bitmap=new RenderTargetBitmap(baseImage.PixelWidth,baseImage.PixelHeight,baseImage.DpiX,baseImage.DpiY,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();return bitmap;
    }

    internal static BitmapSource RenderAiOverlay(BitmapSource source,IReadOnlyList<AiAnnotation> annotations,double? videoTime)
    {
        var visual=new DrawingVisual();using(var drawing=visual.RenderOpen())
        {
            var pixelated=ApplyAiMosaics(source,annotations,videoTime);var bounds=new Rect(0,0,source.PixelWidth,source.PixelHeight);
            foreach(var annotation in annotations.Where(item=>item.Kind==AiAnnotationKind.Mosaic).Take(16))
            {
                var frame=new VideoAnnotationKeyframe(videoTime??0,annotation.X,annotation.Y,annotation.Width,annotation.Height);
                if(annotation.IsVideoTimeline&&(!videoTime.HasValue||videoTime<annotation.StartTime||videoTime>annotation.EndTime||!VideoAnnotationTimeline.TryInterpolate(annotation,videoTime.Value,out frame)))continue;
                var clip=new RectangleGeometry(new Rect(frame.X*source.PixelWidth,frame.Y*source.PixelHeight,frame.Width*source.PixelWidth,frame.Height*source.PixelHeight));drawing.PushClip(clip);drawing.DrawImage(pixelated,bounds);drawing.Pop();
            }
            drawing.DrawImage(RenderAiOverlay(source.PixelWidth,source.PixelHeight,annotations,videoTime),bounds);
        }
        var bitmap=new RenderTargetBitmap(source.PixelWidth,source.PixelHeight,96,96,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();return bitmap;
    }

    private static StreamGeometry CreatePolyline(IReadOnlyList<Point> points)
    {
        var geometry=new StreamGeometry();using(var context=geometry.Open()){context.BeginFigure(points[0],false,false);context.PolyLineTo(points.Skip(1).ToArray(),true,true);}geometry.Freeze();return geometry;
    }

    private static void DrawArrow(DrawingContext drawing,Pen pen,Point start,Point end,double head)
    {
        drawing.DrawLine(pen,start,end);var angle=Math.Atan2(end.Y-start.Y,end.X-start.X);var first=new Point(end.X-head*Math.Cos(angle-Math.PI/6),end.Y-head*Math.Sin(angle-Math.PI/6));var second=new Point(end.X-head*Math.Cos(angle+Math.PI/6),end.Y-head*Math.Sin(angle+Math.PI/6));drawing.DrawLine(pen,end,first);drawing.DrawLine(pen,end,second);
    }

    private static void DrawText(DrawingContext drawing,string value,Rect bounds,double size,Brush brush)
    {
        var text=new FormattedText(value,CultureInfo.CurrentUICulture,FlowDirection.LeftToRight,new Typeface("Microsoft YaHei UI"),size,brush,1){MaxTextWidth=Math.Max(1,bounds.Width),MaxTextHeight=Math.Max(1,bounds.Height)};drawing.DrawText(text,bounds.TopLeft);
    }

    private static void DrawCenteredText(DrawingContext drawing,string value,Rect bounds,double size,Brush brush)
    {
        var text=new FormattedText(value,CultureInfo.InvariantCulture,FlowDirection.LeftToRight,new Typeface(new FontFamily("Microsoft YaHei UI"),FontStyles.Normal,FontWeights.Bold,FontStretches.Normal),size,brush,1);drawing.DrawText(text,new Point(bounds.Left+(bounds.Width-text.Width)/2,bounds.Top+(bounds.Height-text.Height)/2));
    }

    private static Color ParseColor(string value,double opacity)
    {
        var color=(Color)ColorConverter.ConvertFromString(value);color.A=(byte)Math.Round(Math.Clamp(opacity,0,1)*255);return color;
    }

    private static Brush WithOpacity(Brush source,double factor){var clone=source.Clone();clone.Opacity*=factor;return clone;}
    private static Brush Contrast(Color color)=>color.R*.299+color.G*.587+color.B*.114>155?Brushes.Black:Brushes.White;

    internal static BitmapSource Composite(BitmapSource source,params BitmapSource?[] overlays)
    {
        var visual=new DrawingVisual();using(var drawing=visual.RenderOpen())
        {
            var bounds=new Rect(0,0,source.PixelWidth,source.PixelHeight);drawing.DrawImage(source,bounds);
            foreach(var overlay in overlays)if(overlay is not null)drawing.DrawImage(overlay,bounds);
        }
        var bitmap=new RenderTargetBitmap(source.PixelWidth,source.PixelHeight,96,96,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();return bitmap;
    }

    internal static void SavePng(BitmapSource bitmap,string path)
    {
        var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));using var stream=new FileStream(path,FileMode.CreateNew,FileAccess.Write,FileShare.None);encoder.Save(stream);stream.Flush(true);
    }
}

internal readonly record struct VideoOverlayFrame(TimeSpan Start,TimeSpan Duration,TimeSpan SampleTime);

internal static class VideoAnnotationOverlayPlan
{
    internal const int MaximumOverlayFrames=240;
    internal static IReadOnlyList<VideoOverlayFrame> Create(IReadOnlyList<AiAnnotation> annotations,TimeSpan videoDuration,int framesPerSecond=10,int maximumFrames=MaximumOverlayFrames)
    {
        if(videoDuration<=TimeSpan.Zero)return [];
        framesPerSecond=Math.Clamp(framesPerSecond,1,15);ArgumentOutOfRangeException.ThrowIfLessThan(maximumFrames,1);var step=TimeSpan.FromSeconds(1d/framesPerSecond);var samples=new SortedSet<long>();
        foreach(var note in annotations.Where(note=>note.IsVideoTimeline))
        {
            var start=TimeSpan.FromSeconds(Math.Clamp(note.StartTime!.Value,0,videoDuration.TotalSeconds));var end=TimeSpan.FromSeconds(Math.Clamp(note.EndTime!.Value,0,videoDuration.TotalSeconds));if(end<start)continue;
            if(end-start<=TimeSpan.FromMilliseconds(50)){samples.Add(start.Ticks);continue;}
            // Sample each interval directly into a bounded set. Generating a
            // many-hour timeline at full FPS and truncating afterwards creates
            // avoidable memory/CPU spikes during video export.
            var idealCount=(long)Math.Ceiling((end-start).Ticks/(double)Math.Max(1,step.Ticks))+1;
            var count=(int)Math.Clamp(idealCount,2,maximumFrames);
            var last=Math.Max(start.Ticks,end.Ticks-1);
            for(var index=0;index<count;index++)
            {
                var ticks=count==1?start.Ticks:start.Ticks+(long)Math.Round((last-start.Ticks)*(index/(double)(count-1)));
                samples.Add(ticks);
            }
        }
        var all=samples.ToArray();if(all.Length>maximumFrames)all=Enumerable.Range(0,maximumFrames).Select(index=>all[(int)Math.Round(index*(all.Length-1d)/Math.Max(1,maximumFrames-1))]).Distinct().ToArray();
        var result=new List<VideoOverlayFrame>(all.Length);
        for(var index=0;index<all.Length;index++)
        {
            var sample=TimeSpan.FromTicks(all[index]);var pointOnly=annotations.Any(note=>note.IsVideoTimeline&&Math.Abs(note.StartTime!.Value-note.EndTime!.Value)<=.05&&Math.Abs(note.StartTime.Value-sample.TotalSeconds)<=.051);TimeSpan wanted;
            if(pointOnly)wanted=TimeSpan.FromMilliseconds(750);
            else
            {
                var activeEnd=annotations.Where(note=>note.IsVideoTimeline&&note.EndTime!.Value-note.StartTime!.Value>.05&&sample.TotalSeconds>=note.StartTime.Value-.0001&&sample.TotalSeconds<=note.EndTime.Value+.0001).Select(note=>TimeSpan.FromSeconds(note.EndTime!.Value)).DefaultIfEmpty(sample+step).Max();var next=index+1<all.Length?TimeSpan.FromTicks(all[index+1]):activeEnd;wanted=next>sample&&next<=activeEnd?next-sample:activeEnd-sample;if(wanted<=TimeSpan.Zero)wanted=step;
            }
            var duration=videoDuration-sample<wanted?videoDuration-sample:wanted;if(duration>TimeSpan.Zero)result.Add(new VideoOverlayFrame(sample,duration,sample));
        }
        return result;
    }
}
