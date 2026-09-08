// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows.Media;

namespace mewu_ai_Assistant.Services;

internal static class AnnotationPalette
{
    internal const string DefaultColor="#5B85E8";
    internal static string ResolveColor(string color)=>string.Equals(color,"#2AAEFF",StringComparison.OrdinalIgnoreCase)?DefaultColor:color;
    internal static Color Resolve(string color)=>(Color)ColorConverter.ConvertFromString(ResolveColor(color));
    internal static readonly Brush Accent=CreateAccent();
    internal static readonly Brush Referenced=CreateBrush("#8396CB");
    internal static readonly Brush Inactive=CreateBrush("#90A6C5");
    internal static readonly System.Windows.Media.Effects.DropShadowEffect SelectionGlow=CreateGlow();
    private static Brush CreateBrush(string color){var brush=new SolidColorBrush(Resolve(color));brush.Freeze();return brush;}
    private static System.Windows.Media.Effects.DropShadowEffect CreateGlow(){var glow=new System.Windows.Media.Effects.DropShadowEffect{Color=Resolve(DefaultColor),BlurRadius=8,ShadowDepth=0,Opacity=.3};glow.Freeze();return glow;}
    private static Brush CreateAccent(){var brush=new SolidColorBrush(Resolve(DefaultColor));brush.Freeze();return brush;}
}
