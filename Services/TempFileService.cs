namespace mewu_ai_Assistant.Services;
public sealed class TempFileService
{
    public string DirectoryPath { get; }
    public TempFileService(string? directoryPath=null){DirectoryPath=directoryPath??Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","Temp");Directory.CreateDirectory(DirectoryPath);}
    public string NewFile(string extension)=>Path.Combine(DirectoryPath,$"{DateTime.UtcNow:yyyyMMdd-HHmmss}-{Guid.NewGuid():N}{extension}");
    public string NewDirectory(){var p=Path.Combine(DirectoryPath,$"frames-{Guid.NewGuid():N}");Directory.CreateDirectory(p);return p;}
    public void Cleanup(TimeSpan age){foreach(var f in Directory.EnumerateFiles(DirectoryPath)){try{if(DateTime.UtcNow-File.GetLastWriteTimeUtc(f)>age)File.Delete(f);}catch{}}foreach(var d in Directory.EnumerateDirectories(DirectoryPath)){try{if(DateTime.UtcNow-Directory.GetLastWriteTimeUtc(d)>age)Directory.Delete(d,true);}catch{}}}
}
