using System.Text.Json;
using mewu_ai_Assistant.Services;

var settingsService=new SettingsService();
var settings=settingsService.Load();
if(new EnvironmentProviderBootstrap().Import(settings))settingsService.Save(settings);
var requested=Environment.GetEnvironmentVariable("MEWU_SMOKE_PROVIDER");
if(!string.IsNullOrWhiteSpace(requested))settings.DefaultProviderId=settings.Providers.Last(x=>x.Name.Contains(requested,StringComparison.OrdinalIgnoreCase)).Id;
var path=await new ProviderVerificationService().VerifyAsync(settings,CancellationToken.None);
using var report=JsonDocument.Parse(await File.ReadAllTextAsync(path));
var root=report.RootElement;
Console.WriteLine(JsonSerializer.Serialize(new{
    connection=root.GetProperty("connection").GetBoolean(),
    text=root.GetProperty("text").GetBoolean(),
    streaming=root.GetProperty("streaming").GetBoolean(),
    image=root.GetProperty("image").GetBoolean(),
    errors=root.GetProperty("errors").EnumerateArray().Select(x=>x.GetString()).ToArray()
},new JsonSerializerOptions{WriteIndented=true}));
return root.GetProperty("connection").GetBoolean()&&root.GetProperty("text").GetBoolean()&&root.GetProperty("streaming").GetBoolean()&&root.GetProperty("image").GetBoolean()?0:1;
