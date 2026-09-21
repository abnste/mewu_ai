// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
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
    string Answer)
{
    [System.Text.Json.Serialization.JsonIgnore]
    public AiMessage? ContinuationMessage { get; init; }
    /// <summary>Stable archive key. Empty values are legacy flat records.</summary>
    [System.Text.Json.Serialization.JsonIgnore(Condition=System.Text.Json.Serialization.JsonIgnoreCondition.WhenWritingNull)]
    public string? SessionId { get; init; }
    [System.Text.Json.Serialization.JsonIgnore(Condition=System.Text.Json.Serialization.JsonIgnoreCondition.WhenWritingNull)]
    public string? SessionTitle { get; init; }
}

/// <summary>
/// A conversation-level projection used by archive surfaces. Keeping the
/// grouping and fallback title policy in the history service ensures the
/// launcher and capture overlay present the same sessions.
/// </summary>
public sealed record ConversationSessionArchive(
    string Id,
    string Title,
    DateTimeOffset LastUpdated,
    string Provider,
    string Model,
    IReadOnlyList<ConversationHistoryEntry> Entries)
{
    public int TurnCount=>Entries.Count;
    public string LastPrompt=>Entries.Count==0?string.Empty:Entries[^1].Prompt;
}

public sealed class ConversationHistoryService
{
    private static readonly SemaphoreSlim WriteGate=new(1,1);
    private const int MaxReadRecords=100;
    private const int MaxReadLineCharacters=64*1024;
    internal const int MaxReadBytes=8*1024*1024;
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
    public Task AppendAsync(string provider,string model,string prompt,string answer,CancellationToken token=default)
        =>AppendAsync(provider,model,prompt,answer,null,null,token);

    public async Task AppendAsync(string provider,string model,string prompt,string answer,string? sessionId,string? sessionTitle,CancellationToken token=default)
    {
        var record=new ConversationHistoryEntry(DateTimeOffset.UtcNow,provider,model,prompt,answer)
        {
            SessionId=NormalizeSessionId(sessionId),SessionTitle=NormalizeSessionTitle(sessionTitle)
        };
        await WriteGate.WaitAsync(token).ConfigureAwait(false);
        try
        {
            await _beforeCommit(token).ConfigureAwait(false);
            token.ThrowIfCancellationRequested();
            await File.AppendAllTextAsync(_path,JsonSerializer.Serialize(record,JsonOptions)+Environment.NewLine,new System.Text.UTF8Encoding(false),CancellationToken.None).ConfigureAwait(false);
        }
        finally{WriteGate.Release();}
    }
    public Task<bool> TryAppendAsync(string provider,string model,string prompt,string answer,CancellationToken token=default)
        =>TryAppendAsync(provider,model,prompt,answer,null,null,token);

    public async Task<bool> TryAppendAsync(string provider,string model,string prompt,string answer,string? sessionId,string? sessionTitle,CancellationToken token=default)
    {
        try{await AppendAsync(provider,model,prompt,answer,sessionId,sessionTitle,token).ConfigureAwait(false);return true;}
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
            await SeekToRecentRecordsAsync(stream,token).ConfigureAwait(false);
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

    public async Task<IReadOnlyList<ConversationSessionArchive>> ReadSessionsAsync(int maxSessions=24,CancellationToken token=default)
    {
        maxSessions=Math.Clamp(maxSessions,1,48);
        // ReadRecentAsync is intentionally bounded, so the launcher remains
        // fast even if an older installation has accumulated a large JSONL.
        var records=await ReadRecentAsync(MaxReadRecords,token).ConfigureAwait(false);
        return CreateSessionArchive(records,maxSessions);
    }

    internal static IReadOnlyList<ConversationSessionArchive> CreateSessionArchive(
        IEnumerable<ConversationHistoryEntry> records,
        int maxSessions=24)
    {
        ArgumentNullException.ThrowIfNull(records);
        maxSessions=Math.Clamp(maxSessions,1,48);
        var normalized=records
            .Where(static entry=>entry is not null)
            .GroupBy(static entry=>$"{ArchiveKey(entry)}\n{entry.Prompt}\n{entry.Answer}",StringComparer.Ordinal)
            .Select(static group=>group.OrderByDescending(static entry=>entry.Timestamp).First());
        return normalized
            .GroupBy(static entry=>ArchiveKey(entry),StringComparer.Ordinal)
            .Select(static group=>
            {
                var entries=group.OrderBy(static entry=>entry.Timestamp).ToArray();
                var latest=entries[^1];
                return new ConversationSessionArchive(
                    NormalizeArchiveId(latest),
                    BuildArchiveTitle(latest),
                    latest.Timestamp,
                    latest.Provider,
                    latest.Model,
                    entries);
            })
            .OrderByDescending(static session=>session.LastUpdated)
            .Take(maxSessions)
            .ToArray();
    }

    private static string ArchiveKey(ConversationHistoryEntry entry)
        =>string.IsNullOrWhiteSpace(entry.SessionId)
            ?$"legacy\n{entry.Provider}\n{entry.Model}"
            :$"session\n{entry.Provider}\n{entry.Model}\n{entry.SessionId.Trim()}";
    private static string NormalizeArchiveId(ConversationHistoryEntry entry)
        =>string.IsNullOrWhiteSpace(entry.SessionId)?string.Empty:entry.SessionId.Trim();
    private static string BuildArchiveTitle(ConversationHistoryEntry entry)
    {
        var title=string.IsNullOrWhiteSpace(entry.SessionTitle)?entry.Prompt:entry.SessionTitle;
        var value=(title??string.Empty).Replace('\r',' ').Replace('\n',' ').Trim();
        while(value.Contains("  ",StringComparison.Ordinal))value=value.Replace("  "," ",StringComparison.Ordinal);
        if(value.Length==0)return LocalizationService.T("未命名会话","Untitled conversation");
        return value[..Math.Min(48,value.Length)];
    }

    private static async Task SeekToRecentRecordsAsync(FileStream stream,CancellationToken token)
    {
        if(stream.Length<=MaxReadBytes)return;
        stream.Seek(-MaxReadBytes,SeekOrigin.End);
        // Find the next complete JSONL line in bytes before decoding UTF-8.
        // A byte-budget boundary may split either a record or a Chinese/emoji
        // character. Decoding that partial prefix would corrupt the whole read.
        var buffer=new byte[4096];
        int count;
        while((count=await stream.ReadAsync(buffer,token).ConfigureAwait(false))>0)
        {
            var newline=Array.IndexOf(buffer,(byte)'\n',0,count);
            if(newline<0)continue;
            stream.Seek(newline+1-count,SeekOrigin.Current);
            return;
        }
    }

    private static readonly JsonSerializerOptions JsonOptions=new(){PropertyNameCaseInsensitive=true,PropertyNamingPolicy=JsonNamingPolicy.CamelCase,MaxDepth=8};
    private static string? NormalizeSessionId(string? value)=>string.IsNullOrWhiteSpace(value)?null:value.Trim()[..Math.Min(96,value.Trim().Length)];
    private static string? NormalizeSessionTitle(string? value)=>string.IsNullOrWhiteSpace(value)?null:value.Trim()[..Math.Min(120,value.Trim().Length)];
    public async Task ClearAsync(CancellationToken token=default)
    {
        await WriteGate.WaitAsync(token).ConfigureAwait(false);
        try{token.ThrowIfCancellationRequested();if(File.Exists(_path))File.Delete(_path);}
        finally{WriteGate.Release();}
    }
    private void Log(string component,Exception exception){try{_logError?.Invoke(component,exception);}catch{}}
}
