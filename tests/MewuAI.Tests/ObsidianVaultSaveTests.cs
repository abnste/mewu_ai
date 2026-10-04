// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ObsidianVaultSaveTests
{
    [Theory]
    [InlineData("../outside")]
    [InlineData(".. /outside")]
    [InlineData("folder/../../outside")]
    [InlineData("/outside")]
    [InlineData("C:\\outside")]
    [InlineData("notes:stream")]
    [InlineData("folder.")]
    public async Task RejectsNonRelativeAndWindowsAliasPathsBeforeWriting(string folder)
    {
        using var vault=new FixtureVault();
        var settings=vault.Settings();settings.ObsidianAttachFolder=folder;
        await Assert.ThrowsAsync<InvalidOperationException>(()=>ObsidianVaultService.SaveNoteAsync(settings,[1,2,3],TestContext.Current.CancellationToken));
        Assert.Empty(Directory.EnumerateFiles(vault.Root,"*",SearchOption.AllDirectories));
    }

    [Fact]
    public async Task SavesExactPngAndAnUnambiguousRelativeLinkWithoutOpeningApp()
    {
        using var vault=new FixtureVault();
        var settings=vault.Settings();settings.ObsidianAttachFolder="attachments [one]";settings.ObsidianNoteFolder="notes/nested";
        var bytes=new byte[]{1,2,3,4};
        var note=await ObsidianVaultService.SaveNoteAsync(settings,bytes,TestContext.Current.CancellationToken);
        var image=Assert.Single(Directory.EnumerateFiles(Path.Combine(vault.Root,"attachments [one]")));
        Assert.Equal(bytes,await File.ReadAllBytesAsync(image,TestContext.Current.CancellationToken));
        var markdown=await File.ReadAllTextAsync(note,TestContext.Current.CancellationToken);
        Assert.Contains("![](../../attachments%20%5Bone%5D/"+Path.GetFileName(image)+")",markdown);
        Assert.False((await File.ReadAllBytesAsync(note,TestContext.Current.CancellationToken)).AsSpan().StartsWith(new byte[]{0xef,0xbb,0xbf}));
        Assert.Equal(2,Directory.EnumerateFiles(vault.Root,"*",SearchOption.AllDirectories).Count());
    }

    [Fact]
    public async Task RejectsExistingAttachmentLinkOutsideVault()
    {
        using var vault=new FixtureVault();
        var outside=Path.Combine(vault.Container,"outside");Directory.CreateDirectory(outside);
        var link=Path.Combine(vault.Root,"attachments");
        try{Directory.CreateSymbolicLink(link,outside);}
        catch(UnauthorizedAccessException){Assert.Skip("Creating a directory symbolic link requires Windows developer mode or elevation.");return;}
        catch(IOException ex) when((ex.HResult&0xffff)==1314){Assert.Skip("Creating a directory symbolic link requires Windows developer mode or elevation.");return;}
        try
        {
            await Assert.ThrowsAsync<InvalidOperationException>(()=>ObsidianVaultService.SaveNoteAsync(vault.Settings(),[1],TestContext.Current.CancellationToken));
            Assert.Empty(Directory.EnumerateFileSystemEntries(outside));
        }
        finally{Directory.Delete(link);}
    }

    [Fact]
    public async Task PreCancelledSaveDoesNotCreateDirectoriesOrFiles()
    {
        using var vault=new FixtureVault();
        using var cancelled=new CancellationTokenSource();cancelled.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>ObsidianVaultService.SaveNoteAsync(vault.Settings(),[1],cancelled.Token));
        Assert.Equal(new[]{".obsidian"},Directory.EnumerateDirectories(vault.Root).Select(Path.GetFileName));
        Assert.Empty(Directory.EnumerateFiles(vault.Root,"*",SearchOption.AllDirectories));
    }

    [Fact]
    public void InvalidRegistryFallsBackToOnlyTheSuppliedTemporaryRoot()
    {
        using var vault=new FixtureVault();
        var registry=Path.Combine(vault.Container,"registry.json");
        File.WriteAllText(registry,"{broken");
        var found=ObsidianVaultService.FindVaults(registry,[vault.Container],TestContext.Current.CancellationToken);
        Assert.Equal(vault.Root,Assert.Single(found).Path);
    }

    [Fact]
    public async Task ExistingNoteDirectoryFilePreventsAnyImageWrite()
    {
        using var vault=new FixtureVault();
        var settings=vault.Settings();settings.ObsidianNoteFolder="blocked";
        var marker=Path.Combine(vault.Root,"blocked");File.WriteAllText(marker,"fixture-preserved");
        await Assert.ThrowsAsync<IOException>(()=>ObsidianVaultService.SaveNoteAsync(settings,[1],TestContext.Current.CancellationToken));
        Assert.Equal("fixture-preserved",File.ReadAllText(marker));
        Assert.Equal(new[]{marker},Directory.EnumerateFiles(vault.Root,"*",SearchOption.AllDirectories));
    }

    private sealed class FixtureVault:IDisposable
    {
        internal string Container {get;}=Path.Combine(Path.GetTempPath(),"MewuAI.Tests","Obsidian",Guid.NewGuid().ToString("N"));
        internal string Root {get;}
        internal FixtureVault(){Root=Path.Combine(Container,"vault");Directory.CreateDirectory(Path.Combine(Root,".obsidian"));}
        internal AppSettings Settings()=>new(){ObsidianEnabled=true,ObsidianVaultPath=Root,ObsidianOpenAfterSave=false,ObsidianAttachFolder="attachments",ObsidianNoteFolder="notes"};
        public void Dispose()=>Directory.Delete(Container,true);
    }
}
