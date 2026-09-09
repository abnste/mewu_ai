// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class HermesReplyMediaServiceTests
{
    [Fact]
    public void NativeWindowsMarkdownAndMediaMarkerProduceOneAuthorizedImage()
    {
        const string path=@"C:\Users\Artist\output\portrait.jpg";
        var result=HermesReplyMediaService.Complete(new AiResult($"自画像\n\n![自画像]({path})\n\nMEDIA:{path}",[]),[]);
        Assert.Single(result.LocalReplyImageSources);Assert.Equal(path,result.LocalReplyImageSources[0]);
        Assert.DoesNotContain("MEDIA:",result.Answer);Assert.Equal(1,result.Answer.Split("![").Length-1);
    }

    [Fact]
    public void MediaOnlySupportsQuotedSpacesAndEncodesFileUri()
    {
        var result=HermesReplyMediaService.Complete(new AiResult("MEDIA: \"C:\\Users\\Artist\\my art\\portrait.png\"",[]),[]);
        Assert.Single(result.LocalReplyImageSources);Assert.Contains("![Hermes",result.Answer);Assert.Contains("my%20art",result.Answer);
    }

    [Fact]
    public void MediaExampleInsideCodeFenceDoesNotReadLocalFiles()
    {
        const string answer="```text\nMEDIA:C:/output/portrait.png\n```";
        var result=HermesReplyMediaService.Complete(new AiResult(answer,[]),[]);
        Assert.Equal(answer,result.Answer);Assert.Empty(result.LocalReplyImageSources);
    }

    [Fact]
    public void SuccessfulImageToolResultIsShownEvenWhenFinalTextOmitsLink()
    {
        using var payload=JsonDocument.Parse("""{"name":"image_generate","result":{"success":true,"image":"C:/output/portrait.png"}}""");
        var result=HermesReplyMediaService.Complete(new AiResult("画好了。",[]),HermesReplyMediaService.ReadGeneratedImages(payload.RootElement));
        Assert.Contains("![Hermes",result.Answer);Assert.Single(result.LocalReplyImageSources);
    }

    [Fact]
    public void ToolResultSupportsSerializedJsonAndDoesNotDuplicateFinalImage()
    {
        var json=JsonSerializer.Serialize(new{name="image_generate",result=JsonSerializer.Serialize(new{success=true,image="https://example.com/portrait.png"})});
        using var payload=JsonDocument.Parse(json);
        var result=HermesReplyMediaService.Complete(new AiResult("![画像](https://example.com/portrait.png)",[]),HermesReplyMediaService.ReadGeneratedImages(payload.RootElement));
        Assert.Equal(1,result.Answer.Split("![").Length-1);Assert.Empty(result.LocalReplyImageSources);
    }

    [Theory]
    [InlineData("image_generate",false)]
    [InlineData("terminal",true)]
    public void FailedOrOtherToolOutputCannotRegisterImages(string name,bool success)
    {
        using var payload=JsonDocument.Parse(JsonSerializer.Serialize(new{name,result=new{success,image="C:/output/portrait.png"}}));
        Assert.Empty(HermesReplyMediaService.ReadGeneratedImages(payload.RootElement));
    }

    [Theory]
    [InlineData(@"\\server\share\portrait.png")]
    [InlineData("file://server/share/portrait.png")]
    [InlineData("file://localhost/C:/output/portrait.png")]
    [InlineData(@"C:\output\secret.txt")]
    [InlineData(@"C:\output\portrait.png:stream")]
    [InlineData(@"\\?\C:\output\portrait.png")]
    [InlineData("../portrait.png")]
    public void RejectsNetworkDeviceNonImageAndRelativeFilePaths(string path)=>Assert.False(ReplyImageService.TryGetLocalPath(path,out _));

    [Fact]
    public void GeneratedImageCountIsBounded()
    {
        var result=HermesReplyMediaService.Complete(new AiResult("完成",[]),Enumerable.Range(0,40).Select(i=>$"C:/output/p{i}.png").ToArray());
        Assert.Equal(16,result.LocalReplyImageSources.Count);Assert.Equal(16,result.Answer.Split("![").Length-1);
    }

    [Fact]
    public async Task LocalPathIsNotReadableWithoutExplicitHermesAuthorization()
    {
        await Assert.ThrowsAsync<System.IO.InvalidDataException>(()=>ReplyImageService.LoadAsync("C:/output/portrait.png",CancellationToken.None));
    }
}
