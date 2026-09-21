// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net;
using System.Net.Http;
using System.Text.Json;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using Xunit;

namespace MewuAI.Tests;

// Synthetic Chat Completions responses follow DeepSeek's documented parallel
// content/reasoning_content fields; they do not replay a real account response.
public sealed class DeepSeekResponseIntegrationTests
{
    private const string Reasoning = "先核对语法，再给出示例。";

    public static TheoryData<bool, string> OrdinaryAnswers => new()
    {
        { false, "```csharp\nvar answer = 42;\n```" },
        { true, "```csharp\nvar answer = 42;\n```" },
        { false, "```python\nprint(42)\n```" },
        { true, "```python\nprint(42)\n```" },
        { false, "```markdown\n# 示例\n这是正文。\n```" },
        { true, "```markdown\n# 示例\n这是正文。\n```" },
        { false, "```\nvar answer = 42;\n```" },
        { true, "```\nvar answer = 42;\n```" },
        { false, "普通 JSON 示例：{\"answer\":\"示例值\",\"annotations\":[]}" },
        { true, "普通 JSON 示例：{\"answer\":\"示例值\",\"annotations\":[]}" }
    };

    [Theory]
    [MemberData(nameof(OrdinaryAnswers))]
    public async Task CompleteAnswerSurvivesAfterReasoningEvenWhenVisualJsonWasRequested(bool streaming, string answer)
    {
        var progress = new RecordingProgress();
        var result = await Provider(Response(answer, streaming)).SendAsync(new AiRequest
        {
            Prompt = "请解释示例", ExpectStructuredResponse = true,
            StreamingProgress = streaming ? progress : null
        }, TestContext.Current.CancellationToken);

        Assert.Equal(answer, result.Answer);
        Assert.Equal(Reasoning, result.Reasoning);
        Assert.Empty(result.Annotations);
        Assert.Null(AiResultValidation.GetEmptyAnswerMessage(result));
        if (streaming)
        {
            var receivedAnswer = string.Concat(progress.Values.Select(value => value.Content));
            Assert.Equal(answer, receivedAnswer);
            Assert.Equal(Reasoning, string.Concat(progress.Values.Select(value => value.ReasoningContent)));
            Assert.Equal(result.Answer, StructuredResponseParser.GetStreamingTextPreview(receivedAnswer));
        }
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task OfficialStringContentPreservesStructuredAnswerAndIndependentReasoning(bool streaming)
    {
        const string content = "{\"answer\":\"最终回答\",\"annotations\":[]}";
        var result = await Provider(Response(content, streaming)).SendAsync(new AiRequest
        {
            Prompt = "请回答", ExpectStructuredResponse = true,
            StreamingProgress = streaming ? new RecordingProgress() : null
        }, TestContext.Current.CancellationToken);

        Assert.Equal("最终回答", result.Answer);
        Assert.Equal(Reasoning, result.Reasoning);
        Assert.Empty(result.Annotations);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task ReasoningWithoutContentRemainsAnEmptyAnswerFailure(bool streaming)
    {
        var result = await Provider(Response(null, streaming)).SendAsync(new AiRequest
        {
            Prompt = "请回答", ExpectStructuredResponse = true,
            StreamingProgress = streaming ? new RecordingProgress() : null
        }, TestContext.Current.CancellationToken);

        Assert.Empty(result.Answer);
        Assert.Equal(Reasoning, result.Reasoning);
        Assert.NotNull(AiResultValidation.GetEmptyAnswerMessage(result));
    }

    [Fact]
    public void DeepSeekReasoningDeltasAreInProgressUntilAnswerArrives()
    {
        Assert.Equal(AiResponseStreamState.Waiting,
            AiResponseStreamStatePolicy.Classify(new AiStreamDelta("", "")));
        Assert.Equal(AiResponseStreamState.Thinking,
            AiResponseStreamStatePolicy.Classify(new AiStreamDelta("", Reasoning)));
        Assert.Equal(AiResponseStreamState.Answering,
            AiResponseStreamStatePolicy.Classify(new AiStreamDelta("最终回答", "")));
        Assert.Equal(AiResultValidation.EmptyAnswerKind.ReasoningOnly,
            AiResultValidation.ClassifyEmptyAnswer(new AiResult("", [], Reasoning)));
        Assert.Equal(AiResultValidation.EmptyAnswerKind.NoContent,
            AiResultValidation.ClassifyEmptyAnswer(new AiResult("", [])));
    }

    public static TheoryData<string> InvalidStructuredAnswers => new()
    {
        "```json\n{not valid}\n```",
        "```JSON\n{\"answer\":\"unfinished",
        "```json\n[{\"answer\":\"wrong root\"}]\n```",
        "```\n{\"message\":\"wrong field\"}\n```",
        "```\n[{\"answer\":\"wrong root\"}]\n```",
        "```\n{\"answer\":\"unfinished\n```",
        "```\njson\n{\"answer\":\"unfinished\n```",
        "```\njs\n```",
        "```\nvar answer = 42;",
        "```\nvar answer = 42;```",
        "```j",
        "{\"message\":\"wrong field\",\"annotations\":[]}"
    };

    [Theory]
    [MemberData(nameof(InvalidStructuredAnswers))]
    public async Task CompleteTransportDoesNotTurnBrokenProtocolIntoOrdinaryMarkdown(string content)
    {
        foreach (var streaming in new[] { false, true })
        {
            var result = await Provider(Response(content, streaming)).SendAsync(new AiRequest
            {
                Prompt = "请回答", ExpectStructuredResponse = true,
                StreamingProgress = streaming ? new RecordingProgress() : null
            }, TestContext.Current.CancellationToken);

            Assert.Empty(result.Answer);
            Assert.Equal(Reasoning, result.Reasoning);
            Assert.Empty(result.Annotations);
            Assert.NotNull(AiResultValidation.GetEmptyAnswerMessage(result));
        }
    }

    [Theory]
    [InlineData(null)]
    [InlineData("length")]
    public async Task VisibleMarkdownCannotBypassAnIncompleteStream(string? finish)
    {
        const string answer = "```csharp\nvar answer = 42;\n```";
        var response = Response(answer, streaming: true, finish: finish);
        if (finish == "length") response += "data: [DONE]\n\n";
        await Assert.ThrowsAsync<InvalidDataException>(() => Provider(response).SendAsync(new AiRequest
        {
            Prompt = "请解释示例", ExpectStructuredResponse = true, StreamingProgress = new RecordingProgress()
        }, TestContext.Current.CancellationToken));
    }

    [Fact]
    public async Task NonStreamingLengthFinishCannotBecomeACompleteAnswer()
    {
        await Assert.ThrowsAsync<InvalidDataException>(() => Provider(Response("```csharp\nvar answer", streaming: false, finish: "length"))
            .SendAsync(new AiRequest { Prompt = "请解释示例", ExpectStructuredResponse = true }, TestContext.Current.CancellationToken));
    }

    private static string Response(string? answer, bool streaming, string? finish = "stop")
    {
        if (!streaming)
            return JsonSerializer.Serialize(new { choices = new[] { new
            {
                index = 0, message = new { role = "assistant", content = answer, reasoning_content = Reasoning }, finish_reason = finish
            } } });

        // Null content while thinking is valid. The final fixture deliberately
        // carries its last text alongside stop to protect last-delta ordering.
        var reasoning = JsonSerializer.Serialize(new { choices = new[] { new
        {
            index = 0, delta = new { content = (string?)null, reasoning_content = Reasoning }, finish_reason = (string?)null
        } } });
        var content = JsonSerializer.Serialize(new { choices = new[] { new
        {
            index = 0, delta = new { content = answer, reasoning_content = (string?)null }, finish_reason = finish
        } } });
        return "data: " + reasoning + "\n\n" + "data: " + content + "\n\n";
    }

    private static OpenAiCompatibleProvider Provider(string response) => new(
        new AiProviderSettings { Type = "OpenAICompatible", BaseUrl = "https://api.deepseek.com/v1", Model = "deepseek-reasoner" },
        "synthetic-test-key",
        async (request, completionOption, token) =>
        {
            var rawBody=await request.Content!.ReadAsStringAsync(token);
            using var body = JsonDocument.Parse(rawBody);
            var recovery=rawBody.Contains("Recovery instruction",StringComparison.Ordinal);
            if(!recovery)
            {
                Assert.False(body.RootElement.TryGetProperty("thinking", out _));
                Assert.False(body.RootElement.TryGetProperty("reasoning_effort", out _));
            }
            return new HttpResponseMessage(HttpStatusCode.OK) { Content = new StringContent(response) };
        },
        _ => TimeSpan.FromSeconds(10));

    private sealed class RecordingProgress : IProgress<AiStreamDelta>
    {
        internal List<AiStreamDelta> Values { get; } = [];
        public void Report(AiStreamDelta value) => Values.Add(value);
    }
}
