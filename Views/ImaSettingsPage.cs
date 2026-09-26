// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;

/// <summary>ima 设置页：腾讯 ima 知识库归档。
/// 凭证（Client ID + API Key）在 ima.qq.com/agent-interface 生成；API Key 经 DPAPI 保存。
/// 圈选截图后工具栏出现“ima”按钮，把图片上传到所选知识库。</summary>
internal sealed class ImaSettingsPage : StackPanel
{
    private readonly CheckBox _enable=new();
    private readonly TextBox _clientId=new(){Padding=new Thickness(8,5,8,5)};
    private readonly PasswordBox _apiKey=new(){Padding=new Thickness(8,5,8,5)};
    private readonly ComboBox _knowledgeBase=new(){Padding=new Thickness(8,5,8,5)};
    private readonly TextBlock _status=new();
    private readonly Button _saveSecret=new(),_test=new(),_clearSecret=new();
    private readonly CancellationToken _token;
    private bool _loaded;
    internal bool Enabled=>_enable.IsChecked==true;
    internal string ClientId=>_clientId.Text.Trim();
    internal string KnowledgeBaseId=>(_knowledgeBase.SelectedItem as KnowledgeBaseOption)?.Id??string.Empty;
    internal string KnowledgeBaseName=>(_knowledgeBase.SelectedItem as KnowledgeBaseOption)?.Name??string.Empty;

    private sealed class KnowledgeBaseOption
    {
        internal string Id{get;}
        internal string Name{get;}
        internal KnowledgeBaseOption(string id,string name){Id=id;Name=string.IsNullOrWhiteSpace(name)?id[..Math.Min(12,id.Length)]+"…":name;}
        public override string ToString()=>Name;
    }

    internal ImaSettingsPage(AppSettings settings,CancellationToken token)
    {
        _token=token;
        var form=new AiSettingsForm("ima",T("把圈选截图归档进腾讯 ima 知识库：图片经 ima 官方 OpenAPI 上传（create_media → COS 直传 → add_knowledge），自动 OCR 入库、多端同步。凭证在 ima.qq.com/agent-interface 生成（ima 未向第三方开放扫码授权，官方接入方式即 Client ID/API Key）；API Key 只保存在本机（DPAPI 加密）。","Archive the selected screenshot into a Tencent ima knowledge base: the image is uploaded via the official ima OpenAPI (create_media → direct COS upload → add_knowledge) with automatic OCR and multi-device sync. Get the credentials at ima.qq.com/agent-interface (ima offers no third-party QR authorization; Client ID/API Key is the official way). The API Key stays on this machine (DPAPI encrypted)."),_status);
        Children.Add(form);
        form.AddAction(_saveSecret,T("保存 API Key","Save API Key"));
        form.AddAction(_test,T("测试连接","Test connection"));
        form.AddAction(_clearSecret,T("清除 API Key","Clear API Key"));
        _enable.Content=T("启用 ima 知识库归档（工具栏出现“ima”按钮）","Enable ima archiving (adds the ima toolbar button)");
        _enable.IsChecked=settings.ImaEnabled;
        _clientId.Text=settings.ImaClientId;
        _apiKey.ToolTip=T("点击“保存 API Key”后才会写入本机（DPAPI 加密）。","Only written locally (DPAPI) after clicking “Save API Key”.");
        form.Fields.Children.Add(AiSettingsForm.Field(T("Client ID","Client ID"),_clientId));
        form.Fields.Children.Add(AiSettingsForm.Field(T("API Key","API Key"),_apiKey));
        _knowledgeBase.ToolTip=T("点击“测试连接”后从账号中加载；留空默认第一个可写知识库。","Loaded after clicking “Test connection”; empty = first addable knowledge base.");
        form.Fields.Children.Add(AiSettingsForm.Field(T("知识库（留空为默认）","Knowledge base (empty = default)"),_knowledgeBase));
        form.Fields.Children.Add(_enable);
        if(!string.IsNullOrWhiteSpace(settings.ImaKnowledgeBaseId))
        {
            var saved=new KnowledgeBaseOption(settings.ImaKnowledgeBaseId,settings.ImaKnowledgeBaseName);
            _knowledgeBase.Items.Add(saved);_knowledgeBase.SelectedItem=saved;
        }
        _status.Text=ImaVaultService.IsConfigured(settings)?T("已配置。","Configured."):T("填写 Client ID 并保存 API Key 后点击“测试连接”。","Enter the Client ID, save the API Key, then click “Test connection”.");
        _saveSecret.Click+=(_,_)=>SaveSecret();
        _clearSecret.Click+=(_,_)=>ClearSecret();
        _test.Click+=async(_,_)=>await TestConnectionAsync();
        Loaded+=async(_,_)=>
        {
            if(_loaded||!ImaVaultService.IsConfigured(settings))return;_loaded=true;
            await LoadKnowledgeBasesAsync(preferSaved:false);
        };
    }

