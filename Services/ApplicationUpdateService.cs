using System.Diagnostics;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace mewu_ai_Assistant.Services;

internal sealed record ApplicationUpdatePackage(Version Version,string TagName,string InstallerPath,string Sha256);

internal sealed record ApplicationUpdateResult(Version CurrentVersion,Version LatestVersion,string TagName,ApplicationUpdatePackage? Package)
{
    internal bool IsUpdateAvailable=>Package is not null;
}

internal sealed record ApplicationUpdateProgress(string Message,long BytesReceived=0,long? TotalBytes=null);

internal sealed class ApplicationUpdateService
{
    internal const long MaximumInstallerBytes=512L*1024*1024;
    private const long MaximumMetadataBytes=1024*1024;
    private const long MaximumChecksumBytes=64*1024;
    private const string RepositoryOwner="abnste";
    private const string RepositoryName="mewu_ai";
    private static readonly Uri LatestReleaseUri=new($"https://api.github.com/repos/{RepositoryOwner}/{RepositoryName}/releases/latest");
    private static readonly Uri LatestReleasePageUri=new($"https://github.com/{RepositoryOwner}/{RepositoryName}/releases/latest");
    private static readonly HttpClient SharedClient=CreateHttpClient();
    private static readonly HttpClient LatestRedirectClient=CreateLatestRedirectClient();
    private readonly Func<HttpRequestMessage,HttpCompletionOption,CancellationToken,Task<HttpResponseMessage>> _sendAsync;
    private readonly Func<HttpRequestMessage,CancellationToken,Task<HttpResponseMessage>> _sendLatestRedirectAsync;
    private readonly string _updatesDirectory;

