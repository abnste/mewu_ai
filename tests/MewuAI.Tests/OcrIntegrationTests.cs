using System.Globalization;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.OCR;
using Xunit;
namespace MewuAI.Tests;
public sealed class OcrIntegrationTests
{
    [Fact] public async Task PaddleOcrRecognizesRenderedEnglishTextWithPixelBounds()
    {
        var bitmap=RenderText("HELLO SCREEN 123","Segoe UI","en-US");var result=await new WindowsOcrService().RecognizeAsync(bitmap,TestContext.Current.CancellationToken);Assert.Equal("PP-OCRv6 本地 OCR",result.Engine);Assert.Contains("HELLO",result.Text,StringComparison.OrdinalIgnoreCase);Assert.All(result.Lines,line=>{Assert.True(line.Width>0);Assert.True(line.Height>0);});
    }

    [Fact] public async Task PaddleOcrRecognizesChineseEnglishAndNumbersTogether()
    {
        var bitmap=RenderText("喵呜 AI助手 2026","Microsoft YaHei UI","zh-CN");var result=await new WindowsOcrService().RecognizeAsync(bitmap,TestContext.Current.CancellationToken);Assert.Equal("PP-OCRv6 本地 OCR",result.Engine);Assert.Contains("AI",result.Text,StringComparison.OrdinalIgnoreCase);Assert.Contains("2026",result.Text);Assert.Contains("助手",result.Text);
    }

    private static RenderTargetBitmap RenderText(string text,string font,string culture)
    {
        var visual=new DrawingVisual();using(var dc=visual.RenderOpen()){dc.DrawRectangle(Brushes.White,null,new Rect(0,0,760,150));dc.DrawText(new FormattedText(text,CultureInfo.GetCultureInfo(culture),FlowDirection.LeftToRight,new Typeface(font),48,Brushes.Black,1),new Point(20,32));}var bitmap=new RenderTargetBitmap(760,150,96,96,PixelFormats.Pbgra32);bitmap.Render(visual);return bitmap;
    }
}
