// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net;
using System.Net.Http;
using System.Text.Json;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ScreenOutputBudgetTests
{
    [Theory]
    [InlineData(false,false)]
    [InlineData(true,false)]
    [InlineData(false,true)]
    [InlineData(true,true)]
    public async Task M3ScreenAndTableRequestsUseOfficialMaximumWithoutDisablingThinking(bool stream,bool tableRecognition)
    {
        var table="| ID | Value |\n| --- | --- |\n"+string.Join("\n",Enumerable.Range(1,1000).Select(index=>$"| {index} | data-{index} |"));
        var provider=new OpenAiCompatibleProvider(new AiProviderSettings(),"test",async (message,completionOption,token)=>
        {
            using var body=JsonDocument.Parse(await message.Content!.ReadAsStringAsync(token));
            Assert.Equal(524288,body.RootElement.GetProperty("max_completion_tokens").GetInt32());
            Assert.False(body.RootElement.TryGetProperty("max_tokens",out _));
            Assert.Equal("adaptive",body.RootElement.GetProperty("thinking").GetProperty("type").GetString());
            Assert.Equal(!tableRecognition,body.RootElement.GetProperty("messages").GetRawText().Contains("prior-conversation",StringComparison.Ordinal));
            var content=JsonSerializer.Serialize(new{answer=table,annotationMode="preserve",annotations=Array.Empty<object>()});
            var response=stream
                ?"data: "+JsonSerializer.Serialize(new{choices=new[]{new{delta=new{content,reasoning_content="checked"},finish_reason="stop"}}})+"\n\n"
                :JsonSerializer.Serialize(new{choices=new[]{new{message=new{content,reasoning_content="checked"},finish_reason="stop"}}});
            return new HttpResponseMessage(HttpStatusCode.OK){Content=new StringContent(response)};
        },_=>TimeSpan.FromSeconds(10));
        var request=CaptureOverlayPolicy.CreateScreenAiRequest("extract table",
            [new("user","prior-conversation"),new("assistant","prior-conversation")],[],stream?new DiscardProgress():null,tableRecognition:tableRecognition);
        Assert.False(request.DisableReasoning);
        var result=await provider.SendAsync(request,TestContext.Current.CancellationToken);
        Assert.Equal(table,result.Answer);Assert.Equal("checked",result.Reasoning);
    }

    [Fact] public async Task ExplicitSmallUtilityRequestBudgetIsPreserved()
    {
        var provider=new OpenAiCompatibleProvider(new AiProviderSettings(),"test",async (message,completionOption,token)=>
        {
            using var body=JsonDocument.Parse(await message.Content!.ReadAsStringAsync(token));
            Assert.Equal(32,body.RootElement.GetProperty("max_completion_tokens").GetInt32());
            return new HttpResponseMessage(HttpStatusCode.OK){Content=new StringContent("{\"choices\":[{\"message\":{\"content\":\"OK\"},\"finish_reason\":\"stop\"}]}")};
        },_=>TimeSpan.FromSeconds(10));
        await provider.SendAsync(new AiRequest{Prompt="probe",MaxOutputTokens=32},TestContext.Current.CancellationToken);
    }

    [Fact] public async Task UnknownProviderDoesNotReceiveAnInventedMaximumOrThinkingOverride()
    {
        var provider=new OpenAiCompatibleProvider(new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="custom"},"test",async (message,completionOption,token)=>
        {
            using var body=JsonDocument.Parse(await message.Content!.ReadAsStringAsync(token));
            Assert.False(body.RootElement.TryGetProperty("max_tokens",out _));
            Assert.False(body.RootElement.TryGetProperty("max_completion_tokens",out _));
            Assert.False(body.RootElement.TryGetProperty("thinking",out _));
            return new HttpResponseMessage(HttpStatusCode.OK){Content=new StringContent("{\"choices\":[{\"message\":{\"content\":\"OK\"},\"finish_reason\":\"stop\"}]}")};
        },_=>TimeSpan.FromSeconds(10));
        await provider.SendAsync(CaptureOverlayPolicy.CreateScreenAiRequest("table",[],[],null,tableRecognition:true),TestContext.Current.CancellationToken);
    }

    private sealed class DiscardProgress:IProgress<AiStreamDelta>{public void Report(AiStreamDelta value){}}
}