    internal ApplicationUpdateService():this(
        (request,completion,token)=>SharedClient.SendAsync(request,completion,token),
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","Updates")){}

    internal ApplicationUpdateService(
        Func<HttpRequestMessage,HttpCompletionOption,CancellationToken,Task<HttpResponseMessage>> sendAsync,
        string updatesDirectory,
        Func<HttpRequestMessage,CancellationToken,Task<HttpResponseMessage>>? sendLatestRedirectAsync=null)
    {
        _sendAsync=sendAsync??throw new ArgumentNullException(nameof(sendAsync));
        _sendLatestRedirectAsync=sendLatestRedirectAsync??((request,token)=>LatestRedirectClient.SendAsync(request,HttpCompletionOption.ResponseHeadersRead,token));
        _updatesDirectory=Path.GetFullPath(updatesDirectory??throw new ArgumentNullException(nameof(updatesDirectory)));
    }

    internal async Task<ApplicationUpdateResult> CheckAndDownloadAsync(
        Version currentVersion,
        IProgress<ApplicationUpdateProgress>? progress,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(currentVersion);
        progress?.Report(new ApplicationUpdateProgress("正在检查 GitHub 更新…"));
        var release=await GetLatestReleaseAsync(cancellationToken).ConfigureAwait(false);
        var latestVersion=ParseReleaseVersion(release.TagName);
        if(release.Draft||release.Prerelease)throw new InvalidDataException("GitHub 最新版本不是正式 Release");
        if(latestVersion<=currentVersion)
            return new ApplicationUpdateResult(currentVersion,latestVersion,release.TagName,null);

        var versionText=release.TagName[1..];
        var installerName=$"MewuAI-Setup-{versionText}-win-x64.exe";
        const string checksumsName="SHA256SUMS.txt";
        var installer=RequireSingleAsset(release.Assets,installerName,MaximumInstallerBytes);
        var checksums=RequireSingleAsset(release.Assets,checksumsName,MaximumChecksumBytes);
        ValidateReleaseDownloadUri(installer.DownloadUrl,release.TagName,installerName);
        ValidateReleaseDownloadUri(checksums.DownloadUrl,release.TagName,checksumsName);

        progress?.Report(new ApplicationUpdateProgress("正在读取更新校验信息…"));
        var checksumText=await DownloadStringAsync(checksums,MaximumChecksumBytes,cancellationToken).ConfigureAwait(false);
        var expectedHash=ParseExpectedSha256(checksumText,installerName);
        var versionDirectory=Path.Combine(_updatesDirectory,versionText);
        Directory.CreateDirectory(versionDirectory);
        var destination=Path.Combine(versionDirectory,installerName);

        if(File.Exists(destination))
        {
            progress?.Report(new ApplicationUpdateProgress("正在校验已下载的安装包…"));
            var existingHash=await ComputeSha256Async(destination,cancellationToken).ConfigureAwait(false);
            if(string.Equals(existingHash,expectedHash,StringComparison.OrdinalIgnoreCase))
                return new ApplicationUpdateResult(currentVersion,latestVersion,release.TagName,new ApplicationUpdatePackage(latestVersion,release.TagName,destination,expectedHash));
            TryDelete(destination);
        }

            progress?.Report(new ApplicationUpdateProgress("正在下载更新…",0,installer.Size));
        await DownloadInstallerAsync(installer,destination,expectedHash,progress,cancellationToken).ConfigureAwait(false);
        return new ApplicationUpdateResult(currentVersion,latestVersion,release.TagName,new ApplicationUpdatePackage(latestVersion,release.TagName,destination,expectedHash));
    }

    internal async Task LaunchInstallerAsync(ApplicationUpdatePackage package,CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(package);
        var installer=Path.GetFullPath(package.InstallerPath);
        var rootWithSeparator=Path.TrimEndingDirectorySeparator(_updatesDirectory)+Path.DirectorySeparatorChar;
        if(!installer.StartsWith(rootWithSeparator,StringComparison.OrdinalIgnoreCase)||!File.Exists(installer))
            throw new InvalidOperationException("更新安装包路径无效，请重新检查更新");
        var expectedName=$"MewuAI-Setup-{package.TagName[1..]}-win-x64.exe";
        if(!string.Equals(Path.GetFileName(installer),expectedName,StringComparison.Ordinal))
            throw new InvalidOperationException("更新安装包名称无效，请重新检查更新");
        var actualHash=await ComputeSha256Async(installer,cancellationToken).ConfigureAwait(false);
        if(!string.Equals(actualHash,package.Sha256,StringComparison.OrdinalIgnoreCase))
        {
            TryDelete(installer);
            throw new InvalidDataException("更新安装包校验失败，已删除损坏文件，请重试");
        }

        var startInfo=new ProcessStartInfo(installer)
        {
            UseShellExecute=false,
            WorkingDirectory=Path.GetDirectoryName(installer),
            CreateNoWindow=true
        };
        foreach(var argument in new[]{"/VERYSILENT","/SUPPRESSMSGBOXES","/NORESTART","/CLOSEAPPLICATIONS","/NOCANCEL","/SP-"})
            startInfo.ArgumentList.Add(argument);
        if(Process.Start(startInfo) is null)throw new InvalidOperationException("无法启动更新安装程序");
    }

    private async Task<GitHubRelease> GetLatestReleaseAsync(CancellationToken cancellationToken)
    {
        try
        {
            using var request=CreateRequest(LatestReleaseUri,"application/vnd.github+json");
            using var response=await SendAsync(request,TimeSpan.FromSeconds(30),cancellationToken).ConfigureAwait(false);
            EnsureSuccess(response,"检查 GitHub 更新");
            var json=await ReadBoundedStringAsync(response.Content,MaximumMetadataBytes,cancellationToken).ConfigureAwait(false);
            var release=JsonSerializer.Deserialize<GitHubRelease>(json,JsonOptions)??throw new InvalidDataException("GitHub Release 响应为空");
            if(string.IsNullOrWhiteSpace(release.TagName)||release.Assets is null)throw new InvalidDataException("GitHub Release 响应缺少版本或附件");
            return release;
        }
        catch(HttpRequestException exception) when(exception.StatusCode is HttpStatusCode.Forbidden or HttpStatusCode.TooManyRequests)
        {
            // Shared networks can exhaust GitHub's unauthenticated REST quota.
            // The official latest-release redirect has a distinct limit and
            // still lets us constrain the tag and assets to this repository.
            return await GetLatestReleaseFromRedirectAsync(cancellationToken).ConfigureAwait(false);
        }
    }

    private async Task<GitHubRelease> GetLatestReleaseFromRedirectAsync(CancellationToken cancellationToken)
    {
        using var request=CreateRequest(LatestReleasePageUri,"text/html");
        using var response=await SendLatestRedirectAsync(request,TimeSpan.FromSeconds(30),cancellationToken).ConfigureAwait(false);
        if(response.StatusCode is not (HttpStatusCode.MovedPermanently or HttpStatusCode.Found or HttpStatusCode.SeeOther or HttpStatusCode.TemporaryRedirect or HttpStatusCode.PermanentRedirect))
            throw new HttpRequestException($"检查 GitHub 更新失败：HTTP {(int)response.StatusCode}",null,response.StatusCode);
        var target=response.Headers.Location;
        if(target is null)throw new InvalidDataException("GitHub 最新版本跳转缺少目标地址");
        if(!target.IsAbsoluteUri)target=new Uri(LatestReleasePageUri,target);
        if(target.Scheme!=Uri.UriSchemeHttps||!string.Equals(target.Host,"github.com",StringComparison.OrdinalIgnoreCase)||
           !target.AbsolutePath.StartsWith($"/{RepositoryOwner}/{RepositoryName}/releases/tag/",StringComparison.Ordinal)||
           !string.IsNullOrEmpty(target.Query)||!string.IsNullOrEmpty(target.Fragment))
            throw new InvalidDataException("GitHub 最新版本跳转地址无效");
        var tagName=target.AbsolutePath[(target.AbsolutePath.LastIndexOf('/')+1)..];
        ParseReleaseVersion(tagName);
        var versionText=tagName[1..];
        return new GitHubRelease(tagName,false,false,
        [
            new GitHubAsset($"MewuAI-Setup-{versionText}-win-x64.exe",null,$"https://github.com/{RepositoryOwner}/{RepositoryName}/releases/download/{tagName}/MewuAI-Setup-{versionText}-win-x64.exe"),
            new GitHubAsset("SHA256SUMS.txt",null,$"https://github.com/{RepositoryOwner}/{RepositoryName}/releases/download/{tagName}/SHA256SUMS.txt")
        ]);
    }

    private async Task<string> DownloadStringAsync(GitHubAsset asset,long maximumBytes,CancellationToken cancellationToken)
    {
        using var request=CreateRequest(new Uri(asset.DownloadUrl,UriKind.Absolute),"application/octet-stream");
        using var response=await SendAsync(request,TimeSpan.FromMinutes(2),cancellationToken).ConfigureAwait(false);
        EnsureSuccess(response,"下载更新校验信息");
        ValidateFinalDownloadHost(response.RequestMessage?.RequestUri);
        return await ReadBoundedStringAsync(response.Content,maximumBytes,cancellationToken).ConfigureAwait(false);
    }

    private async Task DownloadInstallerAsync(
        GitHubAsset asset,
        string destination,
        string expectedHash,
        IProgress<ApplicationUpdateProgress>? progress,
        CancellationToken cancellationToken)
    {
        var temporary=Path.Combine(Path.GetDirectoryName(destination)!,"."+Path.GetFileName(destination)+"."+Guid.NewGuid().ToString("N")+".download");
        try
        {
            using var request=CreateRequest(new Uri(asset.DownloadUrl,UriKind.Absolute),"application/octet-stream");
            using var response=await SendAsync(request,TimeSpan.FromMinutes(20),cancellationToken).ConfigureAwait(false);
            EnsureSuccess(response,"下载更新安装包");
            ValidateFinalDownloadHost(response.RequestMessage?.RequestUri);
            var declaredLength=response.Content.Headers.ContentLength;
            if(declaredLength is <=0 or >MaximumInstallerBytes)throw new InvalidDataException("更新安装包大小无效");
            if(asset.Size is >0&&declaredLength!=asset.Size)throw new InvalidDataException("更新安装包大小与 GitHub Release 不一致");

            string actualHash;
            await using(var input=await response.Content.ReadAsStreamAsync(cancellationToken).ConfigureAwait(false))
            await using(var output=new FileStream(temporary,FileMode.CreateNew,FileAccess.Write,FileShare.None,128*1024,FileOptions.Asynchronous|FileOptions.SequentialScan))
            using(var hash=IncrementalHash.CreateHash(HashAlgorithmName.SHA256))
            {
                var buffer=new byte[128*1024];
                long received=0;
                while(true)
                {
                    var count=await input.ReadAsync(buffer,cancellationToken).ConfigureAwait(false);
                    if(count==0)break;
                    received=checked(received+count);
                    if(received>MaximumInstallerBytes||(asset.Size is >0&&received>asset.Size))throw new InvalidDataException("更新安装包超过 Release 声明大小");
                    hash.AppendData(buffer,0,count);
                    await output.WriteAsync(buffer.AsMemory(0,count),cancellationToken).ConfigureAwait(false);
                    progress?.Report(new ApplicationUpdateProgress("正在下载更新…",received,asset.Size));
                }
                if(asset.Size is >0&&received!=asset.Size)throw new InvalidDataException("更新安装包下载不完整");
                await output.FlushAsync(cancellationToken).ConfigureAwait(false);
                output.Flush(true);
                actualHash=Convert.ToHexString(hash.GetHashAndReset()).ToLowerInvariant();
            }
            if(!string.Equals(actualHash,expectedHash,StringComparison.OrdinalIgnoreCase))throw new InvalidDataException("更新安装包 SHA-256 校验失败");
            File.Move(temporary,destination,true);
        }
        finally{TryDelete(temporary);}
    }

    private async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,TimeSpan timeout,CancellationToken cancellationToken)
    {
        using var timeoutSource=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeoutSource.CancelAfter(timeout);
        try
        {
            var response=await _sendAsync(request,HttpCompletionOption.ResponseHeadersRead,timeoutSource.Token).ConfigureAwait(false);
            response.RequestMessage??=request;
            return response;
        }
        catch(OperationCanceledException) when(!cancellationToken.IsCancellationRequested)
        {
            throw new TimeoutException("连接 GitHub 更新服务超时");
        }
    }

