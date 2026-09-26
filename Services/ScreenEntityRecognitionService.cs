using System.Text.RegularExpressions;

namespace mewu_ai_Assistant.Services;

internal enum ScreenEntityType { Url, Email, Phone }
internal sealed record ScreenEntity(ScreenEntityType Type,string Value,string? MailProvider=null)
{
    public string DisplayProvider=>MailProvider switch { "qq"=>"QQ 邮箱", "netease"=>"网易邮箱", _=>"邮箱" };
}

internal static partial class ScreenEntityRecognitionService
{
    private const RegexOptions Options=RegexOptions.IgnoreCase|RegexOptions.CultureInvariant;
    [GeneratedRegex(@"(?<![\w@])https?://[^\s<>，。；：！？、]+",Options)] private static partial Regex UrlRegex();
    [GeneratedRegex(@"(?<![\w.+-])[A-Z0-9._%+-]+@[A-Z0-9-]+(?:\.[A-Z0-9-]+)*\.[A-Z]{2,}(?![\w-])",Options)] private static partial Regex EmailRegex();
    [GeneratedRegex(@"(?<![\d-])(?:\+?86[ -]?)?1[3-9]\d[ -]?\d{4}[ -]?\d{4}(?![\d-])",Options)] private static partial Regex PhoneRegex();
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
            var value=NormalizeEmailCandidate(match.Value);
            if(value.Length==0)continue;
            var domain=value[(value.LastIndexOf('@')+1)..].ToLowerInvariant();
            var provider=domain switch { "qq.com" or "foxmail.com"=>"qq", "163.com" or "126.com" or "yeah.net"=>"netease", _=>null };
            results.Add(new(ScreenEntityType.Email,value,provider));
        }
        foreach(Match match in PhoneRegex().Matches(text))
        {
            var value=Regex.Replace(match.Value,"[^0-9+]",string.Empty);
            if(value.StartsWith("86",StringComparison.Ordinal)&&value.Length==13)value="+"+value;
            results.Add(new(ScreenEntityType.Phone,value));
        }
        return results.GroupBy(entity=>$"{entity.Type}:{entity.Value}",StringComparer.OrdinalIgnoreCase).Select(group=>group.First()).Take(12).ToArray();
    }
    private static string NormalizeEmailCandidate(string candidate)
    {
        var value=candidate.TrimEnd('.',',',';',':','。','，','；','：');
        var at=value.LastIndexOf('@');
        if(at<=0||at==value.Length-1)return string.Empty;

        // OCR/UIA snapshots can concatenate the next word directly after a known
        // mailbox domain (for example "ing@gmail.comshuzi"). Keep the domain
        // boundary at the known suffix instead of presenting a bogus address.
        var domain=value[(at+1)..];
        var knownDomains=new[]{"gmail.com","outlook.com","hotmail.com","live.com","qq.com","foxmail.com","163.com","126.com","yeah.net"};
        var suffix=knownDomains.FirstOrDefault(domain.StartsWith);
        if(suffix is not null)value=value[..(at+1+suffix.Length)];
        return value;
    }
    private static string TrimPunctuation(string value)=>value.TrimEnd('.',',',';',':','!','?','。','，','；','：','！','？',')',']','}','）','】','》');
}
