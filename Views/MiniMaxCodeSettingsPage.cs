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
    internal MiniMaxCodeModel? SelectedModel=>_model.SelectedItem as MiniMaxCodeModel;

    internal MiniMaxCodeSettingsPage(AppSettings settings,CancellationToken token)
    {
        _settings=settings;_token=token;
        var form=new AiSettingsForm("MiniMax Code", "直接复用本机 MiniMax Code 桌面版的登录状态和额度，不需要安装 CLI。桌面版未登录时，可点击打开客户端完成登录。", _status);
        Children.Add(form);
        form.AddAction(_test,"测试连接");form.AddAction(_refresh,"刷新状态");form.AddAction(_open,"打开 MiniMax Code");
        _test.ToolTip="发送一条简短验证消息，会使用少量 MiniMax Code 额度。";
        form.Fields.Children.Add(AiSettingsForm.Field("模型",_model));
        foreach(var model in MiniMaxCodeRuntime.KnownModels)_model.Items.Add(model);
        _model.SelectedItem=MiniMaxCodeRuntime.KnownModels.FirstOrDefault(item=>item.Model.Equals(settings.MiniMaxCodeModel,StringComparison.OrdinalIgnoreCase))??MiniMaxCodeRuntime.KnownModels[0];
        _open.Click+=(_,_)=>OpenDesktop();_refresh.Click+=async(_,_)=>await RefreshAsync();_test.Click+=async(_,_)=>await TestAsync();
        Loaded+=async(_,_)=>{if(_loaded)return;_loaded=true;await RefreshAsync();};
    }

    private void OpenDesktop()
    {
        if(MiniMaxCodeRuntime.LaunchDesktop())SetStatus("已打开 MiniMax Code，请在官方客户端完成登录后点击“刷新状态”。",false);
        else SetStatus("未找到 MiniMax Code 桌面版，请先安装官方客户端。",true);
    }

    private async Task RefreshAsync()
    {
        if(!_refresh.IsEnabled||_token.IsCancellationRequested)return;
        _refresh.IsEnabled=false;_test.IsEnabled=false;SetStatus("正在检查 MiniMax Code 桌面登录状态…",false);
        try
        {
            await Task.Yield();_token.ThrowIfCancellationRequested();
            var session=MiniMaxCodeRuntime.TryGetDesktopSession();
            SetStatus(session is null?"未发现桌面登录会话，请点击“打开 MiniMax Code”登录。":"已发现桌面登录会话，可测试连接。",session is null);
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        finally{_refresh.IsEnabled=true;_test.IsEnabled=true;}
    }

    private async Task TestAsync()
    {
        if(SelectedModel is not { } model)return;
        _test.IsEnabled=false;_refresh.IsEnabled=false;_open.IsEnabled=false;_model.IsEnabled=false;SetStatus("正在验证 MiniMax Code 回复…",false);
        try
        {
            var ok=await new MiniMaxCodeAiProvider(model.Model).TestConnectionAsync(_token);_token.ThrowIfCancellationRequested();
            SetStatus(ok?"已连接 MiniMax Code":"未返回验证标记，请检查桌面登录状态和额度。",!ok,ok);
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex)when(ex is IOException or InvalidOperationException or TimeoutException or System.Net.Http.HttpRequestException or System.Text.Json.JsonException){SetStatus(ex.Message,true);}
        finally{_test.IsEnabled=true;_refresh.IsEnabled=true;_open.IsEnabled=true;_model.IsEnabled=true;}
    }

    private void SetStatus(string text,bool error,bool success=false){_status.Text=text;_status.Foreground=error?Brushes.Firebrick:success?Brushes.SeaGreen:Brushes.SlateGray;}
}
