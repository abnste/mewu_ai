using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

public sealed class EnvironmentProviderBootstrap
{
    private static readonly ImportDefinition[] Definitions=
    [
        new("XAI_API_KEY","xAI Vision","OpenAICompatible","https://api.x.ai/v1","grok-4.6"),
        new("MINIMAX_CN_API_KEY","MiniMax M3","MiniMax","https://api.minimaxi.com/v1","MiniMax-M3"),
        new("VOLCENGINE_ARK_API_KEY","Volcengine Ark","OpenAICompatible",VolcengineModelPolicy.StandardBaseUrl,"doubao-seed-2-1-pro-260628"),
        new("VOLCENGINE_AGENTPLAN_API_KEY","Volcengine Agent Plan","OpenAICompatible","https://ark.cn-beijing.volces.com/api/plan/v3","doubao-seed-2-0-pro-260215")
    ];

    private readonly CredentialService _credentials;
    private readonly Func<string,string?> _readEnvironment;
    private readonly Func<AppSettings,CancellationToken,Task<string>> _verifyRequired;
    private readonly Action<string,Exception>? _logError;

    public EnvironmentProviderBootstrap()
        :this(
            new CredentialService(),
            Environment.GetEnvironmentVariable,
            static (settings,token)=>new ProviderVerificationService().VerifyRequiredAsync(settings,token),
            static (component,exception)=>new PrivacyLogger().Error(component,exception)){}

    internal EnvironmentProviderBootstrap(
        CredentialService credentials,
        Func<string,string?> readEnvironment,
        Func<AppSettings,CancellationToken,Task<string>> verifyRequired,
        Action<string,Exception>? logError=null)
    {
        _credentials=credentials??throw new ArgumentNullException(nameof(credentials));
        _readEnvironment=readEnvironment??throw new ArgumentNullException(nameof(readEnvironment));
        _verifyRequired=verifyRequired??throw new ArgumentNullException(nameof(verifyRequired));
        _logError=logError;
    }

    public Task<EnvironmentProviderImportResult> ImportAndCommitAsync(
        SettingsService settingsService,
        bool requireVerification,
        CancellationToken token)
    {
        ArgumentNullException.ThrowIfNull(settingsService);
        return ImportAndCommitAsync(settingsService,settingsService.Load(),requireVerification,token);
    }

