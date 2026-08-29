using System.Globalization;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.OCR;
using Xunit;
namespace MewuAI.Tests;
public sealed class OcrIntegrationTests
{
    [Fact] public async Task WindowsOcrRecognizesRenderedEnglishText()
    {
        var visual=new DrawingVisual();using(var dc=visual.RenderOpen()){dc.DrawRectangle(Brushes.White,null,new Rect(0,0,600,140));dc.DrawText(new FormattedText("HELLO SCREEN 123",CultureInfo.GetCultureInfo("en-US"),FlowDirection.LeftToRight,new Typeface("Segoe UI"),48,Brushes.Black,1),new Point(20,30));}var bitmap=new RenderTargetBitmap(600,140,96,96,PixelFormats.Pbgra32);bitmap.Render(visual);var result=await new WindowsOcrService().RecognizeAsync(bitmap,TestContext.Current.CancellationToken);Assert.Contains("HELLO",result.Text,StringComparison.OrdinalIgnoreCase);
    }
}
