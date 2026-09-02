using System.Text.Json;
using System.Text.Json.Serialization;
using System.Text;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;
public sealed class SettingsService
{
    private static readonly string[] HermesReasoningEfforts=["none","minimal","low","medium","high","xhigh","max","ultra"];
    private readonly string _path;
    private readonly ProviderHeaderCredentialService _headerCredentials;
    private readonly Action<string,Exception>? _logError;
    private static readonly JsonSerializerOptions Options=new() { WriteIndented=true,Converters={new JsonStringEnumConverter()} };
    public SettingsService(string? path=null,ProviderHeaderCredentialService? headerCredentials=null)
        :this(path,headerCredentials,static (component,exception)=>new PrivacyLogger().Error(component,exception)){}
    internal SettingsService(string? path,ProviderHeaderCredentialService? headerCredentials,Action<string,Exception>? logError)
    {
        var configuredPath=path??Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","settings.json");
        if(string.IsNullOrWhiteSpace(configuredPath))throw new ArgumentException("设置文件路径不能为空",nameof(path));
        // Resolve relative paths before deriving the directory. Path.GetDirectoryName
        // returns null for a file name such as "settings.json", which previously
        // made the otherwise valid overload fail during construction.
        _path=Path.GetFullPath(configuredPath);
        var directory=Path.GetDirectoryName(_path)??throw new ArgumentException("设置文件路径无效",nameof(path));
        Directory.CreateDirectory(directory);
        _headerCredentials=headerCredentials??new ProviderHeaderCredentialService(new CredentialService(Path.Combine(directory,"Credentials")));
        _logError=logError;
    }
    public AppSettings Load()
    {
        if(!File.Exists(_path))return CreateDefaults();
        AppSettings settings;
        try
        {
            settings=JsonSerializer.Deserialize<AppSettings>(File.ReadAllText(_path),Options)??throw new JsonException("设置文件根节点不能为 null");
            NormalizeExisting(settings);
        }
        catch(Exception ex)
        {
            Log("SettingsLoad",ex);
            settings=NormalizeCommon(new AppSettings());
            settings.ConfigurationErrors.Add("设置文件无法解析，请打开设置并重新保存有效配置");
            return settings;
        }
        // Never rewrite a structurally invalid existing settings file during
        // credential migration. The original bytes must remain available for
        // explicit repair in the settings UI.
        if(settings.ConfigurationErrors.Count>0)return settings;
        var createdCredentialIds=new HashSet<string>(StringComparer.Ordinal);
        var migrationCommitted=false;
        try
        {
            var originalCredentialIds=ReferencedHeaderCredentialIds(settings.Providers);
            var migratedProviders=settings.Providers.Select(ProviderHeaderCredentialService.Clone).ToList();
            var migrated=migratedProviders.Aggregate(false,(changed,provider)=>_headerCredentials.MigratePlaintextHeaders(provider,createdCredentialIds)||changed);
            if(migrated)
            {
                var originalProviders=settings.Providers;settings.Providers=migratedProviders;
                try{SaveCore(settings);migrationCommitted=true;}
                catch{settings.Providers=originalProviders;throw;}
                var retainedCredentialIds=ReferencedCredentialIds(migratedProviders);
                foreach(var credentialId in originalCredentialIds.Except(retainedCredentialIds,StringComparer.OrdinalIgnoreCase))TryDeleteHeaderCredential(credentialId,"ProviderHeaderMigrationCleanup");
            }
        }
        catch(Exception ex)
        {
            if(!migrationCommitted)foreach(var credentialId in createdCredentialIds)TryDeleteHeaderCredential(credentialId,"ProviderHeaderMigrationRollback");
            Log("ProviderHeaderMigration",ex);
            settings.HasSensitiveCredentialErrors=true;
            settings.ConfigurationErrors.Add("敏感 Header 加密迁移失败，请在设置中重新输入并保存");
        }
        return settings;
    }
    public void Save(AppSettings settings)
    {
        ValidateForSave(settings);
        SaveCore(settings);
    }

