// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;
internal static class CodexSettingsPolicy
{
    internal static void Validate(AppSettings settings)
    {
        if(!settings.CodexEnabled)return;
        if(string.IsNullOrWhiteSpace(settings.CodexModel)||settings.CodexModel.Length>160||settings.CodexModel.Any(char.IsControl))throw new InvalidOperationException("请在 Codex 页重新选择可用模型。");
        if(!new[]{"none","minimal","low","medium","high","xhigh","max","ultra"}.Contains(settings.CodexReasoningEffort,StringComparer.Ordinal))throw new InvalidOperationException("请在 Codex 页重新选择思考程度。");
    }
}
