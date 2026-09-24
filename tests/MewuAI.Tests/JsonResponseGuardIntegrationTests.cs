// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text;
using System.Text.Json;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public class JsonResponseGuardIntegrationTests
{
    [Theory]
    [InlineData("null")]
    [InlineData("[]")]
    [InlineData("{\"choices\":null}")]
    [InlineData("{\"choices\":[null]}")]
    [InlineData("{\"choices\":[{\"finish_reason\":\"stop\",\"delta\":42}]}")]
    public void InvalidStreamShapeCannotComplete(string payload)
    {
        Assert.False(StreamingResponseParser.TryParse("data: "+payload,out var delta,out var done,out var truncated));
        Assert.False(done);Assert.False(truncated);Assert.Equal("",delta.Content);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task MissingNotificationFieldBecomesSafeProtocolFailure(bool workBuddy)
    {
        using var stream=new MemoryStream(Encoding.UTF8.GetBytes("{\"threadId\":\"ours\"}\n"));
        Task Receive(JsonElement data)
        {
            data.GetProperty("turn");return Task.CompletedTask;
        }
        var error=await Assert.ThrowsAsync<InvalidDataException>(()=>workBuddy
            ?WorkBuddyAcpServer.ReadMessagesAsync(stream,Receive,CancellationToken.None)
            :CodexAppServer.ReadMessagesAsync(stream,Receive,CancellationToken.None));
        Assert.DoesNotContain("ours",error.Message);
    }

    [Theory]
    [InlineData("{\"id\":1,\"error\":null}")]
    [InlineData("{\"id\":1,\"error\":{\"code\":\"secret-value\"}}")]
    [InlineData("{\"method\":42}")]
    public void InvalidRpcEnvelopeFailsWithoutEchoingPayload(string payload)
    {
        using var document=JsonDocument.Parse(payload);
        var error=Assert.Throws<InvalidDataException>(()=>JsonResponseGuard.Rpc(document.RootElement));
        Assert.DoesNotContain("secret-value",error.Message);
    }
}
