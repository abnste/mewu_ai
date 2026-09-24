// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;

/// <summary>Obsidian 设置页：选择 vault 与附件/笔记目录。
/// 圈选截图后工具栏出现“Obsidian”按钮，把图片存入 vault 并生成引用它的 Markdown 笔记。</summary>
internal sealed class ObsidianSettingsPage : StackPanel
{
    private readonly CheckBox _enable=new();
    private readonly ComboBox _vault=new(){Padding=new Thickness(8,5,8,5)};
    private readonly TextBox _attach=new(){Padding=new Thickness(8,5,8,5)};
    private readonly TextBox _notes=new(){Padding=new Thickness(8,5,8,5)};
    private readonly CheckBox _openAfterSave=new();
    private readonly TextBlock _status=new();
    private readonly Button _refresh=new();
    private readonly CancellationToken _token;
    private bool _loaded;
    internal bool Enabled=>_enable.IsChecked==true;
    internal string VaultPath=>(_vault.SelectedItem as VaultOption)?.Path??string.Empty;
    internal string AttachFolder=>_attach.Text.Trim();
    internal string NoteFolder=>_notes.Text.Trim();
    internal bool OpenAfterSave=>_openAfterSave.IsChecked==true;

    private sealed class VaultOption
    {
        private readonly string _name;
        internal string Path{get;}
        internal VaultOption(string path,string name){Path=path;_name=name;}
        public override string ToString()=>$"{_name} — {Path}";
    }

    internal ObsidianSettingsPage(AppSettings settings,CancellationToken token)
    {
        _token=token;
        var form=new AiSettingsForm("Obsidian",T("把圈选截图保存进 Obsidian vault 作为笔记：图片存到附件目录，并自动创建一条引用该图片的 Markdown 笔记（可选直接在 Obsidian 中打开）。纯本地文件操作，无凭据、无网络。","Save the selected screenshot into an Obsidian vault as a note: the image goes to the attachments folder and a Markdown note referencing it is created (optionally opened in Obsidian). Fully local file operations — no credentials, no network."),_status);
        Children.Add(form);
        form.AddAction(_refresh,T("刷新 vault 列表","Refresh vault list"));
        _attach.Text=settings.ObsidianAttachFolder;
        _notes.Text=settings.ObsidianNoteFolder;
        _enable.Content=T("启用 Obsidian 截图笔记（工具栏出现“Obsidian”按钮）","Enable Obsidian screenshot notes (adds the Obsidian toolbar button)");
        _enable.IsChecked=settings.ObsidianEnabled;
        _openAfterSave.Content=T("保存后在 Obsidian 中打开笔记","Open the note in Obsidian after saving");
        _openAfterSave.IsChecked=settings.ObsidianOpenAfterSave;
        _vault.ToolTip=T("从本机检测到的 vault 中选择（需已安装 Obsidian 并创建 vault）。","Choose from vaults detected on this machine (Obsidian must be installed).");
        form.Fields.Children.Add(AiSettingsForm.Field(T("Vault","Vault"),_vault));
        form.Fields.Children.Add(AiSettingsForm.Field(T("附件目录（vault 内相对路径）","Attachments folder (relative to vault)"),_attach));
        form.Fields.Children.Add(AiSettingsForm.Field(T("笔记目录（vault 内相对路径，留空为根目录）","Notes folder (relative to vault; empty = root)"),_notes));
        form.Fields.Children.Add(_openAfterSave);
        form.Fields.Children.Add(_enable);
        _status.Text=T("设置页打开后在后台检测 vault。","Vaults are detected in the background when this page opens.");
        if(!string.IsNullOrWhiteSpace(settings.ObsidianVaultPath))
        {
            var saved=new VaultOption(settings.ObsidianVaultPath,Path.GetFileName(settings.ObsidianVaultPath.TrimEnd(Path.DirectorySeparatorChar,Path.AltDirectorySeparatorChar)));
            _vault.Items.Add(saved);_vault.SelectedItem=saved;
        }
        _refresh.Click+=async(_,_)=>await RefreshVaultsAsync(VaultPath);
        Loaded+=async(_,_)=>
        {
            if(_loaded)return;_loaded=true;
            await RefreshVaultsAsync(settings.ObsidianVaultPath);
        };
    }

    private async Task RefreshVaultsAsync(string preferred)
    {
        if(!_refresh.IsEnabled||_token.IsCancellationRequested)return;
        _refresh.IsEnabled=false;_status.Foreground=Brushes.SlateGray;
        _status.Text=T("正在后台检测 vault…","Detecting vaults in the background…");
        try
        {
            var vaults=await Task.Run(()=>ObsidianVaultService.FindVaults(_token),_token);
            _token.ThrowIfCancellationRequested();
            _vault.Items.Clear();
            foreach(var (path,name) in vaults)_vault.Items.Add(new VaultOption(path,name));
            var match=_vault.Items.OfType<VaultOption>().FirstOrDefault(option=>string.Equals(option.Path,preferred,StringComparison.OrdinalIgnoreCase));
            if(match is null&&!string.IsNullOrWhiteSpace(preferred))
            {
                match=new VaultOption(preferred,Path.GetFileName(preferred.TrimEnd(Path.DirectorySeparatorChar,Path.AltDirectorySeparatorChar)));
                _vault.Items.Add(match);
            }
            _vault.SelectedItem=match??_vault.Items.OfType<VaultOption>().FirstOrDefault();
            if(vaults.Count==0)
            {
                _status.Foreground=Brushes.Firebrick;
                _status.Text=T("未检测到 vault。请先在 Obsidian 中创建 vault 后点击“刷新”。","No vaults detected. Create one in Obsidian, then click “Refresh”.");
            }
            else
            {
                _status.Foreground=Brushes.SlateGray;
                _status.Text=T($"检测到 {vaults.Count} 个 vault。",$"{vaults.Count} vault(s) detected.");
            }
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex)
        {
            _status.Foreground=Brushes.Firebrick;
            _status.Text=T("检测 vault 失败，请点击“刷新 vault 列表”重试。","Vault detection failed. Click “Refresh vault list” to retry.");
            new PrivacyLogger().Info("ObsidianVaultDiscovery",ex.GetType().Name);
        }
        finally{_refresh.IsEnabled=true;}
    }

    private static string T(string zh,string en)=>LocalizationService.T(zh,en);
}