    internal async Task<EnvironmentProviderImportResult> ImportAndCommitAsync(
        SettingsService settingsService,
        AppSettings current,
        bool requireVerification,
        CancellationToken token)
    {
        ArgumentNullException.ThrowIfNull(settingsService);
        ArgumentNullException.ThrowIfNull(current);
        token.ThrowIfCancellationRequested();

        var imports=Definitions
            .Select(definition=>(Definition:definition,Secret:_readEnvironment(definition.Variable)))
            .Where(import=>!string.IsNullOrWhiteSpace(import.Secret))
            .ToList();
        if(imports.Count==0)
        {
            var existingReport=requireVerification?await _verifyRequired(current,token):null;
            return new(false,existingReport);
        }
        if(current.HasSensitiveCredentialErrors)
        {
            throw new InvalidOperationException("敏感 Header 凭据迁移失败，环境 Provider 导入已停止；请先在设置中重新输入并保存这些 Header");
        }
        if(current.ConfigurationErrors.Count>0)
        {
            throw new InvalidOperationException($"当前设置包含配置错误，环境 Provider 导入已停止；请先在设置页修复并保存：{string.Join("；",current.ConfigurationErrors.Distinct(StringComparer.Ordinal))}");
        }

        var candidate=CloneSettings(current);
        var originalCredentialIds=ReferencedCredentialIds(current);
        var createdCredentialIds=new HashSet<string>(StringComparer.Ordinal);
        var importedProviders=new List<AiProviderSettings>(imports.Count);
        var committed=false;
        try
        {
            foreach(var import in imports)
            {
                token.ThrowIfCancellationRequested();
                var provider=candidate.Providers.FirstOrDefault(item=>SameEndpoint(item.BaseUrl,import.Definition.BaseUrl));
                if(provider is null)
                {
                    provider=new AiProviderSettings{BaseUrl=import.Definition.BaseUrl};
                    candidate.Providers.Add(provider);
                }

                provider.Name=import.Definition.Name;
                provider.Type=import.Definition.Type;
                provider.BaseUrl=import.Definition.BaseUrl;
                provider.Model=import.Definition.Model;
                RemoveAuthenticationHeaders(provider);
                var credentialId=Guid.NewGuid().ToString("N");
                _credentials.Save(credentialId,import.Secret!);
                createdCredentialIds.Add(credentialId);
                provider.CredentialId=credentialId;
                importedProviders.Add(provider);
            }

            RepairProviderIdentities(candidate);
            var importedMiniMax=importedProviders.LastOrDefault(provider=>
                provider.Type.Equals("MiniMax",StringComparison.OrdinalIgnoreCase)&&
                provider.Model.Equals("MiniMax-M3",StringComparison.OrdinalIgnoreCase));
            // A freshly created settings file contains an empty MiniMax
            // placeholder so that the settings page has something to render.
            // Its ID is unique, but it is not a usable default until a primary
            // API key or authentication header can actually be read.  Treat a
            // missing/unreadable credential as invalid here; otherwise an
            // xAI/Volcengine-only environment import would keep the empty
            // placeholder selected and fail the final save validation.
            var defaultProvider=candidate.Providers.SingleOrDefault(provider=>provider.Id==candidate.DefaultProviderId);
            var defaultIsValid=defaultProvider is not null&&HasUsableAuthentication(defaultProvider);
            if(importedMiniMax is not null)candidate.DefaultProviderId=importedMiniMax.Id;
            else if(!defaultIsValid)candidate.DefaultProviderId=importedProviders[0].Id;

            SettingsService.RevalidateProviderConfiguration(candidate);
            if(candidate.ConfigurationErrors.Count>0)
            {
                throw new InvalidOperationException(string.Join("；",candidate.ConfigurationErrors.Distinct(StringComparer.Ordinal)));
            }
            SettingsService.ValidateForSave(candidate);

            var reportPath=requireVerification?await _verifyRequired(candidate,token):null;
            token.ThrowIfCancellationRequested();
            settingsService.Save(candidate);
            committed=true;

            var retainedCredentialIds=ReferencedCredentialIds(candidate);
            foreach(var credentialId in originalCredentialIds.Except(retainedCredentialIds,StringComparer.OrdinalIgnoreCase))
            {
                TryDeleteCredential(credentialId,"ProviderCredentialCleanup");
            }
            return new(true,reportPath);
        }
        finally
        {
            if(!committed)
            {
                foreach(var credentialId in createdCredentialIds)TryDeleteCredential(credentialId,"ProviderCredentialRollback");
            }
        }
    }

    private static AppSettings CloneSettings(AppSettings source)
    {
        var clone=new AppSettings
        {
            CaptureHotkey=new(){Key=source.CaptureHotkey?.Key??System.Windows.Input.Key.A,Modifiers=source.CaptureHotkey?.Modifiers??(System.Windows.Input.ModifierKeys.Control|System.Windows.Input.ModifierKeys.Shift)},
            LaunchAtStartup=source.LaunchAtStartup,
            UiLanguage=source.UiLanguage,
            OverlayOpacity=source.OverlayOpacity,
            CaptureDelaySeconds=source.CaptureDelaySeconds,
            DefaultImageFormat=source.DefaultImageFormat,
            IncludeCaptureCursor=source.IncludeCaptureCursor,
            RecordingFps=source.RecordingFps,
            RecordingQuality=source.RecordingQuality,
            GifFps=source.GifFps,
            IncludeRecordingCursor=source.IncludeRecordingCursor,
            TempCleanupDays=source.TempCleanupDays,
            SaveConversationHistory=source.SaveConversationHistory,
            EnableVoiceInput=source.EnableVoiceInput,
            AutomaticallyStartListening=source.AutomaticallyStartListening,
            VoiceLanguage=source.VoiceLanguage,
            HermesEnabled=source.HermesEnabled,
            HermesProfile=source.HermesProfile,
            HermesProvider=source.HermesProvider,
            HermesModel=source.HermesModel,
            HermesReasoningEffort=source.HermesReasoningEffort,
            HermesAutoReadAloud=source.HermesAutoReadAloud,
            DefaultProviderId=source.DefaultProviderId,
            Providers=(source.Providers??[]).Where(provider=>provider is not null).Select(CloneProvider).ToList(),
            HasSensitiveCredentialErrors=source.HasSensitiveCredentialErrors
        };
        clone.ConfigurationErrors.AddRange(source.ConfigurationErrors);
        return clone;
    }

