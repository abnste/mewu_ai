// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;

/// <summary>网易邮箱设置页。上半部分为“网页版扫码授权”（与 QQ 邮箱体验对齐：
/// 用手机“网易邮箱大师”App 扫码确认，会话只保存在本机；授权后可实时读取
/// 收件箱上下文。发送邮件仍要求 SMTP 授权码。</summary>
internal sealed class NetEaseMailSettingsPage : StackPanel
{
    private readonly CheckBox _enable=new();
    private readonly TextBox _account=new(){Padding=new Thickness(8,5,8,5)};
    private readonly TextBox _fromName=new(){Padding=new Thickness(8,5,8,5)};
    private readonly PasswordBox _authCode=new(){Padding=new Thickness(8,5,8,5)};
    private readonly TextBlock _status=new();
    private readonly Button _authorize=new(),_test=new(),_clearCode=new(),_clearSession=new(),_saveCode=new();
    private readonly Image _qrImage=new(){Width=208,Height=208,Stretch=Stretch.Uniform,Visibility=Visibility.Collapsed,Margin=new Thickness(0,8,0,4)};
    private readonly CancellationToken _token;
    private bool _statusLoaded;
    internal bool Enabled=>_enable.IsChecked==true;
    internal string Account=>_account.Text.Trim();
    internal string FromName=>_fromName.Text.Trim();

    internal NetEaseMailSettingsPage(AppSettings settings,CancellationToken token)
    {
        _token=token;
        var form=new AiSettingsForm("网易邮箱",T(
            "扫码授权用于读取收件箱；发送邮件还需填写账号和 SMTP 授权码（在网易邮箱网页设置中生成）。会话和授权码仅在本机加密保存。",
            "QR authorization reads the inbox. Sending also requires your account and an SMTP authorization code from NetEase web settings. Sessions and codes are encrypted on this machine."),_status);
        Children.Add(form);
        form.AddAction(_authorize,T("扫码授权","Scan QR code to authorize"));
        form.AddAction(_test,T("测试连接","Test connection"));
        form.AddAction(_clearSession,T("清除扫码会话","Clear scan session"));
        form.AddAction(_saveCode,T("保存授权码","Save auth code"));
        form.AddAction(_clearCode,T("清除授权码","Clear auth code"));
        _authorize.ToolTip=T("打开本页内嵌二维码，用手机“网易邮箱大师”App 内的“扫一扫”扫码并在手机上确认登录（相机/微信扫码只会跳到下载页）。无需 IMAP/SMTP 授权码。","Shows an embedded QR code; scan it with the built-in scanner inside the NetEase Mailmaster mobile app and confirm on the phone (camera/WeChat scanners only open a download page). No IMAP/SMTP code needed.");
        _test.ToolTip=T("验证扫码会话是否仍有效（读取 1 封收件箱邮件，不发信、不改动已读状态）。","Checks whether the scan session is still valid (reads one inbox message; sends nothing, marks nothing read).");
        _clearSession.ToolTip=T("删除本机保存的网易邮箱网页会话（收件箱读取将不可用，SMTP 不受影响）。","Deletes the NetEase web session stored on this machine (inbox reading stops; SMTP is unaffected).");
        _account.Text=settings.NetEaseMailAccount;
        _fromName.Text=settings.NetEaseMailFromName;
        _enable.Content=T("启用网易邮箱（识别到 163/126 等后缀即可发件，授权后还可实时读取收件箱）","Enable NetEase Mail (recognized 163/126 addresses can be emailed; live inbox reading works after authorization)");
        _enable.IsChecked=settings.NetEaseMailEnabled;
        _authCode.ToolTip=T("点击“保存授权码”后才会写入本机（DPAPI 加密）。","Only written locally (DPAPI) after clicking “Save auth code”.");
        form.Fields.Children.Add(_qrImage);
        form.Fields.Children.Add(AiSettingsForm.Field(T("邮箱账号（如 someone@163.com）","Account (e.g. someone@163.com)"),_account));
        form.Fields.Children.Add(AiSettingsForm.Field(T("发件人名称（可选，仅 SMTP 通道使用）","Sender display name (optional, SMTP channel only)"),_fromName));
        form.Fields.Children.Add(AiSettingsForm.Field(T("SMTP 授权码（发件必需）","SMTP authorization code (required for sending)"),_authCode));
        form.Fields.Children.Add(_enable);
        _status.Text=T("正在读取本机授权状态…","Reading local authorization status…");
        _authorize.Click+=async(_,_)=>await AuthorizeAsync();
        _test.Click+=async(_,_)=>await TestAsync();
        _clearSession.Click+=(_,_)=>ClearSession();
        _saveCode.Click+=(_,_)=>SaveCode();
        _clearCode.Click+=(_,_)=>ClearCode();
        Loaded+=async(_,_)=>
        {
            if(_statusLoaded)return;_statusLoaded=true;
            try{var status=await Task.Run(IdleStatusText,_token);if(!_token.IsCancellationRequested)_status.Text=status;}
            catch(OperationCanceledException)when(_token.IsCancellationRequested){}
            catch(Exception ex){new PrivacyLogger().Info("NetEaseMailStatus",ex.GetType().Name);}
        };
    }

