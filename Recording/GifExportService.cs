using System.Windows.Media.Imaging;
namespace mewu_ai_Assistant.Recording;
public static class GifExportService
{
    public static void Export(string framesDirectory,string outputPath,int fps)
    {
        var encoder=new GifBitmapEncoder();var delay=Math.Max(2,100/Math.Clamp(fps,1,15));foreach(var file in Directory.EnumerateFiles(framesDirectory,"*.png").OrderBy(x=>x)){using var stream=File.OpenRead(file);var decoder=new PngBitmapDecoder(stream,BitmapCreateOptions.PreservePixelFormat,BitmapCacheOption.OnLoad);var frame=decoder.Frames[0];var metadata=new BitmapMetadata("gif");metadata.SetQuery("/grctlext/Delay",(ushort)delay);metadata.SetQuery("/grctlext/Disposal",(byte)2);encoder.Frames.Add(BitmapFrame.Create(frame,frame.Thumbnail,metadata,frame.ColorContexts));}if(encoder.Frames.Count==0)throw new InvalidOperationException("没有可导出的录屏帧");using var output=File.Create(outputPath);encoder.Save(output);
    }
}
