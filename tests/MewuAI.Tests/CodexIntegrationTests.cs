// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text;
using System.Text.Json;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class CodexIntegrationTests
{
    private static JsonElement Json(string value)=>JsonSerializer.Deserialize<JsonElement>(value);
    private static void Start(CodexTurnCollector turn)=>turn.Receive("turn/started",Json("""{"threadId":"ours","turn":{"id":"turn1"}}"""));
    private static void Complete(CodexTurnCollector turn,string status="completed")=>turn.Receive("turn/completed",JsonSerializer.SerializeToElement(new{threadId="ours",turn=new{id="turn1",status}}));

    [Fact]
    public async Task CompletionRequiresOwnedFinalMessageAndTerminalEvent()
    {
        var turn=new CodexTurnCollector("ours",new(),CancellationToken.None);Start(turn);
        turn.Receive("item/completed",Json("""{"threadId":"other","turnId":"turn1","item":{"type":"agentMessage","text":"wrong"}}"""));
        turn.Receive("item/completed",Json("""{"threadId":"ours","turnId":"old","item":{"type":"agentMessage","text":"old"}}"""));
        turn.Receive("item/completed",Json("""{"threadId":"ours","turnId":"turn1","item":{"type":"agentMessage","phase":"commentary","text":"working"}}"""));
        Assert.False(turn.Completion.IsCompleted);
        turn.Receive("item/completed",Json("""{"threadId":"ours","turnId":"turn1","item":{"type":"agentMessage","phase":"final_answer","text":"finished"}}"""));
        Assert.False(turn.Completion.IsCompleted);Complete(turn);
        Assert.Equal("finished",(await turn.Completion).Answer);
    }

    [Theory]
    [InlineData("failed")]
    [InlineData("interrupted")]
    [InlineData("completed")]
    public async Task EmptyOrFailedTurnNeverSucceeds(string status)
    {
        var turn=new CodexTurnCollector("ours",new(),CancellationToken.None);Start(turn);Complete(turn,status);
        await Assert.ThrowsAnyAsync<Exception>(async()=>await turn.Completion);
    }

    [Fact]
    public void CancellationAndStopRejectLateEvents()
    {
        using var cancellation=new CancellationTokenSource();
        var turn=new CodexTurnCollector("ours",new(),cancellation.Token);Start(turn);cancellation.Cancel();Complete(turn);
        Assert.False(turn.Completion.IsCompleted);
        var stopped=new CodexTurnCollector("ours",new(),CancellationToken.None);Start(stopped);stopped.Stop();Complete(stopped);
        Assert.False(stopped.Completion.IsCompleted);
    }

    [Theory]
    [InlineData("{\"account\":null}")]
    [InlineData("{\"account\":{\"type\":\"apiKey\"}}")]
    public void RefusesMissingLoginOrSeparateApiBilling(string value)=>Assert.Throws<InvalidOperationException>(()=>CodexAppServer.EnsureChatGptAccount(Json(value)));

    [Fact]
    public void ExistingChatGptLoginIsAcceptedWithoutCredentials()=>CodexAppServer.EnsureChatGptAccount(Json("""{"account":{"type":"chatgpt"}}"""));

    [Fact]
    public async Task RpcFramingHandlesSplitUtf8AndMultipleEvents()
    {
        using var stream=new FragmentedStream(Encoding.UTF8.GetBytes("{\"value\":\"中文😀\"}\n{\"value\":\"second\"}\r\n"));
        var received=new List<string>();
        await CodexAppServer.ReadMessagesAsync(stream,item=>{received.Add(item.GetProperty("value").GetString()!);return Task.CompletedTask;},CancellationToken.None);
        Assert.Equal(new[]{"中文😀","second"},received);
    }

    [Fact]
    public async Task RpcRejectsTruncatedAndOversizedRecords()
    {
        using var truncated=new MemoryStream(Encoding.UTF8.GetBytes("{\"value\":1}"));
        await Assert.ThrowsAsync<InvalidDataException>(()=>CodexAppServer.ReadMessagesAsync(truncated,_=>Task.CompletedTask,CancellationToken.None));
        using var oversized=new MemoryStream(new byte[4*1024*1024+1]);
        await Assert.ThrowsAsync<InvalidDataException>(()=>CodexAppServer.ReadMessagesAsync(oversized,_=>Task.CompletedTask,CancellationToken.None));
    }

    [Fact]
    public void BackendsAreExclusiveAndIncompleteCodexSettingsFailClosed()
    {
        Assert.Throws<InvalidOperationException>(()=>CodexSettingsPolicy.Validate(new(){CodexEnabled=true}));
        Assert.Throws<InvalidOperationException>(()=>CodexSettingsPolicy.Validate(new(){CodexEnabled=true,HermesEnabled=true,CodexModel="model"}));
        CodexSettingsPolicy.Validate(new(){CodexEnabled=true,CodexModel="model"});
    }

    [Fact]
    public async Task RejectedRequestClearsOwnedScreenBytes()
    {
        var bytes=new byte[]{1,2,3};var borrowed=new byte[]{4,5,6};
        var request=new AiRequest{Prompt="test",Attachments=[new(AiAttachmentType.Video,"video/unsupported",bytes),new(AiAttachmentType.Image,"image/png",borrowed,ProviderOwnsData:false)]};
        await Assert.ThrowsAsync<InvalidOperationException>(()=>new CodexAiProvider("model","medium",true).SendAsync(request,CancellationToken.None));
        Assert.All(bytes,item=>Assert.Equal(0,item));Assert.Equal(new byte[]{4,5,6},borrowed);
    }

    [Fact]
    public void VideoUsesLocalFileCapabilityAndTextModelRejectsIt()
    {
        var request=new AiRequest{Prompt="video",Attachments=[new(AiAttachmentType.Video,"video/mp4",new byte[]{1})]};
        CodexAiProvider.Validate(request,true);
        Assert.Throws<InvalidOperationException>(()=>CodexAiProvider.Validate(request,false));
    }

    [Fact]
    public void BrokenCodexRouteCannotFallBackToRemoteProvider()
    {
        using var hermes=new HermesRuntimeService();
        var settings=new AppSettings{CodexEnabled=true};
        var provider=AppHost.CreateConversationProviderCore(HermesConversationKind.Screen,()=>settings,hermes,new AiProviderFactory(),out var error);
        Assert.Null(provider);Assert.Contains("Codex",error);
    }

    [Fact]
    public void VideoToolPermissionsMustBeAppliedAtProcessStart()
    {
        var text=CodexAppServer.SafeConfig();var video=CodexAppServer.SafeConfig(true);
        Assert.Equal("read-only",text["sandbox_mode"]);Assert.Equal(false,text["features.shell_tool"]);
        Assert.Equal("workspace-write",video["sandbox_mode"]);Assert.Equal(true,video["features.shell_tool"]);
        Assert.Equal(false,video["sandbox_workspace_write.network_access"]);
        Assert.Equal(false,video["features.hooks"]);Assert.Equal("on-request",video["approval_policy"]);
    }

    private sealed class FragmentedStream(byte[] data):MemoryStream(data)
    {
        public override ValueTask<int> ReadAsync(Memory<byte> buffer,CancellationToken cancellationToken=default)=>base.ReadAsync(buffer[..Math.Min(buffer.Length,3)],cancellationToken);
    }
}
