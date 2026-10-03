// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json.Nodes;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class MailContextFallbackTests
{
    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task QqFailureOrTimeoutAllowsReadOnlyNetEaseFallback(bool timeout)
    {
        var netCalls=0;
        var result=await QqMailContextService.TryBuildFromSourcesAsync(
            _=>Task.FromException<string?>(timeout?new OperationCanceledException():new InvalidDataException("synthetic")),
            ct=>{Assert.False(ct.IsCancellationRequested);netCalls++;return Task.FromResult<string?>("NetEase synthetic data");},
            ()=>"unavailable",CancellationToken.None);
        Assert.Equal("NetEase synthetic data",result);Assert.Equal(1,netCalls);
    }

    [Fact]
    public async Task UserCancellationNeverReadsTheFallbackAccount()
    {
        using var source=new CancellationTokenSource();var called=false;
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>QqMailContextService.TryBuildFromSourcesAsync(
            _=>{source.Cancel();throw new OperationCanceledException(source.Token);},
            _=>{called=true;return Task.FromResult<string?>("must not read");},()=>"unavailable",source.Token));
        Assert.False(called);
    }

    [Fact]
    public async Task SuccessfulQqReadDoesNotReadAnotherMailbox()
    {
        var result=await QqMailContextService.TryBuildFromSourcesAsync(_=>Task.FromResult<string?>("QQ synthetic data"),
            _=>throw new InvalidOperationException("must not read"),()=>"unavailable",CancellationToken.None);
        Assert.Equal("QQ synthetic data",result);
    }

    [Theory]
    [InlineData("{\"data\":[]}")]
    [InlineData("{\"data\":{\"data\":[]}}")]
    public void BothSupportedListShapesAcceptAnEmptyMailbox(string json)
        =>Assert.Empty(QqMailContextService.ParseMessageList(json));

    [Theory]
    [InlineData("{}")]
    [InlineData("{\"error\":\"denied\"}")]
    [InlineData("{\"data\":{\"data\":{}}}")]
    public void MissingListIsNotReportedAsAnEmptyMailbox(string json)
        =>Assert.Throws<InvalidDataException>(()=>QqMailContextService.ParseMessageList(json));

    [Fact]
    public void OptionalWrongTypesDoNotDiscardTheWholeMailbox()
    {
        var identity=QqMailContextService.ParseQqIdentity("""{"data":{"data":{"aliases":[null,7,{"email":"primary@example.test","is_primary":true}],"scopes":[null,"mail:read",{}]}}}""");
        Assert.Equal("primary@example.test",identity.Account);Assert.Equal(new[]{"mail:read"},identity.Scopes);
        var notes=JsonNode.Parse("""[null,1,{"from":{},"subject":"synthetic subject","created_at":1700000000000,"is_read":"unknown","snippet":"synthetic body"}]""")!.AsArray();
        var result=QqMailContextService.Format(identity.Account,identity.Scopes,"inbox",notes,[],false);
        Assert.Contains("synthetic subject",result);Assert.Contains("synthetic body",result);
        var net=QqMailContextService.FormatNetEase("synthetic@126.com",JsonNode.Parse("""[null,{"receivedDate":1700000000000,"flags":[],"subject":"net synthetic"}]""")!.AsArray(),[],false);
        Assert.Contains("net synthetic",net);
    }
}
