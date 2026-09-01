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
            var cardWidth=Math.Clamp(width*.3,145,360);var font=Math.Clamp(width/70d,11,22);var slots=new List<double>();
            foreach(var annotation in annotations.Take(6))
            {
                var frame=new VideoAnnotationKeyframe(videoTime??0,annotation.X,annotation.Y,annotation.Width,annotation.Height);
                if(annotation.IsVideoTimeline&&(!videoTime.HasValue||videoTime<annotation.StartTime||videoTime>annotation.EndTime||!VideoAnnotationTimeline.TryInterpolate(annotation,videoTime.Value,out frame)))continue;
                var x=Math.Clamp(frame.X,0,1)*width;var y=Math.Clamp(frame.Y,0,1)*height;var boxWidth=Math.Max(14,Math.Clamp(frame.Width,0,1)*width);var boxHeight=Math.Max(14,Math.Clamp(frame.Height,0,1)*height);
                var boxPen=new Pen(Cyan,Math.Max(2,width/900d));boxPen.Freeze();
                drawing.DrawRoundedRectangle(new SolidColorBrush(Color.FromArgb(14,55,170,255)),boxPen,new Rect(x,y,boxWidth,boxHeight),5,5);

                var right=x+boxWidth+cardWidth+28<width;var cardX=right?x+boxWidth+24:Math.Max(5,x-cardWidth-24);var cardHeight=Math.Max(font*3.2,font*1.8);var cardY=AnnotationLayoutService.FindCardTop(y+boxHeight*.5-font*1.5,5,Math.Max(5,height-font*4),cardHeight,slots);slots.Add(cardY);var startX=right?x+boxWidth:x;var endX=right?cardX:cardX+cardWidth;
                var linePen=new Pen(Cyan,Math.Max(1,width/1200d));linePen.Freeze();drawing.DrawLine(linePen,new Point(startX,y+boxHeight*.5),new Point(endX,cardY+font*1.4));drawing.DrawEllipse(Cyan,null,new Point(endX,cardY+font*1.4),2.5,2.5);
                drawing.DrawRoundedRectangle(new SolidColorBrush(Color.FromArgb(248,255,255,255)),new Pen(new SolidColorBrush(Color.FromArgb(145,61,174,242)),1),new Rect(cardX,cardY,cardWidth,cardHeight),8,8);
                var text=new FormattedText(annotation.Text,CultureInfo.CurrentUICulture,FlowDirection.LeftToRight,new Typeface("Microsoft YaHei UI"),font,new SolidColorBrush(Color.FromRgb(35,48,70)),1){MaxTextWidth=Math.Max(1,cardWidth-font*1.3),MaxTextHeight=Math.Max(1,cardHeight-font)};
                drawing.DrawText(text,new Point(cardX+font*.65,cardY+font*.45));
            }
        }
        var bitmap=new RenderTargetBitmap(width,height,96,96,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();return bitmap;
    }

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
            for(var time=start;time<end;time+=step)samples.Add(time.Ticks);
            samples.Add(Math.Max(start.Ticks,end.Ticks-1));
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
