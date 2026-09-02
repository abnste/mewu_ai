using System.Text.RegularExpressions;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static partial class VideoAnnotationActionLabelFormatter
{
    internal static string Create(AiAnnotation annotation,VideoAnnotationAnswerActionKind kind)
    {
        var subject=CondenseSubject(annotation.Text);var start=FormatCompactTime(TimeSpan.FromSeconds(annotation.StartTime??annotation.Keyframes?.FirstOrDefault()?.Time??0));
        if(kind==VideoAnnotationAnswerActionKind.JumpToFrame)return $"{start}：{subject}";
        var end=FormatCompactTime(TimeSpan.FromSeconds(annotation.EndTime??annotation.StartTime??0));return $"{start}–{end}：{subject}";
    }

    internal static string FormatCompactTime(TimeSpan value)
    {
        var totalSeconds=Math.Max(0,Math.Round(value.TotalSeconds,1,MidpointRounding.AwayFromZero));var hours=(int)(totalSeconds/3600);var minutes=(int)(totalSeconds%3600/60);var remainder=totalSeconds-hours*3600-minutes*60;var seconds=remainder.ToString("0.#",System.Globalization.CultureInfo.InvariantCulture);if(hours>0)return $"{hours}时{minutes}分{seconds}秒";if(minutes>0)return $"{minutes}分{seconds}秒";return $"{seconds}秒";
    }

    internal static string CondenseSubject(string? value)
    {
        var text=WhitespaceRegex().Replace(value??string.Empty," ").Trim(' ','·','。','，',',',':','：');
        if(text.Contains("圈",StringComparison.Ordinal)&&text.Contains("文字",StringComparison.Ordinal))return "圈出的文字";
        if(text.Contains("下划线",StringComparison.Ordinal)&&text.Contains("文字",StringComparison.Ordinal))return "下划线文字";
        if(text.Contains("手机",StringComparison.Ordinal))
        {
            var color=ColorRegex().Match(text);if(color.Success)return color.Groups[1].Value+"手机";
            foreach(var known in new[]{"黑色","白色","红色","蓝色","粉色","银色","金色"})if(text.Contains(known,StringComparison.Ordinal))return known+"手机";
            if(text.Contains("受访者",StringComparison.Ordinal))return "受访者手机";
            return "手机";
        }
        var colon=text.IndexOfAny(['：',':']);if(colon>0)text=text[..colon];text=QuoteRegex().Replace(text,string.Empty).Trim();return text.Length switch{0=>"视频标记",<=14=>text,_=>text[..14]+"…"};
    }

    [GeneratedRegex(@"\s+")]
    private static partial Regex WhitespaceRegex();
    [GeneratedRegex("[（(]([^）)]{1,6}(?:色)?)[）)]")]
    private static partial Regex ColorRegex();
    [GeneratedRegex("[‘’“”'\"]")]
    private static partial Regex QuoteRegex();
}
