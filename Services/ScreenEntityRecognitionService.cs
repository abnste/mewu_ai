using System.Text.RegularExpressions;

namespace mewu_ai_Assistant.Services;

internal enum ScreenEntityType { Url, Email }
internal sealed record ScreenEntity(ScreenEntityType Type,string Value,string? MailProvider=null)
{
    public string DisplayProvider=>MailProvider switch { "qq"=>"QQ 邮箱", "netease"=>"网易邮箱", _=>"邮箱" };
}

internal static partial class ScreenEntityRecognitionService
{
    private const RegexOptions Options=RegexOptions.IgnoreCase|RegexOptions.CultureInvariant;
    [GeneratedRegex(@"(?<![\w@])https?://[^\s<>，。；：！？、]+",Options)] private static partial Regex UrlRegex();
    [GeneratedRegex(@"(?<![\w.+-])[A-Z0-9._%+-]+@[A-Z0-9-]+(?:\.[A-Z0-9-]+)*\.[A-Z]{2,}(?![\w-])",Options)] private static partial Regex EmailRegex();
    internal static IReadOnlyList<ScreenEntity> Extract(string? text)
    {
        if(string.IsNullOrWhiteSpace(text))return [];
        var results=new List<ScreenEntity>();
        foreach(Match match in UrlRegex().Matches(text))
        {
            var value=TrimPunctuation(match.Value);
            if(Uri.TryCreate(value,UriKind.Absolute,out var uri)&&uri.Scheme is "http" or "https")results.Add(new(ScreenEntityType.Url,value));
        }
        foreach(Match match in EmailRegex().Matches(text))
        {
            var value=match.Value.TrimEnd('.',',',';',':','。','，','；','：');
            var domain=value[(value.LastIndexOf('@')+1)..].ToLowerInvariant();
            var provider=domain switch { "qq.com" or "foxmail.com"=>"qq", "163.com" or "126.com" or "yeah.net"=>"netease", _=>null };
            results.Add(new(ScreenEntityType.Email,value,provider));
        }
        return results.GroupBy(entity=>$"{entity.Type}:{entity.Value}",StringComparer.OrdinalIgnoreCase).Select(group=>group.First()).Take(12).ToArray();
    }
    private static string TrimPunctuation(string value)=>value.TrimEnd('.',',',';',':','!','?','。','，','；','：','！','？',')',']','}','）','】','》');
}
