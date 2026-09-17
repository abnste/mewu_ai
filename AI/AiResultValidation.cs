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

    /// <summary>Safe, user-facing detail for an incomplete provider response.</summary>
    public static string? GetEmptyAnswerGuidance(AiResult result)
        => ClassifyEmptyAnswer(result) switch
        {
            EmptyAnswerKind.ReasoningOnly => "回复不完整：模型/API 返回了推理字段，但未提供最终正文。本轮不会写入对话历史或后续上下文；可直接重试，或更换支持当前输入的模型。",
            EmptyAnswerKind.NoContent => "回复不完整：模型/API 没有提供可显示的正文。本轮不会写入对话历史或后续上下文；请重试并检查模型与 API 配置。",
            _ => null
        };
}
