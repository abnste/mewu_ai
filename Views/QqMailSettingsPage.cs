// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Diagnostics;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Text.Json.Nodes;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;

/// <summary>QQ 邮箱 MCP 设置页：扫码授权登录（每个用户授权自己的账号）、连接测试、清除授权。
/// 授权走腾讯 OAuth 2.0 授权码 + PKCE：应用打开授权页（由腾讯渲染并展示二维码），
/// 用户用手机 QQ 邮箱 App 扫码确认，令牌仅保存在本机（CredentialService/DPAPI），
/// 不与任何第三方客户端（含 WorkBuddy）共享。</summary>
internal sealed class QqMailSettingsPage : StackPanel
{
    private readonly CheckBox _enable=new();
    private readonly TextBlock _status=new();
    private readonly Button _test=new(),_authorize=new(),_clear=new();
    private readonly CancellationToken _token;
    private bool _statusLoaded;
    internal bool Enabled=>_enable.IsChecked==true;

    internal QqMailSettingsPage(AppSettings settings,CancellationToken token)
    {
        _token=token;
        var form=new AiSettingsForm("QQ 邮箱",T("接入官方 QQ 邮箱 MCP（api.mail.qq.com）。每个用户用自己的账号扫码授权，令牌只保存在本机；开启后，屏幕助手中出现邮件相关提问时自动拉取收件箱实时上下文，识别到邮箱地址还可经确认后代发邮件。","Connects the official QQ Mail MCP (api.mail.qq.com). Each user authorizes with their own account by scanning a QR code; the token stays on this machine. When enabled, mail-related prompts in the screen assistant pull live inbox context, and recognized email addresses can receive drafted mail after explicit confirmation."),_status);
        Children.Add(form);
        form.AddAction(_test,T("测试连接","Test connection"));
        form.AddAction(_authorize,T("扫码授权","Scan QR code to authorize"));
        form.AddAction(_clear,T("清除授权","Clear authorization"));
        _test.ToolTip=T("解析令牌并调用 GetMe，显示账号与权限，不发送对话。","Resolves the token and calls GetMe to show the account and scopes without sending a turn.");
        _authorize.ToolTip=T("打开腾讯授权页并用手机 QQ 邮箱 App 扫码确认。授权页由腾讯渲染，令牌只保存在本机。","Opens the Tencent authorization page; confirm by scanning the QR code with the QQ Mail mobile app. The token stays on this machine.");
        _clear.ToolTip=T("删除本机保存的 QQ 邮箱令牌（不影响其他客户端）。","Deletes the QQ Mail token stored on this machine (other clients are unaffected).");
        _enable.Content=T("在屏幕助手对话中启用 QQ 邮箱实时上下文与代发邮件","Enable live QQ Mail context and confirmed mail sending in screen assistant conversations");
        _enable.IsChecked=settings.QqMailMcpEnabled;
        _enable.Margin=new Thickness(0,0,0,4);
        form.Fields.Children.Add(_enable);
        _status.Text=T("正在读取本机授权状态…","Reading local authorization status…");
        _test.Click+=async(_,_)=>await TestAsync();
        _authorize.Click+=async(_,_)=>await AuthorizeAsync();
        _clear.Click+=(_,_)=>ClearAuthorization();
        Loaded+=async(_,_)=>
        {
            if(_statusLoaded)return;_statusLoaded=true;
            try{var status=await Task.Run(StatusIdleText,_token);if(!_token.IsCancellationRequested)_status.Text=status;}
            catch(OperationCanceledException)when(_token.IsCancellationRequested){}
            catch(Exception ex){new PrivacyLogger().Info("QqMailStatus",ex.GetType().Name);}
        };
    }

    private static string StatusIdleText()
    {
        var cached=QqMailMcpService.ReadCachedToken();
        if(cached is not null)
        {
            var expiry=cached.IsUsable
                ?T($"已授权 · 令牌 {Math.Max(0,(int)cached.ExpiresAt.Subtract(DateTimeOffset.UtcNow).TotalMinutes)} 分钟后到期（到期自动刷新）",$"authorized · token expires in {Math.Max(0,(int)cached.ExpiresAt.Subtract(DateTimeOffset.UtcNow).TotalMinutes)} min (auto refreshes)")
                :T("令牌已过期，将尝试自动刷新","token expired; auto refresh will be attempted");
            return T($"已保存授权（{expiry}）。",$"Authorization stored ({expiry}).");
        }
        return T("尚未授权。点击“扫码授权”，在打开的页面中用手机 QQ 邮箱 App 扫码确认即可。","Not authorized yet. Click “Scan QR code to authorize” and confirm with the QQ Mail mobile app.");
    }

