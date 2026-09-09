// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Security.Cryptography;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Windows.Data.Pdf;
using Windows.Storage;
using Windows.Storage.Streams;

namespace mewu_ai_Assistant.Services;

internal static class TeachingImportService
{
    internal static IReadOnlyList<int> ParsePages(string text,int total)
    {
        if(total<1)throw new InvalidDataException("Empty document");
        if(string.IsNullOrWhiteSpace(text))return Enumerable.Range(1,Math.Min(total,TeachingSession.PageLimit)).ToArray();
        if(text.Length>160)throw new InvalidDataException("Page range is too long");
        var result=new SortedSet<int>();
        foreach(var part in text.Split(',',StringSplitOptions.TrimEntries))
        {
            var ends=part.Split('-',StringSplitOptions.TrimEntries);
            if(ends.Length is <1 or >2||!int.TryParse(ends[0],out var first)||first<1||first>total)throw new InvalidDataException("Invalid page range");
            var last=first;if(ends.Length==2&&(!int.TryParse(ends[1],out last)||last<first||last>total))throw new InvalidDataException("Invalid page range");
            if(last-first>=TeachingSession.PageLimit)throw new InvalidDataException("Select at most 16 pages");
            for(var n=first;n<=last;n++)result.Add(n);
            if(result.Count>TeachingSession.PageLimit)throw new InvalidDataException("Select at most 16 pages");
        }
        return result.ToArray();
    }
    internal static async Task<IReadOnlyList<TeachingPage>> ImportAsync(string path,string submission,string pages,int firstPage,CancellationToken token,bool nativeWorker=false)
    {
        var info=new FileInfo(path);
        if(!info.Exists||info.Length is <=0 or >52_428_800)throw new InvalidDataException(LocalizationService.T("文件为空或超过 50 MB。","The file is empty or exceeds 50 MB."));
        using var deadline=CancellationTokenSource.CreateLinkedTokenSource(token);deadline.CancelAfter(TimeSpan.FromSeconds(60));token=deadline.Token;
        if(info.Extension.Equals(".pdf",StringComparison.OrdinalIgnoreCase))
        {
            if(!nativeWorker)return await TeachingPdfProcess.ImportAsync(info.FullName,submission,pages,firstPage,token);
            var file=await StorageFile.GetFileFromPathAsync(info.FullName).AsTask(token);
            var pdf=await PdfDocument.LoadFromFileAsync(file).AsTask(token);
            var chosen=ParsePages(pages,checked((int)pdf.PageCount));var output=new List<TeachingPage>();
            foreach(var number in chosen)
            {
                token.ThrowIfCancellationRequested();using var page=pdf.GetPage((uint)(number-1));
                var size=page.Size;var scale=1600/Math.Max(size.Width,size.Height);
                if(!double.IsFinite(scale)||scale<=0)throw new InvalidDataException("Invalid PDF page size");
                using var stream=new InMemoryRandomAccessStream();
                await page.RenderToStreamAsync(stream,new PdfPageRenderOptions{DestinationWidth=(uint)Math.Max(1,size.Width*scale),DestinationHeight=(uint)Math.Max(1,size.Height*scale)}).AsTask(token);
                using var input=stream.AsStreamForRead();output.Add(Create(Decode(input),submission,checked(firstPage+number-1)));
            }
            token.ThrowIfCancellationRequested();return output;
        }
        if(!new[]{".png",".jpg",".jpeg",".bmp",".tif",".tiff",".webp"}.Contains(info.Extension.ToLowerInvariant()))throw new InvalidDataException("Choose PDF, PNG, JPEG, BMP, TIFF or WebP");
        return await Task.Run(()=>{using var input=File.OpenRead(info.FullName);var page=Create(Decode(input),submission,firstPage);token.ThrowIfCancellationRequested();return (IReadOnlyList<TeachingPage>)new[]{page};},token);
    }
    private static BitmapSource Decode(Stream input)
    {
        var metadata=BitmapDecoder.Create(input,BitmapCreateOptions.DelayCreation,BitmapCacheOption.None).Frames[0];
        if((long)metadata.PixelWidth*metadata.PixelHeight>40_000_000)throw new InvalidDataException("Image exceeds 40 million source pixels");
        input.Position=0;var image=new BitmapImage();image.BeginInit();image.CacheOption=BitmapCacheOption.OnLoad;
        if(metadata.PixelWidth>=metadata.PixelHeight)image.DecodePixelWidth=Math.Min(1600,metadata.PixelWidth);else image.DecodePixelHeight=Math.Min(1600,metadata.PixelHeight);
        image.StreamSource=input;image.EndInit();image.Freeze();return image;
    }
    internal static TeachingPage Create(BitmapSource source,string submission,int number)
    {
        var scale=Math.Min(1,1600d/Math.Max(source.PixelWidth,source.PixelHeight));
        if(scale<1){source=new TransformedBitmap(source,new ScaleTransform(scale,scale));source.Freeze();}
        var bgra=new FormatConvertedBitmap(source,PixelFormats.Bgra32,null,0);var bytes=new byte[checked(bgra.PixelWidth*bgra.PixelHeight*4)];
        try
        {
            bgra.CopyPixels(bytes,bgra.PixelWidth*4,0);var hash=Convert.ToHexString(SHA256.HashData(bytes));
            var image=BitmapSource.Create(bgra.PixelWidth,bgra.PixelHeight,96,96,PixelFormats.Bgra32,null,bytes,bgra.PixelWidth*4);image.Freeze();
            return new("page-"+Guid.NewGuid().ToString("N"),submission.Trim(),number,image,hash);
        }
        finally{CryptographicOperations.ZeroMemory(bytes);}
    }
}
