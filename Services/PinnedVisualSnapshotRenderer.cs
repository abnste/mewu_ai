using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

internal static class PinnedVisualSnapshotRenderer
{
    internal static BitmapSource Render(FrameworkElement root,int physicalWidth,int physicalHeight)
    {
        if(physicalWidth<=0||physicalHeight<=0||root.ActualWidth<=0||root.ActualHeight<=0)throw new InvalidOperationException("贴图尺寸无效");
        // Zoom is unbounded, snapshot allocation is not. Preserve the physical
        // destination rectangle while downsampling exceptionally large pins.
        var scale=Math.Min(1,Math.Min(8192d/Math.Max(physicalWidth,physicalHeight),Math.Sqrt(16_000_000d/physicalWidth/physicalHeight)));
        var width=Math.Max(1,(int)(physicalWidth*scale));var height=Math.Max(1,(int)(physicalHeight*scale));
        var visual=new DrawingVisual();
        using(var context=visual.RenderOpen())
        {
            var brush=new VisualBrush(root){ViewboxUnits=BrushMappingMode.Absolute,Viewbox=new Rect(0,0,root.ActualWidth,root.ActualHeight),Stretch=Stretch.Fill};
            context.DrawRectangle(brush,null,new Rect(0,0,width,height));
        }
        var bitmap=new RenderTargetBitmap(width,height,96,96,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();return bitmap;
    }
}
