using System.Text.Json;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;
public sealed class ConversationHistoryService
{
    private readonly string _path;
    public ConversationHistoryService(){var dir=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","History");Directory.CreateDirectory(dir);_path=Path.Combine(dir,"conversations.jsonl");}
    public Task AppendAsync(string provider,string model,string prompt,string answer,CancellationToken token=default){var record=new{timestamp=DateTimeOffset.UtcNow,provider,model,prompt,answer};return File.AppendAllTextAsync(_path,JsonSerializer.Serialize(record)+Environment.NewLine,new System.Text.UTF8Encoding(false),token);}
    public void Append(string provider,string model,string prompt,string answer){var write=AppendAsync(provider,model,prompt,answer);_=write.ContinueWith(task=>_ = task.Exception,CancellationToken.None,TaskContinuationOptions.OnlyOnFaulted,TaskScheduler.Default);}
    public void Clear(){if(File.Exists(_path))File.Delete(_path);}
}
