using System.Runtime.InteropServices.WindowsRuntime;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
using Windows.Graphics.Imaging;
using Windows.Media.Ocr;
namespace mewu_ai_Assistant.OCR;
public sealed class WindowsOcrService : IOcrService
{
    public async Task<OcrDocument> RecognizeAsync(BitmapSource image,CancellationToken token)
    {
        var converted=new FormatConvertedBitmap(image,PixelFormats.Bgra32,null,0);var stride=converted.PixelWidth*4;var pixels=new byte[stride*converted.PixelHeight];converted.CopyPixels(pixels,stride,0);
        using var bitmap=new SoftwareBitmap(BitmapPixelFormat.Bgra8,converted.PixelWidth,converted.PixelHeight,BitmapAlphaMode.Premultiplied);bitmap.CopyFromBuffer(pixels.AsBuffer());
        var engine=OcrEngine.TryCreateFromUserProfileLanguages()??throw new InvalidOperationException("Windows 未安装可用的 OCR 语言包");
        var result=await engine.RecognizeAsync(bitmap).AsTask(token);var lines=new List<Models.OcrLine>();
        foreach(var line in result.Lines){var words=line.Words.Select(w=>new Models.OcrWord(w.Text,w.BoundingRect.X,w.BoundingRect.Y,w.BoundingRect.Width,w.BoundingRect.Height)).ToList();if(words.Count==0)continue;var x=words.Min(w=>w.X);var y=words.Min(w=>w.Y);var right=words.Max(w=>w.X+w.Width);var bottom=words.Max(w=>w.Y+w.Height);lines.Add(new Models.OcrLine(line.Text,x,y,right-x,bottom-y,words));}
        return new OcrDocument(result.Text,lines);
    }
}
