using System.Text.Json;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;
public sealed class ConversationHistoryService
{
    private readonly string _path;
    public ConversationHistoryService(){var dir=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","History");Directory.CreateDirectory(dir);_path=Path.Combine(dir,"conversations.jsonl");}
    public void Append(string provider,string model,string prompt,string answer){var record=new{timestamp=DateTimeOffset.UtcNow,provider,model,prompt,answer};File.AppendAllText(_path,JsonSerializer.Serialize(record)+Environment.NewLine,new System.Text.UTF8Encoding(false));}
    public void Clear(){if(File.Exists(_path))File.Delete(_path);}
}
