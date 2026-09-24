// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;

/// <summary>钉钉设置页：企业内部应用（AppKey/AppSecret/AgentId）。
/// 圈选截图后工具栏出现“钉钉”按钮，把图片经工作通知发给指定 userid 联系人。</summary>
internal sealed class DingTalkSettingsPage : StackPanel
{
    private readonly CheckBox _enable=new();
    private readonly TextBox _appKey=new(){Padding=new Thickness(8,5,8,5)};
    private readonly TextBox _agentId=new(){Padding=new Thickness(8,5,8,5)};
    private readonly TextBox _users=new(){Padding=new Thickness(8,5,8,5)};
    private readonly PasswordBox _secret=new(){Padding=new Thickness(8,5,8,5)};
    private readonly TextBlock _status=new();
    private readonly Button _saveSecret=new(),_test=new(),_clear=new();
    private readonly CancellationToken _token;
    private bool _statusLoaded;
    internal bool Enabled=>_enable.IsChecked==true;
    internal string AppKey=>_appKey.Text.Trim();
    internal string AgentId=>_agentId.Text.Trim();
    internal string TargetUsers=>_users.Text.Trim();

    internal DingTalkSettingsPage(AppSettings settings,CancellationToken token)
    {
        _token=token;
        var form=new AiSettingsForm("钉钉",T("通过钉钉开放平台的企业内部应用分享截图：圈选区域后点击工具栏“钉钉”按钮，图片会以工作通知发给指定联系人。需要在钉钉开放平台（open-dev.dingtalk.com）创建应用并获取 AppKey/AppSecret 与 AgentId；Secret 只保存在本机。","Share screenshots via a DingTalk enterprise app from the open platform: after selecting a region, the DingTalk toolbar button sends the image to chosen contacts as a work notification. Create an app on open-dev.dingtalk.com to get the AppKey/AppSecret and AgentId; the secret stays on this machine."),_status);
        Children.Add(form);
        form.AddAction(_saveSecret,T("保存 Secret","Save secret"));
        form.AddAction(_test,T("测试连接","Test connection"));
        form.AddAction(_clear,T("清除 Secret","Clear secret"));
        _appKey.Text=settings.DingTalkAppKey;
        _agentId.Text=settings.DingTalkAgentId;
        _users.Text=settings.DingTalkTargetUsers;
        _enable.Content=T("启用钉钉分享（工具栏出现“钉钉”按钮）","Enable DingTalk sharing (adds the DingTalk toolbar button)");
        _enable.IsChecked=settings.DingTalkEnabled;
        _secret.ToolTip=T("点击“保存 Secret”后才会写入本机（DPAPI 加密）。","Only written locally (DPAPI) after clicking “Save secret”.");
        form.Fields.Children.Add(AiSettingsForm.Field(T("AppKey","AppKey"),_appKey));
        form.Fields.Children.Add(AiSettingsForm.Field(T("AgentId（工作通知所需）","AgentId (for work notifications)"),_agentId));
        form.Fields.Children.Add(AiSettingsForm.Field(T("接收人 userid（多个用逗号分隔）","Recipient userids (comma separated)"),_users));
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
                var configured=await Task.Run(()=>DingTalkService.IsConfigured(settings),_token);
                if(!_token.IsCancellationRequested)_status.Text=configured?T("已配置（Secret 已保存）。","Configured (secret saved)."):T("尚未配置完整。","Not fully configured yet.");
            }
            catch(OperationCanceledException)when(_token.IsCancellationRequested){}
            catch(Exception ex){new PrivacyLogger().Info("DingTalkStatus",ex.GetType().Name);}
        };
    }

    private void SaveSecret()
    {
        if(_token.IsCancellationRequested)return;
        try
        {
            DingTalkService.SaveSecret(_secret.Password);
            _secret.Clear();
            _status.Foreground=Brushes.SeaGreen;
            _status.Text=T("App Secret 已保存（本机加密）。","App Secret saved (encrypted on this machine).");
        }
        catch(Exception ex){ShowError(ex.Message);}
    }

    private void ClearSecret()
    {
        if(_token.IsCancellationRequested)return;
        DingTalkService.ClearSecret();
        _status.Foreground=Brushes.SlateGray;
        _status.Text=T("App Secret 已清除。","App Secret cleared.");
    }

    private async Task TestAsync()
    {
        if(!_test.IsEnabled||_token.IsCancellationRequested)return;
        _test.IsEnabled=false;_saveSecret.IsEnabled=false;_clear.IsEnabled=false;
        _status.Foreground=Brushes.SlateGray;
        _status.Text=T("正在连接钉钉开放平台…","Connecting to the DingTalk open platform…");
        try
        {
            var probe=new AppSettings{DingTalkEnabled=true,DingTalkAppKey=AppKey};
            await DingTalkService.TestConnectionAsync(probe,_token);
            _status.Text=T("access_token 获取成功，凭据有效。","access_token acquired; credentials valid.");
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
