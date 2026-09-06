// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using mewu_ai_Assistant;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;
namespace MewuAI.Tests;

public sealed class WorkBuddyIntegrationTests
{
    private static JsonElement Json(string text)=>JsonSerializer.Deserialize<JsonElement>(text);
    private static JsonElement Chunk(string session,string type,string text)=>JsonSerializer.SerializeToElement(new{sessionId=session,update=new{sessionUpdate=type,content=new{type="text",text}}});

    [Fact]
    public void OwnSessionAndSuccessfulCompletionAreRequired()
    {
        var turn=new WorkBuddyTurnCollector("ours",new(),CancellationToken.None);
        turn.Receive("session/update",Chunk("other","agent_message_chunk","wrong"));
        turn.Receive("session/update",Chunk("ours","agent_thought_chunk","thinking"));
        turn.Receive("session/update",Chunk("ours","agent_message_chunk","hello"));
        var result=turn.Finish(Json("""{"stopReason":"end_turn","_meta":{"codebuddy.ai/outcome":"SUCCESS"}}"""));
        Assert.Equal("hello",result.Answer);Assert.Equal("thinking",result.Reasoning);
        turn.Receive("session/update",Chunk("ours","agent_message_chunk","late"));
        Assert.Throws<OperationCanceledException>(()=>turn.Finish(Json("""{"stopReason":"end_turn"}""")));
    }

    [Theory]
    [InlineData("max_tokens")]
    [InlineData("cancelled")]
    [InlineData("refusal")]
    [InlineData("max_turn_requests")]
    public void TruncatedOrCancelledAnswerFails(string reason)
    {
        var turn=new WorkBuddyTurnCollector("ours",new(),CancellationToken.None);
        turn.Receive("session/update",Chunk("ours","agent_message_chunk","partial"));
        Assert.Throws<InvalidDataException>(()=>turn.Finish(JsonSerializer.SerializeToElement(new{stopReason=reason})));
    }

    [Fact]
    public void EmptyOrCancelledOrErrorOutcomeDoesNotSucceed()
    {
        var empty=new WorkBuddyTurnCollector("ours",new(),CancellationToken.None);
        empty.Receive("session/update",Chunk("ours","agent_thought_chunk","thinking"));
        Assert.Throws<InvalidDataException>(()=>empty.Finish(Json("""{"stopReason":"end_turn"}""")));
        using var cts=new CancellationTokenSource();var cancelled=new WorkBuddyTurnCollector("ours",new(),cts.Token);
        cts.Cancel();cancelled.Receive("session/update",Chunk("ours","agent_message_chunk","late"));
        Assert.Throws<OperationCanceledException>(()=>cancelled.Finish(Json("""{"stopReason":"end_turn"}""")));
        var failed=new WorkBuddyTurnCollector("ours",new(),CancellationToken.None);
        failed.Receive("session/update",Chunk("ours","agent_message_chunk","failure explanation"));
        Assert.Throws<InvalidDataException>(()=>failed.Finish(Json("""{"stopReason":"end_turn","_meta":{"codebuddy.ai/outcome":"ERROR"}}""")));
    }

    [Fact]
    public void ToolCommentaryCannotPolluteStructuredAnswerOrBecomeFinalAnswer()
    {
        var turn=new WorkBuddyTurnCollector("ours",new(){ExpectStructuredResponse=true},CancellationToken.None);
        turn.Receive("session/update",Chunk("ours","agent_message_chunk","Let me inspect frames."));
        turn.Receive("session/update",Json("""{"sessionId":"ours","update":{"sessionUpdate":"tool_call"}}"""));
        turn.Receive("session/update",Chunk("ours","agent_message_chunk","{\"answer\":\"red then blue\",\"annotations\":[]}"));
        Assert.Equal("red then blue",turn.Finish(Json("""{"stopReason":"end_turn"}""")).Answer);
        var incomplete=new WorkBuddyTurnCollector("ours",new(),CancellationToken.None);
        incomplete.Receive("session/update",Chunk("ours","agent_message_chunk","I will look."));
        incomplete.Receive("session/update",Json("""{"sessionId":"ours","update":{"sessionUpdate":"tool_call"}}"""));
        Assert.Throws<InvalidDataException>(()=>incomplete.Finish(Json("""{"stopReason":"end_turn"}""")));
    }

    [Fact]
    public void RecoveredToolErrorWithFinalAnswerIsNotATruncatedTurn()
    {
        var turn=new WorkBuddyTurnCollector("ours",new(),CancellationToken.None);
        turn.Receive("session/update",Chunk("ours","agent_message_chunk","verified answer"));
        Assert.Equal("verified answer",turn.Finish(Json("""{"stopReason":"end_turn","_meta":{"codebuddy.ai/outcome":"PARTIAL_SUCCESS"}}""")).Answer);
        var failed=new WorkBuddyTurnCollector("ours",new(),CancellationToken.None);
        failed.Receive("session/update",Chunk("ours","agent_message_chunk","partial"));
        Assert.Throws<InvalidDataException>(()=>failed.Finish(Json("""{"stopReason":"end_turn","_meta":{"codebuddy.ai/outcome":"PARTIAL_SUCCESS","codebuddy.ai/errorMessage":"terminal error"}}""")));
    }

