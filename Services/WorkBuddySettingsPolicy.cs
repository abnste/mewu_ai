// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
namespace mewu_ai_Assistant.Services;
internal static class WorkBuddySettingsPolicy
{
    internal static readonly HashSet<string> Efforts=new(StringComparer.Ordinal){ "disabled","minimal","low","medium","high","xhigh","max","enabled" };
    internal static void Validate(string model,string effort)
    {
        if(string.IsNullOrWhiteSpace(model)||model.Length>160||model.Any(char.IsControl)||!Efforts.Contains(effort))
            throw new InvalidOperationException("请在 WorkBuddy 页重新选择可用模型和思考程度。");
    }
}