    private void SaveSecret()
    {
        try
        {
            ImaVaultService.SaveSecret(_apiKey.Password);
            _status.Foreground=Brushes.SlateGray;
            _status.Text=T("API Key 已保存（本机加密）。","API Key saved (encrypted on this machine).");
        }
        catch(Exception ex)
        {
            _status.Foreground=Brushes.Firebrick;
            _status.Text=ex.Message;
        }
    }

    private void ClearSecret()
    {
        ImaVaultService.ClearSecret();
        _apiKey.Clear();
        _status.Foreground=Brushes.SlateGray;
        _status.Text=T("API Key 已清除。","API Key cleared.");
    }

    private AppSettings CurrentSettings()=>new()
    {
        ImaEnabled=true,
        ImaClientId=ClientId,
        ImaKnowledgeBaseId=KnowledgeBaseId,
        ImaKnowledgeBaseName=KnowledgeBaseName
    };

    private async Task TestConnectionAsync()
    {
        if(_token.IsCancellationRequested)return;
        // 若密码框里已输入新 Key，先自动保存，避免“填了 Key 却没点保存”导致测试失败。
        if(_apiKey.Password.Length>0){try{ImaVaultService.SaveSecret(_apiKey.Password);}catch{}}
        _test.IsEnabled=false;
        _status.Foreground=Brushes.SlateGray;
        _status.Text=T("正在连接 ima…","Connecting to ima…");
        try
        {
            var list=await Task.Run(()=>ImaVaultService.GetKnowledgeBasesAsync(CurrentSettings(),_token),_token);
            _token.ThrowIfCancellationRequested();
            var preferred=KnowledgeBaseId;
            _knowledgeBase.Items.Clear();
            foreach(var kb in list)_knowledgeBase.Items.Add(new KnowledgeBaseOption(kb.Id,kb.Name));
            _knowledgeBase.SelectedItem=_knowledgeBase.Items.OfType<KnowledgeBaseOption>().FirstOrDefault(kb=>kb.Id==preferred)
                ??_knowledgeBase.Items.OfType<KnowledgeBaseOption>().FirstOrDefault();
            _status.Foreground=Brushes.SlateGray;
            _status.Text=list.Count==0
                ?T("连接成功，但账号下没有可添加内容的知识库，请先在 ima 中创建。","Connected, but no addable knowledge bases exist. Create one in ima first.")
                :T($"连接成功，加载到 {list.Count} 个知识库。",$"Connected. {list.Count} knowledge base(s) loaded.");
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex)
        {
            _status.Foreground=Brushes.Firebrick;
            _status.Text=T($"连接失败：{ex.Message}",$"Connection failed: {ex.Message}");
            new PrivacyLogger().Info("ImaTestConnection",$"{ex.GetType().Name}: {ex.Message}");
        }
        finally{_test.IsEnabled=true;}
    }

    private async Task LoadKnowledgeBasesAsync(bool preferSaved)
    {
        try
        {
            var preferred=KnowledgeBaseId;
            var list=await Task.Run(()=>ImaVaultService.GetKnowledgeBasesAsync(CurrentSettings(),_token),_token);
            _token.ThrowIfCancellationRequested();
            _knowledgeBase.Items.Clear();
            foreach(var kb in list)_knowledgeBase.Items.Add(new KnowledgeBaseOption(kb.Id,kb.Name));
            _knowledgeBase.SelectedItem=_knowledgeBase.Items.OfType<KnowledgeBaseOption>().FirstOrDefault(kb=>kb.Id==preferred)
                ??(preferSaved?null:_knowledgeBase.Items.OfType<KnowledgeBaseOption>().FirstOrDefault());
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex){new PrivacyLogger().Info("ImaLoadKnowledgeBases",ex.GetType().Name);}
    }

    private static string T(string zh,string en)=>LocalizationService.T(zh,en);
}
