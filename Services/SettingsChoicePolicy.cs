namespace mewu_ai_Assistant.Services;

internal static class SettingsChoicePolicy
{
    internal static IReadOnlyList<int> IncludeCurrent(IEnumerable<int> standardValues,int currentValue)=>
        standardValues.Append(currentValue).Distinct().OrderBy(value=>value).ToArray();
}
