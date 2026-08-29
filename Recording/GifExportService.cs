using System.Windows.Media.Imaging;
namespace mewu_ai_Assistant.Recording;
public static class GifExportService
{
    public static void Export(string framesDirectory,string outputPath,int fps)
    {
        var encoder=new GifBitmapEncoder();var delay=Math.Max(2,100/Math.Clamp(fps,1,15));foreach(var file in Directory.EnumerateFiles(framesDirectory,"*.png").OrderBy(x=>x)){using var stream=File.OpenRead(file);var decoder=new PngBitmapDecoder(stream,BitmapCreateOptions.PreservePixelFormat,BitmapCacheOption.OnLoad);var frame=decoder.Frames[0];var metadata=new BitmapMetadata("gif");metadata.SetQuery("/grctlext/Delay",(ushort)delay);metadata.SetQuery("/grctlext/Disposal",(byte)2);encoder.Frames.Add(BitmapFrame.Create(frame,null,metadata,null));}if(encoder.Frames.Count==0)throw new InvalidOperationException("没有可导出的录屏帧");using(var output=File.Create(outputPath))encoder.Save(output);ApplyFrameDelay(outputPath,(ushort)delay);
    }

    // .NET 10 WPF currently emits a Graphic Control Extension with a zero delay even when
    // BitmapMetadata contains /grctlext/Delay. Walk the GIF block structure and correct it.
    private static void ApplyFrameDelay(string path,ushort delay)
    {
        using var stream=new FileStream(path,FileMode.Open,FileAccess.ReadWrite,FileShare.None);using var reader=new BinaryReader(stream,System.Text.Encoding.ASCII,true);if(new string(reader.ReadChars(6)) is not ("GIF87a" or "GIF89a"))throw new InvalidDataException("GIF 文件头无效");var descriptor=reader.ReadBytes(7);if(descriptor.Length!=7)throw new EndOfStreamException();if((descriptor[4]&0x80)!=0)stream.Position+=3*(1<<((descriptor[4]&7)+1));var updated=0;while(stream.Position<stream.Length){var marker=reader.ReadByte();if(marker==0x3B)break;if(marker==0x21){var label=reader.ReadByte();if(label==0xF9){if(reader.ReadByte()!=4)throw new InvalidDataException("GIF 图形控制扩展长度无效");reader.ReadByte();stream.WriteByte((byte)(delay&0xFF));stream.WriteByte((byte)(delay>>8));reader.ReadByte();if(reader.ReadByte()!=0)throw new InvalidDataException("GIF 图形控制扩展终止符无效");updated++;}else SkipSubBlocks(reader);}else if(marker==0x2C){var imageDescriptor=reader.ReadBytes(9);if(imageDescriptor.Length!=9)throw new EndOfStreamException();if((imageDescriptor[8]&0x80)!=0)stream.Position+=3*(1<<((imageDescriptor[8]&7)+1));reader.ReadByte();SkipSubBlocks(reader);}else throw new InvalidDataException($"未知 GIF 块标记 0x{marker:X2}");}if(updated==0)throw new InvalidDataException("GIF 中没有可设置时长的帧");
    }
    private static void SkipSubBlocks(BinaryReader reader){while(true){var size=reader.ReadByte();if(size==0)return;if(reader.ReadBytes(size).Length!=size)throw new EndOfStreamException();}}
}
