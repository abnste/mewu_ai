using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.AI;

public static class AiResultValidation
{
    public static string? GetEmptyAnswerMessage(AiResult result)
        => string.IsNullOrWhiteSpace(result.Answer)
            ? string.IsNullOrWhiteSpace(result.Reasoning)
                ? "AI 未返回有效正文，请重试"
                : "模型只返回了思考内容，未返回最终回答，请重试"
            : null;
}