    [Fact]
    public void VideoToolsAreScopedAndConfigurationDoesNotEnableToolsForText()
    {
        var installation=new WorkBuddyInstallation(@"C:\Program Files\WorkBuddy\WorkBuddy.exe",@"C:\Program Files\WorkBuddy\resources\app.asar.unpacked\cli\bin\codebuddy");
        var text=WorkBuddyAcpServer.CreateStartInfo(installation,@"C:\MewuAI-Test");
        var video=WorkBuddyAcpServer.CreateStartInfo(installation,@"C:\MewuAI-Test",true);
        Assert.Equal("",text.ArgumentList[text.ArgumentList.IndexOf("--tools")+1]);
        Assert.Equal("Read,Bash,PowerShell",video.ArgumentList[video.ArgumentList.IndexOf("--tools")+1]);
        Assert.Contains("--no-session-persistence",video.ArgumentList);
        Assert.Contains("--strict-mcp-config",video.ArgumentList);
        Assert.DoesNotContain("--dangerously-skip-permissions",video.ArgumentList);
        using var settings=JsonDocument.Parse(video.ArgumentList[video.ArgumentList.IndexOf("--settings")+1]);
        Assert.True(settings.RootElement.GetProperty("disableAllHooks").GetBoolean());
        var sandbox=settings.RootElement.GetProperty("sandbox");
        Assert.True(sandbox.GetProperty("enabled").GetBoolean());
        Assert.False(sandbox.GetProperty("allowUnsandboxedCommands").GetBoolean());
        Assert.Empty(sandbox.GetProperty("network").GetProperty("allowedDomains").EnumerateArray());
    }

    [Fact]
    public async Task RejectedAttachmentsReleaseOwnedScreenBuffers()
    {
        var owned=new byte[]{1,2,3};var borrowed=new byte[]{4,5,6};
        var provider=new WorkBuddyAiProvider("auto","enabled",true);
        await Assert.ThrowsAsync<InvalidOperationException>(()=>provider.SendAsync(new(){Prompt="test",Attachments=[new(AiAttachmentType.Image,"image/unsupported",owned),new(AiAttachmentType.Image,"image/png",borrowed,ProviderOwnsData:false)]},CancellationToken.None));
        Assert.All(owned,value=>Assert.Equal(0,value));Assert.Equal(new byte[]{4,5,6},borrowed);
    }

    [Fact]
    public void VideoIsLocalFileInputAndImagesHaveAggregateBudget()
    {
        var request=new AiRequest{Prompt="video",Attachments=[new(AiAttachmentType.Video,"video/mp4",new byte[]{1})]};
        WorkBuddyAiProvider.Validate(request,true);
        Assert.Throws<InvalidOperationException>(()=>WorkBuddyAiProvider.Validate(request,false));
        var image=new byte[20*1024*1024];
        Assert.Throws<InvalidOperationException>(()=>WorkBuddyAiProvider.Validate(new(){Prompt="images",Attachments=Enumerable.Range(0,3).Select(_=>new AiAttachment(AiAttachmentType.Image,"image/png",image,ProviderOwnsData:false)).ToList()},true));
    }

    [Fact]
    public void BrokenWorkBuddyCannotFallBackAndHomeShowsSelectedModel()
    {
        var settings=new AppSettings{WorkBuddyEnabled=true,WorkBuddyModel="model-a",WorkBuddyReasoningEffort="low",DefaultProviderId="minimax",Providers=[new(){Id="minimax"}]};
        Assert.Contains("WorkBuddy · model-a",MainWindow.BuildAiStatusText(settings));
        settings.WorkBuddyModel="";
        using var hermes=new HermesRuntimeService();
        Assert.Null(AppHost.CreateConversationProviderCore(HermesConversationKind.Screen,()=>settings,hermes,new(),out var error));
        Assert.Contains("WorkBuddy",error);
    }

    [Fact]
    public void SettingsPersistWorkBuddyWithoutApiCredentialAndRejectMultipleChannels()
    {
        var directory=Path.Combine(Path.GetTempPath(),"MewuAI.Tests",Guid.NewGuid().ToString("N"));Directory.CreateDirectory(directory);
        try
        {
            var settings=new AppSettings{WorkBuddyEnabled=true,WorkBuddyModel="auto",WorkBuddyReasoningEffort="low",WorkBuddySupportsImage=true,DefaultProviderId="api",Providers=[new(){Id="api"}]};
            var service=new SettingsService(Path.Combine(directory,"settings.json"));service.Save(settings);
            var loaded=service.Load();Assert.True(loaded.WorkBuddyEnabled);Assert.Equal("auto",loaded.WorkBuddyModel);Assert.Equal("low",loaded.WorkBuddyReasoningEffort);Assert.True(loaded.WorkBuddySupportsImage);
            settings.CodexEnabled=true;settings.CodexModel="codex-model";
            Assert.Throws<InvalidOperationException>(()=>service.Save(settings));
        }
        finally{Directory.Delete(directory,true);}
    }
}
