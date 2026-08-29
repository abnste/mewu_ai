namespace mewu_ai_Assistant.Services;
public sealed class PrivacyLogger
{
    private readonly string _directory=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","Logs");
    public PrivacyLogger(){Directory.CreateDirectory(_directory);Rotate();}
    public void Error(string component,Exception exception){try{var line=$"{DateTimeOffset.Now:O} [{component}] {exception.GetType().Name}: {Sanitize(exception.Message)}{Environment.NewLine}{exception.StackTrace}{Environment.NewLine}";File.AppendAllText(Path.Combine(_directory,$"mewu-{DateTime.UtcNow:yyyyMMdd}.log"),line);}catch{}}
    private static string Sanitize(string value){if(value.Length>1000)value=value[..1000];return System.Text.RegularExpressions.Regex.Replace(value,"(?i)(authorization|api[_ -]?key|bearer)\\s*[:=]?\\s*[^\\s,;]+","$1=[REDACTED]");}
    private void Rotate(){try{var files=Directory.EnumerateFiles(_directory,"*.log").OrderByDescending(File.GetLastWriteTimeUtc).ToList();foreach(var f in files.Skip(7))File.Delete(f);}catch{}}
}
