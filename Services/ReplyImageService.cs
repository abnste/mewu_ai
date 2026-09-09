// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net;
using System.Net.Http;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

internal static class ReplyImageService
{
    internal const int MaximumBytes=8*1024*1024;
    private static readonly HttpClient Client=new(new HttpClientHandler{AllowAutoRedirect=false,UseCookies=false,UseDefaultCredentials=false}){Timeout=Timeout.InfiniteTimeSpan};
    private static readonly SemaphoreSlim Downloads=new(2);

    internal static bool TryGetWebUri(string? value,out Uri uri)
    {
        uri=null!;
        if(!Uri.TryCreate(value,UriKind.Absolute,out var candidate)||candidate.Scheme is not ("https" or "http")||
            !candidate.IsDefaultPort||candidate.UserInfo.Length>0||candidate.IsLoopback||candidate.HostNameType!=UriHostNameType.Dns||
            !candidate.DnsSafeHost.Contains('.')||candidate.DnsSafeHost.EndsWith(".local",StringComparison.OrdinalIgnoreCase)||
            candidate.DnsSafeHost.EndsWith(".localhost",StringComparison.OrdinalIgnoreCase))return false;
        uri=candidate;return true;
    }

    internal static bool TryGetLocalPath(string? source,out string path)
    {
        path=string.Empty;
        if(string.IsNullOrWhiteSpace(source)||source.Length>32767)return false;
        var value=source;
        if(Uri.TryCreate(value,UriKind.Absolute,out var uri)&&uri.IsFile)
        {
            if(uri.IsUnc||uri.Host.Length>0||uri.Query.Length>0||uri.Fragment.Length>0)return false;
            value=uri.LocalPath;
        }
        // Drive-rooted local files only: no UNC/device paths, ADS or network drives.
        if(value.Length<4||!char.IsAsciiLetter(value[0])||value[1]!=':'||value[2] is not ('/' or '\\')||value[2..].Contains(':'))return false;
        try
        {
            value=Path.GetFullPath(value);
            if(new DriveInfo(Path.GetPathRoot(value)!).DriveType==DriveType.Network)return false;
            if(Path.GetExtension(value).ToLowerInvariant() is not (".png" or ".jpg" or ".jpeg" or ".gif" or ".bmp" or ".webp" or ".tif" or ".tiff"))return false;
            path=value;return true;
        }
        catch(Exception ex)when(ex is ArgumentException or IOException or NotSupportedException or System.Security.SecurityException){return false;}
    }

    internal static async Task<BitmapSource> LoadAsync(string source,CancellationToken token,bool allowLocal=false)
    {
        using var deadline=CancellationTokenSource.CreateLinkedTokenSource(token);deadline.CancelAfter(TimeSpan.FromSeconds(20));
        await Downloads.WaitAsync(deadline.Token).ConfigureAwait(false);
        byte[]? bytes=null;
        try
        {
            if(allowLocal&&TryGetLocalPath(source,out var localPath))
            {
                await using var file=new FileStream(localPath,FileMode.Open,FileAccess.Read,FileShare.Read|FileShare.Delete,32*1024,FileOptions.Asynchronous|FileOptions.SequentialScan);
                if(file.Length>MaximumBytes)throw new InvalidDataException();
                bytes=await ReadBoundedAsync(file,deadline.Token).ConfigureAwait(false);
            }
            else if(source.StartsWith("data:image/",StringComparison.OrdinalIgnoreCase))
            {
                var comma=source.IndexOf(',');
                if(comma<0||comma>40||!source[..comma].EndsWith(";base64",StringComparison.OrdinalIgnoreCase)||source.Length-comma-1>(MaximumBytes+2L)/3*4)throw new InvalidDataException();
                bytes=Convert.FromBase64String(source[(comma+1)..]);
            }
            else
            {
                if(!TryGetWebUri(source,out var uri))throw new InvalidDataException();
                for(var redirect=0;redirect<4;redirect++)
                {
                    using var response=await Client.GetAsync(uri,HttpCompletionOption.ResponseHeadersRead,deadline.Token).ConfigureAwait(false);
                    if(response.StatusCode is HttpStatusCode.MovedPermanently or HttpStatusCode.Redirect or HttpStatusCode.SeeOther or HttpStatusCode.TemporaryRedirect or HttpStatusCode.PermanentRedirect)
                    {
                        var location=response.Headers.Location;
                        if(location is null||!TryGetWebUri(new Uri(uri,location).AbsoluteUri,out uri))throw new InvalidDataException();
                        continue;
                    }
                    response.EnsureSuccessStatusCode();
                    if(response.Content.Headers.ContentLength>MaximumBytes)throw new InvalidDataException();
                    await using var stream=await response.Content.ReadAsStreamAsync(deadline.Token).ConfigureAwait(false);
                    bytes=await ReadBoundedAsync(stream,deadline.Token).ConfigureAwait(false);break;
                }
            }
            if(bytes is null||bytes.Length==0||bytes.Length>MaximumBytes)throw new InvalidDataException();
            deadline.Token.ThrowIfCancellationRequested();
            var image=await Task.Run(()=>Decode(bytes),deadline.Token).ConfigureAwait(false);
            deadline.Token.ThrowIfCancellationRequested();
            return image;
        }
        finally{if(bytes is not null)Array.Clear(bytes);Downloads.Release();}
    }

    internal static async Task<byte[]> ReadBoundedAsync(Stream stream,CancellationToken token)
    {
        using var buffer=new MemoryStream();var block=new byte[32*1024];
        try
        {
            int count;while((count=await stream.ReadAsync(block,token).ConfigureAwait(false))>0)
            {
                if(buffer.Length+count>MaximumBytes)throw new InvalidDataException();
                buffer.Write(block,0,count);
            }
            token.ThrowIfCancellationRequested();return buffer.ToArray();
        }
        finally{Array.Clear(block);if(buffer.TryGetBuffer(out var data))Array.Clear(data.Array!);}
    }

    internal static BitmapSource Decode(byte[] bytes)
    {
        using var stream=new MemoryStream(bytes,false);
        var decoder=BitmapDecoder.Create(stream,BitmapCreateOptions.DelayCreation,BitmapCacheOption.None);
        var frame=decoder.Frames[0];var width=frame.PixelWidth;var height=frame.PixelHeight;
        if(width<1||height<1||(long)width*height>40_000_000)throw new InvalidDataException();
        var scale=Math.Min(1,1024d/Math.Max(width,height));stream.Position=0;
        var image=new BitmapImage();image.BeginInit();image.CacheOption=BitmapCacheOption.OnLoad;
        image.DecodePixelWidth=Math.Max(1,(int)(width*scale));image.DecodePixelHeight=Math.Max(1,(int)(height*scale));
        image.StreamSource=stream;image.EndInit();image.Freeze();return image;
    }
}
