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
        try { return Normalize(File.Exists(_path)?JsonSerializer.Deserialize<AppSettings>(File.ReadAllText(_path),Options)??new():new()); }
        catch { return new(); }
    }
    public void Save(AppSettings settings) { var temp=_path+".tmp"; File.WriteAllText(temp,JsonSerializer.Serialize(settings,Options)); File.Move(temp,_path,true); }
    private static AppSettings Normalize(AppSettings settings){settings.CaptureHotkey??=new();settings.Providers??=[];settings.DefaultImageFormat=string.IsNullOrWhiteSpace(settings.DefaultImageFormat)?"png":settings.DefaultImageFormat;settings.VoiceLanguage=string.IsNullOrWhiteSpace(settings.VoiceLanguage)?"system":settings.VoiceLanguage;settings.RecordingFps=Math.Clamp(settings.RecordingFps,10,60);settings.RecordingQuality=Math.Clamp(settings.RecordingQuality,20,100);settings.GifFps=Math.Clamp(settings.GifFps,1,15);settings.TempCleanupDays=Math.Clamp(settings.TempCleanupDays,1,30);settings.OverlayOpacity=Math.Clamp(settings.OverlayOpacity,.4,.75);foreach(var provider in settings.Providers){if(string.IsNullOrWhiteSpace(provider.Id))provider.Id=Guid.NewGuid().ToString("N");provider.Name??="Provider";provider.Type??="OpenAICompatible";provider.BaseUrl??="https://api.openai.com/v1";provider.Model??="";provider.CredentialId??="";provider.CustomHeaders??=[];}if(settings.DefaultProviderId is not null&&settings.Providers.All(x=>x.Id!=settings.DefaultProviderId))settings.DefaultProviderId=settings.Providers.FirstOrDefault()?.Id;return settings;}
}
