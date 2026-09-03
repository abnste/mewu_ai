using System.Drawing;
using System.Drawing.Drawing2D;
using DrawingPixelFormat=System.Drawing.Imaging.PixelFormat;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Keeps the user-visible bitmap for a pinned image available to an explicit
/// screenshot without disabling the window's display-affinity protection.
/// </summary>
internal static class PinnedImageCaptureRegistry
{
    private static readonly object Gate=new();
    private static readonly List<Func<PinnedImageCaptureSnapshot?>> Providers=[];

    internal static IDisposable Register(Func<PinnedImageCaptureSnapshot?> provider)
    {
        ArgumentNullException.ThrowIfNull(provider);
        lock(Gate)Providers.Add(provider);
        return new Registration(provider);
    }

    internal static void CompositeInto(Bitmap destination,ScreenRect virtualBounds)
    {
        ArgumentNullException.ThrowIfNull(destination);
        if(virtualBounds.IsEmpty)return;
        var snapshots=GetSnapshots();
        if(snapshots.Count==0)return;
        using var graphics=Graphics.FromImage(destination);
        graphics.CompositingMode=CompositingMode.SourceOver;
        graphics.CompositingQuality=CompositingQuality.HighQuality;
        graphics.InterpolationMode=InterpolationMode.HighQualityBicubic;
        graphics.PixelOffsetMode=PixelOffsetMode.HighQuality;
        // Registrations are kept in creation order. Draw older pins first so
        // a newer pin is composited above it when their visible rectangles
        // overlap, matching the normal topmost window stacking order.
        for(var index=0;index<snapshots.Count;index++)
        {
            var snapshot=snapshots[index];
            if(snapshot.Image.PixelWidth<=0||snapshot.Image.PixelHeight<=0||snapshot.ContentBounds.IsEmpty||snapshot.Opacity<=0)continue;
            var clipped=snapshot.ContentBounds.Intersect(virtualBounds);
            if(clipped.IsEmpty)continue;
            using var source=ToBitmap(snapshot.Image);
            var destinationRect=new Rectangle(clipped.X-virtualBounds.X,clipped.Y-virtualBounds.Y,clipped.Width,clipped.Height);
            var sourceX=(int)Math.Round((clipped.X-snapshot.ContentBounds.X)/(double)snapshot.ContentBounds.Width*source.Width);
            var sourceY=(int)Math.Round((clipped.Y-snapshot.ContentBounds.Y)/(double)snapshot.ContentBounds.Height*source.Height);
            var sourceRight=(int)Math.Round((clipped.Right-snapshot.ContentBounds.X)/(double)snapshot.ContentBounds.Width*source.Width);
            var sourceBottom=(int)Math.Round((clipped.Bottom-snapshot.ContentBounds.Y)/(double)snapshot.ContentBounds.Height*source.Height);
            var sourceWidth=Math.Clamp(sourceRight-sourceX,1,source.Width-sourceX);
            var sourceHeight=Math.Clamp(sourceBottom-sourceY,1,source.Height-sourceY);
            if(sourceX<0||sourceY<0||sourceX>=source.Width||sourceY>=source.Height)continue;
            using var attributes=CreateOpacityAttributes(snapshot.Opacity);
            graphics.DrawImage(source,destinationRect,sourceX,sourceY,sourceWidth,sourceHeight,GraphicsUnit.Pixel,attributes);
        }
    }

    private static List<PinnedImageCaptureSnapshot> GetSnapshots()
    {
        Func<PinnedImageCaptureSnapshot?>[] providers;
        lock(Gate)providers=Providers.ToArray();
        var snapshots=new List<PinnedImageCaptureSnapshot>(providers.Length);
        foreach(var provider in providers)
        {
            try
            {
                if(provider() is { } snapshot)snapshots.Add(snapshot);
            }
            catch(InvalidOperationException)
            {
                // A capture request can be issued from a worker while a WPF
                // pin is closing; that pin simply is not part of this frame.
            }
            catch(InvalidCastException)
            {
                // Ignore a torn-down visual provider just like a closed pin.
            }
        }
        return snapshots;
    }

    private static Bitmap ToBitmap(BitmapSource source)
    {
        var converted=new FormatConvertedBitmap(source,PixelFormats.Pbgra32,null,0);converted.Freeze();
        var width=converted.PixelWidth;var height=converted.PixelHeight;var stride=checked(width*4);var pixels=new byte[checked(stride*height)];
        try
        {
            converted.CopyPixels(pixels,stride,0);
            var bitmap=new Bitmap(width,height,DrawingPixelFormat.Format32bppPArgb);
            var data=bitmap.LockBits(new Rectangle(0,0,width,height),ImageLockMode.WriteOnly,DrawingPixelFormat.Format32bppPArgb);
            try{Marshal.Copy(pixels,0,data.Scan0,pixels.Length);}catch{bitmap.Dispose();throw;}finally{bitmap.UnlockBits(data);}
            return bitmap;
        }
        finally{Array.Clear(pixels);}
    }

    private static ImageAttributes? CreateOpacityAttributes(double opacity)
    {
        if(opacity>=.999)return null;
        var matrix=new ColorMatrix{Matrix33=(float)Math.Clamp(opacity,0,1)};var attributes=new ImageAttributes();attributes.SetColorMatrix(matrix);return attributes;
    }

    private sealed class Registration(Func<PinnedImageCaptureSnapshot?> provider):IDisposable
    {
        private Func<PinnedImageCaptureSnapshot?>? _provider=provider;
        public void Dispose()
        {
            var provider=Interlocked.Exchange(ref _provider,null);if(provider is null)return;
            lock(Gate)Providers.Remove(provider);
        }
    }
}

internal sealed record PinnedImageCaptureSnapshot(ScreenRect ContentBounds,BitmapSource Image,double Opacity);
