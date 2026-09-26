// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;

/// <summary>飞书设置页：自建应用（AppId/AppSecret）。
/// 圈选截图后工具栏出现“飞书”按钮，图片上传后发送到指定会话（chat_id）或联系人（open_id）。
/// 应用需开通 im:message（发送）与 im:chat（群列表）权限。</summary>
internal sealed class FeishuSettingsPage : StackPanel
{
    private sealed class ChatOption
    {
        private readonly string _name;
        internal string ChatId{get;}
        internal ChatOption(string chatId,string name){ChatId=chatId;_name=name;}
        public override string ToString()=>$"{_name}（{ChatId}）";
    }

    private readonly CheckBox _enable=new();
    private readonly TextBox _appId=new(){Padding=new Thickness(8,5,8,5)};
    private readonly TextBox _targetId=new(){Padding=new Thickness(8,5,8,5)};
    private readonly ComboBox _targetType=new();
    private readonly ComboBox _chats=new(){Padding=new Thickness(8,5,8,5)};
    private readonly PasswordBox _secret=new(){Padding=new Thickness(8,5,8,5)};
    private readonly TextBlock _status=new();
    private readonly Button _saveSecret=new(),_test=new(),_clear=new();
    private readonly CancellationToken _token;
    private bool _statusLoaded;
    internal bool Enabled=>_enable.IsChecked==true;
    internal string AppId=>_appId.Text.Trim();
    internal string TargetId=>_targetId.Text.Trim();
    internal string TargetType=>(_targetType.SelectedItem as ComboBoxItem)?.Tag?.ToString()??"chat_id";

    internal FeishuSettingsPage(AppSettings settings,CancellationToken token)
    {
        _token=token;
        var form=new AiSettingsForm("飞书",T("通过飞书自建应用分享截图：圈选区域后点击工具栏“飞书”按钮，图片会发送到指定群或联系人。需要在飞书开放平台（open.feishu.cn）创建企业自建应用并开通 im:message、im:chat 权限；Secret 只保存在本机。点击“测试连接”可拉取群列表供选择。","Share screenshots via a Feishu self-built app: after selecting a region, the Feishu toolbar button sends the image to a chosen chat or contact. Create an enterprise app on open.feishu.cn with the im:message and im:chat scopes; the secret stays on this machine. “Test connection” also fetches your chat list."),_status);
        Children.Add(form);
        form.AddAction(_saveSecret,T("保存 Secret","Save secret"));
        form.AddAction(_test,T("测试连接","Test connection"));
        form.AddAction(_clear,T("清除 Secret","Clear secret"));
        _appId.Text=settings.FeishuAppId;
        _targetId.Text=settings.FeishuTargetId;
        _targetType.Items.Add(new ComboBoxItem{Content=T("群聊 chat_id","Chat chat_id"),Tag="chat_id"});
        _targetType.Items.Add(new ComboBoxItem{Content=T("联系人 open_id","Contact open_id"),Tag="open_id"});
        _targetType.SelectedIndex=string.Equals(settings.FeishuTargetType,"open_id",StringComparison.OrdinalIgnoreCase)?1:0;
        _enable.Content=T("启用飞书分享（工具栏出现“飞书”按钮）","Enable Feishu sharing (adds the Feishu toolbar button)");
        _enable.IsChecked=settings.FeishuEnabled;
        _secret.ToolTip=T("点击“保存 Secret”后才会写入本机（DPAPI 加密）。","Only written locally (DPAPI) after clicking “Save secret”.");
        _chats.ToolTip=T("点击“测试连接”后可从群列表中选择目标。","Populated by “Test connection”; pick a target chat here.");
        _chats.SelectionChanged+=(_,_)=>{if(_chats.SelectedItem is ChatOption option)_targetId.Text=option.ChatId;};
        form.Fields.Children.Add(AiSettingsForm.Field(T("App ID","App ID"),_appId));
        form.Fields.Children.Add(AiSettingsForm.Field(T("接收人类型","Receiver type"),_targetType));
        form.Fields.Children.Add(AiSettingsForm.Field(T("接收人 ID（chat_id 或 open_id）","Receiver ID (chat_id or open_id)"),_targetId));
        form.Fields.Children.Add(AiSettingsForm.Field(T("选择群聊（测试连接后可选）","Pick a chat (after test connection)"),_chats));
        form.Fields.Children.Add(AiSettingsForm.Field(T("App Secret","App Secret"),_secret));
        form.Fields.Children.Add(_enable);
        _status.Text=T("正在读取本机授权状态…","Reading local authorization status…");
        _saveSecret.Click+=(_,_)=>SaveSecret();
        _test.Click+=async(_,_)=>await TestAsync();
        _clear.Click+=(_,_)=>ClearSecret();
        Loaded+=async(_,_)=>
        {
            if(_statusLoaded)return;_statusLoaded=true;
            try
            {
                var configured=await Task.Run(()=>FeishuService.IsConfigured(settings),_token);
                if(!_token.IsCancellationRequested)_status.Text=configured?T("已配置（Secret 已保存）。","Configured (secret saved)."):T("尚未配置完整。","Not fully configured yet.");
            }
            catch(OperationCanceledException)when(_token.IsCancellationRequested){}
            catch(Exception ex){new PrivacyLogger().Info("FeishuStatus",ex.GetType().Name);}
        };
    }

    private void SaveSecret()
    {
        if(_token.IsCancellationRequested)return;
        try
        {
            FeishuService.SaveSecret(_secret.Password);
            _secret.Clear();
            _status.Foreground=Brushes.SeaGreen;
            _status.Text=T("App Secret 已保存（本机加密）。","App Secret saved (encrypted on this machine).");
        }
        catch(Exception ex){ShowError(ex.Message);}
    }

    private void ClearSecret()
    {
        if(_token.IsCancellationRequested)return;
        FeishuService.ClearSecret();
        _status.Foreground=Brushes.SlateGray;
        _status.Text=T("App Secret 已清除。","App Secret cleared.");
    }

    private async Task TestAsync()
    {
        if(!_test.IsEnabled||_token.IsCancellationRequested)return;
        _test.IsEnabled=false;_saveSecret.IsEnabled=false;_clear.IsEnabled=false;
        _status.Foreground=Brushes.SlateGray;
        _status.Text=T("正在连接飞书开放平台并拉取群列表…","Connecting to Feishu and fetching chats…");
        try
        {
            var probe=new AppSettings{FeishuEnabled=true,FeishuAppId=AppId};
            var chats=await FeishuService.ListChatsAsync(probe,_token);
            _chats.Items.Clear();
            foreach(var (chatId,name) in chats.Take(50))_chats.Items.Add(new ChatOption(chatId,name));
            _status.Text=T($"凭据有效，发现 {chats.Count} 个可见群（可在“选择群聊”中挑选）。",$"Credentials valid; {chats.Count} visible chat(s) found (pick one below).");
            _status.Foreground=Brushes.SeaGreen;
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex){ShowError(ex.Message);}
        finally{_test.IsEnabled=true;_saveSecret.IsEnabled=true;_clear.IsEnabled=true;}
    }

    private void ShowError(string message)
    {
        _status.Text=message;
        _status.Foreground=Brushes.Firebrick;
    }

    private static string T(string zh,string en)=>LocalizationService.T(zh,en);
}
