using System.Runtime.ExceptionServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Effects;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class TranslationOverlayLayoutServiceTests
{
    [Fact]
    public void TranslationExpandsBeyondTheOriginalOcrLineInsteadOfClippingText()
    {
        var result=TranslationOverlayLayoutService.Place(new Rect(20,30,60,20),new Size(500,300),240,24);
        Assert.Equal(new Rect(20,28,240,24),result);
    }

    [Fact]
    public void ExpandedTranslationShiftsLeftAtTheRightEdge()
    {
        var result=TranslationOverlayLayoutService.Place(new Rect(430,30,60,20),new Size(500,300),220,24);
        Assert.Equal(new Rect(280,28,220,24),result);
    }

    [Fact]
    public void OversizedTranslationStaysInsideTheSelectionWithoutBeingTruncatedVertically()
    {
        var result=TranslationOverlayLayoutService.Place(new Rect(10,260,80,20),new Size(320,300),600,80);
        Assert.Equal(new Rect(0,220,320,80),result);
    }

    [Fact]
    public void LongTranslationWrapsOnlyAtTheSelectionEdgeWithoutDroppingCharacters()
    {
        const string text="一段非常非常长的译文内容";var rows=TranslationOverlayLayoutService.WrapText(text,50,value=>value.Length*10);
        Assert.True(rows.Count>1);Assert.Equal(text,string.Concat(rows));Assert.All(rows,row=>Assert.True(row.Length<=5));
    }

    [Fact]
    public void TranslationBackdropUsesGaussianSourcePixelsWithoutATextBoxBorder()
    {
        RunSta(() =>
        {
            var source=Checkerboard(160,60);var bounds=new Rect(20,10,120,36);var region=TranslationOverlayLayoutService.ToImagePixelRect(bounds,source,1,1);var average=TranslationOverlayLayoutService.GetAverageColor(source,region);var backdrop=TranslationOverlayLayoutService.CreateBackdrop(source,bounds,1,1,average);var canvas=Assert.IsType<Canvas>(Assert.Single(backdrop.Children));var image=Assert.IsType<Image>(canvas.Children[0]);var blur=Assert.IsType<BlurEffect>(image.Effect);Assert.Equal(KernelType.Gaussian,blur.KernelType);Assert.True(blur.Radius>=8);Assert.DoesNotContain(canvas.Children.Cast<UIElement>(),element=>element is Border);
            backdrop.Measure(new Size(bounds.Width,bounds.Height));backdrop.Arrange(new Rect(0,0,bounds.Width,bounds.Height));var rendered=new RenderTargetBitmap((int)bounds.Width,(int)bounds.Height,96,96,PixelFormats.Pbgra32);rendered.Render(backdrop);var pixel=new byte[4];rendered.CopyPixels(new Int32Rect((int)bounds.Width/2,(int)bounds.Height/2,1,1),pixel,4,0);Assert.True(pixel[3]>0);Assert.InRange(pixel[0],25,230);Assert.InRange(pixel[1],25,230);Assert.InRange(pixel[2],25,230);
        });
    }

    private static BitmapSource Checkerboard(int width,int height)
    {
        var stride=width*4;var pixels=new byte[stride*height];for(var y=0;y<height;y++)for(var x=0;x<width;x++){var value=(byte)(((x/4+y/4)&1)==0?20:235);var offset=y*stride+x*4;pixels[offset]=pixels[offset+1]=pixels[offset+2]=value;pixels[offset+3]=255;}var bitmap=BitmapSource.Create(width,height,96,96,PixelFormats.Bgra32,null,pixels,stride);bitmap.Freeze();return bitmap;
    }

    private static void RunSta(Action action)
    {
        Exception? failure=null;var thread=new Thread(()=>{try{action();}catch(Exception ex){failure=ex;}});thread.SetApartmentState(ApartmentState.STA);thread.Start();thread.Join();if(failure is not null)ExceptionDispatchInfo.Capture(failure).Throw();
    }
}
