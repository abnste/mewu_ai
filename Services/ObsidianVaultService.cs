// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Text.Json.Nodes;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;

/// <summary>
/// Obsidian 截图笔记：把圈选图片保存进用户指定的 vault（附件目录），
/// 并创建一条引用该图片的 Markdown 笔记；可选通过 obsidian:// URI 直接打开。
/// 纯本地文件操作，无凭据、无网络。vault 列表优先从 Obsidian 的
/// %APPDATA%\obsidian\obsidian.json 注册表读取，失败时浅扫常见目录。
/// </summary>
internal static class ObsidianVaultService
{
    private const int MaxRegisteredVaults=64;
    private const int MaxFallbackDirectories=128;
    internal static byte[] EncodePng(BitmapSource image)
    {
        using var stream=new MemoryStream();
        var encoder=new PngBitmapEncoder();
        encoder.Frames.Add(BitmapFrame.Create(image));
        encoder.Save(stream);
        return stream.ToArray();
    }

    /// <summary>发现本机 Obsidian vault（不含 .obsidian 校验失败的条目）。</summary>
    internal static IReadOnlyList<(string Path,string Name)> FindVaults(CancellationToken cancellationToken=default)
    {
        var vaults=new List<(string,string)>();
        try
        {
            cancellationToken.ThrowIfCancellationRequested();
            var registry=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),"obsidian","obsidian.json");
            if(File.Exists(registry))
            {
                var payload=JsonNode.Parse(File.ReadAllText(registry))?["vaults"]?.AsObject();
                if(payload is not null)
                    foreach(var entry in payload)
                    {
                        cancellationToken.ThrowIfCancellationRequested();
                        if(vaults.Count>=MaxRegisteredVaults)break;
                        var path=entry.Value?["path"]?.GetValue<string>();
                        if(!string.IsNullOrWhiteSpace(path)&&Directory.Exists(Path.Combine(path,".obsidian")))
                            vaults.Add((path!,new DirectoryInfo(path!).Name));
                    }
            }
        }
                catch(IOException){/* 注册表读取失败时回退到目录扫描 */}
                catch(UnauthorizedAccessException){}
                catch(System.Security.SecurityException){}
        if(vaults.Count==0)
        {
            foreach(var root in new[]{Environment.SpecialFolder.MyDocuments,Environment.SpecialFolder.DesktopDirectory})
            {
                try
                {
                    var baseDir=Environment.GetFolderPath(root);
                    if(string.IsNullOrWhiteSpace(baseDir))continue;
                    var probed=0;
                    foreach(var dir in Directory.EnumerateDirectories(baseDir))
                    {
                        cancellationToken.ThrowIfCancellationRequested();
                        if(++probed>MaxFallbackDirectories)break;
                        if(Directory.Exists(Path.Combine(dir,".obsidian")))
                            vaults.Add((dir,new DirectoryInfo(dir).Name));
                        if(vaults.Count>=8)break;
                    }
                }
                catch(IOException){}
                catch(UnauthorizedAccessException){}
                catch(System.Security.SecurityException){}
                if(vaults.Count>0)break;
            }
        }
        return vaults.DistinctBy(vault=>vault.Item1,StringComparer.OrdinalIgnoreCase).ToList();
    }

    internal static bool IsConfigured(AppSettings settings)
        =>settings.ObsidianEnabled&&!string.IsNullOrWhiteSpace(settings.ObsidianVaultPath)
          &&Directory.Exists(Path.Combine(settings.ObsidianVaultPath,".obsidian"));

    /// <summary>保存截图并生成笔记，返回笔记的完整路径。</summary>
    internal static async Task<string> SaveNoteAsync(AppSettings settings,byte[] png,CancellationToken cancellationToken)
    {
        if(!IsConfigured(settings))
            throw new InvalidOperationException(LocalizationService.T("Obsidian 未启用或 vault 无效。请到 设置 → MCP → Obsidian 选择 vault。","Obsidian is not enabled or the vault is invalid. Pick a vault under Settings → MCP → Obsidian."));
        var vault=settings.ObsidianVaultPath;
        var stamp=DateTime.Now.ToString("yyyyMMdd-HHmmss",System.Globalization.CultureInfo.InvariantCulture);
        var attachFolder=string.IsNullOrWhiteSpace(settings.ObsidianAttachFolder)?"attachments":settings.ObsidianAttachFolder.Trim().Trim('\\','/');
        var noteFolder=string.IsNullOrWhiteSpace(settings.ObsidianNoteFolder)?string.Empty:settings.ObsidianNoteFolder.Trim().Trim('\\','/');
        var attachDir=SafeCombine(vault,attachFolder);
        var noteDir=noteFolder.Length==0?vault:SafeCombine(vault,noteFolder);
        Directory.CreateDirectory(attachDir);
        Directory.CreateDirectory(noteDir);
        var imageName=$"MewuAI-{stamp}.png";
        var imagePath=Path.Combine(attachDir,imageName);
        var noteName=$"MewuAI-{stamp}.md";
        var notePath=Path.Combine(noteDir,noteName);
        await File.WriteAllBytesAsync(imagePath,png,cancellationToken).ConfigureAwait(false);
        var english=LocalizationService.IsEnglish;
        var content=new System.Text.StringBuilder()
            .Append(english?"# Screen note ":"# 屏幕笔记 ").AppendLine(stamp)
            .AppendLine()
            .AppendLine($"![[{imageName}]]")
            .AppendLine()
            .AppendLine(english?$"- Captured by mewu_ai at {DateTime.Now:yyyy-MM-dd HH:mm}":$"- 由 mewu_ai 屏幕截图创建于 {DateTime.Now:yyyy-MM-dd HH:mm}");
        await File.WriteAllTextAsync(notePath,content.ToString(),new System.Text.UTF8Encoding(false),cancellationToken).ConfigureAwait(false);
        if(settings.ObsidianOpenAfterSave)
        {
            try
            {
                var vaultName=new DirectoryInfo(vault).Name;
                var relative=Uri.EscapeDataString(noteFolder.Length==0?noteName:$"{noteFolder}/{noteName}");
                System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo($"obsidian://open?vault={Uri.EscapeDataString(vaultName)}&file={relative}"){UseShellExecute=true});
            }
            catch(Exception ex){new PrivacyLogger().Info("ObsidianOpen",ex.GetType().Name);}
        }
        return notePath;
    }

    private static string SafeCombine(string vault,string relative)
    {
        // 拒绝以驱动器根/绝对路径越界，仅允许 vault 内的相对子目录。
        var sanitized=relative.Replace('/','\\');
        var combined=Path.GetFullPath(Path.Combine(vault,sanitized));
        var vaultRoot=Path.GetFullPath(vault);
        if(!combined.StartsWith(vaultRoot+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase)&&!string.Equals(combined,vaultRoot,StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException(LocalizationService.T("Obsidian 附件/笔记目录必须位于 vault 内。","The Obsidian attachment/note folder must stay inside the vault."));
        return combined;
    }
}
