using System.Net;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ApplicationUpdateServiceTests
{
    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task GitHubDigestVerifiesInstallerWithoutReadingChecksumAsset(bool includeChecksums)
    {
        var root=TestDirectory();
        try
        {
            var installer=Encoding.UTF8.GetBytes("GitHub digest installer");
            var hash=Convert.ToHexString(SHA256.HashData(installer));var requests=new List<string>();
            var responses=new Queue<HttpResponseMessage>([
                JsonResponse(ReleaseJson("v0.1.1",installer.Length,"sha256:"+hash,includeChecksums)),
                BytesResponse(installer)]);
            var accepted=false;
            var service=new ApplicationUpdateService((request,_,_)=>
            {
                if(requests.Count>0)Assert.True(accepted);
                requests.Add(request.RequestUri!.AbsoluteUri);return Task.FromResult(responses.Dequeue());
            },root);
            var result=await service.CheckAndDownloadAsync(new Version(0,1,0),null,TestContext.Current.CancellationToken,
                (_,_,_)=>{accepted=true;return Task.FromResult(true);});
            Assert.True(result.IsUpdateAvailable);
            Assert.Equal(hash.ToLowerInvariant(),result.Package!.Sha256);
            Assert.Equal(installer,await File.ReadAllBytesAsync(result.Package.InstallerPath,TestContext.Current.CancellationToken));
            Assert.Equal(2,requests.Count);
            Assert.DoesNotContain(requests,url=>url.EndsWith("SHA256SUMS.txt",StringComparison.Ordinal));
            Assert.Empty(responses);
        }
        finally{Directory.Delete(root,true);}
    }

    [Theory]
    [InlineData("",64,'a')]
    [InlineData(" ",64,'a')]
    [InlineData("sha512:",64,'a')]
    [InlineData("SHA256:",64,'a')]
    [InlineData("sha256:",64,'g')]
    [InlineData("sha256:",63,'a')]
    [InlineData("sha256:",65,'a')]
    [InlineData("sha256:",64,'ａ')]
    public async Task InvalidDigestNeverFallsBackOrPrompts(string prefix,int length,char character)
    {
        var root=TestDirectory();
        try
        {
            var requests=0;var digest=prefix+new string(character,length);
            var service=new ApplicationUpdateService((_,_,_)=>
            {
                requests++;return Task.FromResult(JsonResponse(ReleaseJson("v0.1.1",20,digest)));
            },root);
            await Assert.ThrowsAsync<InvalidDataException>(()=>service.CheckAndDownloadAsync(new Version(0,1,0),null,TestContext.Current.CancellationToken,
                (_,_,_)=>throw new InvalidOperationException("Invalid digest must fail before confirmation.")));
            Assert.Equal(1,requests);Assert.Empty(Directory.EnumerateFileSystemEntries(root));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task DigestMismatchNeverPublishesInstallerOrUsesLegacyHash()
    {
        var root=TestDirectory();
        try
        {
            var installer=Encoding.UTF8.GetBytes("tampered payload");
            var responses=new Queue<HttpResponseMessage>([
                JsonResponse(ReleaseJson("v0.1.1",installer.Length,"sha256:"+new string('0',64))),
                BytesResponse(installer)]);
            var service=new ApplicationUpdateService((_,_,_)=>Task.FromResult(responses.Dequeue()),root);
            await Assert.ThrowsAsync<InvalidDataException>(()=>service.CheckAndDownloadAsync(new Version(0,1,0),null,TestContext.Current.CancellationToken));
            Assert.Empty(responses);
            Assert.Empty(Directory.EnumerateFiles(root,"*",SearchOption.AllDirectories));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task MissingDigestAndLegacyChecksumNeverDownloadsUnverifiedInstaller()
    {
        var root=TestDirectory();
        try
        {
            var requests=0;
            var service=new ApplicationUpdateService((_,_,_)=>
            {
                requests++;return Task.FromResult(JsonResponse(ReleaseJson("v0.1.1",20,null,false)));
            },root);
            await Assert.ThrowsAsync<InvalidDataException>(()=>service.CheckAndDownloadAsync(new Version(0,1,0),null,TestContext.Current.CancellationToken));
            Assert.Equal(1,requests);Assert.Empty(Directory.EnumerateFileSystemEntries(root));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task RateLimitedNewReleaseStillSupportsLegacyChecksumVerification()
    {
        var root=TestDirectory();
        try
        {
            var installer=Encoding.UTF8.GetBytes("legacy fallback installer");
            var hash=Convert.ToHexString(SHA256.HashData(installer)).ToLowerInvariant();
            var responses=new Queue<HttpResponseMessage>([
                new(HttpStatusCode.TooManyRequests),
                BytesResponse(Encoding.UTF8.GetBytes($"{hash}  MewuAI-Setup-0.1.1-win-x64.exe\n")),
                BytesResponse(installer)]);
            var service=new ApplicationUpdateService((_,_,_)=>Task.FromResult(responses.Dequeue()),root,
                (_,_)=>Task.FromResult(new HttpResponseMessage(HttpStatusCode.Found){Headers={Location=new Uri("https://github.com/abnste/mewu_ai/releases/tag/v0.1.1")}}));
            var result=await service.CheckAndDownloadAsync(new Version(0,1,0),null,TestContext.Current.CancellationToken);
            Assert.Equal(hash,result.Package!.Sha256);Assert.Empty(responses);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task DecliningUpdateDoesNotDownloadAssetsOrCreateUpdateDirectory()
    {
        var root=Path.Combine(TestDirectory(),"not-created");
        try
        {
            var requests=0;var prompts=0;
            var service=new ApplicationUpdateService((_,_,_)=>
            {
                requests++;return Task.FromResult(JsonResponse(ReleaseJson("v0.1.1",20)));
            },root);
            var result=await service.CheckAndDownloadAsync(new Version(0,1,0),null,TestContext.Current.CancellationToken,
                (version,tag,_)=>{prompts++;Assert.Equal(new Version(0,1,1),version);Assert.Equal("v0.1.1",tag);return Task.FromResult(false);});
            Assert.True(result.IsUpdateAvailable);
            Assert.Null(result.Package);
            Assert.Equal(1,requests);Assert.Equal(1,prompts);
            Assert.False(Directory.Exists(root));
        }
        finally{Directory.Delete(Path.GetDirectoryName(root)!,true);}
    }

    [Fact]
    public async Task ClosingWhileConfirmationIsPendingPreventsAllDownloads()
    {
        var root=TestDirectory();
        try
        {
            using var cancellation=new CancellationTokenSource();var requests=0;
            var service=new ApplicationUpdateService((_,_,_)=>
            {
                requests++;return Task.FromResult(JsonResponse(ReleaseJson("v0.1.1",20)));
            },root);
            await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>service.CheckAndDownloadAsync(new Version(0,1,0),null,cancellation.Token,
                (_,_,_)=>{cancellation.Cancel();return Task.FromResult(true);}));
            Assert.Equal(1,requests);Assert.Empty(Directory.EnumerateFileSystemEntries(root));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task NewOfficialReleaseDownloadsAndVerifiesExactInstaller()
    {
        var root=TestDirectory();
        try
        {
            var installer=Encoding.UTF8.GetBytes("verified installer payload");
            var hash=Convert.ToHexString(SHA256.HashData(installer)).ToLowerInvariant();
            var responses=new Queue<HttpResponseMessage>(
            [
                JsonResponse(ReleaseJson("v0.1.1",installer.Length)),
                BytesResponse(Encoding.UTF8.GetBytes($"{hash}  MewuAI-Setup-0.1.1-win-x64.exe\n")),
                BytesResponse(installer)
            ]);
            var requested=new List<Uri>();var accepted=false;
            var service=new ApplicationUpdateService((request,_,_)=>
            {
                if(requested.Count>0)Assert.True(accepted);
                requested.Add(request.RequestUri!);
                return Task.FromResult(responses.Dequeue());
            },root);

            var result=await service.CheckAndDownloadAsync(new Version(0,1,0),null,TestContext.Current.CancellationToken,
                (_,_,_)=>{Assert.Single(requested);accepted=true;return Task.FromResult(true);});

            Assert.True(result.IsUpdateAvailable);
            Assert.Equal(new Version(0,1,1),result.LatestVersion);
            Assert.Equal(hash,result.Package!.Sha256);
            Assert.Equal(installer,await File.ReadAllBytesAsync(result.Package.InstallerPath,TestContext.Current.CancellationToken));
            Assert.Equal(3,requested.Count);
            Assert.Empty(responses);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task CurrentReleaseDoesNotDownloadAnyAssets()
    {
        var root=TestDirectory();
        try
        {
            var calls=0;
            var service=new ApplicationUpdateService((_,_,_)=>
            {
                calls++;
                return Task.FromResult(JsonResponse(ReleaseJson("v0.1.0",20)));
            },root);

            var result=await service.CheckAndDownloadAsync(new Version(0,1,0),null,TestContext.Current.CancellationToken,(_,_,_)=>throw new InvalidOperationException("最新版不应询问更新"));

            Assert.False(result.IsUpdateAvailable);
            Assert.Null(result.Package);
            Assert.Equal(1,calls);
        }
        finally{if(Directory.Exists(root))Directory.Delete(root,true);}
    }

    [Fact]
    public async Task HashMismatchNeverPublishesDownloadedInstaller()
    {
        var root=TestDirectory();
        try
        {
            var installer=Encoding.UTF8.GetBytes("tampered payload");
            var responses=new Queue<HttpResponseMessage>(
            [
                JsonResponse(ReleaseJson("v0.1.1",installer.Length)),
                BytesResponse(Encoding.UTF8.GetBytes($"{new string('0',64)}  MewuAI-Setup-0.1.1-win-x64.exe\n")),
                BytesResponse(installer)
            ]);
            var service=new ApplicationUpdateService((_,_,_)=>Task.FromResult(responses.Dequeue()),root);

            await Assert.ThrowsAsync<InvalidDataException>(()=>service.CheckAndDownloadAsync(new Version(0,1,0),null,TestContext.Current.CancellationToken));

            Assert.Empty(Directory.Exists(root)?Directory.EnumerateFiles(root,"*.exe",SearchOption.AllDirectories):[]);
            Assert.Empty(Directory.Exists(root)?Directory.EnumerateFiles(root,"*.download",SearchOption.AllDirectories):[]);
        }
        finally{if(Directory.Exists(root))Directory.Delete(root,true);}
    }

    [Fact]
    public void ChecksumParserRequiresOneExactFilename()
    {
        var expected=new string('a',64);
        Assert.Equal(expected,ApplicationUpdateService.ParseExpectedSha256($"{expected}  MewuAI-Setup-0.1.1-win-x64.exe\n", "MewuAI-Setup-0.1.1-win-x64.exe"));
        Assert.Throws<InvalidDataException>(()=>ApplicationUpdateService.ParseExpectedSha256($"{expected}  other.exe\n", "MewuAI-Setup-0.1.1-win-x64.exe"));
        Assert.Throws<InvalidDataException>(()=>ApplicationUpdateService.ParseExpectedSha256($"{expected}  MewuAI-Setup-0.1.1-win-x64.exe\n{expected}  MewuAI-Setup-0.1.1-win-x64.exe\n", "MewuAI-Setup-0.1.1-win-x64.exe"));
    }

    [Fact]
    public async Task RateLimitedRestApiFallsBackToOfficialLatestReleaseRedirect()
    {
        var root=TestDirectory();
        try
        {
            var service=new ApplicationUpdateService(
                (_,_,_)=>Task.FromResult(new HttpResponseMessage(HttpStatusCode.Forbidden)),
                root,
                (_,_)=>Task.FromResult(new HttpResponseMessage(HttpStatusCode.Found)
                {
                    Headers={Location=new Uri("/abnste/mewu_ai/releases/tag/v0.1.0",UriKind.Relative)}
                }));

            var result=await service.CheckAndDownloadAsync(new Version(0,1,0),null,TestContext.Current.CancellationToken);

            Assert.False(result.IsUpdateAvailable);
            Assert.Equal("v0.1.0",result.TagName);
        }
        finally{if(Directory.Exists(root))Directory.Delete(root,true);}
    }

    private static string ReleaseJson(string tag,int installerSize,string? digest=null,bool includeChecksums=true)
    {
        var assets=new List<object>{new
        {
            name=$"MewuAI-Setup-{tag[1..]}-win-x64.exe",size=installerSize,
            browser_download_url=$"https://github.com/abnste/mewu_ai/releases/download/{tag}/MewuAI-Setup-{tag[1..]}-win-x64.exe",
            digest
        }};
        if(includeChecksums)assets.Add(new
        {
            name="SHA256SUMS.txt",size=199,
            browser_download_url=$"https://github.com/abnste/mewu_ai/releases/download/{tag}/SHA256SUMS.txt"
        });
        return JsonSerializer.Serialize(new{tag_name=tag,draft=false,prerelease=false,assets});
    }

    private static HttpResponseMessage JsonResponse(string json)=>new(HttpStatusCode.OK){Content=new StringContent(json,Encoding.UTF8,"application/json")};
    private static HttpResponseMessage BytesResponse(byte[] bytes)=>new(HttpStatusCode.OK){Content=new ByteArrayContent(bytes)};
    private static string TestDirectory(){var path=Path.Combine(Path.GetTempPath(),"MewuAI-UpdateTests",Guid.NewGuid().ToString("N"));Directory.CreateDirectory(path);return path;}
}
