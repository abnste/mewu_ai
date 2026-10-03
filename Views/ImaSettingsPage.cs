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
    private readonly Func<AppSettings,CancellationToken,Task<IReadOnlyList<ImaVaultService.ImaKnowledgeBase>>> _getKnowledgeBases;
    private readonly Func<AppSettings,bool> _isConfigured;
    private readonly Action<string> _saveApiKey;
    private readonly Action _clearApiKey;
    private bool _loaded;
    private int _revision;
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
        :this(settings,token,ImaVaultService.GetKnowledgeBasesAsync,ImaVaultService.IsConfigured,ImaVaultService.SaveSecret,ImaVaultService.ClearSecret){}

    internal ImaSettingsPage(AppSettings settings,CancellationToken token,
        Func<AppSettings,CancellationToken,Task<IReadOnlyList<ImaVaultService.ImaKnowledgeBase>>> getKnowledgeBases,
        Func<AppSettings,bool> isConfigured,Action<string> saveApiKey,Action clearApiKey)
    {
        _token=token;
        _getKnowledgeBases=getKnowledgeBases;_isConfigured=isConfigured;_saveApiKey=saveApiKey;_clearApiKey=clearApiKey;
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
        _knowledgeBase.Items.Add(DefaultKnowledgeBase());
        _knowledgeBase.SelectedIndex=0;
        form.Fields.Children.Add(AiSettingsForm.Field(T("知识库（留空为默认）","Knowledge base (empty = default)"),_knowledgeBase));
        form.Fields.Children.Add(_enable);
        if(!string.IsNullOrWhiteSpace(settings.ImaKnowledgeBaseId))
        {
            var saved=new KnowledgeBaseOption(settings.ImaKnowledgeBaseId,settings.ImaKnowledgeBaseName);
            _knowledgeBase.Items.Add(saved);_knowledgeBase.SelectedItem=saved;
        }
        _status.Text=T("正在读取本机授权状态…","Reading local authorization status…");
        _clientId.TextChanged+=(_,_)=>
        {
            _revision++;
            _knowledgeBase.Items.Clear();_knowledgeBase.Items.Add(DefaultKnowledgeBase());_knowledgeBase.SelectedIndex=0;
            _status.Text=T("Client ID 已修改，请重新测试连接。","Client ID changed. Test the connection again.");
        };
        _saveSecret.Click+=(_,_)=>SaveSecret();
        _clearSecret.Click+=(_,_)=>ClearSecret();
        _test.Click+=async(_,_)=>await TestConnectionAsync();
        Loaded+=async(_,_)=>
        {
            if(_loaded||_token.IsCancellationRequested)return;_loaded=true;
            var snapshot=CurrentSettings();var revision=_revision;
            try
            {
                var configured=await Task.Run(()=>_isConfigured(snapshot),_token);
                if(!_token.IsCancellationRequested&&revision==_revision&&_test.IsEnabled)
                    _status.Text=configured?T("已保存凭据，点击“测试连接”加载知识库。","Credentials saved. Test the connection to load knowledge bases."):T("填写 Client ID 并保存 API Key 后点击“测试连接”。","Enter the Client ID, save the API Key, then click “Test connection”.");
            }
            catch(OperationCanceledException)when(_token.IsCancellationRequested){}
            catch(Exception ex){new PrivacyLogger().Info("ImaStatus",ex.GetType().Name);}
        };
    }

    private void SaveSecret()
    {
        if(_token.IsCancellationRequested)return;
        try
        {
            _saveApiKey(_apiKey.Password);
            _apiKey.Clear();_revision++;
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
        if(_token.IsCancellationRequested)return;
        try
        {
            _clearApiKey();_apiKey.Clear();_revision++;
            _status.Foreground=Brushes.SlateGray;
            _status.Text=T("API Key 已清除。","API Key cleared.");
        }
        catch(Exception ex){_status.Foreground=Brushes.Firebrick;_status.Text=ex.Message;}
    }

    private AppSettings CurrentSettings()=>new()
    {
        ImaEnabled=true,
        ImaClientId=ClientId,
        ImaKnowledgeBaseId=KnowledgeBaseId,
        ImaKnowledgeBaseName=KnowledgeBaseName
    };

    internal async Task TestConnectionAsync()
    {
        if(!_test.IsEnabled||_token.IsCancellationRequested)return;
        if(_apiKey.Password.Length>0)
        {
            _status.Foreground=Brushes.SlateGray;
            _status.Text=T("请先点击“保存 API Key”，再测试连接。","Save the API Key before testing the connection.");
            return;
        }
        // Capture WPF values before dispatching network work to a background thread.
        var snapshot=CurrentSettings();var revision=++_revision;
        _test.IsEnabled=false;_saveSecret.IsEnabled=false;_clearSecret.IsEnabled=false;
        _status.Foreground=Brushes.SlateGray;
        _status.Text=T("正在连接 ima…","Connecting to ima…");
        try
        {
            var list=await Task.Run(()=>_getKnowledgeBases(snapshot,_token),_token);
            _token.ThrowIfCancellationRequested();
            if(revision!=_revision)return;
            var preferred=KnowledgeBaseId;var preferredName=KnowledgeBaseName;
            _knowledgeBase.Items.Clear();
            _knowledgeBase.Items.Add(DefaultKnowledgeBase());
            foreach(var kb in list)_knowledgeBase.Items.Add(new KnowledgeBaseOption(kb.Id,kb.Name));
            var selected=_knowledgeBase.Items.OfType<KnowledgeBaseOption>().FirstOrDefault(kb=>kb.Id==preferred);
            var missing=selected is null&&!string.IsNullOrEmpty(preferred);
            if(missing){selected=new KnowledgeBaseOption(preferred,preferredName);_knowledgeBase.Items.Add(selected);}
            _knowledgeBase.SelectedItem=selected??_knowledgeBase.Items[0];
            _status.Foreground=Brushes.SlateGray;
            _status.Text=missing
                ?T("连接成功，但原知识库不可用，已保留原选择；请选择一个可写知识库。","Connected, but the saved knowledge base is unavailable. Its selection was preserved; choose an addable knowledge base.")
                :list.Count==0
                ?T("连接成功，但账号下没有可添加内容的知识库，请先在 ima 中创建。","Connected, but no addable knowledge bases exist. Create one in ima first.")
                :T($"连接成功，加载到 {list.Count} 个知识库。",$"Connected. {list.Count} knowledge base(s) loaded.");
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex)
        {
            if(_token.IsCancellationRequested||revision!=_revision)return;
            _status.Foreground=Brushes.Firebrick;
            _status.Text=T($"连接失败：{ex.Message}",$"Connection failed: {ex.Message}");
            new PrivacyLogger().Info("ImaTestConnection",ex.GetType().Name);
        }
        finally{_test.IsEnabled=true;_saveSecret.IsEnabled=true;_clearSecret.IsEnabled=true;}
    }

    private static KnowledgeBaseOption DefaultKnowledgeBase()=>new(string.Empty,T("默认（第一个可写知识库）","Default (first addable knowledge base)"));

    private static string T(string zh,string en)=>LocalizationService.T(zh,en);
}
