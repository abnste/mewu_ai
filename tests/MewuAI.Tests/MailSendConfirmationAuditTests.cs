// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json.Nodes;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class MailSendConfirmationAuditTests
{
    private static QqMailDraft Draft()=>new([("recipient@example.test",null)],[],"Subject","Body");
    private const string Confirmation="""{"error":{"details":{"confirmation_token":"synthetic-confirmation"}}}""";

    [Theory]
    [InlineData("mail:read",false)]
    [InlineData("mail:read mail:send",true)]
    [InlineData("mail:sender",false)]
    [InlineData("MAIL:SEND",false)]
    public void AutoUsesOnlyAnAccountWithExactSendPermission(string scope,bool permitted)
    {
        var token=new QqMailMcpToken("synthetic","","client",DateTimeOffset.UtcNow.AddHours(1),scope);
        var ready=QqMailSendService.CanSend(token);
        Assert.Equal(permitted,ready);
        Assert.Equal(permitted?MailChannel.Qq:MailChannel.NetEase,QqMailSendService.SelectChannel(MailChannel.Auto,ready,true));
        Assert.Equal(permitted?MailChannel.Qq:(MailChannel?)null,QqMailSendService.SelectChannel(MailChannel.Qq,ready,true));
    }

    [Fact]
    public async Task UnverifiedSubmissionHasADistinctOutcome()
    {
        var result=await QqMailSendService.DeliverQqAsync(Draft(),_=>Task.FromResult(true),
            (_,_)=>Task.FromResult(("{}",false)),CancellationToken.None);
        Assert.False(result.Success);Assert.True(result.IsUnconfirmed);
    }

    [Theory]
    [InlineData(false,0)]
    [InlineData(false,1)]
    [InlineData(false,2)]
    [InlineData(false,3)]
    [InlineData(true,0)]
    [InlineData(true,1)]
    [InlineData(true,2)]
    [InlineData(true,3)]
    public async Task LostOrUnreadableReplyAfterEitherSendPhaseIsUnconfirmed(bool secondPhase,int kind)
    {
        var calls=0;
        var result=await QqMailSendService.DeliverQqAsync(Draft(),_=>Task.FromResult(true),(_,_)=>
        {
            calls++;
            if(secondPhase&&calls==1)return Task.FromResult((Confirmation,true));
            return Task.FromException<(string,bool)>(kind switch
            {
                0=>new OperationCanceledException(),1=>new IOException("synthetic disconnect"),
                2=>new System.Net.Http.HttpRequestException("synthetic transport failure"),
                _=>new InvalidDataException("synthetic malformed response")
            });
        },TestContext.Current.CancellationToken);
        Assert.False(result.Success);Assert.True(result.IsUnconfirmed);Assert.Equal(secondPhase?2:1,calls);
    }

    [Fact]
    public async Task CancellationBeforeDispatchMakesNoSendCall()
    {
        using var canceled=CancellationTokenSource.CreateLinkedTokenSource(TestContext.Current.CancellationToken);
        var calls=0;
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>QqMailSendService.DeliverQqAsync(Draft(),_=>
        {canceled.Cancel();return Task.FromResult(true);},(_,_)=>
        {calls++;return Task.FromResult(("{}",false));},canceled.Token));
        Assert.Equal(0,calls);
    }

    [Fact]
    public async Task ExplicitAuthorizationFailureRemainsADefiniteFailure()
    {
        await Assert.ThrowsAsync<InvalidOperationException>(()=>QqMailSendService.DeliverQqAsync(Draft(),_=>Task.FromResult(true),
            (_,_)=>Task.FromException<(string,bool)>(new InvalidOperationException("synthetic authorization refused")),TestContext.Current.CancellationToken));
    }

    [Fact]
    public void MalformedCcCannotBeSilentlyOmitted()
    {
        const string block="""
            ```mewu-mail-send
            {"to":[{"email":"valid@example.test"}],"cc":"other@example.test","subject":"Subject","body":"Body"}
            ```
            """;
        Assert.False(QqMailSendService.TryExtract(block,out _,out _));
    }

    [Fact]
    public async Task DecliningNeverContactsTheMailServer()
    {
        var calls=0;
        var result=await QqMailSendService.DeliverQqAsync(Draft(),_=>Task.FromResult(false),
            (_,_)=>{calls++;throw new InvalidOperationException();},CancellationToken.None);
        Assert.False(result.Success);Assert.Equal(0,calls);
    }

    [Theory]
    [InlineData("{}",false)]
    [InlineData("not JSON",false)]
    [InlineData("{\"error\":{\"message\":\"denied\"}}",false)]
    [InlineData("{\"data\":{\"error\":{\"message\":\"denied\"}}}",false)]
    [InlineData("{\"success\":true}",false)]
    [InlineData("failure",true)]
    public async Task ToolFlagAloneNeverClaimsMailWasSent(string response,bool isError)
    {
        var calls=0;
        var result=await QqMailSendService.DeliverQqAsync(Draft(),_=>Task.FromResult(true),
            (_,_)=>{calls++;return Task.FromResult((response,isError));},CancellationToken.None);
        Assert.False(result.Success);Assert.Equal(1,calls);
    }

    [Theory]
    [InlineData("{}",false)]
    [InlineData("not JSON",false)]
    [InlineData(Confirmation,false)]
    [InlineData("{\"error\":{\"message\":\"denied\"}}",false)]
    [InlineData("failure",true)]
    public async Task ConfirmationDoesNotCauseAThirdAttemptOrUnverifiedSuccess(string final,bool isError)
    {
        var calls=0;var confirmed=false;
        var result=await QqMailSendService.DeliverQqAsync(Draft(),_=>{confirmed=true;return Task.FromResult(true);},
            (args,_)=>
            {
                Assert.True(confirmed);calls++;
                if(calls==1){Assert.False(args.ContainsKey("confirmation_token"));return Task.FromResult((Confirmation,true));}
                Assert.Equal("synthetic-confirmation",args["confirmation_token"]!.GetValue<string>());
                return Task.FromResult((final,isError));
            },CancellationToken.None);
        Assert.False(result.Success);Assert.Equal(2,calls);
    }

    [Theory]
    [InlineData("first@example.test second@example.test")]
    [InlineData("first@example.test\r\nRCPT TO:<second@example.test>")]
    [InlineData("invalid")]
    public void PartialRecipientsAreNotSilentlyAccepted(string invalid)
    {
        var json=new JsonObject{["to"]=new JsonArray(new JsonObject{["email"]="valid@example.test"},new JsonObject{["email"]=invalid}),["subject"]="Subject",["body"]="Body"};
        Assert.False(QqMailSendService.TryExtract("```mewu-mail-send\n"+json.ToJsonString()+"\n```",out _,out _));
    }

    [Fact]
    public void DraftPreservesUserTextAndRejectsAmbiguousMultipleBlocks()
    {
        var json=new JsonObject{["to"]=new JsonArray(new JsonObject{["email"]="valid@example.test"}),["subject"]=" Subject ",["body"]="  Body\n"};
        var block="```mewu-mail-send\n"+json.ToJsonString()+"\n```";
        Assert.True(QqMailSendService.TryExtract(block,out var draft,out _));
        Assert.Equal(" Subject ",draft.Subject);Assert.Equal("  Body\n",draft.Body);
        Assert.False(QqMailSendService.TryExtract(block+"\n"+block,out _,out _));
    }

    [Fact]
    public async Task ConfirmedRecipientListCannotChangeWhileDialogIsOpen()
    {
        var recipients=new List<(string,string?)>{("original@example.test",null)};
        var draft=new QqMailDraft(recipients,[],"Subject","Body");
        await QqMailSendService.DeliverQqAsync(draft,_=>{recipients[0]=("changed@example.test",null);return Task.FromResult(true);},
            (args,_)=>{Assert.Equal("original@example.test",args["to"]![0]!["email"]!.GetValue<string>());return Task.FromResult(("{}",false));},CancellationToken.None);
    }
}
