using System.Runtime.InteropServices.WindowsRuntime;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using RapidOcrNet;
using SkiaSharp;
using Windows.Graphics.Imaging;
using Windows.Media.Ocr;

namespace mewu_ai_Assistant.OCR;

public sealed class WindowsOcrService : IOcrService
{
    private static readonly SemaphoreSlim PaddleGate=new(1,1);
    private static readonly Lazy<RapidOcr> PaddleEngine=new(CreatePaddleEngine,LazyThreadSafetyMode.ExecutionAndPublication);

    public async Task<OcrDocument> RecognizeAsync(BitmapSource image,CancellationToken token)
    {
        try
        {
            var result=await RecognizeWithPaddleAsync(image,token);
            if(result.Lines.Count>0)return result;
        }
        catch(OperationCanceledException){throw;}
        catch(Exception ex){new PrivacyLogger().Error("PP-OCRv6",ex);}
        return await RecognizeWithLegacyEngineAsync(image,token);
    }

    private static async Task<OcrDocument> RecognizeWithPaddleAsync(BitmapSource image,CancellationToken token)
    {
        using var bitmap=CreateSkBitmap(image);
        await PaddleGate.WaitAsync(token);
        try
        {
            var options=RapidOcrOptions.PPOCRv6 with {ReturnWordBox=true,ReturnSingleCharBox=true,TextScore=.45f};
            var result=await PaddleEngine.Value.DetectAsync(bitmap,options,null,token);
            var lines=result.TextBlocks.Select(ToLine).Where(line=>!string.IsNullOrWhiteSpace(line.Text)&&line.Width>0&&line.Height>0).ToList();
            return new OcrDocument(string.Join(Environment.NewLine,lines.Select(line=>line.Text)),lines,"PP-OCRv6 本地 OCR");
        }
        finally{PaddleGate.Release();}
    }

    private static async Task<OcrDocument> RecognizeWithLegacyEngineAsync(BitmapSource image,CancellationToken token)
    {
        using var bitmap=CreateSoftwareBitmap(image,out var sourceWidth,out var sourceHeight,out var convertedWidth,out var convertedHeight);
        var engine=OcrEngine.TryCreateFromUserProfileLanguages()??throw new InvalidOperationException("Windows 未安装可用的 OCR 语言包");var result=await engine.RecognizeAsync(bitmap).AsTask(token);var lines=new List<Models.OcrLine>();var scaleX=sourceWidth/(double)convertedWidth;var scaleY=sourceHeight/(double)convertedHeight;
        foreach(var line in result.Lines){var words=line.Words.Select(word=>new Models.OcrWord(word.Text,word.BoundingRect.X*scaleX,word.BoundingRect.Y*scaleY,word.BoundingRect.Width*scaleX,word.BoundingRect.Height*scaleY)).ToList();if(words.Count==0)continue;var x=words.Min(word=>word.X);var y=words.Min(word=>word.Y);var right=words.Max(word=>word.X+word.Width);var bottom=words.Max(word=>word.Y+word.Height);lines.Add(new Models.OcrLine(line.Text,x,y,right-x,bottom-y,words));}
        return new OcrDocument(result.Text,lines,"Windows 传统 OCR");
    }

    private static SoftwareBitmap CreateSoftwareBitmap(BitmapSource image,out int sourceWidth,out int sourceHeight,out int convertedWidth,out int convertedHeight)
    {
        sourceWidth=image.PixelWidth;sourceHeight=image.PixelHeight;var maxDimension=OcrEngine.MaxImageDimension;var scale=Math.Min(1d,maxDimension/(double)Math.Max(sourceWidth,sourceHeight));BitmapSource input=image;if(scale<1){var resized=new TransformedBitmap(image,new ScaleTransform(scale,scale));resized.Freeze();input=resized;}var converted=new FormatConvertedBitmap(input,PixelFormats.Bgra32,null,0);convertedWidth=converted.PixelWidth;convertedHeight=converted.PixelHeight;var stride=convertedWidth*4;var pixels=new byte[stride*convertedHeight];converted.CopyPixels(pixels,stride,0);var bitmap=new SoftwareBitmap(BitmapPixelFormat.Bgra8,convertedWidth,convertedHeight,BitmapAlphaMode.Premultiplied);bitmap.CopyFromBuffer(pixels.AsBuffer());Array.Clear(pixels);return bitmap;
    }

    private static RapidOcr CreatePaddleEngine()
    {
        var model=RapidOcrModelSet.PPOCRv6Small;string Resolve(string path)=>Path.Combine(AppContext.BaseDirectory,path);
        model=model with {DetModelPath=Resolve(model.DetModelPath),ClsModelPath=Resolve(model.ClsModelPath),RecModelPath=Resolve(model.RecModelPath),KeysPath=Resolve(model.KeysPath)};
        var engine=new RapidOcr();engine.InitModels(model);return engine;
    }

    private static SKBitmap CreateSkBitmap(BitmapSource image)
    {
        var encoder=new PngBitmapEncoder();encoder.Frames.Add(System.Windows.Media.Imaging.BitmapFrame.Create(image));using var stream=new MemoryStream();encoder.Save(stream);stream.Position=0;return SKBitmap.Decode(stream)??throw new InvalidOperationException("无法转换截图供 OCR 识别");
    }

    private static Models.OcrLine ToLine(RapidOcrNet.TextBlock block)
    {
        var bounds=Bounds(block.BoxPoints);
        var confidence=block.CharScores is {Length:>0}?block.CharScores.Average():block.BoxScore;
        var words=block.WordResults?.Select(word=>{var b=Bounds(word.BoxPoints);return new Models.OcrWord(word.Text,b.X,b.Y,b.Width,b.Height,word.Score);}).Where(word=>word.Width>0&&word.Height>0).ToList()
            ??[new Models.OcrWord(block.Text,bounds.X,bounds.Y,bounds.Width,bounds.Height,confidence)];
        return new Models.OcrLine(block.Text,bounds.X,bounds.Y,bounds.Width,bounds.Height,words);
    }

    private static (double X,double Y,double Width,double Height) Bounds(IReadOnlyList<SKPointI> points){var left=points.Min(point=>point.X);var top=points.Min(point=>point.Y);var right=points.Max(point=>point.X);var bottom=points.Max(point=>point.Y);return(left,top,right-left,bottom-top);}
}
