using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Effects;
using System.Windows.Media.Imaging;
using System.Windows.Shapes;

namespace mewu_ai_Assistant.Services;

internal static class TranslationOverlayLayoutService
{
    internal const double HorizontalPadding=6;
    internal const double VerticalPadding=2;

    internal static Rect Place(Rect source,Size canvas,double contentWidth,double contentHeight)
    {
        if(!IsFinitePositive(canvas.Width)||!IsFinitePositive(canvas.Height))return Rect.Empty;
        var width=Math.Clamp(Math.Max(source.Width,contentWidth),1,canvas.Width);
        var height=Math.Clamp(Math.Max(source.Height,contentHeight),1,canvas.Height);
        var left=Math.Clamp(source.Left,0,Math.Max(0,canvas.Width-width));
        var top=Math.Clamp(source.Top-Math.Max(0,(height-source.Height)/2),0,Math.Max(0,canvas.Height-height));
        return new Rect(left,top,width,height);
    }

    internal static IReadOnlyList<string> WrapText(string value,double maxWidth,Func<string,double> measure)
    {
        ArgumentNullException.ThrowIfNull(value);ArgumentNullException.ThrowIfNull(measure);value=value.Replace('\r',' ').Replace('\n',' ').Trim();if(value.Length==0||!IsFinitePositive(maxWidth))return [];var elements=new List<string>();var enumerator=StringInfo.GetTextElementEnumerator(value);while(enumerator.MoveNext())elements.Add(enumerator.GetTextElement());var rows=new List<string>();
        for(var start=0;start<elements.Count;)
        {
            var low=start+1;var high=elements.Count;var best=low;while(low<=high){var middle=low+(high-low)/2;var candidate=string.Concat(elements.Skip(start).Take(middle-start));if(measure(candidate)<=maxWidth){best=middle;low=middle+1;}else high=middle-1;}if(best<elements.Count){for(var boundary=best-1;boundary>start;boundary--){if(!IsWrapBoundary(elements[boundary]))continue;best=boundary+1;break;}}var row=string.Concat(elements.Skip(start).Take(best-start)).Trim();if(row.Length>0)rows.Add(row);start=Math.Max(best,start+1);while(start<elements.Count&&string.IsNullOrWhiteSpace(elements[start]))start++;
        }
        return rows;
    }

    internal static Int32Rect ToImagePixelRect(Rect bounds,BitmapSource image,double scaleX,double scaleY)
    {
        ArgumentNullException.ThrowIfNull(image);if(!IsFinitePositive(scaleX)||!IsFinitePositive(scaleY)||image.PixelWidth<1||image.PixelHeight<1)return Int32Rect.Empty;
        var left=Math.Clamp((int)Math.Floor(bounds.Left/scaleX),0,image.PixelWidth-1);var top=Math.Clamp((int)Math.Floor(bounds.Top/scaleY),0,image.PixelHeight-1);var right=Math.Clamp((int)Math.Ceiling(bounds.Right/scaleX),left+1,image.PixelWidth);var bottom=Math.Clamp((int)Math.Ceiling(bounds.Bottom/scaleY),top+1,image.PixelHeight);return new Int32Rect(left,top,right-left,bottom-top);
    }

    internal static Color GetAverageColor(BitmapSource image,Int32Rect region)
    {
        ArgumentNullException.ThrowIfNull(image);if(region.IsEmpty)return Color.FromRgb(245,247,250);BitmapSource bgra=image;if(image.Format!=PixelFormats.Bgra32){var converted=new FormatConvertedBitmap(image,PixelFormats.Bgra32,null,0);converted.Freeze();bgra=converted;}var crop=new CroppedBitmap(bgra,region);crop.Freeze();var scale=Math.Min(1d,64d/Math.Max(crop.PixelWidth,crop.PixelHeight));BitmapSource sample=crop;if(scale<1){var resized=new TransformedBitmap(crop,new ScaleTransform(scale,scale));resized.Freeze();sample=resized;}var stride=sample.PixelWidth*4;var pixels=new byte[stride*sample.PixelHeight];sample.CopyPixels(pixels,stride,0);long red=0,green=0,blue=0,count=0;for(var offset=0;offset+3<pixels.Length;offset+=4){blue+=pixels[offset];green+=pixels[offset+1];red+=pixels[offset+2];count++;}return count==0?Color.FromRgb(245,247,250):Color.FromRgb((byte)(red/count),(byte)(green/count),(byte)(blue/count));
    }

    internal static Grid CreateBackdrop(BitmapSource image,Rect bounds,double scaleX,double scaleY,Color average)
    {
        ArgumentNullException.ThrowIfNull(image);var target=ToImagePixelRect(bounds,image,scaleX,scaleY);if(target.IsEmpty)return new Grid{Width=bounds.Width,Height=bounds.Height};const int margin=14;var left=Math.Max(0,target.X-margin);var top=Math.Max(0,target.Y-margin);var right=Math.Min(image.PixelWidth,target.X+target.Width+margin);var bottom=Math.Min(image.PixelHeight,target.Y+target.Height+margin);var cropRect=new Int32Rect(left,top,Math.Max(1,right-left),Math.Max(1,bottom-top));var crop=new CroppedBitmap(image,cropRect);crop.Freeze();var canvas=new Canvas{Width=bounds.Width,Height=bounds.Height,ClipToBounds=true,IsHitTestVisible=false};var background=new Image{Source=crop,Width=cropRect.Width*scaleX,Height=cropRect.Height*scaleY,Stretch=Stretch.Fill,IsHitTestVisible=false,Effect=new BlurEffect{Radius=9,KernelType=KernelType.Gaussian,RenderingBias=RenderingBias.Quality}};Canvas.SetLeft(background,cropRect.X*scaleX-bounds.Left);Canvas.SetTop(background,cropRect.Y*scaleY-bounds.Top);canvas.Children.Add(background);canvas.Children.Add(new Rectangle{Width=bounds.Width,Height=bounds.Height,Fill=new SolidColorBrush(Color.FromArgb(52,average.R,average.G,average.B)),IsHitTestVisible=false});var result=new Grid{Width=bounds.Width,Height=bounds.Height,ClipToBounds=true,IsHitTestVisible=false};result.Children.Add(canvas);return result;
    }

    private static bool IsFinitePositive(double value)=>double.IsFinite(value)&&value>0;
    private static bool IsWrapBoundary(string value)=>string.IsNullOrWhiteSpace(value)||"，。！？；：、,.!?;:".Contains(value,StringComparison.Ordinal);
}