    private static string IdleStatusText()
    {
        var session=NetEaseMailWebMcpService.ReadSession();
        if(session is not null)
        {
            var age=Math.Max(0,(int)DateTimeOffset.UtcNow.Subtract(session.ObtainedAt).TotalHours);
            return T($"已扫码授权：{session.Account}（会话获取于 {age} 小时前，网页端会话过期后需重新扫码）。",$"Scan-authorized: {session.Account} (session obtained {age} h ago; rescan after it expires).");
        }
        if(NetEaseMailService.ReadAuthCode() is not null)
            return T("未扫码授权，但已保存 SMTP 授权码（仅可代发，不能读取收件箱）。","No scan authorization yet, but an SMTP code is saved (sending only; no inbox reading).");
        return T("尚未授权。推荐点击“扫码授权”（无需授权码）；或使用下方 SMTP 授权码。","Not authorized yet. Prefer “Scan QR code to authorize” (no code needed), or use the SMTP code below.");
    }

    private async Task AuthorizeAsync()
    {
        if(!_authorize.IsEnabled||_token.IsCancellationRequested)return;
        _authorize.IsEnabled=false;_test.IsEnabled=false;_clearSession.IsEnabled=false;
        _status.Foreground=Brushes.SlateGray;
        SetStatus(T("正在获取二维码…","Fetching the QR code…"));
        try
        {
            var session=await NetEaseMailWebMcpService.AuthorizeAsync(
                async png=>
                {
                    // 二维码在 UI 线程渲染显示（PNG 由 QRCoder 在本地生成）。
                    await Dispatcher.InvokeAsync(()=>
                    {
                        var image=new BitmapImage();
                        using(var stream=new MemoryStream(png))
                        {
                            image.BeginInit();
                            image.CacheOption=BitmapCacheOption.OnLoad;
                            image.StreamSource=stream;
                            image.EndInit();
                        }
                        image.Freeze();
                        _qrImage.Source=image;
                        _qrImage.Visibility=Visibility.Visible;
                    });
                },
                text=>Dispatcher.Invoke(()=>SetStatus(text)),
                _token).ConfigureAwait(true);
            if(session.Account.Contains('@')&&string.IsNullOrWhiteSpace(_account.Text))
                _account.Text=session.Account;
            SetStatus(T($"扫码授权成功：{session.Account}。可读取收件箱；发件需另行配置 SMTP 授权码。",$"Scan authorization succeeded: {session.Account}. Inbox reading is now available; sending requires an SMTP authorization code."));
            _status.Foreground=Brushes.SeaGreen;
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex)
        {
            ShowError($"{ex.Message}{(LocalizationService.IsEnglish?" Retry by clicking “Scan QR code to authorize”.":" 点击“扫码授权”重试。")}");
        }
        finally{_authorize.IsEnabled=true;_test.IsEnabled=true;_clearSession.IsEnabled=true;}
    }

    private async Task TestAsync()
    {
        if(!_test.IsEnabled||_token.IsCancellationRequested)return;
        _test.IsEnabled=false;_authorize.IsEnabled=false;_clearSession.IsEnabled=false;
        _status.Foreground=Brushes.SlateGray;
        var session=NetEaseMailWebMcpService.ReadSession();
        if(session is not null)
        {
            SetStatus(T("正在验证网易邮箱网页会话…","Validating the NetEase web session…"));
            try
            {
                await NetEaseMailWebMcpService.ValidateSessionAsync(session,_token);
                SetStatus(T($"网页会话有效：{session.Account}。",$"Web session valid: {session.Account}."));
                _status.Foreground=Brushes.SeaGreen;
                return;
            }
            catch(OperationCanceledException)when(_token.IsCancellationRequested){return;}
            catch(Exception ex){ShowError(ex.Message);return;}
            finally{_test.IsEnabled=true;_authorize.IsEnabled=true;_clearSession.IsEnabled=true;}
        }
        // 无网页会话：回退到 SMTP 通道测试。
        SetStatus(T("未找到扫码会话，正在测试 SMTP 连接…","No scan session found; testing SMTP…"));
        try
        {
            var probe=new AppSettings{NetEaseMailEnabled=true,NetEaseMailAccount=Account};
            await NetEaseMailService.TestConnectionAsync(probe,_token);
            SetStatus(T($"SMTP 授权码有效（{Account}）。",$"SMTP authorization code valid ({Account})."));
            _status.Foreground=Brushes.SeaGreen;
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex){ShowError(ex.Message);}
        finally{_test.IsEnabled=true;_authorize.IsEnabled=true;_clearSession.IsEnabled=true;}
    }

    private void ClearSession()
    {
        if(_token.IsCancellationRequested)return;
        NetEaseMailWebMcpService.ClearSession();
        _qrImage.Visibility=Visibility.Collapsed;
        _qrImage.Source=null;
        _status.Foreground=Brushes.SlateGray;
        SetStatus(IdleStatusText());
    }

    private void SaveCode()
    {
        if(_token.IsCancellationRequested)return;
        try
        {
            NetEaseMailService.SaveAuthCode(_authCode.Password);
            _authCode.Clear();
            _status.Foreground=Brushes.SeaGreen;
            SetStatus(T("授权码已保存（本机加密）。","Authorization code saved (encrypted on this machine)."));
        }
        catch(Exception ex){ShowError(ex.Message);}
    }

    private void ClearCode()
    {
        if(_token.IsCancellationRequested)return;
        NetEaseMailService.ClearAuthCode();
        _status.Foreground=Brushes.SlateGray;
        SetStatus(IdleStatusText());
    }

    private void SetStatus(string text)=>_status.Text=text;

    private void ShowError(string message)
    {
        _status.Text=message;
        _status.Foreground=Brushes.Firebrick;
    }

    private static string T(string zh,string en)=>LocalizationService.T(zh,en);
}
