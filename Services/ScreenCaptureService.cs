using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Forms=System.Windows.Forms;
namespace mewu_ai_Assistant.Services;
public sealed class ScreenCaptureService
{
    public CaptureFrame CaptureDesktop(bool includeCursor=false)
    {
        var bounds=Forms.SystemInformation.VirtualScreen;
        using var bitmap=new Bitmap(bounds.Width,bounds.Height,System.Drawing.Imaging.PixelFormat.Format32bppPArgb);
        using(var graphics=Graphics.FromImage(bitmap)){graphics.CopyFromScreen(bounds.Left,bounds.Top,0,0,bounds.Size,CopyPixelOperation.SourceCopy);if(includeCursor)DrawCursor(graphics,bounds.Left,bounds.Top);}
        return new(bounds.Left,bounds.Top,ToSource(bitmap));
    }
    private static BitmapSource ToSource(Bitmap bitmap)
    {
        var rectangle=new Rectangle(0,0,bitmap.Width,bitmap.Height);
        var data=bitmap.LockBits(rectangle,ImageLockMode.ReadOnly,System.Drawing.Imaging.PixelFormat.Format32bppPArgb);
        try
        {
            if(data.Stride<=0)throw new InvalidOperationException("截图像素缓冲区方向无效");
            // The desktop frame is already an opaque BGRA buffer. Copying it
            // directly avoids PNG-compressing and decoding the entire virtual
            // desktop on every invocation. Bgr32 deliberately ignores the
            // unused alpha byte returned by CopyFromScreen.
            var source=BitmapSource.Create(bitmap.Width,bitmap.Height,bitmap.HorizontalResolution,bitmap.VerticalResolution,PixelFormats.Bgr32,null,data.Scan0,checked(data.Stride*bitmap.Height),data.Stride);
            source.Freeze();
            return source;
        }
        finally
        {
            bitmap.UnlockBits(data);
        }
    }
    public static BitmapSource Crop(BitmapSource source,System.Windows.Int32Rect rect)
    {
        ArgumentNullException.ThrowIfNull(source);
        if(rect.Width<=0||rect.Height<=0)throw new ArgumentOutOfRangeException(nameof(rect),"裁剪区域必须有正的宽高");

        // Clip both edges of the requested rectangle. Clamping only the
        // origin is incorrect for negative coordinates: (-10, 20) must
        // produce a 10-pixel intersection, not a 20-pixel crop shifted right.
        var left=Math.Clamp((long)rect.X,0L,(long)source.PixelWidth);
        var top=Math.Clamp((long)rect.Y,0L,(long)source.PixelHeight);
        var right=Math.Clamp((long)rect.X+rect.Width,0L,(long)source.PixelWidth);
        var bottom=Math.Clamp((long)rect.Y+rect.Height,0L,(long)source.PixelHeight);
        if(right<=left||bottom<=top)throw new ArgumentException("裁剪区域与截图没有交集",nameof(rect));

        var clipped=new System.Windows.Int32Rect((int)left,(int)top,(int)(right-left),(int)(bottom-top));
        var crop=new CroppedBitmap(source,clipped);crop.Freeze();return crop;
    }
    public static void Save(BitmapSource image,string path,bool jpeg)
    {
        BitmapEncoder encoder=jpeg?new JpegBitmapEncoder { QualityLevel=92 }:new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(image));
        var destination=Path.GetFullPath(path);var directory=Path.GetDirectoryName(destination)??throw new InvalidOperationException("图片保存目录无效");
        var temporary=Path.Combine(directory,$".{Path.GetFileName(destination)}.{Guid.NewGuid():N}.tmp");
        try
        {
            using(var stream=new FileStream(temporary,FileMode.CreateNew,FileAccess.Write,FileShare.None)){encoder.Save(stream);stream.Flush(true);}
            File.Move(temporary,destination,true);
        }
        finally
        {
            try{if(File.Exists(temporary))File.Delete(temporary);}catch{}
        }
    }
    private static void DrawCursor(Graphics graphics,int originX,int originY){var info=new CursorInfo{Size=Marshal.SizeOf<CursorInfo>()};if(!GetCursorInfo(ref info)||info.Flags!=1)return;if(!GetIconInfo(info.CursorHandle,out var icon))return;try{var hdc=graphics.GetHdc();try{DrawIconEx(hdc,info.Position.X-originX-icon.HotspotX,info.Position.Y-originY-icon.HotspotY,info.CursorHandle,0,0,0,IntPtr.Zero,3);}finally{graphics.ReleaseHdc(hdc);}}finally{if(icon.Mask!=IntPtr.Zero)DeleteObject(icon.Mask);if(icon.Color!=IntPtr.Zero)DeleteObject(icon.Color);}}
    [StructLayout(LayoutKind.Sequential)]private struct CursorInfo{public int Size;public int Flags;public IntPtr CursorHandle;public NativePoint Position;}
    [StructLayout(LayoutKind.Sequential)]private struct NativePoint{public int X;public int Y;}
    [StructLayout(LayoutKind.Sequential)]private struct IconInfo{[MarshalAs(UnmanagedType.Bool)]public bool IsIcon;public int HotspotX;public int HotspotY;public IntPtr Mask;public IntPtr Color;}
    [DllImport("user32.dll")]private static extern bool GetCursorInfo(ref CursorInfo info);[DllImport("user32.dll")]private static extern bool GetIconInfo(IntPtr icon,out IconInfo info);[DllImport("user32.dll")]private static extern bool DrawIconEx(IntPtr dc,int x,int y,IntPtr icon,int width,int height,int step,IntPtr brush,int flags);[DllImport("gdi32.dll")]private static extern bool DeleteObject(IntPtr value);
}
public sealed record CaptureFrame(int OriginX,int OriginY,BitmapSource Image);
