using System.Text.Json;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;
public sealed class ConversationHistoryService
{
    private static readonly SemaphoreSlim WriteGate=new(1,1);
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
    public async Task ClearAsync(CancellationToken token=default)
    {
        await WriteGate.WaitAsync(token).ConfigureAwait(false);
        try{token.ThrowIfCancellationRequested();if(File.Exists(_path))File.Delete(_path);}
        finally{WriteGate.Release();}
    }
    private void Log(string component,Exception exception){try{_logError?.Invoke(component,exception);}catch{}}
}
