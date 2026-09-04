using System.Windows;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

// Owned by the overlay dispatcher. Acquisition can continue while its single
// consumer awaits background matching; bounded storage prevents memory growth.
internal sealed class LongCaptureSampleBuffer
{
    internal sealed record Sample(BitmapSource Image,Int32Rect? IgnoredRegion,int Direction,bool ShowNoMovement);
    private readonly Queue<Sample> _samples=[];
    private long _pixels;
    internal int Count=>_samples.Count;
    internal bool HasCapacity(int width,int height)
        =>width>0&&height>0&&_samples.Count<8&&_pixels+(long)width*height<=16_000_000;
    internal bool TryEnqueue(Sample sample)
    {
        if(!sample.Image.IsFrozen)throw new ArgumentException("采样帧必须冻结",nameof(sample));
        if(!HasCapacity(sample.Image.PixelWidth,sample.Image.PixelHeight))return false;
        _samples.Enqueue(sample);_pixels+=(long)sample.Image.PixelWidth*sample.Image.PixelHeight;return true;
    }
    internal bool TryDequeue(out Sample sample)
    {
        if(!_samples.TryDequeue(out sample!))return false;
        _pixels-=(long)sample.Image.PixelWidth*sample.Image.PixelHeight;return true;
    }
    internal void Clear(){_samples.Clear();_pixels=0;}
}
