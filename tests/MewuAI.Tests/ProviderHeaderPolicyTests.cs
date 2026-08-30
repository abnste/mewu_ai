using System.Text.Json;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ProviderHeaderPolicyTests
{
    [Fact]
    public void SensitiveHeadersCanBeEditedButCannotBePersistedInPlaintext()
    {
        var headers=new Dictionary<string,string>{{"Authorization","Bearer secret"}};
        ProviderHeaderPolicy.EnsureValid(headers);
        var error=Assert.Throws<InvalidOperationException>(()=>ProviderHeaderPolicy.EnsureSafeToPersist(headers));
        Assert.Contains("明文",error.Message);
    }

    [Fact]
    public void AcceptsNonSensitiveTenantHeaders()
    {
        ProviderHeaderPolicy.EnsureSafeToPersist(new Dictionary<string,string>{{"X-Tenant","demo"}});
    }

    [Theory]
    [InlineData("Ocp-Apim-Subscription-Key")]
    [InlineData("X-Subscription-Key")]
    [InlineData("X-Auth")]
    [InlineData("Cookie")]
    [InlineData("X-Signature")]
    [InlineData("X-Client-Secret")]
    [InlineData("X-Password")]
    public void CommonAuthenticationHeadersAreAlwaysTreatedAsSensitive(string name)
    {
        Assert.True(ProviderHeaderCredentialService.IsSensitive(name));
        Assert.True(ProviderHeaderCredentialService.IsAuthentication(name));
    }

    [Fact]
    public void RejectsHeaderInjection()
    {
        Assert.Throws<InvalidOperationException>(()=>ProviderHeaderPolicy.EnsureValid(new Dictionary<string,string>{{"X-Tenant","demo\r\nAuthorization: secret"}}));
    }

    [Theory]
    [InlineData("Authorization")]
    [InlineData("X-Api-Key")]
    [InlineData("X-Client-Secret")]
    public void RejectsBlankSensitiveAuthenticationHeaders(string name)
    {
        Assert.Throws<InvalidOperationException>(()=>ProviderHeaderPolicy.EnsureValid(new Dictionary<string,string>{{name,"  "}}));
    }

    [Fact]
    public void RejectsInvalidDuplicateAndTransportHeaders()
    {
        Assert.Throws<InvalidOperationException>(()=>ProviderHeaderPolicy.EnsureValid(new Dictionary<string,string>{{"Bad Header","value"}}));
        Assert.Throws<InvalidOperationException>(()=>ProviderHeaderPolicy.EnsureValid(new Dictionary<string,string>{{"X-Test","one"},{"x-test","two"}}));
        Assert.Throws<InvalidOperationException>(()=>ProviderHeaderPolicy.EnsureValid(new Dictionary<string,string>{{"Content-Length","10"}}));
    }

    [Fact]
    public void CredentialMappingsMustBeSensitiveUniqueAndDisjointFromPlainHeaders()
    {
        Assert.Throws<InvalidOperationException>(()=>ProviderHeaderPolicy.EnsureCredentialMappingsValid(
            new Dictionary<string,string>(),new Dictionary<string,string>{{"X-Tenant","credential"}}));
        Assert.Throws<InvalidOperationException>(()=>ProviderHeaderPolicy.EnsureCredentialMappingsValid(
            new Dictionary<string,string>{{"Authorization","plaintext"}},new Dictionary<string,string>{{"authorization","credential"}}));
        Assert.Throws<InvalidOperationException>(()=>ProviderHeaderPolicy.EnsureCredentialMappingsValid(
            new Dictionary<string,string>(),new Dictionary<string,string>{{"Authorization","../other-provider"}}));
    }

    [Fact]
    public void FactoryRejectsTamperedCrossProviderCredentialMappingBeforeHydration()
    {
        var root=TestDirectory();
        try
        {
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));credentials.Save("provider-b-secret","must-not-leak");credentials.Save("provider-a-key","primary");
            var provider=new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model",CredentialId="provider-a-key",SensitiveHeaderCredentialIds=new(){{"X-Tenant","provider-b-secret"}}};
            var settings=new AppSettings{Providers=[provider],DefaultProviderId=provider.Id};
            var factory=new AiProviderFactory(credentials,null);
            Assert.Null(factory.Create(settings,out var error));
            Assert.Contains("不是敏感 Header",error,StringComparison.Ordinal);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void SensitiveHeadersRoundTripThroughDpapiWithoutEnteringSettingsJson()
    {
        var root=TestDirectory();
        try
        {
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));var headers=new ProviderHeaderCredentialService(credentials);
            var provider=new AiProviderSettings{CustomHeaders=new(){{"Authorization","Bearer secret-value"},{"X-Tenant","demo"}}};
            headers.ProtectEditableHeaders(provider);
            Assert.False(provider.CustomHeaders.ContainsKey("Authorization"));Assert.Equal("demo",provider.CustomHeaders["X-Tenant"]);Assert.Single(provider.SensitiveHeaderCredentialIds);
            var json=JsonSerializer.Serialize(provider);Assert.DoesNotContain("secret-value",json,StringComparison.Ordinal);
            var hydrated=headers.CreateHydratedCopy(provider);Assert.Equal("Bearer secret-value",hydrated.CustomHeaders["Authorization"]);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void SettingsLoadMigratesLegacyPlaintextSensitiveHeaders()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");
            var providerId=Guid.NewGuid().ToString("N");
            var legacy=new AppSettings{DefaultProviderId=providerId,Providers=[new AiProviderSettings{Id=providerId,CustomHeaders=new(){{"api-key","legacy-secret"}}}]};
            File.WriteAllText(path,JsonSerializer.Serialize(legacy));
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));var headers=new ProviderHeaderCredentialService(credentials);
            var loaded=new SettingsService(path,headers).Load();var provider=Assert.Single(loaded.Providers);
            Assert.False(provider.CustomHeaders.ContainsKey("api-key"));Assert.Single(provider.SensitiveHeaderCredentialIds);
            Assert.DoesNotContain("legacy-secret",File.ReadAllText(path),StringComparison.Ordinal);
            Assert.Equal("legacy-secret",headers.CreateHydratedCopy(provider).CustomHeaders["api-key"]);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void SettingsLoadDoesNotRewriteStructurallyInvalidLegacyConfigDuringMigration()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");
            var duplicateA=new AiProviderSettings{Id="duplicate",CustomHeaders=new(){{"api-key","legacy-secret"}}};
            var duplicateB=new AiProviderSettings{Id="duplicate"};
            var legacy=new AppSettings{Providers=[duplicateA,duplicateB],DefaultProviderId="duplicate"};
            File.WriteAllText(path,JsonSerializer.Serialize(legacy));
            var original=File.ReadAllText(path);
            var credentialDirectory=Path.Combine(root,"Credentials");
            var loaded=new SettingsService(path,new ProviderHeaderCredentialService(new CredentialService(credentialDirectory)),null).Load();

            Assert.NotEmpty(loaded.ConfigurationErrors);
            Assert.Equal(original,File.ReadAllText(path));
            Assert.Empty(Directory.Exists(credentialDirectory)?Directory.GetFiles(credentialDirectory):[]);
            Assert.Equal("legacy-secret",loaded.Providers[0].CustomHeaders["api-key"]);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void FailedSettingsMigrationRollsBackNewCredentialsAndPreservesTheOldMapping()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");var credentialDirectory=Path.Combine(root,"Credentials");
            var credentials=new CredentialService(credentialDirectory);credentials.Save("existing-header","old-secret");
            var legacyProvider=new AiProviderSettings
            {
                CustomHeaders=new(){{"api-key","new-secret"}},
                SensitiveHeaderCredentialIds=new(){{"api-key","existing-header"}}
            };
            var legacy=new AppSettings{Providers=[legacyProvider],DefaultProviderId=legacyProvider.Id};
            File.WriteAllText(path,JsonSerializer.Serialize(legacy));
            using var settingsLock=new FileStream(path,FileMode.Open,FileAccess.Read,FileShare.Read);

            var loaded=new SettingsService(path,new ProviderHeaderCredentialService(credentials),null).Load();

            Assert.True(loaded.HasSensitiveCredentialErrors);
            Assert.Equal("new-secret",Assert.Single(loaded.Providers).CustomHeaders["api-key"]);
            Assert.Equal("old-secret",credentials.Read("existing-header"));
            Assert.Collection(Directory.GetFiles(credentialDirectory),file=>Assert.EndsWith("existing-header.bin",file,StringComparison.Ordinal));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void SuccessfulHeaderMigrationKeepsAnOldCredentialStillUsedAsAPrimaryApiKey()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"settings.json");var credentials=new CredentialService(Path.Combine(root,"Credentials"));
            credentials.Save("shared-credential","primary-secret");
            var legacyProvider=new AiProviderSettings
            {
                CustomHeaders=new(){{"api-key","header-secret"}},
                SensitiveHeaderCredentialIds=new(){{"api-key","shared-credential"}}
            };
            var primaryProvider=new AiProviderSettings{CredentialId="shared-credential"};
            var legacy=new AppSettings{Providers=[legacyProvider,primaryProvider],DefaultProviderId=legacyProvider.Id};
            File.WriteAllText(path,JsonSerializer.Serialize(legacy));

            var loaded=new SettingsService(path,new ProviderHeaderCredentialService(credentials),null).Load();

            Assert.Equal("primary-secret",credentials.Read("shared-credential"));
            Assert.NotEqual("shared-credential",loaded.Providers[0].SensitiveHeaderCredentialIds["api-key"]);
            Assert.Equal("header-secret",new ProviderHeaderCredentialService(credentials).CreateHydratedCopy(loaded.Providers[0]).CustomHeaders["api-key"]);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void HydrationReportsMissingCredentialWithoutDroppingItsMapping()
    {
        var root=TestDirectory();
        try
        {
            var headers=new ProviderHeaderCredentialService(new CredentialService(Path.Combine(root,"Credentials")));
            var provider=new AiProviderSettings{SensitiveHeaderCredentialIds=new(){{"api-key","missing-credential"}}};
            var unavailable=new HashSet<string>(StringComparer.OrdinalIgnoreCase);var editable=headers.CreateHydratedCopy(provider,unavailable);
            Assert.Contains("api-key",unavailable);Assert.False(editable.CustomHeaders.ContainsKey("api-key"));Assert.Equal("missing-credential",editable.SensitiveHeaderCredentialIds["api-key"]);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void SettingsEditorKeepsDamagedCredentialMappingInAnIsolatedCopy()
    {
        var root=TestDirectory();
        try
        {
            var source=new AiProviderSettings
            {
                Id="damaged-provider",
                Name="Damaged",
                CustomHeaders=new(){{"X-Tenant","demo"}},
                SensitiveHeaderCredentialIds=new(){{"X-Tenant","existing-credential"}}
            };
            var service=new ProviderHeaderCredentialService(new CredentialService(Path.Combine(root,"Credentials")));

            var result=ProviderEditorHydrationPolicy.TryHydrate(source,service);

            Assert.NotNull(result.Warning);
            Assert.Contains("原设置和凭据未被改动",result.Warning,StringComparison.Ordinal);
            Assert.NotSame(source,result.Provider);
            Assert.Equal("existing-credential",result.Provider.SensitiveHeaderCredentialIds["X-Tenant"]);
            Assert.Equal("existing-credential",source.SensitiveHeaderCredentialIds["X-Tenant"]);
            result.Provider.CustomHeaders["X-Tenant"]="changed";
            Assert.Equal("demo",source.CustomHeaders["X-Tenant"]);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void SettingsEditorStillHydratesValidMappingsAndReportsMissingCredentialNormally()
    {
        var root=TestDirectory();
        try
        {
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));
            credentials.Save("authorization-credential","Bearer test-value");
            var service=new ProviderHeaderCredentialService(credentials);
            var valid=new AiProviderSettings{SensitiveHeaderCredentialIds=new(){{"Authorization","authorization-credential"}}};
            var unavailable=new HashSet<string>(StringComparer.OrdinalIgnoreCase);

            var hydrated=ProviderEditorHydrationPolicy.TryHydrate(valid,service,unavailable);

            Assert.Null(hydrated.Warning);
            Assert.Equal("Bearer test-value",hydrated.Provider.CustomHeaders["Authorization"]);
            Assert.Empty(unavailable);

            var missing=new AiProviderSettings{SensitiveHeaderCredentialIds=new(){{"X-Api-Key","missing-credential"}}};
            var missingHeaders=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            var missingResult=ProviderEditorHydrationPolicy.TryHydrate(missing,service,missingHeaders);
            Assert.Null(missingResult.Warning);
            Assert.Contains("X-Api-Key",missingHeaders);
            Assert.Equal("missing-credential",missingResult.Provider.SensitiveHeaderCredentialIds["X-Api-Key"]);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void FactoryAcceptsProviderAuthenticatedOnlyByEncryptedCustomHeader()
    {
        var root=TestDirectory();
        try
        {
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));var headerCredentials=new ProviderHeaderCredentialService(credentials);
            var provider=new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model",CredentialId=string.Empty,CustomHeaders=new(){{"Authorization","Bearer header-only"}}};
            headerCredentials.ProtectEditableHeaders(provider);var settings=new AppSettings{Providers=[provider],DefaultProviderId=provider.Id};
            Assert.NotNull(new AiProviderFactory(credentials,null).Create(settings));
        }
        finally{Directory.Delete(root,true);}
    }

    [Theory]
    [InlineData("X-Api-Key")]
    [InlineData("X-Client-Secret")]
    [InlineData("X-Password")]
    public void FactoryAcceptsSensitiveCustomHeaderWithoutPrimaryApiKey(string headerName)
    {
        var root=TestDirectory();
        try
        {
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));var headerCredentials=new ProviderHeaderCredentialService(credentials);
            var provider=new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model",CredentialId=string.Empty,CustomHeaders=new(){{headerName,"header-only-secret"}}};
            headerCredentials.ProtectEditableHeaders(provider);var settings=new AppSettings{Providers=[provider],DefaultProviderId=provider.Id};
            Assert.NotNull(new AiProviderFactory(credentials,null).Create(settings));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void FactoryRejectsBlankCustomHeaderAsTheOnlyAuthentication()
    {
        var root=TestDirectory();
        try
        {
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));
            var provider=new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model",CredentialId=string.Empty,CustomHeaders=new(){{"X-Api-Key",string.Empty}}};
            var settings=new AppSettings{Providers=[provider],DefaultProviderId=provider.Id};
            Assert.Null(new AiProviderFactory(credentials,null).Create(settings));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void FactoryRejectsWhitespacePrimaryApiKey()
    {
        var root=TestDirectory();
        try
        {
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));
            var provider=new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model",CredentialId="credential"};
            credentials.Save(provider.CredentialId,"   ");
            Assert.Null(new AiProviderFactory(credentials,null).Create(new AppSettings{Providers=[provider],DefaultProviderId=provider.Id}));
        }
        finally{Directory.Delete(root,true);}
    }

    private static string TestDirectory(){var path=Path.Combine(Path.GetTempPath(),"MewuAI.Tests",Guid.NewGuid().ToString("N"));Directory.CreateDirectory(path);return path;}
}
