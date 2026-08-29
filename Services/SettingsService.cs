using System.Text.Json;
using System.Text.Json.Serialization;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;
public sealed class SettingsService
{
    private readonly string _path;
    private static readonly JsonSerializerOptions Options=new() { WriteIndented=true,Converters={new JsonStringEnumConverter()} };
    public SettingsService(string? path=null) { _path=path??Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","settings.json");Directory.CreateDirectory(Path.GetDirectoryName(_path)!); }
    public AppSettings Load()
    {
        try { return File.Exists(_path)?JsonSerializer.Deserialize<AppSettings>(File.ReadAllText(_path),Options)??new():new(); }
        catch { return new(); }
    }
    public void Save(AppSettings settings) { var temp=_path+".tmp"; File.WriteAllText(temp,JsonSerializer.Serialize(settings,Options)); File.Move(temp,_path,true); }
}
