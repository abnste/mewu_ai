using System.Text.RegularExpressions;

namespace mewu_ai_Assistant.Services;

public sealed class PrivacyLogger
{
    private static readonly object Gate=new();
    private static readonly Regex SensitiveAssignment=new(
        "(?i)\\b(?:authorization|proxy-authorization|api[_ -]?key|subscription[_ -]?key|access[_ -]?token|auth[_ -]?token|refresh[_ -]?token|client[_ -]?secret|private[_ -]?key|x[_ -]?(?:api[_ -]?)?key|token|jwt|secret|password|credential|cookie|signature)\\b\\s*[\\\"']?\\s*[:=]\\s*(?:(?:bearer|basic)\\s+)?(?:\\\"(?:\\\\.|[^\\\"\\\\])*\\\"|'(?:\\\\.|[^'\\\\])*'|[^\\\"'\\s,;\\&#]+)",
        RegexOptions.Compiled|RegexOptions.CultureInvariant);
    private static readonly Regex AuthorizationScheme=new(
        "(?i)\\b(?:bearer|basic)\\s+[^\\s,;\\&#]+",
        RegexOptions.Compiled|RegexOptions.CultureInvariant);
    private const long MaxFileBytes=4L*1024*1024;
    private const long MaxTotalBytes=20L*1024*1024;
    private const int MaxComponentCharacters=256;
    private const int MaxMessageCharacters=1000;
    private const int MaxStackTraceCharacters=16*1024;
    private readonly string _directory;
    public PrivacyLogger(string? directory=null){_directory=directory??Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","Logs");try{Directory.CreateDirectory(_directory);}catch{}lock(Gate)Rotate();}
    public void Error(string component,Exception exception)
    {
        try
        {
            // Stack traces are not inherently safe: custom exceptions and
            // generated async frames can include request URLs, query strings,
            // or header values.  Run the same redaction over every field before
            // writing it, while retaining a bounded trace for diagnostics.
            var type=Sanitize(exception.GetType().Name,MaxComponentCharacters);
            var message=Sanitize(exception.Message,MaxMessageCharacters);
            var stack=Sanitize(exception.StackTrace,MaxStackTraceCharacters);
            var line=$"{DateTimeOffset.Now:O} [{Sanitize(component,MaxComponentCharacters)}] {type}: {message}{Environment.NewLine}{stack}{Environment.NewLine}";
            lock(Gate){File.AppendAllText(CurrentPath(),line);Rotate();}
        }
        catch{}
    }
    public void Info(string component,string message)
    {
        try
        {
            var line=$"{DateTimeOffset.Now:O} [{Sanitize(component,MaxComponentCharacters)}] {Sanitize(message,MaxMessageCharacters)}{Environment.NewLine}";
            lock(Gate){File.AppendAllText(CurrentPath(),line);Rotate();}
        }
        catch{}
    }
    private string CurrentPath(){var stem=$"mewu-{DateTime.UtcNow:yyyyMMdd}";for(var i=0;i<100;i++){var path=Path.Combine(_directory,$"{stem}-{i:D2}.log");if(!File.Exists(path)||new FileInfo(path).Length<MaxFileBytes)return path;}return Path.Combine(_directory,$"{stem}-overflow.log");}
    private static string Sanitize(string? value,int maxLength=MaxMessageCharacters)
    {
        if(string.IsNullOrEmpty(value))return string.Empty;
        if(value.Length>maxLength)value=value[..maxLength];
        value=SensitiveAssignment.Replace(value,"[REDACTED]");
        return AuthorizationScheme.Replace(value,static match=>
            match.Value.StartsWith("basic",StringComparison.OrdinalIgnoreCase)
                ? "Basic [REDACTED]"
                : "Bearer [REDACTED]");
    }
    private void Rotate(){try{var files=Directory.EnumerateFiles(_directory,"*.log").Select(x=>new FileInfo(x)).OrderByDescending(x=>x.LastWriteTimeUtc).ToList();foreach(var file in files.Skip(14))file.Delete();files=Directory.EnumerateFiles(_directory,"*.log").Select(x=>new FileInfo(x)).OrderBy(x=>x.LastWriteTimeUtc).ToList();var total=files.Sum(x=>x.Length);foreach(var file in files){if(total<=MaxTotalBytes)break;total-=file.Length;file.Delete();}}catch{}}
}
