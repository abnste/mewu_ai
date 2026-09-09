// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ReplyImageServiceTests
{
    [Theory]
    [InlineData("file:///C:/Windows/secret.png")]
    [InlineData("https://user:password@example.com/image.png")]
    [InlineData("http://127.0.0.1/a.png")]
    [InlineData("http://localhost/a.png")]
    [InlineData("http://printer.local/a.png")]
    [InlineData("https://example.com:8443/a.png")]
    [InlineData("javascript:alert(1)")]
    public void RejectsUnsupportedImageLocations(string value)=>Assert.False(ReplyImageService.TryGetWebUri(value,out _));

    [Fact]
    public void AllowsPublicWebImageWithQuery()=>Assert.True(ReplyImageService.TryGetWebUri("https://example.com/image.png?size=300",out _));

    [Fact]
    public async Task StreamingLimitDoesNotRelyOnContentLength()
    {
        using var source=new MemoryStream(new byte[ReplyImageService.MaximumBytes+1]);
        await Assert.ThrowsAsync<InvalidDataException>(()=>ReplyImageService.ReadBoundedAsync(source,CancellationToken.None));
    }

    [Fact]
    public async Task CancelledReadDoesNotReturnPartialImage()
    {
        using var source=new MemoryStream(new byte[100]);using var cancel=new CancellationTokenSource();cancel.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>ReplyImageService.ReadBoundedAsync(source,cancel.Token));
    }

    [Fact]
    public async Task InvalidInlineImageFailsWithoutNetwork()=>await Assert.ThrowsAsync<FormatException>(()=>ReplyImageService.LoadAsync("data:image/png;base64,invalid",CancellationToken.None));
}