    internal static void ValidateForSave(AppSettings settings)
    {
        ArgumentNullException.ThrowIfNull(settings);
        if(settings.Providers is not {Count:>0})throw new InvalidOperationException("至少需要一个 AI Provider");
        var providerIds=new HashSet<string>(StringComparer.Ordinal);
        foreach(var provider in settings.Providers)
        {
            if(provider is null)throw new InvalidOperationException("Provider 列表不能包含空项");
            if(string.IsNullOrWhiteSpace(provider.Id))throw new InvalidOperationException("Provider ID 不能为空");
            if(!providerIds.Add(provider.Id))throw new InvalidOperationException($"Provider ID 不能重复：{provider.Id}");
            if(!string.Equals(provider.Type,"MiniMax",StringComparison.OrdinalIgnoreCase)&&!string.Equals(provider.Type,"OpenAICompatible",StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException($"不支持的 Provider 类型：{provider.Type}");
            if(string.IsNullOrWhiteSpace(provider.Model))throw new InvalidOperationException($"{provider.Name} 的 Model 不能为空");
            _=ProviderEndpointPolicy.NormalizeBaseUri(provider.BaseUrl);
            ProviderHeaderPolicy.EnsureSafeToPersist(provider.CustomHeaders??throw new InvalidOperationException($"{provider.Name} 的 Custom Headers 不能为空"));
            if(provider.SensitiveHeaderCredentialIds is null)throw new InvalidOperationException($"{provider.Name} 的敏感 Header 凭据映射不能为空");
            ProviderHeaderPolicy.EnsureCredentialMappingsValid(provider.CustomHeaders,provider.SensitiveHeaderCredentialIds);
        }
        if(string.IsNullOrWhiteSpace(settings.DefaultProviderId)||!providerIds.Contains(settings.DefaultProviderId))throw new InvalidOperationException("默认 Provider 必须指向现有 Provider");
        // Hermes is an explicit, fail-closed conversation route.  It must not
        // force users to keep an otherwise unused remote API credential merely
        // so the settings document can be saved.  The Provider entry remains
        // structurally valid for features that still use it, but its
        // authentication is checked only when that route is active.
        if(!settings.HermesEnabled)
            ProviderAuthenticationPolicy.EnsureStoredCredentialReferences(settings.Providers.Single(provider=>provider.Id==settings.DefaultProviderId));
        ValidateHermesForSave(settings);
    }

    private void SaveCore(AppSettings settings)
    {
        var directory=Path.GetDirectoryName(_path)!;
        var temp=Path.Combine(directory,$".{Path.GetFileName(_path)}.{Guid.NewGuid():N}.tmp");
        try
        {
            File.WriteAllText(temp,JsonSerializer.Serialize(settings,Options),new UTF8Encoding(false));
            File.Move(temp,_path,true);
        }
        finally
        {
            try{if(File.Exists(temp))File.Delete(temp);}catch{}
        }
    }

    private static AppSettings CreateDefaults()
    {
        var settings=NormalizeCommon(new AppSettings());
        var provider=new AiProviderSettings();settings.Providers=[provider];settings.DefaultProviderId=provider.Id;
        return settings;
    }

    private static void NormalizeExisting(AppSettings settings)
    {
        NormalizeCommon(settings);
        settings.ConfigurationErrors.Clear();
        if(settings.Providers is null)
        {
            settings.Providers=[];
            settings.ConfigurationErrors.Add("Provider 列表不能为空");
        }
        else
        {
            var providers=new List<AiProviderSettings>(settings.Providers.Count);
            foreach(var provider in settings.Providers)
            {
                if(provider is null)
                {
                    settings.ConfigurationErrors.Add("Provider 列表包含无效空项");
                    continue;
                }
                provider.Id??=string.Empty;provider.Name??="Provider";provider.Type??=string.Empty;provider.BaseUrl??=string.Empty;provider.Model??=string.Empty;provider.CredentialId??=string.Empty;provider.CustomHeaders??=[];provider.SensitiveHeaderCredentialIds??=[];
                providers.Add(provider);
            }
            settings.Providers=providers;
        }

        AppendProviderConfigurationErrors(settings);
    }

    internal static void RevalidateProviderConfiguration(AppSettings settings)
    {
        ArgumentNullException.ThrowIfNull(settings);
        settings.ConfigurationErrors.Clear();
        AppendProviderConfigurationErrors(settings);
    }

    private static void AppendProviderConfigurationErrors(AppSettings settings)
    {
        var providerIds=new HashSet<string>(StringComparer.Ordinal);
        foreach(var provider in settings.Providers)
        {
            var name=string.IsNullOrWhiteSpace(provider.Name)?"Provider":provider.Name;
            if(string.IsNullOrWhiteSpace(provider.Id))settings.ConfigurationErrors.Add("Provider ID 不能为空");
            else if(!providerIds.Add(provider.Id))settings.ConfigurationErrors.Add($"Provider ID 重复：{provider.Id}");
            if(!string.Equals(provider.Type,"MiniMax",StringComparison.OrdinalIgnoreCase)&&!string.Equals(provider.Type,"OpenAICompatible",StringComparison.OrdinalIgnoreCase))settings.ConfigurationErrors.Add($"{name} 的 Provider 类型无效");
            if(string.IsNullOrWhiteSpace(provider.BaseUrl))settings.ConfigurationErrors.Add($"{name} 的 Base URL 不能为空");
            else
            {
                try{_=ProviderEndpointPolicy.NormalizeBaseUri(provider.BaseUrl);}
                catch(InvalidOperationException){settings.ConfigurationErrors.Add($"{name} 的 Base URL 无效");}
            }
            if(string.IsNullOrWhiteSpace(provider.Model))settings.ConfigurationErrors.Add($"{name} 的 Model 不能为空");
        }
        if(string.IsNullOrWhiteSpace(settings.DefaultProviderId))settings.ConfigurationErrors.Add("尚未选择默认 AI Provider");
        else if(settings.Providers.All(provider=>provider.Id!=settings.DefaultProviderId))settings.ConfigurationErrors.Add("默认 AI Provider 已不存在，请重新选择");
        AppendHermesConfigurationErrors(settings);
    }

    private static AppSettings NormalizeCommon(AppSettings settings)
    {
        settings.CaptureHotkey??=new();
        if(settings.CaptureHotkey.Key is < System.Windows.Input.Key.A or > System.Windows.Input.Key.Z)settings.CaptureHotkey.Key=System.Windows.Input.Key.A;
        const System.Windows.Input.ModifierKeys allowed=System.Windows.Input.ModifierKeys.Control|System.Windows.Input.ModifierKeys.Shift|System.Windows.Input.ModifierKeys.Alt;
        settings.CaptureHotkey.Modifiers&=allowed;if(settings.CaptureHotkey.Modifiers==System.Windows.Input.ModifierKeys.None)settings.CaptureHotkey.Modifiers=System.Windows.Input.ModifierKeys.Shift|System.Windows.Input.ModifierKeys.Alt;
        if(settings.CaptureHotkey.Key==System.Windows.Input.Key.A && settings.CaptureHotkey.Modifiers==(System.Windows.Input.ModifierKeys.Control|System.Windows.Input.ModifierKeys.Shift))
            settings.CaptureHotkey=new(){Key=System.Windows.Input.Key.S,Modifiers=System.Windows.Input.ModifierKeys.Shift|System.Windows.Input.ModifierKeys.Alt};
        settings.CaptureDelaySeconds=settings.CaptureDelaySeconds is 3 or 5?settings.CaptureDelaySeconds:0;
        settings.DefaultImageFormat=settings.DefaultImageFormat?.Trim().ToLowerInvariant() is "jpg" or "jpeg"?"jpg":"png";
        settings.VoiceLanguage=settings.VoiceLanguage?.Trim() is "zh-CN" or "en-US"?settings.VoiceLanguage.Trim():"system";
        settings.HermesProvider=settings.HermesProvider?.Trim()??string.Empty;
        settings.HermesProfile=string.IsNullOrWhiteSpace(settings.HermesProfile)?"default":settings.HermesProfile.Trim();
        settings.HermesModel=settings.HermesModel?.Trim()??string.Empty;
        settings.HermesReasoningEffort=string.IsNullOrWhiteSpace(settings.HermesReasoningEffort)?"medium":settings.HermesReasoningEffort.Trim().ToLowerInvariant();
        settings.RecordingFps=Math.Clamp(settings.RecordingFps,10,60);settings.RecordingQuality=Math.Clamp(settings.RecordingQuality,20,100);settings.GifFps=Math.Clamp(settings.GifFps,1,15);settings.TempCleanupDays=Math.Clamp(settings.TempCleanupDays,1,30);settings.OverlayOpacity=double.IsFinite(settings.OverlayOpacity)?Math.Clamp(settings.OverlayOpacity,.4,.75):.6;if(!settings.EnableVoiceInput)settings.AutomaticallyStartListening=false;
        return settings;
    }

    private static void ValidateHermesForSave(AppSettings settings)
    {
        if(!settings.HermesEnabled)return;
        var errors=HermesConfigurationErrors(settings).ToList();
        if(errors.Count>0)throw new InvalidOperationException(string.Join("；",errors));
    }

    private static void AppendHermesConfigurationErrors(AppSettings settings)
    {
        if(!settings.HermesEnabled)return;
        settings.ConfigurationErrors.AddRange(HermesConfigurationErrors(settings));
    }

    private static IEnumerable<string> HermesConfigurationErrors(AppSettings settings)
    {
        if(string.IsNullOrWhiteSpace(settings.HermesProfile))yield return "本机 Hermes 尚未选择 Agent / 人格";
        else if(!IsValidHermesProfile(settings.HermesProfile))yield return "本机 Hermes Agent / 人格无效，请从列表重新选择";
        if(string.IsNullOrWhiteSpace(settings.HermesProvider))yield return "本机 Hermes 尚未选择 Provider";
        else if(ContainsUnsafeHermesToken(settings.HermesProvider))yield return "本机 Hermes Provider 无效，请从模型列表重新选择";
        if(string.IsNullOrWhiteSpace(settings.HermesModel))yield return "本机 Hermes 尚未选择模型";
        else if(ContainsUnsafeHermesToken(settings.HermesModel))yield return "本机 Hermes 模型无效，请从模型列表重新选择";
        if(!HermesReasoningEfforts.Contains(settings.HermesReasoningEffort,StringComparer.Ordinal))
            yield return "本机 Hermes 思考程度无效，请重新选择";
    }

    private static bool ContainsUnsafeHermesToken(string value)=>
        value.StartsWith('-')||
        value.Contains("--",StringComparison.Ordinal)||
        value.Any(character=>char.IsWhiteSpace(character)||char.IsControl(character)||character is '\"' or '\'' or '\\');

    private static bool IsValidHermesProfile(string value)
    {
        try{return string.Equals(HermesRuntimeService.NormalizeProfile(value),value.Trim(),StringComparison.Ordinal);}
        catch(InvalidOperationException){return false;}
    }

    private static HashSet<string> ReferencedHeaderCredentialIds(IEnumerable<AiProviderSettings> providers)=>
        providers
            .SelectMany(provider=>provider.SensitiveHeaderCredentialIds is null?Enumerable.Empty<string>():provider.SensitiveHeaderCredentialIds.Values)
            .Where(id=>!string.IsNullOrWhiteSpace(id))
            .ToHashSet(StringComparer.OrdinalIgnoreCase);

    private static HashSet<string> ReferencedCredentialIds(IEnumerable<AiProviderSettings> providers)
    {
        var result=ReferencedHeaderCredentialIds(providers);
        foreach(var provider in providers)
            if(!string.IsNullOrWhiteSpace(provider.CredentialId))result.Add(provider.CredentialId);
        return result;
    }

    private void TryDeleteHeaderCredential(string credentialId,string component)
    {
        try{_headerCredentials.DeleteCredential(credentialId);}
        catch(Exception ex){Log(component,ex);}
    }

    private void Log(string component,Exception exception){try{_logError?.Invoke(component,exception);}catch{}}
}
