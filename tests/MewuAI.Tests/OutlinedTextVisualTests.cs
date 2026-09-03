using System.Runtime.ExceptionServices;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Views;
using Xunit;

namespace MewuAI.Tests;

public sealed class OutlinedTextVisualTests
{
    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void TranslationTextUsesTheOppositeOutlineAndRendersBothTones(bool whiteText)
    {
        RunSta(() =>
        {
            var fill=whiteText?Colors.White:Colors.Black;var outline=whiteText?Colors.Black:Colors.White;var visual=new OutlinedTextVisual(["翻译 Translation"],"Segoe UI",24,30,fill,outline,3,2){Width=240,Height=36};Assert.Equal(fill,visual.FillColor);Assert.Equal(outline,visual.OutlineColor);Assert.InRange(visual.StrokeThickness,.65,1.25);visual.Measure(new Size(240,36));visual.Arrange(new Rect(0,0,240,36));visual.UpdateLayout();var bitmap=new RenderTargetBitmap(240,36,96,96,PixelFormats.Pbgra32);bitmap.Render(visual);var stride=240*4;var pixels=new byte[stride*36];bitmap.CopyPixels(pixels,stride,0);var hasDark=false;var hasLight=false;var opaque=0;var minimum=255;var maximum=0;for(var offset=0;offset+3<pixels.Length;offset+=4){var alpha=pixels[offset+3];if(alpha<20)continue;opaque++;var premultiplied=(pixels[offset]+pixels[offset+1]+pixels[offset+2])/3;var luminance=Math.Clamp(premultiplied*255/alpha,0,255);minimum=Math.Min(minimum,luminance);maximum=Math.Max(maximum,luminance);if(luminance<80)hasDark=true;if(luminance>175)hasLight=true;}Assert.True(opaque>0,"Outlined text did not render");Assert.True(hasDark,$"No dark outline/fill pixels; range={minimum}..{maximum}");Assert.True(hasLight,$"No light outline/fill pixels; range={minimum}..{maximum}");
        });
    }

    private static void RunSta(Action action)
    {
        Exception? failure=null;var thread=new Thread(()=>{try{action();}catch(Exception ex){failure=ex;}});thread.SetApartmentState(ApartmentState.STA);thread.Start();thread.Join();if(failure is not null)ExceptionDispatchInfo.Capture(failure).Throw();
    }
}
