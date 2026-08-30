using System;
using System.Collections.Specialized;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

internal static class ClipboardService
{
    internal const int RetryCount = 4;
    internal const int RetryDelayMilliseconds = 40;
    // The configured retention is applied at startup. While the app remains open, only prune
    // beyond the maximum supported setting so a 30-day preference is never shortened to 7 days.
    internal static readonly TimeSpan DefaultStagingMaxAge = TimeSpan.FromDays(30);
    private static readonly object FileDropGate = new();
    internal static string FileDropStagingDirectory => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "MewuAI",
        "Clipboard");

    internal static bool TrySetImage(BitmapSource image, out string? error)
    {
        ArgumentNullException.ThrowIfNull(image);
        return TryExecute(() => Clipboard.SetImage(image), out error);
    }

    internal static bool TrySetText(string text, out string? error)
    {
        ArgumentNullException.ThrowIfNull(text);
        return TryExecute(() => Clipboard.SetText(text), out error);
    }

    internal static bool TrySetFileDropList(string path, out string? error)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        return TrySetPersistentFileDropList(
            path,
            FileDropStagingDirectory,
            SetFileDropListCore,
            out _,
            out error,
            RetryCount,
            null,
            static (component,exception)=>new PrivacyLogger().Error(component,exception));
    }

    internal static async Task<ClipboardFileDropResult> TrySetFileDropListAsync(
        string path,
        CancellationToken cancellationToken = default)
        =>await TrySetPersistentFileDropListAsync(
            path,
            FileDropStagingDirectory,
            SetFileDropListCore,
            cancellationToken,
            RetryCount,
            null,
            static (component,exception)=>new PrivacyLogger().Error(component,exception));

    internal static async Task<ClipboardFileDropResult> TrySetPersistentFileDropListAsync(
        string path,
        string stagingDirectory,
        Action<string> clipboardSetter,
        CancellationToken cancellationToken = default,
        int retryCount = RetryCount,
        Action<int>? delay = null,
        Action<string,Exception>? logError = null)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        ArgumentException.ThrowIfNullOrWhiteSpace(stagingDirectory);
        ArgumentNullException.ThrowIfNull(clipboardSetter);
        string? candidate=null;
        try
        {
            candidate=await Task.Run(
                ()=>StagePersistentFileDrop(path,stagingDirectory),
                cancellationToken);
            cancellationToken.ThrowIfCancellationRequested();
            var stagedCandidate=candidate??throw new InvalidOperationException("未能创建剪贴板媒体副本");
            var copied=TryExecute(()=>clipboardSetter(stagedCandidate),out var error,retryCount,delay,logError);
            if(copied)return new(true,stagedCandidate,null);
            TryDelete(stagedCandidate);
            return new(false,null,error);
        }
        catch(OperationCanceledException)
        {
            TryDelete(candidate);
            throw;
        }
        catch(Exception ex)
        {
            try{logError?.Invoke("ClipboardStaging",ex);}catch{}
            TryDelete(candidate);
            var error=ex is FileNotFoundException
                ?"视频文件已不可用，请重新录制"
                :$"无法准备可持久复制的视频文件：{ex.Message}";
            return new(false,null,error);
        }
    }

    internal static bool TrySetPersistentFileDropList(
        string path,
        string stagingDirectory,
        Action<string> clipboardSetter,
        out string? stagedPath,
        out string? error,
        int retryCount = RetryCount,
        Action<int>? delay = null,
        Action<string,Exception>? logError = null)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        ArgumentException.ThrowIfNullOrWhiteSpace(stagingDirectory);
        ArgumentNullException.ThrowIfNull(clipboardSetter);
        string? candidate=null;
        try
        {
            candidate=StagePersistentFileDrop(path,stagingDirectory);
            var copied=TryExecute(()=>clipboardSetter(candidate),out error,retryCount,delay,logError);
            if(copied)
            {
                stagedPath=candidate;
                return true;
            }
        }
        catch(Exception ex)
        {
            try{logError?.Invoke("ClipboardStaging",ex);}catch{}
            error=ex is FileNotFoundException?"视频文件已不可用，请重新录制":$"无法准备可持久复制的视频文件：{ex.Message}";
        }

        TryDelete(candidate);
        stagedPath=null;
        return false;
    }

    private static string StagePersistentFileDrop(string path,string stagingDirectory)
    {
        var sourcePath=Path.GetFullPath(path);
        var stagingRoot=Path.GetFullPath(stagingDirectory);
        string? candidate=null;
        lock(FileDropGate)
        {
            try
            {
                if(!File.Exists(sourcePath))throw new FileNotFoundException("视频文件已不可用",sourcePath);
                Directory.CreateDirectory(stagingRoot);
                if(new DirectoryInfo(stagingRoot).Attributes.HasFlag(FileAttributes.ReparsePoint))throw new InvalidOperationException("剪贴板媒体目录不能是文件系统链接");
                CleanupStagedFilesCore(stagingRoot,DefaultStagingMaxAge,false,DateTime.UtcNow);
                var extension=SafeExtension(sourcePath);
                candidate=Path.Combine(stagingRoot,$"{DateTime.UtcNow:yyyyMMdd-HHmmss}-{Guid.NewGuid():N}{extension}");
                AtomicFileService.Copy(sourcePath,candidate);
                return candidate;
            }
            catch
            {
                TryDelete(candidate);
                throw;
            }
        }
    }

    internal static ClipboardStagingCleanupResult CleanupStagedFiles(
        TimeSpan age,
        string? stagingDirectory=null,
        bool throwOnFailure=false,
        DateTime? utcNow=null)
    {
        if(age<TimeSpan.Zero)throw new ArgumentOutOfRangeException(nameof(age));
        var directory=Path.GetFullPath(stagingDirectory??FileDropStagingDirectory);
        lock(FileDropGate)return CleanupStagedFilesCore(directory,age,throwOnFailure,utcNow??DateTime.UtcNow);
    }

    private static ClipboardStagingCleanupResult CleanupStagedFilesCore(string directory,TimeSpan age,bool throwOnFailure,DateTime utcNow)
    {
        if(!Directory.Exists(directory))return new ClipboardStagingCleanupResult(0,0);
        if(new DirectoryInfo(directory).Attributes.HasFlag(FileAttributes.ReparsePoint))
        {
            var exception=new InvalidOperationException("剪贴板媒体目录不能是文件系统链接");
            if(throwOnFailure)throw new AggregateException("无法清理剪贴板媒体目录",exception);
            return new ClipboardStagingCleanupResult(0,1);
        }

        var deleted=0;var failures=new List<Exception>();
        FileSystemInfo[] entries;
        try{entries=new DirectoryInfo(directory).EnumerateFileSystemInfos().ToArray();}
        catch(Exception ex)
        {
            if(throwOnFailure)throw new AggregateException("无法枚举剪贴板媒体目录",ex);
            return new ClipboardStagingCleanupResult(0,1);
        }

        foreach(var entry in entries)
        {
            if(utcNow-entry.LastWriteTimeUtc<=age)continue;
            try
            {
                if(entry is DirectoryInfo child)child.Delete(!child.Attributes.HasFlag(FileAttributes.ReparsePoint));
                else entry.Delete();
                deleted++;
            }
            catch(Exception ex){failures.Add(ex);}
        }

        if(throwOnFailure&&failures.Count>0)throw new AggregateException("部分剪贴板媒体未能清理",failures);
        return new ClipboardStagingCleanupResult(deleted,failures.Count);
    }

    private static string SafeExtension(string path)
    {
        var extension=Path.GetExtension(path);
        return extension.Length is >=2 and <=16&&extension[0]=='.'&&extension.Skip(1).All(char.IsAsciiLetterOrDigit)
            ?extension.ToLowerInvariant()
            :".bin";
    }

    private static void SetFileDropListCore(string path)
    {
        var files = new StringCollection { path };
        var data = new DataObject();
        data.SetFileDropList(files);
        Clipboard.SetDataObject(data, true);
    }

    private static void TryDelete(string? path)
    {
        if(string.IsNullOrWhiteSpace(path))return;
        try{if(File.Exists(path))File.Delete(path);}catch{}
    }

    internal static bool TryExecute(
        Action operation,
        out string? error,
        int retryCount = RetryCount,
        Action<int>? delay = null)
        =>TryExecute(operation,out error,retryCount,delay,static (component,exception)=>new PrivacyLogger().Error(component,exception));

    internal static bool TryExecute(
        Action operation,
        out string? error,
        int retryCount,
        Action<int>? delay,
        Action<string,Exception>? logError)
    {
        ArgumentNullException.ThrowIfNull(operation);
        ArgumentOutOfRangeException.ThrowIfLessThan(retryCount, 1);
        delay ??= Thread.Sleep;
        Exception? lastError = null;

        for (var attempt = 0; attempt < retryCount; attempt++)
        {
            try
            {
                operation();
                error = null;
                return true;
            }
            catch (ExternalException ex)
            {
                lastError = ex;
                if (attempt + 1 < retryCount)
                {
                    delay(RetryDelayMilliseconds);
                    continue;
                }
            }
            catch (Exception ex)
            {
                lastError = ex;
            }

            break;
        }

        if (lastError is not null)
        {
            try { logError?.Invoke("Clipboard", lastError); }
            catch { }
        }

        error = "系统剪贴板暂时不可用，请稍后重试";
        return false;
    }
}

internal readonly record struct ClipboardStagingCleanupResult(int DeletedCount,int FailureCount);
internal readonly record struct ClipboardFileDropResult(bool Success,string? StagedPath,string? Error);