    private void ClearAuthorization()
    {
        if(_token.IsCancellationRequested)return;
        QqMailMcpService.ClearCachedToken();
        _status.Foreground=Brushes.SlateGray;
        _status.Text=StatusIdleText();
    }

    private async Task AuthorizeAsync()
    {
        if(!_authorize.IsEnabled||_token.IsCancellationRequested)return;
        _authorize.IsEnabled=false;_test.IsEnabled=false;_clear.IsEnabled=false;
        _status.Foreground=Brushes.SlateGray;
        _status.Text=T("正在打开腾讯授权页…请在浏览器中用手机 QQ 邮箱 App 扫码确认（5 分钟内有效）。","Opening the Tencent authorization page… scan the QR code with the QQ Mail mobile app (valid for 5 minutes).");
        try
        {
            var token=await QqMailMcpService.AuthorizeAsync(url=>
            {
                try{Process.Start(new ProcessStartInfo(url){UseShellExecute=true});}
                catch(Exception ex){new PrivacyLogger().Info("QqMailOpenBrowser",ex.GetType().Name);}
            },_token);
            var account=await TryGetAccountAsync(token,_token);
            var minutes=Math.Max(0,(int)token.ExpiresAt.Subtract(DateTimeOffset.UtcNow).TotalMinutes);
            _status.Text=T($"授权成功：{account}（令牌 {minutes} 分钟后到期，自动刷新）。",$"Authorized as {account} (token expires in {minutes} min, refreshes automatically).");
            _status.Foreground=Brushes.SeaGreen;
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex)
        {
            ShowError($"{ex.Message}{(LocalizationService.IsEnglish?" Still not authorized: click “Scan QR code to authorize” to retry.":"仍未授权：点击“扫码授权”重试。")}");
        }
        finally{_authorize.IsEnabled=true;_test.IsEnabled=true;_clear.IsEnabled=true;}
    }

    private async Task TestAsync()
    {
        if(!_test.IsEnabled||_token.IsCancellationRequested)return;
        _test.IsEnabled=false;_authorize.IsEnabled=false;_clear.IsEnabled=false;
        _status.Foreground=Brushes.SlateGray;
        _status.Text=T("正在连接 QQ 邮箱 MCP…","Connecting to QQ Mail MCP…");
        try
        {
            var token=await QqMailMcpService.ResolveTokenAsync(_token);
            if(token is null)throw new InvalidOperationException(T("没有可用令牌且自动刷新失败。请先扫码授权。","No usable token and auto refresh failed. Scan the QR code to authorize first."));
            var account=await TryGetAccountAsync(token,_token);
            var scopes=token.Scope;
            var minutes=Math.Max(0,(int)token.ExpiresAt.Subtract(DateTimeOffset.UtcNow).TotalMinutes);
            _status.Text=T($"已连接 {account} · 权限：{scopes} · 令牌 {minutes} 分钟后到期",$"Connected as {account} · scopes: {scopes} · token expires in {minutes} min");
            _status.Foreground=Brushes.SeaGreen;
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex){ShowError(ex.Message);}
        finally{_test.IsEnabled=true;_authorize.IsEnabled=true;_clear.IsEnabled=true;}
    }

    /// <summary>调用 GetMe 取主别名地址用于状态展示；失败时退回占位文案。</summary>
    private static async Task<string> TryGetAccountAsync(QqMailMcpToken token,CancellationToken cancellationToken)
    {
        try
        {
            var text=await QqMailMcpService.CallToolAsync(token,"GetMe",new JsonObject(),cancellationToken);
            var data=JsonNode.Parse(text)?["data"]?["data"]??JsonNode.Parse(text)?["data"];
            var alias=data?["aliases"]?.AsArray()?.FirstOrDefault(entry=>entry?["is_primary"]?.GetValue<bool>()==true)??data?["aliases"]?.AsArray()?.FirstOrDefault();
            var account=alias?["email"]?.GetValue<string>();
            return string.IsNullOrEmpty(account)?"—":account!;
        }
        catch
        {
            return LocalizationService.IsEnglish?"(account unknown)":"（账号未知）";
        }
    }

    private void ShowError(string message)
    {
        if(_token.IsCancellationRequested)return;
        _status.Text=message;
        _status.Foreground=Brushes.Firebrick;
    }

    private static string T(string zh,string en)=>LocalizationService.T(zh,en);
}
