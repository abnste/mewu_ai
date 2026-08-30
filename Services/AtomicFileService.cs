namespace mewu_ai_Assistant.Services;

public static class AtomicFileService
{
    public static void Copy(string sourcePath,string destinationPath)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(sourcePath);
        ArgumentException.ThrowIfNullOrWhiteSpace(destinationPath);
        var source=Path.GetFullPath(sourcePath);
        var destination=Path.GetFullPath(destinationPath);
        if(string.Equals(source,destination,StringComparison.OrdinalIgnoreCase))
        {
            using var existingLease=TempMediaRegistry.Shared.AcquireExistingFile(source);
            return;
        }
        using var sourceLease=TempMediaRegistry.Shared.AcquireExistingFile(source);
        var directory=Path.GetDirectoryName(destination)??throw new InvalidOperationException("目标保存目录无效");
        if(!Directory.Exists(directory))throw new DirectoryNotFoundException($"目标保存目录不存在：{directory}");
        var temporary=Path.Combine(directory,$".{Path.GetFileName(destination)}.{Guid.NewGuid():N}.tmp");
        try
        {
            using(var input=new FileStream(source,FileMode.Open,FileAccess.Read,FileShare.Read))
            using(var output=new FileStream(temporary,FileMode.CreateNew,FileAccess.Write,FileShare.None))
            {
                input.CopyTo(output);
                output.Flush(true);
            }
            File.Move(temporary,destination,true);
        }
        finally
        {
            try{if(File.Exists(temporary))File.Delete(temporary);}catch{}
        }
    }
}
