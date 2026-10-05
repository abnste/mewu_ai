// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class MemoryPersistenceTests
{
    [Fact]
    public void SettingsPersistEncryptedValuesAndRemoveReplacedAndDeletedCredentials()
    {
        InDirectory(root =>
        {
            var path = Path.Combine(root, "settings.json");
            var service = new SettingsService(path);
            var settings = Settings("本机测试-secret-42");
            var credentials = new CredentialService(Path.Combine(root, "Credentials"));
            service.Save(settings);
            var first = settings.MemoryEntries.Single().CredentialId;
            Assert.Equal(string.Empty, settings.MemoryEntries.Single().Value);
            Assert.DoesNotContain("本机测试-secret-42", File.ReadAllText(path));
            var loaded = service.Load();
            Assert.Equal("本机测试-secret-42", MemoryStore.Read(loaded.MemoryEntries.Single(), credentials));
            loaded.MemoryEntries.Single().Value = "changed-secret";
            service.Save(loaded);
            Assert.Null(credentials.Read(first));
            Assert.Equal("changed-secret", MemoryStore.Read(service.Load().MemoryEntries.Single(), credentials));
            loaded.MemoryEntries.Clear(); service.Save(loaded);
            Assert.Empty(Directory.GetFiles(Path.Combine(root, "Credentials")));
        });
    }

    [Fact]
    public void FailedSettingsCommitRetainsPreviousSecretAndRollsBackNewCredentials()
    {
        InDirectory(root =>
        {
            var path = Path.Combine(root, "settings.json");
            var service = new SettingsService(path);
            var settings = Settings("previous-secret"); service.Save(settings);
            var previousBytes = File.ReadAllBytes(path);
            var id = settings.MemoryEntries.Single().CredentialId;
            settings.MemoryEntries.Single().Value = "replacement-secret";
            File.SetAttributes(path, FileAttributes.ReadOnly);
            try { Assert.Throws<UnauthorizedAccessException>(() => service.Save(settings)); }
            finally { File.SetAttributes(path, FileAttributes.Normal); }
            Assert.Equal(previousBytes, File.ReadAllBytes(path));
            Assert.Equal(id, settings.MemoryEntries.Single().CredentialId);
            Assert.Equal("replacement-secret", settings.MemoryEntries.Single().Value);
            var credentials = new CredentialService(Path.Combine(root, "Credentials"));
            Assert.Equal("previous-secret", credentials.Read(id));
            Assert.Single(Directory.GetFiles(Path.Combine(root, "Credentials")));
        });
    }

    [Fact]
    public void InvalidOtherSettingsNeverChangeStoredMemory()
    {
        InDirectory(root =>
        {
            var path = Path.Combine(root, "settings.json"); var service = new SettingsService(path);
            var settings = Settings("keep-this-secret"); service.Save(settings);
            var previousBytes = File.ReadAllBytes(path);
            settings.MemoryEntries.Single().Value = "new-secret"; settings.DefaultProviderId = "missing-provider";
            Assert.Throws<InvalidOperationException>(() => service.Save(settings));
            Assert.Equal(previousBytes, File.ReadAllBytes(path));
            Assert.Single(Directory.GetFiles(Path.Combine(root, "Credentials")));
        });
    }

    [Fact]
    public void NormalizesNullMemoryFieldsWithoutLosingProviderConfiguration()
    {
        InDirectory(root =>
        {
            var path = Path.Combine(root, "settings.json"); var settings = Settings("legacy-secret");
            settings.MemoryEntries.Single().Keywords = [null!, " 账号 ", "账号"];
            settings.MemoryEntries.Single().FieldKind = null!;
            settings.MemoryEntries.Add(null!);
            File.WriteAllText(path, JsonSerializer.Serialize(settings));
            var loaded = new SettingsService(path).Load();
            Assert.Empty(loaded.ConfigurationErrors);
            Assert.Equal("provider", loaded.Providers.Single().Id);
            Assert.Equal(new[] { "账号" }, Assert.Single(loaded.MemoryEntries).Keywords);
            Assert.Equal("auto", loaded.MemoryEntries.Single().FieldKind);
        });
    }

    [Fact]
    public void CanceledScanDoesNotAccessTargetWindow()
    {
        using var canceled = new CancellationTokenSource(); canceled.Cancel();
        Assert.Throws<OperationCanceledException>(() => MemoryFillService.FindInputs(new(0, 0, 0), new(0, 0, 100, 100), canceled.Token));
    }

    [Fact]
    public void KeyboardNativeInputUsesCompleteWindowsUnionLayout()
        => Assert.Equal(IntPtr.Size == 8 ? 40 : 28, NativeMethods.NativeInputSize);

    private static AppSettings Settings(string value) => new()
    {
        Providers = [new() { Id = "provider", Name = "Provider", Type = "OpenAICompatible", BaseUrl = "https://example.invalid/v1", Model = "model", CredentialId = "credential" }],
        DefaultProviderId = "provider", MemoryEntries = [new() { Keywords = ["账号"], Value = value }]
    };

    private static void InDirectory(Action<string> test)
    {
        var root = Path.Combine(Path.GetTempPath(), "MewuAI.Tests", "memory-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try { test(root); } finally { Directory.Delete(root, true); }
    }
}
