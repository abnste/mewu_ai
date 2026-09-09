// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.Windows.Media;

namespace mewu_ai_Assistant.Services;

internal static class ThinkingGlowAppearance
{
    internal const string DefaultColor="#A7C7FF";
    internal static string NormalizeColor(string? value)
    {
        var text=value?.Trim();
        return text is {Length:7}&&text[0]=='#'&&text.AsSpan(1).IndexOfAnyExcept("0123456789abcdefABCDEF")<0
            ?text.ToUpperInvariant():DefaultColor;
    }
    internal static Color ParseColor(string? value)
    {
        var text=NormalizeColor(value);
        return Color.FromRgb(byte.Parse(text.AsSpan(1,2),NumberStyles.HexNumber,CultureInfo.InvariantCulture),
            byte.Parse(text.AsSpan(3,2),NumberStyles.HexNumber,CultureInfo.InvariantCulture),
            byte.Parse(text.AsSpan(5,2),NumberStyles.HexNumber,CultureInfo.InvariantCulture));
    }
}
