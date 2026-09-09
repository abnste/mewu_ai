// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Diagnostics;
using System.Text.Json;
using System.Security.Cryptography;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

/// <summary>Isolates the Windows PDF renderer from the WPF/D3D desktop device.
/// Only bounded pixels cross redirected pipes; no settings, logs or temp media.</summary>
internal static class TeachingPdfProcess
{
    internal const string Argument="--teaching-pdf-worker";
    private sealed record Request(string Path,string Submission,string Pages,int FirstPage);
    internal static async Task<IReadOnlyList<TeachingPage>> ImportAsync(string path,string submission,string pages,int firstPage,CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        var exe=System.IO.Path.ChangeExtension(typeof(App).Assembly.Location,".exe");
        using var process=new Process{StartInfo=new ProcessStartInfo(exe){UseShellExecute=false,CreateNoWindow=true,WindowStyle=ProcessWindowStyle.Hidden,RedirectStandardInput=true,RedirectStandardOutput=true,RedirectStandardError=true}};
        process.StartInfo.ArgumentList.Add(Argument);
        if(!process.Start())throw new IOException("Cannot start the local PDF reader");
        // The helper never writes raw errors. Drain stderr without retaining it.
        var errorDrain=process.StandardError.BaseStream.CopyToAsync(Stream.Null,token);
        try
        {
            var payload=JsonSerializer.SerializeToUtf8Bytes(new Request(path,submission,pages,firstPage));
            if(payload.Length>40_000)throw new InvalidDataException("PDF request too large");
            try{await WriteInt(process.StandardInput.BaseStream,payload.Length,token);await process.StandardInput.BaseStream.WriteAsync(payload,token);process.StandardInput.Close();}
            finally{CryptographicOperations.ZeroMemory(payload);}
            var stream=process.StandardOutput.BaseStream;var count=await ReadInt(stream,token);
            if(count is <1 or >TeachingSession.PageLimit)throw new InvalidDataException(LocalizationService.T("PDF 无法读取，请检查文件、密码及选页范围。","Cannot read this PDF. Check the file, password and page range."));
            var result=new List<TeachingPage>();long pixels=0;
            for(var n=0;n<count;n++)
            {
                var number=await ReadInt(stream,token);var width=await ReadInt(stream,token);var height=await ReadInt(stream,token);
                if(number is <1 or >10000||width is <1 or >1600||height is <1 or >1600||(pixels+=(long)width*height)>TeachingSession.PixelLimit)throw new InvalidDataException("Invalid PDF pixel budget");
                var bytes=new byte[checked(width*height*4)];
                try
                {
                    await stream.ReadExactlyAsync(bytes,token);token.ThrowIfCancellationRequested();
                    var image=BitmapSource.Create(width,height,96,96,PixelFormats.Bgra32,null,bytes,width*4);image.Freeze();
                    result.Add(new("page-"+Guid.NewGuid().ToString("N"),submission,number,image,Convert.ToHexString(SHA256.HashData(bytes))));
                }
                finally{CryptographicOperations.ZeroMemory(bytes);}
            }
            await process.WaitForExitAsync(token);await errorDrain;token.ThrowIfCancellationRequested();
            if(process.ExitCode!=0)throw new IOException(LocalizationService.T("PDF 读取器未正常结束，请重试。","The PDF reader did not exit cleanly. Retry the import."));return result;
        }
        catch(EndOfStreamException){throw new InvalidDataException(LocalizationService.T("PDF 无法完整读取，请检查文件、密码及选页范围。","Cannot read the complete PDF. Check the file, password and page range."));}
        finally
        {
            if(!process.HasExited){try{process.Kill(true);}catch(InvalidOperationException){}await process.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(5));}
            try{await errorDrain;}catch(OperationCanceledException){}
        }
    }
    internal static async Task<int> RunWorkerAsync()
    {
        using var deadline=new CancellationTokenSource(TimeSpan.FromSeconds(55));var token=deadline.Token;
        using var input=Console.OpenStandardInput();using var output=Console.OpenStandardOutput();
        try
        {
            var length=await ReadInt(input,token);if(length is <1 or >40_000)return 1;
            var bytes=new byte[length];Request request;
            try{await input.ReadExactlyAsync(bytes,token);request=JsonSerializer.Deserialize<Request>(bytes)??throw new InvalidDataException();}
            finally{CryptographicOperations.ZeroMemory(bytes);}
            if(string.IsNullOrWhiteSpace(request.Submission)||request.Submission.Length>80||request.Pages.Length>160||request.FirstPage is <1 or >10000)return 1;
            var pages=await TeachingImportService.ImportAsync(request.Path,request.Submission,request.Pages,request.FirstPage,token,nativeWorker:true);
            if(pages.Sum(p=>(long)p.Image.PixelWidth*p.Image.PixelHeight)>TeachingSession.PixelLimit)return 1;
            await WriteInt(output,pages.Count,token);
            foreach(var page in pages)
            {
                await WriteInt(output,page.PageNumber,token);await WriteInt(output,page.Image.PixelWidth,token);await WriteInt(output,page.Image.PixelHeight,token);
                var buffer=new byte[checked(page.Image.PixelWidth*page.Image.PixelHeight*4)];
                try{page.Image.CopyPixels(buffer,page.Image.PixelWidth*4,0);await output.WriteAsync(buffer,token);}
                finally{CryptographicOperations.ZeroMemory(buffer);}
            }
            await output.FlushAsync(token);return 0;
        }
        catch(Exception){return 1;}
    }
    private static async Task<int> ReadInt(Stream stream,CancellationToken token){var buffer=new byte[4];await stream.ReadExactlyAsync(buffer,token);return System.Buffers.Binary.BinaryPrimitives.ReadInt32LittleEndian(buffer);}
    private static async Task WriteInt(Stream stream,int value,CancellationToken token){var buffer=new byte[4];System.Buffers.Binary.BinaryPrimitives.WriteInt32LittleEndian(buffer,value);await stream.WriteAsync(buffer,token);}
}