    private async Task<HttpResponseMessage> SendLatestRedirectAsync(HttpRequestMessage request,TimeSpan timeout,CancellationToken cancellationToken)
    {
        using var timeoutSource=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeoutSource.CancelAfter(timeout);
        try
        {
            var response=await _sendLatestRedirectAsync(request,timeoutSource.Token).ConfigureAwait(false);
            response.RequestMessage??=request;
            return response;
        }
        catch(OperationCanceledException) when(!cancellationToken.IsCancellationRequested)
        {
            throw new TimeoutException("连接 GitHub 更新服务超时");
        }
    }

    private static HttpRequestMessage CreateRequest(Uri uri,string accept)
    {
        var request=new HttpRequestMessage(HttpMethod.Get,uri);
        request.Headers.UserAgent.ParseAdd("MewuAI-Update/0.1");
        request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue(accept));
        request.Headers.TryAddWithoutValidation("X-GitHub-Api-Version","2022-11-28");
        return request;
    }

    private static GitHubAsset RequireSingleAsset(IReadOnlyList<GitHubAsset> assets,string expectedName,long maximumBytes)
    {
        var matches=assets.Where(asset=>string.Equals(asset.Name,expectedName,StringComparison.Ordinal)).ToArray();
        if(matches.Length!=1)throw new InvalidDataException($"GitHub Release 缺少唯一的 {expectedName}");
        var asset=matches[0];
        if((asset.Size is { } size&&(size<=0||size>maximumBytes))||string.IsNullOrWhiteSpace(asset.DownloadUrl))throw new InvalidDataException($"GitHub Release 中 {expectedName} 的信息无效");
        return asset;
    }

    private static Version ParseReleaseVersion(string tagName)
    {
        if(tagName.Length<2||tagName[0]!='v'||tagName.Count(character=>character=='.') is <2 or >3||
           tagName[1..].Any(character=>!char.IsAsciiDigit(character)&&character!='.')||
           !Version.TryParse(tagName[1..],out var version))
            throw new InvalidDataException("GitHub Release 标签不是受支持的 v主版本.次版本.修订版本 格式");
        return version;
    }

    private static void ValidateReleaseDownloadUri(string value,string tagName,string fileName)
    {
        if(!Uri.TryCreate(value,UriKind.Absolute,out var uri)||uri.Scheme!=Uri.UriSchemeHttps||
           !string.Equals(uri.Host,"github.com",StringComparison.OrdinalIgnoreCase)||
           !string.Equals(uri.AbsolutePath,$"/{RepositoryOwner}/{RepositoryName}/releases/download/{tagName}/{fileName}",StringComparison.Ordinal))
            throw new InvalidDataException("GitHub Release 附件地址无效");
    }

    private static void ValidateFinalDownloadHost(Uri? uri)
    {
        if(uri is null||uri.Scheme!=Uri.UriSchemeHttps||
           !(string.Equals(uri.Host,"github.com",StringComparison.OrdinalIgnoreCase)||
             string.Equals(uri.Host,"objects.githubusercontent.com",StringComparison.OrdinalIgnoreCase)||
             string.Equals(uri.Host,"release-assets.githubusercontent.com",StringComparison.OrdinalIgnoreCase)))
            throw new InvalidDataException("更新下载被重定向到不受信任的地址");
    }

    internal static string ParseExpectedSha256(string contents,string expectedFileName)
    {
        var matches=new List<string>();
        using var reader=new StringReader(contents??string.Empty);
        while(reader.ReadLine() is { } line)
        {
            var separator=line.IndexOf("  ",StringComparison.Ordinal);
            if(separator!=64||line.Length<=separator+2)continue;
            var hash=line[..separator];
            var name=line[(separator+2)..].Trim();
            if(string.Equals(name,expectedFileName,StringComparison.Ordinal)&&hash.All(Uri.IsHexDigit))matches.Add(hash.ToLowerInvariant());
        }
        if(matches.Count!=1)throw new InvalidDataException($"SHA256SUMS.txt 缺少唯一的 {expectedFileName} 校验值");
        return matches[0];
    }

    private static async Task<string> ReadBoundedStringAsync(HttpContent content,long maximumBytes,CancellationToken cancellationToken)
    {
        if(content.Headers.ContentLength is >0 and var contentLength&&contentLength>maximumBytes)throw new InvalidDataException("更新响应超过允许大小");
        await using var input=await content.ReadAsStreamAsync(cancellationToken).ConfigureAwait(false);
        using var memory=new MemoryStream();
        var buffer=new byte[16*1024];
        while(true)
        {
            var count=await input.ReadAsync(buffer,cancellationToken).ConfigureAwait(false);
            if(count==0)break;
            if(memory.Length+count>maximumBytes)throw new InvalidDataException("更新响应超过允许大小");
            await memory.WriteAsync(buffer.AsMemory(0,count),cancellationToken).ConfigureAwait(false);
        }
        return Encoding.UTF8.GetString(memory.ToArray());
    }

    private static async Task<string> ComputeSha256Async(string path,CancellationToken cancellationToken)
    {
        await using var stream=new FileStream(path,FileMode.Open,FileAccess.Read,FileShare.Read,128*1024,FileOptions.Asynchronous|FileOptions.SequentialScan);
        using var algorithm=SHA256.Create();
        var hash=await algorithm.ComputeHashAsync(stream,cancellationToken).ConfigureAwait(false);
        return Convert.ToHexString(hash).ToLowerInvariant();
    }

    private static void EnsureSuccess(HttpResponseMessage response,string operation)
    {
        if(response.IsSuccessStatusCode)return;
        throw new HttpRequestException($"{operation}失败：HTTP {(int)response.StatusCode}",null,response.StatusCode);
    }

    private static void TryDelete(string path){try{if(File.Exists(path))File.Delete(path);}catch{}}

    private static HttpClient CreateHttpClient()=>new(new SocketsHttpHandler
    {
        AutomaticDecompression=DecompressionMethods.All,
        AllowAutoRedirect=true,
        MaxAutomaticRedirections=5,
        PooledConnectionLifetime=TimeSpan.FromMinutes(10)
    }){Timeout=Timeout.InfiniteTimeSpan};

    private static HttpClient CreateLatestRedirectClient()=>new(new SocketsHttpHandler
    {
        AutomaticDecompression=DecompressionMethods.All,
        AllowAutoRedirect=false,
        PooledConnectionLifetime=TimeSpan.FromMinutes(10)
    }){Timeout=Timeout.InfiniteTimeSpan};

    private static readonly JsonSerializerOptions JsonOptions=new(){PropertyNameCaseInsensitive=true};

    private sealed record GitHubRelease(
        [property:JsonPropertyName("tag_name")] string TagName,
        [property:JsonPropertyName("draft")] bool Draft,
        [property:JsonPropertyName("prerelease")] bool Prerelease,
        [property:JsonPropertyName("assets")] IReadOnlyList<GitHubAsset> Assets);

    private sealed record GitHubAsset(
        [property:JsonPropertyName("name")] string Name,
        [property:JsonPropertyName("size")] long? Size,
        [property:JsonPropertyName("browser_download_url")] string DownloadUrl);
}
