// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.ComponentModel;
using System.Diagnostics;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ScreenEntityMcpServiceTests
{
    [Fact]
    public void ExistingBrowserDispatchWithoutNewProcessIsSuccessful()
    {
        ProcessStartInfo? actual=null;
        Assert.True(ScreenEntityMcpService.OpenUrl("https://example.org/path?q=sample",info=>{actual=info;return null;}));
        Assert.NotNull(actual);
        Assert.True(actual.UseShellExecute);
        Assert.Equal("https://example.org/path?q=sample",actual.FileName);
        Assert.Empty(actual.Arguments);
    }

    [Theory]
    [InlineData("file:///C:/example.txt")]
    [InlineData("javascript:alert(1)")]
    [InlineData("mailto:sample@example.org")]
    [InlineData("relative/path")]
    public void UnsupportedUrlDoesNotInvokeShell(string value)
    {
        var invoked=false;
        Assert.False(ScreenEntityMcpService.OpenUrl(value,_=>{invoked=true;return null;}));
        Assert.False(invoked);
    }

    [Fact]
    public void FailedDispatchDoesNotLogScreenDerivedData()
    {
        string? logged=null;
        Assert.False(ScreenEntityMcpService.OpenUrl("https://example.org/private-screen-path",
            _=>throw new Win32Exception("Failed to open private-screen-path for sample@example.org"),value=>logged=value));
        Assert.Equal(nameof(Win32Exception),logged);
    }

    [Fact]
    public void ShellProcessWrapperIsReleasedAfterDispatch()
    {
        var process=new ObservedProcess();
        Assert.True(ScreenEntityMcpService.OpenUrl("https://example.org",_=>process));
        Assert.True(process.WasDisposed);
    }

    [Theory]
    [InlineData("qq","https://mail.qq.com/")]
    [InlineData("netease","https://mail.163.com/")]
    [InlineData(null,"mailto:sample%2Btag%40example.org")]
    public void MailProviderDispatchKeepsAddressAndReportsCopySeparately(string? provider,string expectedTarget)
    {
        const string address="sample+tag@example.org";
        string? copied=null;
        ProcessStartInfo? actual=null;
        Assert.True(ScreenEntityMcpService.OpenMailProvider(new(ScreenEntityType.Email,address,provider),out var addressCopied,
            value=>{copied=value;return true;},info=>{actual=info;return null;}));
        Assert.True(addressCopied);
        Assert.Equal(address,copied);
        Assert.Equal(expectedTarget,actual!.FileName);
        Assert.True(actual.UseShellExecute);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void FailedClipboardDoesNotPreventMailClientDispatch(bool throws)
    {
        var invoked=false;
        Assert.True(ScreenEntityMcpService.OpenMailProvider(new(ScreenEntityType.Email,"sample@example.org"),out var copied,
            _=>throws?throw new InvalidOperationException():false,_=>{invoked=true;return null;}));
        Assert.False(copied);
        Assert.True(invoked);
    }

    [Fact]
    public void FailedShellDoesNotEraseSuccessfulCopyResult()
    {
        Assert.False(ScreenEntityMcpService.OpenMailProvider(new(ScreenEntityType.Email,"sample@example.org"),out var copied,
            _=>true,_=>throw new Win32Exception()));
        Assert.True(copied);
    }

    [Fact]
    public void NonMailEntityDoesNotCopyOrOpen()
    {
        var invoked=false;
        Assert.False(ScreenEntityMcpService.OpenMailProvider(new(ScreenEntityType.Url,"https://example.org"),out var copied,
            _=>{invoked=true;return true;},_=>{invoked=true;return null;}));
        Assert.False(copied);
        Assert.False(invoked);
    }

    private sealed class ObservedProcess:Process
    {
        internal bool WasDisposed{get;private set;}
        protected override void Dispose(bool disposing){WasDisposed=disposing;base.Dispose(disposing);}
    }
}
