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
        var sourceWidth=image.PixelWidth;var sourceHeight=image.PixelHeight;var maxDimension=OcrEngine.MaxImageDimension;var scale=Math.Min(1d,maxDimension/(double)Math.Max(sourceWidth,sourceHeight));BitmapSource input=image;if(scale<1){var resized=new TransformedBitmap(image,new ScaleTransform(scale,scale));resized.Freeze();input=resized;}var converted=new FormatConvertedBitmap(input,PixelFormats.Bgra32,null,0);var convertedWidth=converted.PixelWidth;var convertedHeight=converted.PixelHeight;var stride=convertedWidth*4;var pixels=new byte[stride*convertedHeight];converted.CopyPixels(pixels,stride,0);
        using var bitmap=new SoftwareBitmap(BitmapPixelFormat.Bgra8,convertedWidth,convertedHeight,BitmapAlphaMode.Premultiplied);bitmap.CopyFromBuffer(pixels.AsBuffer());Array.Clear(pixels);
        var engine=OcrEngine.TryCreateFromUserProfileLanguages()??throw new InvalidOperationException("Windows 未安装可用的 OCR 语言包");
        var result=await engine.RecognizeAsync(bitmap).AsTask(token);var lines=new List<Models.OcrLine>();var scaleX=sourceWidth/(double)convertedWidth;var scaleY=sourceHeight/(double)convertedHeight;
        foreach(var line in result.Lines){var words=line.Words.Select(w=>new Models.OcrWord(w.Text,w.BoundingRect.X*scaleX,w.BoundingRect.Y*scaleY,w.BoundingRect.Width*scaleX,w.BoundingRect.Height*scaleY)).ToList();if(words.Count==0)continue;var x=words.Min(w=>w.X);var y=words.Min(w=>w.Y);var right=words.Max(w=>w.X+w.Width);var bottom=words.Max(w=>w.Y+w.Height);lines.Add(new Models.OcrLine(line.Text,x,y,right-x,bottom-y,words));}
        return new OcrDocument(result.Text,lines);
    }
}
