// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.AI;

public static class AiResultValidation
{
    internal enum EmptyAnswerKind
    {
        None,
        NoContent,
        ReasoningOnly
    }

    internal static EmptyAnswerKind ClassifyEmptyAnswer(AiResult result)
    {
        ArgumentNullException.ThrowIfNull(result);
        if (!string.IsNullOrWhiteSpace(result.Answer)) return EmptyAnswerKind.None;
        return string.IsNullOrWhiteSpace(result.Reasoning)
            ? EmptyAnswerKind.NoContent
            : EmptyAnswerKind.ReasoningOnly;
    }

    public static string? GetEmptyAnswerMessage(AiResult result)
        => ClassifyEmptyAnswer(result) switch
        {
            EmptyAnswerKind.NoContent => "AI 未返回有效正文，请重试",
            EmptyAnswerKind.ReasoningOnly => "模型只返回了思考内容，未返回最终回答，请重试",
            _ => null
        };
}
