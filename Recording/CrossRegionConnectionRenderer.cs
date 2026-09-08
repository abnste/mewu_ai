// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Recording;

internal static class CrossRegionConnectionRenderer
{
    internal static DrawingImage CreateDrawing(double width,double height,IReadOnlyList<CrossRegionConnection> connections)
    {
        var group=new DrawingGroup();
        using(var drawing=group.Open())
        {
            drawing.DrawRectangle(Brushes.Transparent,null,new Rect(0,0,Math.Max(1,width),Math.Max(1,height)));
            foreach(var connection in connections.Take(CrossRegionConnectionService.MaximumConnections))DrawConnection(drawing,connection,new Size(width,height));
        }
        group.Freeze();var image=new DrawingImage(group);image.Freeze();return image;
    }

    private static void DrawConnection(DrawingContext drawing,CrossRegionConnection connection,Size canvas)
    {
        var color=AnnotationPalette.Resolve(connection.Annotation.EffectiveStyle.Color);var brush=new SolidColorBrush(color);
        var pen=new Pen(brush,1.8){StartLineCap=PenLineCap.Round,EndLineCap=PenLineCap.Round,LineJoin=PenLineJoin.Round};
        var targetCenter=new Point(connection.Destination.X+connection.Destination.Width/2,connection.Destination.Y+connection.Destination.Height/2);
        var from=CrossRegionConnectionService.BoundaryToward(connection.Source,targetCenter);
        var to=CrossRegionConnectionService.BoundaryToward(connection.Destination,from);
        var delta=to-from;var horizontal=Math.Abs(delta.X)>=Math.Abs(delta.Y);var bend=Math.Clamp(delta.Length*.35,20,160);
        var direction=horizontal?new Vector(delta.X>=0?1:-1,0):new Vector(0,delta.Y>=0?1:-1);
        var c1=from+direction*bend;var c2=to-direction*bend;
        var path=new StreamGeometry();using(var context=path.Open()){context.BeginFigure(from,false,false);context.BezierTo(c1,c2,to,true,true);}path.Freeze();
        drawing.PushOpacity(connection.Annotation.EffectiveStyle.Opacity);
        drawing.DrawGeometry(null,new Pen(new SolidColorBrush(Color.FromArgb(210,255,255,255)),4.8),path);drawing.DrawGeometry(null,pen,path);
        drawing.DrawRoundedRectangle(new SolidColorBrush(Color.FromArgb(12,color.R,color.G,color.B)),new Pen(brush,1.4),connection.Source,4,4);
        drawing.DrawRoundedRectangle(new SolidColorBrush(Color.FromArgb(18,color.R,color.G,color.B)),pen,connection.Destination,4,4);
        var normal=new Vector(-direction.Y,direction.X);var arrow=new StreamGeometry();using(var context=arrow.Open()){context.BeginFigure(to,true,true);context.LineTo(to-direction*9+normal*4,true,false);context.LineTo(to-direction*9-normal*4,true,false);}arrow.Freeze();drawing.DrawGeometry(brush,null,arrow);drawing.DrawEllipse(Brushes.White,pen,from,3,3);
        var midpoint=new Point((from.X+to.X)/2,(from.Y+to.Y)/2);
        DrawLabel(drawing,$"{connection.SourceLabel} → {connection.DestinationLabel} · {connection.Annotation.Text}",midpoint,canvas,brush);
        drawing.Pop();
    }

    private static void DrawLabel(DrawingContext drawing,string label,Point at,Size canvas,Brush accent)
    {
        var width=Math.Min(230,Math.Max(1,canvas.Width-12));
        var text=new FormattedText(label.Length>100?label[..100]+"…":label,CultureInfo.CurrentUICulture,FlowDirection.LeftToRight,new Typeface("Segoe UI"),11,Brushes.Black,1){MaxTextWidth=Math.Max(1,width-16),MaxLineCount=2,Trimming=TextTrimming.CharacterEllipsis};
        var actualWidth=Math.Min(width,text.Width+16);var height=text.Height+10;
        var left=Math.Clamp(at.X-actualWidth/2,4,Math.Max(4,canvas.Width-actualWidth-4));var top=Math.Clamp(at.Y-height-10,4,Math.Max(4,canvas.Height-height-4));
        drawing.DrawRoundedRectangle(Brushes.White,new Pen(accent,.8),new Rect(left,top,actualWidth,height),7,7);drawing.DrawText(text,new Point(left+8,top+5));
    }

    internal static BitmapSource RenderRegion(int width,int height,Rect region,string handle,IReadOnlyList<CrossRegionConnection> connections)
    {
        var visual=new DrawingVisual();using(var drawing=visual.RenderOpen())
        {
            drawing.PushClip(new RectangleGeometry(new Rect(0,0,width,height)));
            drawing.PushTransform(new ScaleTransform(width/region.Width,height/region.Height));
            foreach(var link in connections.Where(link=>link.Annotation.ReferenceHandle==handle||link.Annotation.Destination?.ReferenceHandle==handle).Take(CrossRegionConnectionService.MaximumConnections))
            {
                var incoming=link.Annotation.ReferenceHandle!=handle;var bounds=incoming?link.Destination:link.Source;bounds.Offset(-region.X,-region.Y);
                var brush=new SolidColorBrush(AnnotationPalette.Resolve(link.Annotation.EffectiveStyle.Color));drawing.DrawRoundedRectangle(null,new Pen(brush,1.8),bounds,4,4);
                var label=$"{link.SourceLabel} → {link.DestinationLabel}";
                DrawLabel(drawing,label+" · "+link.Annotation.Text,new Point(bounds.Left+bounds.Width/2,bounds.Top),region.Size,brush);
            }
            drawing.Pop();drawing.Pop();
        }
        var bitmap=new RenderTargetBitmap(width,height,96,96,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();return bitmap;
    }
}