    private static AiProviderSettings CloneProvider(AiProviderSettings source)=>new()
    {
        Id=source.Id??string.Empty,
        Name=source.Name??"Provider",
        Type=source.Type??string.Empty,
        BaseUrl=source.BaseUrl??string.Empty,
        Model=source.Model??string.Empty,
        CredentialId=source.CredentialId??string.Empty,
        CustomHeaders=source.CustomHeaders is null?[]:new Dictionary<string,string>(source.CustomHeaders,StringComparer.OrdinalIgnoreCase),
        SensitiveHeaderCredentialIds=source.SensitiveHeaderCredentialIds is null?[]:new Dictionary<string,string>(source.SensitiveHeaderCredentialIds,StringComparer.OrdinalIgnoreCase)
    };

    private static void RepairProviderIdentities(AppSettings settings)
    {
        _=ProviderEditingPolicy.RepairIdentities(settings.Providers,settings.DefaultProviderId);
    }

    private static HashSet<string> ReferencedCredentialIds(AppSettings settings)
    {
        // Credential ids become file names on Windows. Reference tracking must
        // therefore use the same case-insensitive identity as the file system,
        // otherwise a casing-only edit can delete a credential still in use.
        var result=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach(var provider in settings.Providers??[])
        {
            if(provider is null)continue;
            if(!string.IsNullOrWhiteSpace(provider.CredentialId))result.Add(provider.CredentialId);
            if(provider.SensitiveHeaderCredentialIds is null)continue;
            foreach(var credentialId in provider.SensitiveHeaderCredentialIds.Values)
                if(!string.IsNullOrWhiteSpace(credentialId))result.Add(credentialId);
        }
        return result;
    }

    private static void RemoveAuthenticationHeaders(AiProviderSettings provider)
    {
        // An environment import replaces the provider's primary credential. Remove
        // every sensitive inline/mapped header first, including non-auth material
        // such as tenant signing metadata, so the imported provider cannot retain
        // stale secrets alongside the new API key.
        foreach(var name in provider.CustomHeaders.Keys.Where(ProviderHeaderCredentialService.IsSensitive).ToList())
            provider.CustomHeaders.Remove(name);
        foreach(var name in provider.SensitiveHeaderCredentialIds.Keys.Where(ProviderHeaderCredentialService.IsSensitive).ToList())
            provider.SensitiveHeaderCredentialIds.Remove(name);
    }

    private static bool SameEndpoint(string? left,string right)=>
        string.Equals(left?.Trim().TrimEnd('/'),right.TrimEnd('/'),StringComparison.OrdinalIgnoreCase);

    private void TryDeleteCredential(string credentialId,string component)
    {
        try{_credentials.Delete(credentialId);}
        catch(Exception ex){try{_logError?.Invoke(component,ex);}catch{}}
    }

    private bool HasUsableAuthentication(AiProviderSettings provider)
    {
        if(!string.IsNullOrWhiteSpace(provider.CredentialId)&&!string.IsNullOrWhiteSpace(_credentials.Read(provider.CredentialId)))
            return true;

        foreach(var pair in provider.SensitiveHeaderCredentialIds??[])
        {
            if(!ProviderHeaderCredentialService.IsAuthentication(pair.Key))continue;
            if(!string.IsNullOrWhiteSpace(pair.Value)&&!string.IsNullOrWhiteSpace(_credentials.Read(pair.Value)))return true;
        }

        // This is normally empty after SettingsService.Load migrates sensitive
        // headers.  Keep the check defensive for callers that provide an
        // in-memory AppSettings object directly to this overload.
        return (provider.CustomHeaders??[]).Any(pair=>
            ProviderHeaderCredentialService.IsAuthentication(pair.Key)&&
            !string.IsNullOrWhiteSpace(pair.Value));
    }

    private sealed record ImportDefinition(string Variable,string Name,string Type,string BaseUrl,string Model);
}

public sealed record EnvironmentProviderImportResult(bool Changed,string? VerificationReportPath);
