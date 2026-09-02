using System.Text.Json;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;

/// <summary>
/// One complete user/assistant exchange persisted in the local JSONL history.
/// Provider and model are kept with the exchange so profiles cannot silently
/// share a conversation context.
/// </summary>
public sealed record ConversationHistoryEntry(
    DateTimeOffset Timestamp,
    string Provider,
    string Model,
    string Prompt,
    string Answer);

public sealed class ConversationHistoryService
{
    private static readonly SemaphoreSlim WriteGate=new(1,1);
    private const int MaxReadRecords=100;
    private const int MaxReadLineCharacters=64*1024;
    private readonly string _path;
    private readonly Action<string,Exception>? _logError;
    private readonly Func<CancellationToken,Task> _beforeCommit;
    public ConversationHistoryService(string? path=null):this(path,static (component,exception)=>new PrivacyLogger().Error(component,exception),null){}
    internal ConversationHistoryService(string? path,Action<string,Exception>? logError):this(path,logError,null){}
    internal ConversationHistoryService(string? path,Action<string,Exception>? logError,Func<CancellationToken,Task>? beforeCommit)
    {
        _path=Path.GetFullPath(path??Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","History","conversations.jsonl"));
        _logError=logError;
        _beforeCommit=beforeCommit??(static _=>Task.CompletedTask);
        Directory.CreateDirectory(Path.GetDirectoryName(_path)!);
    }
    public async Task AppendAsync(string provider,string model,string prompt,string answer,CancellationToken token=default)
    {
        var record=new{timestamp=DateTimeOffset.UtcNow,provider,model,prompt,answer};
        await WriteGate.WaitAsync(token).ConfigureAwait(false);
        try
        {
            await _beforeCommit(token).ConfigureAwait(false);
            token.ThrowIfCancellationRequested();
            await File.AppendAllTextAsync(_path,JsonSerializer.Serialize(record)+Environment.NewLine,new System.Text.UTF8Encoding(false),CancellationToken.None).ConfigureAwait(false);
        }
        finally{WriteGate.Release();}
    }
    public async Task<bool> TryAppendAsync(string provider,string model,string prompt,string answer,CancellationToken token=default)
    {
        try{await AppendAsync(provider,model,prompt,answer,token).ConfigureAwait(false);return true;}
        catch(OperationCanceledException){return false;}
        catch(Exception ex){Log("ConversationHistory",ex);return false;}
    }

    /// <summary>
    /// Reads only the most recent, valid JSONL records. Reading shares the
    /// same gate as writes so a new overlay can never observe a half-written
    /// line. Malformed or oversized lines are skipped individually; one bad
    /// record must not make the whole history appear empty.
    /// </summary>
    public async Task<IReadOnlyList<ConversationHistoryEntry>> ReadRecentAsync(int maxRecords=24,CancellationToken token=default)
    {
        maxRecords=Math.Clamp(maxRecords,1,MaxReadRecords);
        await WriteGate.WaitAsync(token).ConfigureAwait(false);
        try
        {
            token.ThrowIfCancellationRequested();
            if(!File.Exists(_path))return [];

            var recent=new Queue<ConversationHistoryEntry>(maxRecords);
            var malformed=0;
            await using var stream=new FileStream(_path,FileMode.Open,FileAccess.Read,FileShare.Read,32*1024,FileOptions.Asynchronous|FileOptions.SequentialScan);
            using var reader=new StreamReader(stream,new System.Text.UTF8Encoding(false,true),detectEncodingFromByteOrderMarks:true);
            while(await reader.ReadLineAsync(token).ConfigureAwait(false) is { } line)
            {
                token.ThrowIfCancellationRequested();
                if(line.Length==0)continue;
                if(line.Length>MaxReadLineCharacters){malformed++;continue;}

                ConversationHistoryEntry? entry;
                try{entry=JsonSerializer.Deserialize<ConversationHistoryEntry>(line,JsonOptions);}
                catch(JsonException){malformed++;continue;}
                catch(ArgumentException){malformed++;continue;}
                catch(NotSupportedException){malformed++;continue;}
                if(entry is null||entry.Timestamp==default||string.IsNullOrWhiteSpace(entry.Provider)||string.IsNullOrWhiteSpace(entry.Prompt)||string.IsNullOrWhiteSpace(entry.Answer))
                {
                    malformed++;continue;
                }
                if(recent.Count==maxRecords)recent.Dequeue();
                recent.Enqueue(entry);
            }

            if(malformed>0)Log("ConversationHistoryRead",new InvalidDataException($"跳过 {malformed} 条无效历史记录"));
            return recent.ToArray();
        }
        finally{WriteGate.Release();}
    }

    private static readonly JsonSerializerOptions JsonOptions=new(){PropertyNameCaseInsensitive=true,MaxDepth=8};
    public async Task ClearAsync(CancellationToken token=default)
    {
        await WriteGate.WaitAsync(token).ConfigureAwait(false);
        try{token.ThrowIfCancellationRequested();if(File.Exists(_path))File.Delete(_path);}
        finally{WriteGate.Release();}
    }
    private void Log(string component,Exception exception){try{_logError?.Invoke(component,exception);}catch{}}
}
