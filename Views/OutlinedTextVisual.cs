using System.Globalization;
using System.Windows;
using System.Windows.Media;

namespace mewu_ai_Assistant.Views;

internal sealed class OutlinedTextVisual:FrameworkElement
{
    private readonly IReadOnlyList<string> _lines;
    private readonly Typeface _typeface;
    private readonly double _fontSize;
    private readonly double _lineHeight;
    private readonly double _left;
    private readonly double _top;
    private readonly Brush _fill;
    private readonly Pen _outline;

    internal OutlinedTextVisual(IReadOnlyList<string> lines,string fontFamily,double fontSize,double lineHeight,Color fill,Color outline,double left,double top)
    {
        ArgumentNullException.ThrowIfNull(lines);_lines=lines.ToArray();_typeface=new Typeface(new FontFamily(fontFamily),FontStyles.Normal,FontWeights.Normal,FontStretches.Normal);_fontSize=fontSize;_lineHeight=lineHeight;_left=left;_top=top;FillColor=fill;OutlineColor=outline;StrokeThickness=Math.Clamp(fontSize*.045,.65,1.25);var fillBrush=new SolidColorBrush(fill);fillBrush.Freeze();_fill=fillBrush;var outlineBrush=new SolidColorBrush(outline);outlineBrush.Freeze();_outline=new Pen(outlineBrush,StrokeThickness){LineJoin=PenLineJoin.Round};_outline.Freeze();IsHitTestVisible=false;SnapsToDevicePixels=true;
    }

    internal Color FillColor { get; }
    internal Color OutlineColor { get; }
    internal double StrokeThickness { get; }

    protected override void OnRender(DrawingContext drawingContext)
    {
        base.OnRender(drawingContext);var pixelsPerDip=VisualTreeHelper.GetDpi(this).PixelsPerDip;
        for(var index=0;index<_lines.Count;index++)
        {
            if(string.IsNullOrEmpty(_lines[index]))continue;var formatted=new FormattedText(_lines[index],CultureInfo.CurrentUICulture,FlowDirection.LeftToRight,_typeface,_fontSize,_fill,pixelsPerDip);var geometry=formatted.BuildGeometry(new Point(_left,_top+index*_lineHeight));geometry.Freeze();drawingContext.DrawGeometry(_fill,_outline,geometry);
        }
    }
}
