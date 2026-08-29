using System.Drawing;
using System.Drawing.Imaging;
using System.Windows.Media.Imaging;
using Forms=System.Windows.Forms;
namespace mewu_ai_Assistant.Services;
public sealed class ScreenCaptureService
{
    public CaptureFrame CaptureDesktop()
    {
        var bounds=Forms.SystemInformation.VirtualScreen;
        using var bitmap=new Bitmap(bounds.Width,bounds.Height,PixelFormat.Format32bppPArgb);
        using(var graphics=Graphics.FromImage(bitmap)) graphics.CopyFromScreen(bounds.Left,bounds.Top,0,0,bounds.Size,CopyPixelOperation.SourceCopy);
        return new(bounds.Left,bounds.Top,ToSource(bitmap));
    }
    private static BitmapSource ToSource(Bitmap bitmap)
    {
        using var stream=new MemoryStream(); bitmap.Save(stream,ImageFormat.Png); stream.Position=0;
        var decoder=new PngBitmapDecoder(stream,BitmapCreateOptions.PreservePixelFormat,BitmapCacheOption.OnLoad); var source=decoder.Frames[0]; source.Freeze(); return source;
    }
    public static BitmapSource Crop(BitmapSource source,System.Windows.Int32Rect rect)
    {
        var x=Math.Clamp(rect.X,0,source.PixelWidth);var y=Math.Clamp(rect.Y,0,source.PixelHeight);
        rect=new System.Windows.Int32Rect(x,y,Math.Max(0,Math.Min(rect.Width,source.PixelWidth-x)),Math.Max(0,Math.Min(rect.Height,source.PixelHeight-y)));
        var crop=new CroppedBitmap(source,rect); crop.Freeze(); return crop;
    }
    public static void Save(BitmapSource image,string path,bool jpeg)
    {
        BitmapEncoder encoder=jpeg?new JpegBitmapEncoder { QualityLevel=92 }:new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(image));
        using var stream=File.Create(path); encoder.Save(stream);
    }
    public static byte[] EncodePng(BitmapSource image){var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(image));using var stream=new MemoryStream();encoder.Save(stream);return stream.ToArray();}
}
public sealed record CaptureFrame(int OriginX,int OriginY,BitmapSource Image);
