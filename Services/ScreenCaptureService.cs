using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Windows.Media.Imaging;
using Forms=System.Windows.Forms;
namespace mewu_ai_Assistant.Services;
public sealed class ScreenCaptureService
{
    public CaptureFrame CaptureDesktop(bool includeCursor=false)
    {
        var bounds=Forms.SystemInformation.VirtualScreen;
        using var bitmap=new Bitmap(bounds.Width,bounds.Height,PixelFormat.Format32bppPArgb);
        using(var graphics=Graphics.FromImage(bitmap)){graphics.CopyFromScreen(bounds.Left,bounds.Top,0,0,bounds.Size,CopyPixelOperation.SourceCopy);if(includeCursor)DrawCursor(graphics,bounds.Left,bounds.Top);}
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
    private static void DrawCursor(Graphics graphics,int originX,int originY){var info=new CursorInfo{Size=Marshal.SizeOf<CursorInfo>()};if(!GetCursorInfo(ref info)||info.Flags!=1)return;if(!GetIconInfo(info.CursorHandle,out var icon))return;try{var hdc=graphics.GetHdc();try{DrawIconEx(hdc,info.Position.X-originX-icon.HotspotX,info.Position.Y-originY-icon.HotspotY,info.CursorHandle,0,0,0,IntPtr.Zero,3);}finally{graphics.ReleaseHdc(hdc);}}finally{if(icon.Mask!=IntPtr.Zero)DeleteObject(icon.Mask);if(icon.Color!=IntPtr.Zero)DeleteObject(icon.Color);}}
    [StructLayout(LayoutKind.Sequential)]private struct CursorInfo{public int Size;public int Flags;public IntPtr CursorHandle;public NativePoint Position;}
    [StructLayout(LayoutKind.Sequential)]private struct NativePoint{public int X;public int Y;}
    [StructLayout(LayoutKind.Sequential)]private struct IconInfo{[MarshalAs(UnmanagedType.Bool)]public bool IsIcon;public int HotspotX;public int HotspotY;public IntPtr Mask;public IntPtr Color;}
    [DllImport("user32.dll")]private static extern bool GetCursorInfo(ref CursorInfo info);[DllImport("user32.dll")]private static extern bool GetIconInfo(IntPtr icon,out IconInfo info);[DllImport("user32.dll")]private static extern bool DrawIconEx(IntPtr dc,int x,int y,IntPtr icon,int width,int height,int step,IntPtr brush,int flags);[DllImport("gdi32.dll")]private static extern bool DeleteObject(IntPtr value);
}
public sealed record CaptureFrame(int OriginX,int OriginY,BitmapSource Image);
