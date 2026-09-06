// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
namespace mewu_ai_Assistant.Services;

internal static class SettingsChoicePolicy
{
    internal static IReadOnlyList<int> IncludeCurrent(IEnumerable<int> standardValues,int currentValue)=>
        standardValues.Append(currentValue).Distinct().OrderBy(value=>value).ToArray();
}
