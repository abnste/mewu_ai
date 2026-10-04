// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

internal sealed class MiniMaxCodeSettingsPage : StackPanel
{
    private readonly ComboBox _model=new();
    private readonly TextBlock _status=new();
    private readonly Button _open=new(),_refresh=new(),_test=new();
    private readonly CancellationToken _token;
    private readonly AppSettings _settings;
    private bool _loaded;
    internal MiniMaxCodeModel? SelectedModel=>(_model.SelectedItem as ModelChoice)?.Value;

    internal MiniMaxCodeSettingsPage(AppSettings settings,CancellationToken token)
    {
        _settings=settings;_token=token;
        var form=new AiSettingsForm("MiniMax Code",T("直接复用本机 MiniMax Code 桌面版的登录状态和额度，不需要安装 CLI。桌面版未登录时，可点击打开客户端完成登录。","Uses your MiniMax Code desktop sign-in and allowance without installing a CLI. Open the official desktop app to sign in if needed."),_status);
        Children.Add(form);
        form.AddAction(_test,T("测试连接","Test connection"));form.AddAction(_refresh,T("刷新状态","Refresh status"));form.AddAction(_open,T("打开 MiniMax Code","Open MiniMax Code"));
        _test.ToolTip=T("发送一条简短验证消息，会使用少量 MiniMax Code 额度。","Sends a short verification message and uses a small amount of your MiniMax Code allowance.");
        form.Fields.Children.Add(AiSettingsForm.Field(T("模型","Model"),_model));
        foreach(var model in MiniMaxCodeRuntime.KnownModels)_model.Items.Add(new ModelChoice(model));
        _model.SelectedItem=_model.Items.Cast<ModelChoice>().FirstOrDefault(item=>item.Value.Model.Equals(settings.MiniMaxCodeModel,StringComparison.OrdinalIgnoreCase))??_model.Items[0];
        _open.Click+=(_,_)=>OpenDesktop();_refresh.Click+=async(_,_)=>await RefreshAsync();_test.Click+=async(_,_)=>await TestAsync();
        Loaded+=async(_,_)=>{if(_loaded)return;_loaded=true;await RefreshAsync();};
    }

    private void OpenDesktop()
    {
        if(MiniMaxCodeRuntime.LaunchDesktop())SetStatus(T("已打开 MiniMax Code，请在官方客户端完成登录后点击“刷新状态”。","MiniMax Code is open. Sign in to the official app, then select Refresh status."),false);
        else SetStatus(T("未找到 MiniMax Code 桌面版，请先安装官方客户端。","MiniMax Code desktop was not found. Install the official app first."),true);
    }

    private async Task RefreshAsync()
    {
        if(!_refresh.IsEnabled||_token.IsCancellationRequested)return;
        _refresh.IsEnabled=false;_test.IsEnabled=false;SetStatus(T("正在检查 MiniMax Code 桌面登录状态…","Checking MiniMax Code desktop sign-in…"),false);
        try
        {
            await Task.Yield();_token.ThrowIfCancellationRequested();
            var session=MiniMaxCodeRuntime.TryGetDesktopSession();
            SetStatus(session is null?T("未发现桌面登录会话，请点击“打开 MiniMax Code”登录。","No desktop sign-in was found. Select Open MiniMax Code to sign in."):T("已发现桌面登录会话，可测试连接。","Desktop sign-in found. You can now test the connection."),session is null);
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        finally{_refresh.IsEnabled=true;_test.IsEnabled=true;}
    }

    private async Task TestAsync()
    {
        if(SelectedModel is not { } model)return;
        _test.IsEnabled=false;_refresh.IsEnabled=false;_open.IsEnabled=false;_model.IsEnabled=false;SetStatus(T("正在验证 MiniMax Code 回复…","Verifying a MiniMax Code response…"),false);
        try
        {
            var ok=await new MiniMaxCodeAiProvider(model.Model).TestConnectionAsync(_token);_token.ThrowIfCancellationRequested();
            SetStatus(ok?T("已连接 MiniMax Code","Connected to MiniMax Code"):T("未返回验证标记，请检查桌面登录状态和额度。","The verification marker was not returned. Check desktop sign-in and your allowance."),!ok,ok);
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex)when(ex is IOException or InvalidOperationException or TimeoutException or System.Net.Http.HttpRequestException or System.Text.Json.JsonException){SetStatus(FormatErrorMessage(ex.Message),true);}
        finally{_test.IsEnabled=true;_refresh.IsEnabled=true;_open.IsEnabled=true;_model.IsEnabled=true;}
    }

    private void SetStatus(string text,bool error,bool success=false){_status.Text=text;_status.Foreground=error?Brushes.Firebrick:success?Brushes.SeaGreen:Brushes.SlateGray;}

    internal static string FormatModelName(MiniMaxCodeModel model)=>model.Model switch
    {
        "minimax/MiniMax-M3" when model.Name=="MiniMax-M3（支持图片和视频）"=>T(model.Name,"MiniMax-M3 (images and video)"),
        "minimax/MiniMax-M2.7" when model.Name=="MiniMax-M2.7（文字）"=>T(model.Name,"MiniMax-M2.7 (text)"),
        "minimax/MiniMax-M2.7-highspeed" when model.Name=="MiniMax-M2.7-highspeed（文字）"=>T(model.Name,"MiniMax-M2.7-highspeed (text)"),
        _=>model.Name
    };

    // Translate only messages authored by MewuAI; external error details are retained.
    internal static string FormatErrorMessage(string message)=>message switch
    {
        "请在 MiniMax Code 页选择可用模型。"=>T(message,"Select an available model on the MiniMax Code page."),
        "未发现 MiniMax Code 桌面版登录会话，请点击“打开 MiniMax Code”完成登录后刷新状态。"=>T(message,"No MiniMax Code desktop sign-in was found. Select Open MiniMax Code, sign in, then refresh the status."),
        "MiniMax Code 桌面登录已失效，请点击“打开 MiniMax Code”重新登录。"=>T(message,"Your MiniMax Code desktop sign-in has expired. Select Open MiniMax Code to sign in again."),
        "MiniMax Code 回复超过安全限制，已停止。"=>T(message,"The MiniMax Code response exceeded the size limit and was stopped."),
        "MiniMax Code 连接提前结束，未收到完整回答。"=>T(message,"The MiniMax Code connection ended before the full response was received."),
        "MiniMax Code 未返回有效正文。"=>T(message,"MiniMax Code did not return a valid response."),
        "MiniMax Code 请求超过 10 分钟，已停止。请重试或缩短附件。"=>T(message,"The MiniMax Code request exceeded 10 minutes and was stopped. Retry or use shorter attachments."),
        _=>message
    };

    private static string T(string zh,string en)=>LocalizationService.T(zh,en);
    private sealed record ModelChoice(MiniMaxCodeModel Value)
    {
        public override string ToString()=>FormatModelName(Value);
    }
}
