namespace mewu_ai_Assistant.Services;
public sealed class TempFileService
{
    private readonly TempMediaRegistry _registry;
    public string DirectoryPath { get; }
    public TempFileService(string? directoryPath=null):this(directoryPath,TempMediaRegistry.Shared){}
    internal TempFileService(string? directoryPath,TempMediaRegistry registry)
    {
        _registry=registry??throw new ArgumentNullException(nameof(registry));
        DirectoryPath=Path.GetFullPath(directoryPath??Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","Temp"));Directory.CreateDirectory(DirectoryPath);
        if(new DirectoryInfo(DirectoryPath).Attributes.HasFlag(FileAttributes.ReparsePoint))throw new InvalidOperationException("临时媒体目录不能是文件系统链接");
    }
    public string NewFile(string extension){if(string.IsNullOrWhiteSpace(extension)||extension.Length is <2 or >16||extension[0]!='.'||extension.Skip(1).Any(c=>!char.IsAsciiLetterOrDigit(c)))throw new ArgumentException("临时文件扩展名无效",nameof(extension));return Path.Combine(DirectoryPath,$"{DateTime.UtcNow:yyyyMMdd-HHmmss}-{Guid.NewGuid():N}{extension}");}
    public TempFileCleanupResult Cleanup(TimeSpan age,bool throwOnFailure=false)
    {
        var deleteAll=age<=TimeSpan.Zero;
        var deleted=0;var skippedLeased=0;
        var failures=new List<Exception>();
        FileSystemInfo[] entries;
        try{entries=new DirectoryInfo(DirectoryPath).EnumerateFileSystemInfos().ToArray();}
        catch(Exception ex)
        {
            if(throwOnFailure)throw new AggregateException("无法枚举临时媒体目录",ex);
            return new TempFileCleanupResult(0,0,1);
        }
        foreach(var entry in entries)
        {
            try
            {
                if(!deleteAll&&DateTime.UtcNow-entry.LastWriteTimeUtc<=age)continue;
                var removed=_registry.TryExecuteIfUnleased(entry.FullName,entry is DirectoryInfo,()=>
                {
                    if(entry is DirectoryInfo directory)directory.Delete(!directory.Attributes.HasFlag(FileAttributes.ReparsePoint));
                    else entry.Delete();
                });
                if(removed)deleted++;else skippedLeased++;
            }
            catch(Exception ex){failures.Add(ex);}
        }
        if(throwOnFailure&&failures.Count>0)throw new AggregateException("部分临时媒体正在使用，未能全部清理",failures);
        return new TempFileCleanupResult(deleted,skippedLeased,failures.Count);
    }
}

public readonly record struct TempFileCleanupResult(int DeletedCount,int SkippedLeasedCount,int FailureCount);
