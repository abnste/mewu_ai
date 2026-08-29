using System.Text.RegularExpressions;

namespace mewu_ai_Assistant.Services;

public sealed class PrivacyLogger
{
    private const long MaxFileBytes=4L*1024*1024;
    private const long MaxTotalBytes=20L*1024*1024;
    private readonly string _directory;
    public PrivacyLogger(string? directory=null){_directory=directory??Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","Logs");Directory.CreateDirectory(_directory);Rotate();}
    public void Error(string component,Exception exception){try{var line=$"{DateTimeOffset.Now:O} [{Sanitize(component)}] {exception.GetType().Name}: {Sanitize(exception.Message)}{Environment.NewLine}{exception.StackTrace}{Environment.NewLine}";File.AppendAllText(CurrentPath(),line);Rotate();}catch{}}
    private string CurrentPath(){var stem=$"mewu-{DateTime.UtcNow:yyyyMMdd}";for(var i=0;i<100;i++){var path=Path.Combine(_directory,$"{stem}-{i:D2}.log");if(!File.Exists(path)||new FileInfo(path).Length<MaxFileBytes)return path;}return Path.Combine(_directory,$"{stem}-overflow.log");}
    private static string Sanitize(string value){if(value.Length>1000)value=value[..1000];value=Regex.Replace(value,"(?i)\\b(?:authorization|api[_ -]?key)\\b\\s*[\"']?\\s*[:=]\\s*[\"']?\\s*(?:bearer\\s+)?[^\"'\\s,;]+","[REDACTED]");return Regex.Replace(value,"(?i)\\bbearer\\s+[^\\s,;]+","Bearer [REDACTED]");}
    private void Rotate(){try{var files=Directory.EnumerateFiles(_directory,"*.log").Select(x=>new FileInfo(x)).OrderByDescending(x=>x.LastWriteTimeUtc).ToList();foreach(var file in files.Skip(14))file.Delete();files=Directory.EnumerateFiles(_directory,"*.log").Select(x=>new FileInfo(x)).OrderBy(x=>x.LastWriteTimeUtc).ToList();var total=files.Sum(x=>x.Length);foreach(var file in files){if(total<=MaxTotalBytes)break;total-=file.Length;file.Delete();}}catch{}}
}
