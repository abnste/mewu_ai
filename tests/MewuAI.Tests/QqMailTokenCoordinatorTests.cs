// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class QqMailTokenCoordinatorTests
{
    private static QqMailMcpToken Token(string name,bool usable=false)=>new(name,"refresh-"+name,"synthetic-client",
        DateTimeOffset.UtcNow.AddHours(usable?1:-1),"mail:read");

    [Fact]
    public async Task RevokingDuringRefreshNeverRestoresTheOldAuthorization()
    {
        QqMailMcpToken? stored=Token("old");
        var coordinator=new QqMailTokenCoordinator(()=>stored,value=>stored=value);
        var entered=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var release=new TaskCompletionSource<QqMailMcpToken>(TaskCreationOptions.RunContinuationsAsynchronously);
        var resolving=coordinator.ResolveAsync((_,_)=>{entered.SetResult();return release.Task;},CancellationToken.None);
        await entered.Task.WaitAsync(TimeSpan.FromSeconds(5),TestContext.Current.CancellationToken);
        coordinator.Write(null);release.SetResult(Token("refreshed-old",true));
        Assert.Null(await resolving.WaitAsync(TimeSpan.FromSeconds(5),TestContext.Current.CancellationToken));Assert.Null(stored);
    }

    [Fact]
    public async Task ANewAccountWinsOverThePreviousAccountsRefresh()
    {
        QqMailMcpToken? stored=Token("old");
        var coordinator=new QqMailTokenCoordinator(()=>stored,value=>stored=value);
        var release=new TaskCompletionSource<QqMailMcpToken>(TaskCreationOptions.RunContinuationsAsynchronously);
        var resolving=coordinator.ResolveAsync((_,_)=>release.Task,CancellationToken.None);
        var replacement=Token("replacement",true);coordinator.Write(replacement);
        release.SetResult(Token("refreshed-old",true));
        Assert.Same(replacement,await resolving.WaitAsync(TimeSpan.FromSeconds(5),TestContext.Current.CancellationToken));Assert.Same(replacement,stored);
    }

    [Fact]
    public async Task WaitingRefreshReadsTheLatestStoreInsideTheGate()
    {
        QqMailMcpToken? stored=Token("old");
        var coordinator=new QqMailTokenCoordinator(()=>stored,value=>stored=value);
        var release=new TaskCompletionSource<QqMailMcpToken>(TaskCreationOptions.RunContinuationsAsynchronously);
        var first=coordinator.ResolveAsync((_,_)=>release.Task,CancellationToken.None);
        var refreshNames=new List<string>();
        var second=coordinator.ResolveAsync((latest,_)=>
        {refreshNames.Add(latest.AccessToken);return Task.FromResult(Token("refreshed-new",true));},CancellationToken.None);
        coordinator.Write(Token("new"));release.SetResult(Token("refreshed-old",true));
        Assert.Null(await first.WaitAsync(TimeSpan.FromSeconds(5),TestContext.Current.CancellationToken));
        Assert.Equal("refreshed-new",(await second.WaitAsync(TimeSpan.FromSeconds(5),TestContext.Current.CancellationToken))!.AccessToken);
        Assert.Equal(new[]{"new"},refreshNames);Assert.Equal("refreshed-new",stored!.AccessToken);
    }

    [Fact]
    public async Task FailedClearIsReportedAndCannotCommitAnOlderRefresh()
    {
        QqMailMcpToken? stored=Token("old");
        var coordinator=new QqMailTokenCoordinator(()=>stored,value=>
        {if(value is null)throw new IOException("synthetic-store-failure");stored=value;});
        var release=new TaskCompletionSource<QqMailMcpToken>(TaskCreationOptions.RunContinuationsAsynchronously);
        var resolving=coordinator.ResolveAsync((_,_)=>release.Task,CancellationToken.None);
        Assert.Throws<IOException>(()=>coordinator.Write(null));
        release.SetResult(Token("refreshed-old",true));
        Assert.Null(await resolving.WaitAsync(TimeSpan.FromSeconds(5),TestContext.Current.CancellationToken));Assert.Equal("old",stored!.AccessToken);
    }

    [Fact]
    public void RefreshWithoutAScopeDoesNotGainSendPermission()
    {
        var refreshed=QqMailMcpService.ParseTokenResponse("""{"access_token":"synthetic","expires_in":3600}""",
            "synthetic-client",requireRefreshToken:false,fallbackScope:"mail:read");
        Assert.Equal("mail:read",refreshed.Scope);Assert.False(QqMailSendService.CanSend(refreshed));
    }
}
