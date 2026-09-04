using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Speech;
using System.Globalization;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Runtime.InteropServices;
using Xunit;
namespace MewuAI.Tests;
public sealed class ServicesTests
{
    [Fact] public void CredentialService_RoundTripsWithCurrentUserDpapi(){var root=TestDirectory();try{var service=new CredentialService(root);service.Save("credential","secret-value");Assert.Equal("secret-value",service.Read("credential"));service.Delete("credential");Assert.Null(service.Read("credential"));}finally{Directory.Delete(root,true);}}
    [Fact] public void CredentialService_RejectsPathTraversalIdentifiers(){var root=TestDirectory();try{var service=new CredentialService(root);Assert.Throws<ArgumentException>(()=>service.Save("..\\outside","secret"));Assert.False(File.Exists(Path.Combine(Directory.GetParent(root)!.FullName,"outside.bin")));}finally{Directory.Delete(root,true);}}
    [Fact] public void SettingsService_RoundTripsProvidersAndEnums(){var root=TestDirectory();try{var path=Path.Combine(root,"settings.json");var settings=new AppSettings{CaptureDelaySeconds=5,IncludeCaptureCursor=true,UiLanguage="en-US",VoiceLanguage="zh-CN",CaptureHotkey=new(){Key=System.Windows.Input.Key.Z,Modifiers=System.Windows.Input.ModifierKeys.Control|System.Windows.Input.ModifierKeys.Alt},Providers=[new(){Name="私有模型",Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model-a",CredentialId="credential",CustomHeaders=new(){{"X-Tenant","demo"}}}]};settings.DefaultProviderId=settings.Providers[0].Id;var service=new SettingsService(path);service.Save(settings);var loaded=service.Load();Assert.Equal(5,loaded.CaptureDelaySeconds);Assert.Equal("en-US",loaded.UiLanguage);Assert.Equal(System.Windows.Input.Key.Z,loaded.CaptureHotkey.Key);Assert.Equal("model-a",loaded.Providers.Single().Model);Assert.Equal("demo",loaded.Providers.Single().CustomHeaders["X-Tenant"]);using var document=JsonDocument.Parse(File.ReadAllText(path));Assert.False(document.RootElement.ToString().Contains("secret",StringComparison.OrdinalIgnoreCase));}finally{Directory.Delete(root,true);}}
    [Fact] public void SettingsService_InvalidUiLanguageSafelyFallsBackToSystem(){var root=TestDirectory();try{var path=Path.Combine(root,"settings.json");var provider=ValidProvider("provider");File.WriteAllText(path,JsonSerializer.Serialize(new AppSettings{UiLanguage="unsupported",Providers=[provider],DefaultProviderId=provider.Id}));var loaded=new SettingsService(path).Load();Assert.Equal("system",loaded.UiLanguage);}finally{Directory.Delete(root,true);}}
    [Fact]
    public void SettingsService_RoundTripsHermesModeWithoutRemoteApiCredential()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");
            var provider=ValidProvider("fallback");provider.CredentialId=string.Empty;
            var settings=new AppSettings
            {
                Providers=[provider],DefaultProviderId=provider.Id,
                HermesEnabled=true,HermesProfile="coder",HermesProvider="openrouter",HermesModel="anthropic/claude-sonnet-4.5",
                HermesReasoningEffort="high",HermesAutoReadAloud=true
            };

            var service=new SettingsService(path);service.Save(settings);var loaded=service.Load();

            Assert.True(loaded.HermesEnabled);Assert.Equal("coder",loaded.HermesProfile);Assert.Equal("openrouter",loaded.HermesProvider);
            Assert.Equal("anthropic/claude-sonnet-4.5",loaded.HermesModel);Assert.Equal("high",loaded.HermesReasoningEffort);
            Assert.True(loaded.HermesAutoReadAloud);Assert.Empty(loaded.ConfigurationErrors);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void SettingsService_LegacyDocumentDefaultsHermesToDisabled()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");var provider=ValidProvider("legacy");
            var json=JsonSerializer.SerializeToNode(new AppSettings{Providers=[provider],DefaultProviderId=provider.Id})!.AsObject();
            foreach(var name in new[]{"HermesEnabled","HermesProfile","HermesProvider","HermesModel","HermesReasoningEffort","HermesAutoReadAloud"})json.Remove(name);
            File.WriteAllText(path,json.ToJsonString());

            var loaded=new SettingsService(path).Load();

            Assert.False(loaded.HermesEnabled);Assert.Equal("default",loaded.HermesProfile);Assert.Equal(string.Empty,loaded.HermesProvider);
            Assert.Equal(string.Empty,loaded.HermesModel);Assert.Equal("medium",loaded.HermesReasoningEffort);
            Assert.False(loaded.HermesAutoReadAloud);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void SettingsService_FailsClosedForInvalidEnabledHermesSelection()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");var provider=ValidProvider("provider");
            var invalid=new AppSettings{Providers=[provider],DefaultProviderId=provider.Id,HermesEnabled=true,HermesProvider="bad --global",HermesModel="",HermesReasoningEffort="impossible"};
            Assert.Throws<InvalidOperationException>(()=>new SettingsService(path).Save(invalid));
            File.WriteAllText(path,JsonSerializer.Serialize(invalid));

            var loaded=new SettingsService(path).Load();

            Assert.True(loaded.HermesEnabled);Assert.Contains(loaded.ConfigurationErrors,error=>error.Contains("Hermes Provider",StringComparison.Ordinal));
            Assert.Contains(loaded.ConfigurationErrors,error=>error.Contains("尚未选择模型",StringComparison.Ordinal));
            Assert.Contains(loaded.ConfigurationErrors,error=>error.Contains("思考程度",StringComparison.Ordinal));
        }
        finally{Directory.Delete(root,true);}
    }
    [Theory]
    [InlineData("-provider")]
    [InlineData("bad --global")]
    [InlineData("quoted\"")]
    [InlineData("trailing\\")]
    public void SettingsService_RejectsHermesProviderArgumentInjection(string providerValue)
    {
        var root=TestDirectory();
        try
        {
            var provider=ValidProvider("provider");
            var settings=new AppSettings{Providers=[provider],DefaultProviderId=provider.Id,HermesEnabled=true,HermesProvider=providerValue,HermesModel="model",HermesReasoningEffort="medium"};
            Assert.Throws<InvalidOperationException>(()=>new SettingsService(Path.Combine(root,"settings.json")).Save(settings));
        }
        finally{Directory.Delete(root,true);}
    }
    [Theory]
    [InlineData("../escape")]
    [InlineData("bad profile")]
    [InlineData("--profile")]
    public void SettingsServiceRejectsUnsafeHermesAgentProfiles(string profile)
    {
        var root=TestDirectory();
        try
        {
            var provider=ValidProvider("provider");
            var settings=new AppSettings{Providers=[provider],DefaultProviderId=provider.Id,HermesEnabled=true,HermesProfile=profile,HermesProvider="nous",HermesModel="model",HermesReasoningEffort="medium"};
            var error=Assert.Throws<InvalidOperationException>(()=>new SettingsService(Path.Combine(root,"settings.json")).Save(settings));
            Assert.Contains("Agent / 人格",error.Message,StringComparison.Ordinal);
        }
        finally{Directory.Delete(root,true);}
    }
    [Fact]
    public void SettingsServiceAcceptsAFileNameWithoutAnExplicitDirectory()
    {
        var relativePath=$"mewu-settings-{Guid.NewGuid():N}.json";
        var absolutePath=Path.GetFullPath(relativePath);
        var root=TestDirectory();
        try
        {
            var credentials=new ProviderHeaderCredentialService(new CredentialService(root));
            var loaded=new SettingsService(relativePath,credentials,null).Load();
            Assert.Equal("MiniMax-M3",Assert.Single(loaded.Providers).Model);
        }
        finally
        {
            try{if(File.Exists(absolutePath))File.Delete(absolutePath);}catch{}
            Directory.Delete(root,true);
        }
    }
    [Fact] public void SettingsService_NormalizesUnsafeNumericValues(){var root=TestDirectory();try{var path=Path.Combine(root,"settings.json");File.WriteAllText(path,"{\"RecordingFps\":999,\"RecordingQuality\":-1,\"GifFps\":0,\"TempCleanupDays\":999,\"OverlayOpacity\":9}");var loaded=new SettingsService(path).Load();Assert.Equal(60,loaded.RecordingFps);Assert.Equal(20,loaded.RecordingQuality);Assert.Equal(1,loaded.GifFps);Assert.Equal(30,loaded.TempCleanupDays);Assert.Equal(.75,loaded.OverlayOpacity);}finally{Directory.Delete(root,true);}}
    [Fact] public void SettingsService_DisablesAutomaticListeningWhenVoiceInputIsOff(){var root=TestDirectory();try{var path=Path.Combine(root,"settings.json");File.WriteAllText(path,"{\"EnableVoiceInput\":false,\"AutomaticallyStartListening\":true}");var loaded=new SettingsService(path).Load();Assert.False(loaded.EnableVoiceInput);Assert.False(loaded.AutomaticallyStartListening);}finally{Directory.Delete(root,true);}}
    [Fact] public void SettingsChoices_PreserveNonPresetLegalValues(){Assert.Equal(new[]{15,24,27,30,60},SettingsChoicePolicy.IncludeCurrent(new[]{15,24,30,60},27));Assert.Equal(new[]{1,3,6,7,14,30},SettingsChoicePolicy.IncludeCurrent(new[]{1,3,7,14,30},6));}
    [Fact] public void NewSettingsDefaultToMiniMaxM3(){var root=TestDirectory();try{var settings=new SettingsService(Path.Combine(root,"settings.json")).Load();var provider=Assert.Single(settings.Providers);Assert.Equal("MiniMax",provider.Type);Assert.Equal("https://api.minimaxi.com/v1",provider.BaseUrl);Assert.Equal("MiniMax-M3",provider.Model);Assert.Equal(provider.Id,settings.DefaultProviderId);}finally{Directory.Delete(root,true);}}
    [Theory]
    [InlineData("Id")]
    [InlineData("Type")]
    [InlineData("BaseUrl")]
    [InlineData("Model")]
    public void SettingsService_RejectsProviderJsonMissingRequiredField(string field)
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");var credentialPath=Path.Combine(root,"Credentials");
            var credentials=new CredentialService(credentialPath);credentials.Save("legacy-key","legacy-secret");
            var provider=ValidProvider("legacy-provider");provider.CredentialId="legacy-key";
            var json=JsonSerializer.SerializeToNode(new AppSettings{Providers=[provider],DefaultProviderId=provider.Id})!.AsObject();
            var providerJson=json["Providers"]!.AsArray()[0]!.AsObject();Assert.True(providerJson.Remove(field));
            var original=json.ToJsonString();File.WriteAllText(path,original);

            var loaded=new SettingsService(path,new ProviderHeaderCredentialService(credentials),null).Load();

            Assert.Empty(loaded.Providers);Assert.Contains(loaded.ConfigurationErrors,error=>error.Contains("设置文件无法解析",StringComparison.Ordinal));
            Assert.Equal(original,File.ReadAllText(path));
            Assert.Null(new AiProviderFactory(credentials,null).Create(loaded,out var error));Assert.Contains("设置文件无法解析",error,StringComparison.Ordinal);
            Assert.Equal("legacy-secret",credentials.Read("legacy-key"));
        }
        finally{Directory.Delete(root,true);}
    }

    [Theory]
    [InlineData("Id")]
    [InlineData("Type")]
    [InlineData("BaseUrl")]
    [InlineData("Model")]
    public void SettingsService_FailsClosedWhenRequiredProviderFieldIsExplicitlyNull(string field)
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");var credentialPath=Path.Combine(root,"Credentials");
            var credentials=new CredentialService(credentialPath);credentials.Save("legacy-key","legacy-secret");
            var provider=ValidProvider("legacy-provider");provider.CredentialId="legacy-key";
            var json=JsonSerializer.SerializeToNode(new AppSettings{Providers=[provider],DefaultProviderId=provider.Id})!.AsObject();
            json["Providers"]!.AsArray()[0]!.AsObject()[field]=null;
            var original=json.ToJsonString();File.WriteAllText(path,original);

            var loaded=new SettingsService(path,new ProviderHeaderCredentialService(credentials),null).Load();

            Assert.NotEmpty(loaded.ConfigurationErrors);
            Assert.Equal(original,File.ReadAllText(path));
            Assert.Null(new AiProviderFactory(credentials,null).Create(loaded,out var error));Assert.False(string.IsNullOrWhiteSpace(error));
            Assert.Equal("legacy-secret",credentials.Read("legacy-key"));
        }
        finally{Directory.Delete(root,true);}
    }
    [Fact]
    public void SettingsService_PreservesInvalidProviderIdentityForExplicitRepair()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");const string duplicateId="duplicate-provider";
            var settings=new AppSettings{Providers=[ValidProvider(duplicateId),ValidProvider(duplicateId)],DefaultProviderId=duplicateId};
            File.WriteAllText(path,JsonSerializer.Serialize(settings));
            var loaded=new SettingsService(path).Load();
            Assert.Equal(2,loaded.Providers.Count);Assert.All(loaded.Providers,provider=>Assert.Equal(duplicateId,provider.Id));
            Assert.Equal(duplicateId,loaded.DefaultProviderId);Assert.Contains(loaded.ConfigurationErrors,error=>error.Contains("Provider ID 重复",StringComparison.Ordinal));
            Assert.Null(new AiProviderFactory(null,null).Create(loaded,out var factoryError));Assert.Contains("Provider ID 重复",factoryError,StringComparison.Ordinal);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void ExistingDanglingDefaultFlowsFromLoadToFactoryWithoutSilentFallback()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");var provider=ValidProvider("available");
            File.WriteAllText(path,JsonSerializer.Serialize(new AppSettings{Providers=[provider],DefaultProviderId="missing"}));
            var loaded=new SettingsService(path,null,null).Load();
            Assert.Equal("missing",loaded.DefaultProviderId);Assert.Equal("available",Assert.Single(loaded.Providers).Id);
            Assert.Null(new AiProviderFactory(null,null).Create(loaded,out var error));Assert.Contains("默认 AI Provider 已不存在",error,StringComparison.Ordinal);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void MalformedSettingsAreLoggedThroughInjectedSinkAndRemainUnavailable()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");File.WriteAllText(path,"{ definitely-not-json");
            var logged=new List<(string Component,Exception Exception)>();
            var loaded=new SettingsService(path,null,(component,exception)=>logged.Add((component,exception))).Load();
            var entry=Assert.Single(logged);Assert.Equal("SettingsLoad",entry.Component);Assert.IsType<JsonException>(entry.Exception);
            Assert.Empty(loaded.Providers);Assert.NotEmpty(loaded.ConfigurationErrors);
            Assert.Null(new AiProviderFactory(null,null).Create(loaded,out var error));Assert.Contains("设置文件无法解析",error,StringComparison.Ordinal);
            Assert.Equal("{ definitely-not-json",File.ReadAllText(path));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void SettingsService_RejectsMissingOrDuplicateProviderIdentity()
    {
        var root=TestDirectory();
        try
        {
            var service=new SettingsService(Path.Combine(root,"settings.json"));
            Assert.Throws<InvalidOperationException>(()=>service.Save(new AppSettings{Providers=[]}));
            Assert.Throws<InvalidOperationException>(()=>service.Save(new AppSettings{Providers=[ValidProvider(string.Empty)],DefaultProviderId=string.Empty}));
            Assert.Throws<InvalidOperationException>(()=>service.Save(new AppSettings{Providers=[ValidProvider("same"),ValidProvider("same")],DefaultProviderId="same"}));
            Assert.Throws<InvalidOperationException>(()=>service.Save(new AppSettings{Providers=[ValidProvider("present")],DefaultProviderId="missing"}));
            Assert.False(File.Exists(Path.Combine(root,"settings.json")));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void SettingsServiceRejectsDefaultProviderWithoutAnyCredentialReference()
    {
        var root=TestDirectory();
        try
        {
            var provider=ValidProvider("provider");provider.CredentialId=string.Empty;
            var error=Assert.Throws<InvalidOperationException>(()=>new SettingsService(Path.Combine(root,"settings.json")).Save(new AppSettings{Providers=[provider],DefaultProviderId=provider.Id}));
            Assert.Contains("API Key",error.Message,StringComparison.Ordinal);Assert.False(File.Exists(Path.Combine(root,"settings.json")));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void SettingsService_WritesBomlessUtf8WithoutLeavingTemporaryFiles()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");var provider=ValidProvider("provider");
            new SettingsService(path).Save(new AppSettings{Providers=[provider],DefaultProviderId=provider.Id});
            var bytes=File.ReadAllBytes(path);Assert.False(bytes.AsSpan().StartsWith(new byte[]{0xEF,0xBB,0xBF}));Assert.Empty(Directory.EnumerateFiles(root,"*.tmp"));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task EnvironmentImport_VerificationFailureRollsBackCredentialsAndSettings()
    {
        var root=TestDirectory();
        try
        {
            var settingsPath=Path.Combine(root,"settings.json");var credentialPath=Path.Combine(root,"Credentials");
            var credentials=new CredentialService(credentialPath);credentials.Save("old-key","old-secret");credentials.Save("old-auth","Bearer old-header-secret");
            var provider=new AiProviderSettings{Id="minimax",Name="Existing MiniMax",Type="MiniMax",BaseUrl="https://api.minimaxi.com/v1",Model="MiniMax-M3",CredentialId="old-key",CustomHeaders=new(){{"X-Tenant","tenant-a"}},SensitiveHeaderCredentialIds=new(){{"X-Key-Material","old-auth"}}};
            var settingsService=new SettingsService(settingsPath,new ProviderHeaderCredentialService(credentials),null);
            settingsService.Save(new AppSettings{Providers=[provider],DefaultProviderId=provider.Id});
            var original=File.ReadAllText(settingsPath);string? stagedCredentialId=null;
            var bootstrap=new EnvironmentProviderBootstrap(
                credentials,
                name=>name=="MINIMAX_CN_API_KEY"?"new-secret":null,
                (candidate,_)=>
                {
                    var staged=Assert.Single(candidate.Providers);stagedCredentialId=staged.CredentialId;
                    Assert.NotEqual("old-key",stagedCredentialId);Assert.Equal("new-secret",credentials.Read(stagedCredentialId));
                    Assert.DoesNotContain(staged.SensitiveHeaderCredentialIds.Keys,ProviderHeaderCredentialService.IsAuthentication);
                    Assert.Equal("tenant-a",staged.CustomHeaders["X-Tenant"]);
                    Assert.Equal("Bearer old-header-secret",credentials.Read("old-auth"));
                    Assert.Equal(original,File.ReadAllText(settingsPath));
                    return Task.FromException<string>(new InvalidOperationException("verification failed"));
                },
                null);

            await Assert.ThrowsAsync<InvalidOperationException>(()=>bootstrap.ImportAndCommitAsync(settingsService,true,TestContext.Current.CancellationToken));
            Assert.Equal(original,File.ReadAllText(settingsPath));Assert.Equal("old-secret",credentials.Read("old-key"));Assert.Equal("Bearer old-header-secret",credentials.Read("old-auth"));
            Assert.NotNull(stagedCredentialId);Assert.Null(credentials.Read(stagedCredentialId));
            Assert.Equal(2,Directory.GetFiles(credentialPath).Length);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task EnvironmentImport_CommitsOnlyAfterVerificationAndDeletesReplacedCredential()
    {
        var root=TestDirectory();
        try
        {
            var settingsPath=Path.Combine(root,"settings.json");var credentialPath=Path.Combine(root,"Credentials");
            var credentials=new CredentialService(credentialPath);credentials.Save("old-key","old-secret");credentials.Save("old-auth","Bearer old-header-secret");
            var provider=new AiProviderSettings{Id="minimax",Name="Old",Type="MiniMax",BaseUrl="https://api.minimaxi.com/v1/",Model="old-model",CredentialId="old-key",CustomHeaders=new(){{"X-Tenant","tenant-a"}},SensitiveHeaderCredentialIds=new(){{"X-Key-Material","old-auth"}}};
            var settingsService=new SettingsService(settingsPath,new ProviderHeaderCredentialService(credentials),null);
            settingsService.Save(new AppSettings{Providers=[provider],DefaultProviderId=provider.Id});var original=File.ReadAllText(settingsPath);
            var bootstrap=new EnvironmentProviderBootstrap(
                credentials,
                name=>name=="MINIMAX_CN_API_KEY"?"new-secret":null,
                (candidate,_)=>
                {
                    var staged=Assert.Single(candidate.Providers);Assert.Equal("MiniMax-M3",staged.Model);Assert.Equal("new-secret",credentials.Read(staged.CredentialId));
                    Assert.Empty(staged.SensitiveHeaderCredentialIds);Assert.Equal("tenant-a",staged.CustomHeaders["X-Tenant"]);Assert.Equal("old-secret",credentials.Read("old-key"));Assert.Equal("Bearer old-header-secret",credentials.Read("old-auth"));
                    Assert.Equal(original,File.ReadAllText(settingsPath));return Task.FromResult("verification-report.json");
                },
                null);

            var result=await bootstrap.ImportAndCommitAsync(settingsService,true,TestContext.Current.CancellationToken);
            Assert.True(result.Changed);Assert.Equal("verification-report.json",result.VerificationReportPath);
            var loaded=settingsService.Load();var imported=Assert.Single(loaded.Providers);
            Assert.Equal(imported.Id,loaded.DefaultProviderId);Assert.Equal("MiniMax",imported.Name);Assert.Equal("new-secret",credentials.Read(imported.CredentialId));
            Assert.Equal("tenant-a",imported.CustomHeaders["X-Tenant"]);
            Assert.Null(credentials.Read("old-key"));Assert.Null(credentials.Read("old-auth"));Assert.NotEqual(original,File.ReadAllText(settingsPath));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task EnvironmentImportTreatsCredentialIdsAsCaseInsensitiveFileReferences()
    {
        var root=TestDirectory();
        try
        {
            var settingsPath=Path.Combine(root,"settings.json");
            var credentialPath=Path.Combine(root,"Credentials");
            var credentials=new CredentialService(credentialPath);
            credentials.Save("Shared-Key","old-secret");
            var imported=new AiProviderSettings
            {
                Id="imported",
                Name="Existing MiniMax",
                Type="MiniMax",
                BaseUrl="https://api.minimaxi.com/v1",
                Model="MiniMax-M3",
                CredentialId="Shared-Key"
            };
            // Windows credential blobs are addressed by case-insensitive file
            // names. A second provider may therefore reference the same blob
            // with different casing and must keep it alive during import.
            var other=ValidProvider("other");
            other.CredentialId="shared-key";
            other.BaseUrl="https://example.invalid/v1";
            var settingsService=new SettingsService(settingsPath,new ProviderHeaderCredentialService(credentials),null);
            settingsService.Save(new AppSettings{Providers=[imported,other],DefaultProviderId=imported.Id});
            var current=settingsService.Load();
            var bootstrap=new EnvironmentProviderBootstrap(
                credentials,
                name=>name=="MINIMAX_CN_API_KEY"?"new-secret":null,
                (_,_)=>Task.FromResult("unused"),
                null);

            await bootstrap.ImportAndCommitAsync(settingsService,current,false,TestContext.Current.CancellationToken);

            Assert.Equal("old-secret",credentials.Read("shared-key"));
            var loaded=settingsService.Load();
            Assert.Equal("old-secret",credentials.Read(loaded.Providers.Single(provider=>provider.Id=="other").CredentialId));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task EnvironmentImport_WithVerificationNeverOverwritesMalformedSettings()
    {
        var root=TestDirectory();
        try
        {
            var settingsPath=Path.Combine(root,"settings.json");var malformed="{ invalid-json";File.WriteAllText(settingsPath,malformed);
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));
            var settingsService=new SettingsService(settingsPath,new ProviderHeaderCredentialService(credentials),null);
            var verificationCalled=false;var bootstrap=new EnvironmentProviderBootstrap(
                credentials,
                name=>name=="MINIMAX_CN_API_KEY"?"restored-secret":null,
                (_,_)=>{verificationCalled=true;return Task.FromResult("verified.json");},
                null);

            var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>bootstrap.ImportAndCommitAsync(settingsService,true,TestContext.Current.CancellationToken));
            Assert.Contains("配置错误",error.Message,StringComparison.Ordinal);Assert.False(verificationCalled);
            Assert.Equal(malformed,File.ReadAllText(settingsPath));Assert.Empty(Directory.EnumerateFiles(Path.Combine(root,"Credentials"),"*.bin"));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task EnvironmentImport_WithoutVerificationNeverOverwritesMalformedSettings()
    {
        var root=TestDirectory();
        try
        {
            var settingsPath=Path.Combine(root,"settings.json");const string malformed="{ invalid-json";File.WriteAllText(settingsPath,malformed);
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));var settingsService=new SettingsService(settingsPath,new ProviderHeaderCredentialService(credentials),null);
            var bootstrap=new EnvironmentProviderBootstrap(credentials,name=>name=="MINIMAX_CN_API_KEY"?"new-secret":null,(_,_)=>Task.FromException<string>(new InvalidOperationException("不应验证")),null);

            var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>bootstrap.ImportAndCommitAsync(settingsService,false,TestContext.Current.CancellationToken));

            Assert.Contains("配置错误",error.Message,StringComparison.Ordinal);Assert.Equal(malformed,File.ReadAllText(settingsPath));
            Assert.Empty(Directory.EnumerateFiles(Path.Combine(root,"Credentials"),"*.bin"));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task EnvironmentImport_WithoutVerificationStillCommitsValidSettingsWithoutCallingVerifier()
    {
        var root=TestDirectory();
        try
        {
            var settingsPath=Path.Combine(root,"settings.json");var credentials=new CredentialService(Path.Combine(root,"Credentials"));
            var existing=ValidProvider("existing");var settingsService=new SettingsService(settingsPath,new ProviderHeaderCredentialService(credentials),null);
            settingsService.Save(new AppSettings{Providers=[existing],DefaultProviderId=existing.Id});var verificationCalled=false;
            var bootstrap=new EnvironmentProviderBootstrap(credentials,name=>name=="MINIMAX_CN_API_KEY"?"new-secret":null,(_,_)=>{verificationCalled=true;return Task.FromResult("unused");},null);

            var result=await bootstrap.ImportAndCommitAsync(settingsService,false,TestContext.Current.CancellationToken);

            Assert.True(result.Changed);Assert.Null(result.VerificationReportPath);Assert.False(verificationCalled);
            var loaded=settingsService.Load();Assert.Equal(2,loaded.Providers.Count);Assert.Equal("MiniMax-M3",loaded.Providers.Single(provider=>provider.Id==loaded.DefaultProviderId).Model);
        }
        finally{Directory.Delete(root,true);}
    }

    [Theory]
    [InlineData("XAI_API_KEY","grok-4.6")]
    [InlineData("VOLCENGINE_ARK_API_KEY","doubao-seed-2-1-pro-260628")]
    [InlineData("VOLCENGINE_AGENTPLAN_API_KEY","doubao-seed-2-0-pro-260215")]
    public async Task EnvironmentImportOnFreshInstallSelectsTheOnlyAuthenticatedImportedProvider(
        string environmentVariable,
        string expectedModel)
    {
        var root=TestDirectory();
        try
        {
            var settingsPath=Path.Combine(root,"settings.json");
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));
            var settingsService=new SettingsService(settingsPath,new ProviderHeaderCredentialService(credentials),null);
            var bootstrap=new EnvironmentProviderBootstrap(
                credentials,
                name=>name==environmentVariable?"imported-secret":null,
                (_,_)=>Task.FromException<string>(new InvalidOperationException("不应验证")),
                null);

            var result=await bootstrap.ImportAndCommitAsync(
                settingsService,
                settingsService.Load(),
                false,
                TestContext.Current.CancellationToken);

            Assert.True(result.Changed);Assert.Null(result.VerificationReportPath);
            var loaded=settingsService.Load();
            var selected=loaded.Providers.Single(provider=>provider.Id==loaded.DefaultProviderId);
            Assert.Equal(expectedModel,selected.Model);
            Assert.Equal("imported-secret",credentials.Read(selected.CredentialId));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task EnvironmentImport_DoesNotMaskSensitiveCredentialMigrationFailure()
    {
        var root=TestDirectory();
        try
        {
            var settingsPath=Path.Combine(root,"settings.json");var credentials=new CredentialService(Path.Combine(root,"Credentials"));
            credentials.Save("existing","existing-secret");var provider=ValidProvider("provider");provider.CredentialId="existing";
            var settingsService=new SettingsService(settingsPath,new ProviderHeaderCredentialService(credentials),null);
            var current=new AppSettings{Providers=[provider],DefaultProviderId=provider.Id,HasSensitiveCredentialErrors=true};
            current.ConfigurationErrors.Add("敏感 Header 加密迁移失败");settingsService.Save(current);var original=File.ReadAllText(settingsPath);
            var bootstrap=new EnvironmentProviderBootstrap(credentials,name=>name=="MINIMAX_CN_API_KEY"?"new-secret":null,(_,_)=>Task.FromResult("unused"),null);

            var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>bootstrap.ImportAndCommitAsync(settingsService,current,true,TestContext.Current.CancellationToken));
            Assert.Contains("敏感 Header",error.Message,StringComparison.Ordinal);Assert.Equal(original,File.ReadAllText(settingsPath));
            Assert.Equal("existing-secret",credentials.Read("existing"));Assert.Collection(Directory.GetFiles(Path.Combine(root,"Credentials")),file=>Assert.EndsWith("existing.bin",file,StringComparison.Ordinal));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task EnvironmentImport_PreservesHermesConversationSettings()
    {
        var root=TestDirectory();
        try
        {
            var settingsPath=Path.Combine(root,"settings.json");
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));
            var provider=ValidProvider("provider");provider.CredentialId=string.Empty;
            var settingsService=new SettingsService(settingsPath,new ProviderHeaderCredentialService(credentials),null);
            settingsService.Save(new AppSettings
            {
                Providers=[provider],DefaultProviderId=provider.Id,
                HermesEnabled=true,HermesProfile="research",HermesProvider="nous",HermesModel="hermes-4",
                HermesReasoningEffort="xhigh",HermesAutoReadAloud=true
            });
            var bootstrap=new EnvironmentProviderBootstrap(
                credentials,
                name=>name=="MINIMAX_CN_API_KEY"?"imported-secret":null,
                (_,_)=>Task.FromException<string>(new InvalidOperationException("不应验证")),
                null);

            var result=await bootstrap.ImportAndCommitAsync(settingsService,false,TestContext.Current.CancellationToken);
            var loaded=settingsService.Load();

            Assert.True(result.Changed);Assert.True(loaded.HermesEnabled);Assert.Equal("research",loaded.HermesProfile);Assert.Equal("nous",loaded.HermesProvider);
            Assert.Equal("hermes-4",loaded.HermesModel);Assert.Equal("xhigh",loaded.HermesReasoningEffort);
            Assert.True(loaded.HermesAutoReadAloud);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task RequiredProviderVerificationFailsWhenAnyCheckIsUnavailable()
    {
        var root=TestDirectory();
        try
        {
            var reportPath=Path.Combine(root,"verification.json");
            var verification=new ProviderVerificationService(new AiProviderFactory(new CredentialService(Path.Combine(root,"Credentials")),null),reportPath);
            var error=await Assert.ThrowsAsync<ProviderVerificationException>(()=>verification.VerifyRequiredAsync(new AppSettings{Providers=[]},TestContext.Current.CancellationToken));
            Assert.Equal(reportPath,error.ReportPath);Assert.True(File.Exists(reportPath));
            using var report=JsonDocument.Parse(File.ReadAllText(reportPath));Assert.False(report.RootElement.GetProperty("succeeded").GetBoolean());Assert.NotEmpty(report.RootElement.GetProperty("errors").EnumerateArray());
        }
        finally{Directory.Delete(root,true);}
    }
    [Theory]
    [InlineData("MEWU_OK",true)]
    [InlineData("  mewu_ok\r\n",true)]
    [InlineData("NOT_MEWU_OK",false)]
    [InlineData("不要返回 MEWU_OK",false)]
    [InlineData("收到：mewu_ok。",false)]
    [InlineData("普通非空回答",false)]
    public void ProviderTextVerificationRequiresItsChallengeMarker(string answer,bool expected)=>Assert.Equal(expected,ProviderVerificationService.MatchesTextProbe(answer));
    [Theory]
    [InlineData("MEWU_BLUE",true)]
    [InlineData("  mewu_blue\n",true)]
    [InlineData("NOT_MEWU_BLUE",false)]
    [InlineData("图片不是蓝色；MEWU_BLUE",false)]
    [InlineData("mewu_blue。",false)]
    [InlineData("MEWU_OTHER",false)]
    [InlineData("蓝色",false)]
    public void ProviderImageVerificationRequiresItsVisualChallengeMarker(string answer,bool expected)=>Assert.Equal(expected,ProviderVerificationService.MatchesImageProbe(answer));
    [Fact] public void TempFileService_CleansOnlyExpiredEntries(){var root=TestDirectory();try{var service=new TempFileService(root);var old=Path.Combine(root,"old.tmp");var fresh=Path.Combine(root,"fresh.tmp");File.WriteAllText(old,"old");File.WriteAllText(fresh,"fresh");File.SetLastWriteTimeUtc(old,DateTime.UtcNow-TimeSpan.FromDays(5));service.Cleanup(TimeSpan.FromDays(3));Assert.False(File.Exists(old));Assert.True(File.Exists(fresh));}finally{Directory.Delete(root,true);}}
    [Fact] public void TempFileService_ZeroAgeCleansFutureDatedEntries(){var root=TestDirectory();try{var service=new TempFileService(root);var future=Path.Combine(root,"future.tmp");File.WriteAllText(future,"future");File.SetLastWriteTimeUtc(future,DateTime.UtcNow+TimeSpan.FromDays(1));service.Cleanup(TimeSpan.Zero);Assert.False(File.Exists(future));}finally{Directory.Delete(root,true);}}
    [Fact] public void TempFileService_RejectsPathLikeExtensions(){var root=TestDirectory();try{var service=new TempFileService(root);Assert.Throws<ArgumentException>(()=>service.NewFile("../outside.mp4"));}finally{Directory.Delete(root,true);}}
    [Fact] public void ScreenCaptureService_ReturnsFrozenPhysicalVirtualDesktopFrame(){var expected=System.Windows.Forms.SystemInformation.VirtualScreen;var frame=new ScreenCaptureService().CaptureDesktop();Assert.Equal(expected.Left,frame.OriginX);Assert.Equal(expected.Top,frame.OriginY);Assert.Equal(expected.Width,frame.Image.PixelWidth);Assert.Equal(expected.Height,frame.Image.PixelHeight);Assert.True(frame.Image.IsFrozen);}
    [Fact] public void ScreenCaptureService_SavesAtomicallyWithoutTemporaryFiles(){var root=TestDirectory();try{var path=Path.Combine(root,"capture.png");File.WriteAllText(path,"existing");var pixels=Enumerable.Repeat((byte)255,4*4*4).ToArray();var image=System.Windows.Media.Imaging.BitmapSource.Create(4,4,96,96,System.Windows.Media.PixelFormats.Bgra32,null,pixels,16);ScreenCaptureService.Save(image,path,false);using var input=File.OpenRead(path);var decoder=new System.Windows.Media.Imaging.PngBitmapDecoder(input,System.Windows.Media.Imaging.BitmapCreateOptions.PreservePixelFormat,System.Windows.Media.Imaging.BitmapCacheOption.OnLoad);Assert.Equal(4,decoder.Frames[0].PixelWidth);Assert.Empty(Directory.EnumerateFiles(root,"*.tmp"));}finally{Directory.Delete(root,true);}}
    [Fact] public void AtomicFileService_ReplacesDestinationWithoutChangingSourceOrLeavingTemporaryFiles(){var root=TestDirectory();try{var source=Path.Combine(root,"source.mp4");var destination=Path.Combine(root,"saved.mp4");var bytes=Enumerable.Range(0,4096).Select(index=>(byte)(index%251)).ToArray();File.WriteAllBytes(source,bytes);File.WriteAllText(destination,"old");AtomicFileService.Copy(source,destination);Assert.Equal(bytes,File.ReadAllBytes(source));Assert.Equal(bytes,File.ReadAllBytes(destination));Assert.Empty(Directory.EnumerateFiles(root,"*.tmp"));}finally{Directory.Delete(root,true);}}
    [Fact] public void PrivacyLogger_RedactsAuthorizationAndApiKeys(){var root=TestDirectory();try{new PrivacyLogger(root).Error("AI",new InvalidOperationException("Authorization: Bearer token-123 api_key=secret-456"));var text=File.ReadAllText(Directory.GetFiles(root,"*.log").Single());Assert.DoesNotContain("token-123",text);Assert.DoesNotContain("secret-456",text);Assert.Contains("[REDACTED]",text);}finally{Directory.Delete(root,true);}}
    [Fact] public void MiniMaxM3Provider_EnablesNativeImageAndVideoUnderstanding(){var provider=new MiniMaxProvider(new AiProviderSettings{Type="MiniMax",BaseUrl="https://api.minimax.io/v1",Model="MiniMax-M3"},"unused");Assert.True(provider.Capabilities.SupportsImage);Assert.True(provider.Capabilities.SupportsVideo);Assert.Contains("image/png",provider.Capabilities.AcceptedMimeTypes);Assert.Contains("video/mp4",provider.Capabilities.AcceptedMimeTypes);Assert.Equal(10L*1024*1024,provider.Capabilities.MaxImageSize);Assert.Equal(50L*1024*1024,provider.Capabilities.MaxVideoSize);Assert.Equal(50L*1024*1024,provider.Capabilities.MaxAttachmentSize);}
    [Fact] public void OlderMiniMaxModel_DoesNotClaimM3MultimodalProtocol(){var provider=new MiniMaxProvider(new AiProviderSettings{Type="MiniMax",BaseUrl="https://api.minimax.io/v1",Model="MiniMax-M2.7"},"unused");Assert.False(provider.Capabilities.SupportsImage);Assert.False(provider.Capabilities.SupportsVideo);}
    [Fact] public void OpenAiProvider_DeclaresSupportedImageMimeTypes(){var provider=new OpenAiCompatibleProvider(new AiProviderSettings{Model="gpt-4o"},"unused");Assert.Contains("image/png",provider.Capabilities.AcceptedMimeTypes);Assert.False(provider.Capabilities.SupportsVideo);Assert.Equal(20L*1024*1024,provider.Capabilities.MaxImageSize);Assert.Equal(0,provider.Capabilities.MaxVideoSize);}
    [Theory]
    [InlineData("deepseek-v4-pro-ga-260813")]
    [InlineData("deepseek-v4-flash-ga-260731")]
    [InlineData("glm-5-2-260617")]
    public void VolcengineTextModelsDoNotFalselyClaimImageUnderstanding(string model)
    {
        var provider=new OpenAiCompatibleProvider(new AiProviderSettings{BaseUrl=VolcengineModelPolicy.StandardBaseUrl,Model=model},"unused");
        Assert.False(provider.Capabilities.SupportsImage);Assert.False(provider.Capabilities.SupportsVideo);Assert.Equal(0,provider.Capabilities.MaxImageSize);
    }
    [Theory]
    [InlineData("doubao-seed-2-1-pro-260628")]
    [InlineData("doubao-seed-2-1-turbo-260628")]
    [InlineData("doubao-seed-2-0-lite-260428")]
    [InlineData("glm-5-3-flash")]
    [InlineData("glm-5.3-flash")]
    [InlineData("deepseek-v4-flash-vision-exp")]
    public void LatestVolcengineMultimodalModelsEnableImageAndVideoUnderstanding(string model)
    {
        var provider=new OpenAiCompatibleProvider(new AiProviderSettings{BaseUrl=VolcengineModelPolicy.StandardBaseUrl,Model=model},"unused");
        Assert.True(provider.Capabilities.SupportsImage);Assert.True(provider.Capabilities.SupportsVideo);Assert.Equal(10L*1024*1024,provider.Capabilities.MaxImageSize);Assert.Equal(50L*1024*1024,provider.Capabilities.MaxVideoSize);
    }
    [Theory]
    [InlineData("deepseek-v4-pro-ga-260813",true)]
    [InlineData("glm-5-2-260617",true)]
    [InlineData("doubao-seedream-5-0-pro-260628",false)]
    [InlineData("doubao-seedance-2-5-260628",false)]
    [InlineData("doubao-embedding-vision-251215",false)]
    public void VolcengineCatalogOnlyOffersChatAndUnderstandingModels(string model,bool expected)=>Assert.Equal(expected,VolcengineModelPolicy.IsChatModel(model));
    [Fact] public async Task GenericOpenAiProvider_RejectsVideoBeforeNetwork(){var provider=new OpenAiCompatibleProvider(new AiProviderSettings(),"unused");await Assert.ThrowsAsync<NotSupportedException>(()=>provider.SendAsync(new AiRequest{Attachments=[new(AiAttachmentType.Video,"video/mp4",[1,2,3])]},TestContext.Current.CancellationToken));}
    [Fact] public async Task OpenAiProvider_RejectsUnsupportedMimeBeforeNetwork(){var provider=new OpenAiCompatibleProvider(new AiProviderSettings(),"unused");await Assert.ThrowsAsync<NotSupportedException>(()=>provider.SendAsync(new AiRequest{Attachments=[new(AiAttachmentType.Image,"image/bmp",[1,2,3])]},TestContext.Current.CancellationToken));}
    [Fact] public async Task OpenAiProvider_RejectsInvalidAttachmentTypeBeforeNetwork(){var provider=new OpenAiCompatibleProvider(new AiProviderSettings(),"unused");await Assert.ThrowsAsync<InvalidOperationException>(()=>provider.SendAsync(new AiRequest{Attachments=[new((AiAttachmentType)99,"image/png",[1,2,3])]},TestContext.Current.CancellationToken));}
    [Fact] public async Task OpenAiProvider_RejectsNonPositiveOutputBudgetBeforeNetwork(){var provider=new OpenAiCompatibleProvider(new AiProviderSettings(),"unused");await Assert.ThrowsAsync<ArgumentOutOfRangeException>(()=>provider.SendAsync(new AiRequest{Prompt="test",MaxOutputTokens=0},TestContext.Current.CancellationToken));}
    [Fact] public void SpeechFailureMapper_MapsDesktopRecognizerAndAudioFailures(){Assert.Equal("当前语言缺少可用的语音识别器",SpeechRecognitionFailureMapper.FromException(new COMException("missing",SpeechRecognitionFailureMapper.RecognizerNotFound),SpeechRecognitionFailureContext.RecognizerInitialization));Assert.Equal("未检测到可用麦克风",SpeechRecognitionFailureMapper.FromException(new COMException("no device",SpeechRecognitionFailureMapper.AudioDeviceNotFound),SpeechRecognitionFailureContext.AudioInput));Assert.Equal("麦克风权限未开启，无法使用语音输入",SpeechRecognitionFailureMapper.FromException(new UnauthorizedAccessException(),SpeechRecognitionFailureContext.AudioInput));Assert.Equal("麦克风正被其他应用占用，请稍后重试",SpeechRecognitionFailureMapper.FromException(new COMException("busy",SpeechRecognitionFailureMapper.DeviceBusy),SpeechRecognitionFailureContext.AudioInput));Assert.Equal("没有听到语音，请重试",SpeechRecognitionFailureMapper.FromException(new COMException("timeout",SpeechRecognitionFailureMapper.RecognitionTimeout),SpeechRecognitionFailureContext.Recognition));}
    [Fact] public void SpeechLanguageSelector_PrefersExactAndCompatibleInstalledRecognizers(){CultureInfo[] installed=[CultureInfo.GetCultureInfo("en-GB"),CultureInfo.GetCultureInfo("zh-CN")];Assert.Equal("zh-CN",SpeechRecognizerLanguageSelector.SelectBestCulture(" zh-CN ",installed,CultureInfo.GetCultureInfo("en-US"))?.Name);Assert.Equal("en-GB",SpeechRecognizerLanguageSelector.SelectBestCulture("en-US",installed,CultureInfo.GetCultureInfo("zh-CN"))?.Name);Assert.Null(SpeechRecognizerLanguageSelector.SelectBestCulture("ja-JP",installed,CultureInfo.GetCultureInfo("zh-CN")));}
    [Fact] public void SpeechLanguageSelector_SystemUsesOnlyMatchingWindowsLanguages(){CultureInfo[] installed=[CultureInfo.GetCultureInfo("en-US"),CultureInfo.GetCultureInfo("zh-CN")];Assert.Equal("zh-CN",SpeechRecognizerLanguageSelector.SelectBestCulture("system",installed,CultureInfo.GetCultureInfo("zh-CN"))?.Name);Assert.Null(SpeechRecognizerLanguageSelector.SelectBestCulture("system",installed,CultureInfo.GetCultureInfo("ja-JP")));CultureInfo[] familyFirst=[CultureInfo.GetCultureInfo("zh-CN"),CultureInfo.GetCultureInfo("en-GB")];Assert.Equal("en-GB",SpeechRecognizerLanguageSelector.SelectBestCulture("system",familyFirst,CultureInfo.GetCultureInfo("en-US"),CultureInfo.GetCultureInfo("zh-CN"))?.Name);Assert.Null(SpeechRecognizerLanguageSelector.SelectBestCulture("system",Array.Empty<CultureInfo>(),CultureInfo.GetCultureInfo("zh-CN")));}
    [Fact] public async Task SpeechService_PreCanceledRequestDoesNotInitializeDesktopRecognizer(){using var cancellation=new CancellationTokenSource();cancellation.Cancel();await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>new WindowsSpeechToTextService().RecognizeOnceAsync("system",cancellation.Token));}
    [Fact]
    public async Task SpeechCancellationCompletesWhenRecognizerNeverRaisesCompletion()
    {
        var completion=new TaskCompletionSource<string?>(TaskCreationOptions.RunContinuationsAsynchronously);var cancelCalled=false;
        using var cancellation=new CancellationTokenSource();
        var waiting=WindowsSpeechToTextService.AwaitCompletionWithCancellationAsync(completion,()=>cancelCalled=true,cancellation.Token);
        cancellation.Cancel();
        var error=await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>waiting.WaitAsync(TimeSpan.FromSeconds(1),TestContext.Current.CancellationToken));
        Assert.True(cancelCalled);Assert.Equal(cancellation.Token,error.CancellationToken);
    }
    [Fact]
    public async Task SpeechService_ReturnsTaskBeforeSynchronousRecognizerInitializationCompletes()
    {
        var testCancellation = TestContext.Current.CancellationToken;
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        using var returned = new ManualResetEventSlim();
        var callerThread = 0;
        var coreThread = 0;
        Task<string?>? recognition = null;
        Exception? callerError = null;
        var service = new WindowsSpeechToTextService((_, _) =>
        {
            coreThread = Environment.CurrentManagedThreadId;
            entered.Set();
            release.Wait(testCancellation);
            return Task.FromResult<string?>("完成");
        });
        var caller = new Thread(() =>
        {
            try
            {
                callerThread = Environment.CurrentManagedThreadId;
                recognition = service.RecognizeOnceAsync("system", testCancellation);
                returned.Set();
            }
            catch (Exception ex)
            {
                callerError = ex;
                returned.Set();
            }
        });
        caller.SetApartmentState(ApartmentState.STA);
        caller.Start();

        try
        {
            Assert.True(entered.Wait(TimeSpan.FromSeconds(2), testCancellation), "后台识别核心没有启动");
            Assert.True(returned.Wait(TimeSpan.FromMilliseconds(500), testCancellation), "公开方法被同步识别初始化阻塞");
            Assert.NotEqual(callerThread, coreThread);
        }
        finally
        {
            release.Set();
            Assert.True(caller.Join(TimeSpan.FromSeconds(2)), "调用线程没有正常结束");
        }

        Assert.Null(callerError);
        Assert.Equal(
            "完成",
            await (recognition ?? throw new InvalidOperationException("未返回识别任务"))
                .WaitAsync(TimeSpan.FromSeconds(2), testCancellation));
    }
    private static AiProviderSettings ValidProvider(string id)=>new(){Id=id,Name="Provider",Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model",CredentialId="credential"};
    private static string TestDirectory(){var path=Path.Combine(Path.GetTempPath(),"MewuAI.Tests",Guid.NewGuid().ToString("N"));Directory.CreateDirectory(path);return path;}
}
