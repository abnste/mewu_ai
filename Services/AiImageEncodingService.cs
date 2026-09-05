using System.Security.Cryptography;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal sealed record EncodedAiImage(byte[] Data,string MimeType,int PixelWidth,int PixelHeight);

internal static class AiImageEncodingService
{
    private const int MaxResizeAttempts=8;
    private static readonly int[] JpegQualities=[92,84,74,62];

    internal static EncodedAiImage Encode(BitmapSource image,long maxBytes,IReadOnlySet<string> acceptedMimeTypes,CancellationToken token)
    {
        ArgumentNullException.ThrowIfNull(image);ArgumentNullException.ThrowIfNull(acceptedMimeTypes);
        if(maxBytes<=0)throw new InvalidOperationException("当前 Provider 没有可用的图片大小额度");
        var acceptsPng=acceptedMimeTypes.Count==0||acceptedMimeTypes.Contains("image/png");var acceptsJpeg=acceptedMimeTypes.Count==0||acceptedMimeTypes.Contains("image/jpeg");
        if(!acceptsPng&&!acceptsJpeg)throw new NotSupportedException("当前 Provider 不接受 PNG 或 JPEG 屏幕图片");
        token.ThrowIfCancellationRequested();
        if(acceptsPng)
        {
            byte[]? png=Encode(image,jpegQuality:null);
            try
            {
                token.ThrowIfCancellationRequested();
                if(png.LongLength<=maxBytes){var result=png;png=null;return new EncodedAiImage(result,"image/png",image.PixelWidth,image.PixelHeight);}
            }
            finally{if(png is not null)CryptographicOperations.ZeroMemory(png);}
        }

        var minScale=Math.Min(1,960d/Math.Max(image.PixelWidth,image.PixelHeight));var scale=1d;long smallest=long.MaxValue;
        for(var attempt=0;attempt<MaxResizeAttempts;attempt++)
        {
            token.ThrowIfCancellationRequested();var candidate=scale>=.999?image:Resize(image,scale);byte[]? last=null;
            try
            {
                if(acceptsJpeg)foreach(var quality in JpegQualities)
                {
                    token.ThrowIfCancellationRequested();
                    last=Encode(candidate,quality);
                    token.ThrowIfCancellationRequested();
                    smallest=Math.Min(smallest,last.LongLength);
                    if(last.LongLength<=maxBytes)
                    {
                        // Transfer ownership to the caller before leaving the
                        // try block; otherwise the cleanup finally would wipe
                        // the successful payload along with failed candidates.
                        var result=last;last=null;
                        return new EncodedAiImage(result,"image/jpeg",candidate.PixelWidth,candidate.PixelHeight);
                    }
                    CryptographicOperations.ZeroMemory(last);last=null;
                }
                if(acceptsPng&&scale<.999)
                {
                    last=Encode(candidate,jpegQuality:null);
                    token.ThrowIfCancellationRequested();
                    smallest=Math.Min(smallest,last.LongLength);
                    if(last.LongLength<=maxBytes)
                    {
                        var result=last;last=null;
                        return new EncodedAiImage(result,"image/png",candidate.PixelWidth,candidate.PixelHeight);
                    }
                    CryptographicOperations.ZeroMemory(last);last=null;
                }
            }
            finally{if(last is not null)CryptographicOperations.ZeroMemory(last);}
            if(scale<=minScale+.001)break;var estimated=smallest==long.MaxValue?.72:Math.Clamp(Math.Sqrt(maxBytes/(double)Math.Max(1,smallest))*.92,.48,.82);scale=Math.Max(minScale,scale*estimated);
        }
        throw new InvalidOperationException($"当前截图即使有界压缩后仍超过 {maxBytes/(1024d*1024d):0.##} MB，请缩小选区或减少引用区域");
    }

    internal static void ClearAttachmentBuffers(IEnumerable<AiAttachment> attachments)
    {
        ArgumentNullException.ThrowIfNull(attachments);
        foreach(var attachment in attachments)
            if(attachment.ProviderOwnsData&&attachment.Data is { } data)
                CryptographicOperations.ZeroMemory(data);
    }

    private static byte[] Encode(BitmapSource image,int? jpegQuality)
    {
        BitmapEncoder encoder=jpegQuality is { } quality?new JpegBitmapEncoder{QualityLevel=quality}:new PngBitmapEncoder();
        encoder.Frames.Add(BitmapFrame.Create(image));
        using var stream=new MemoryStream();
        try{encoder.Save(stream);return stream.ToArray();}
        finally
        {
            // ToArray transfers a copy; MemoryStream.Dispose does not erase
            // the original encoded screen content retained by its buffer.
            if(stream.TryGetBuffer(out var buffer))CryptographicOperations.ZeroMemory(buffer.AsSpan());
        }
    }
    private static BitmapSource Resize(BitmapSource source,double scale){var transform=new TransformedBitmap(source,new ScaleTransform(scale,scale));transform.Freeze();return transform;}
}
