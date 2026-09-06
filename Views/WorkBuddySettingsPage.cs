// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;

internal sealed class WorkBuddySettingsPage : StackPanel
{
    private readonly ComboBox _model=new(),_effort=new();
    private readonly TextBlock _status=new();
    private readonly Button _detect=new(),_test=new();
    private readonly CancellationToken _token;
    private readonly AppSettings _settings;
    private bool _loaded;
    internal WorkBuddyModelOption? SelectedModel=>_model.SelectedItem as WorkBuddyModelOption;
    internal string SelectedEffort=>(_effort.SelectedItem as EffortChoice)?.Value??_settings.WorkBuddyReasoningEffort;

    internal WorkBuddySettingsPage(AppSettings settings,CancellationToken token)
    {
        _settings=settings;_token=token;
        var form=new AiSettingsForm("WorkBuddy",T("沿用本机 WorkBuddy 登录与额度，支持文字、截图及本机视频分析。","Uses your local WorkBuddy sign-in and allowance for text, screenshots and local video analysis."),_status);
        Children.Add(form);
        form.AddAction(_test,T("测试连接","Test connection"));
        form.AddAction(_detect,T("刷新模型","Refresh models"));
        _test.ToolTip=T("发送一条简短验证消息，会使用少量 WorkBuddy 额度。","Sends a short verification message using a small amount of WorkBuddy allowance.");
        form.Fields.Children.Add(AiSettingsForm.Field(T("模型","Model"),_model));
        form.Fields.Children.Add(AiSettingsForm.Field(T("思考程度","Reasoning effort"),_effort));
        if(!string.IsNullOrWhiteSpace(settings.WorkBuddyModel))
        {
            var saved=new WorkBuddyModelOption(settings.WorkBuddyModel,settings.WorkBuddyModel,settings.WorkBuddySupportsImage);
            _model.Items.Add(saved);_model.SelectedItem=saved;
        }
        SetEfforts([settings.WorkBuddyReasoningEffort],settings.WorkBuddyReasoningEffort);
        _status.Text=T("打开此页后自动读取模型，测试连接需手动点击。","Models load when this page opens. Click Test connection to verify a response.");
        _detect.Click+=async(_,_)=>await DetectAsync();
        _test.Click+=async(_,_)=>await TestAsync();
        Loaded+=async(_,_)=>{if(_loaded)return;_loaded=true;await DetectAsync();};
    }
    private void SetEfforts(IEnumerable<string> values,string selected)
    {
        _effort.Items.Clear();foreach(var value in values)_effort.Items.Add(new EffortChoice(value));
        _effort.SelectedItem=_effort.Items.Cast<EffortChoice>().FirstOrDefault(item=>item.Value==selected)??_effort.Items.Cast<EffortChoice>().FirstOrDefault();
    }
    internal async Task DetectAsync()
    {
        if(!_detect.IsEnabled||_token.IsCancellationRequested)return;
        _detect.IsEnabled=false;_test.IsEnabled=false;_status.Foreground=Brushes.SlateGray;
        _status.Text=T("正在读取 WorkBuddy 模型…","Reading WorkBuddy models…");
        try
        {
            await using var server=await WorkBuddyAcpServer.StartAsync(_token);
            var catalog=await server.NewSessionAsync(_token);_token.ThrowIfCancellationRequested();
            var previousModel=SelectedModel?.Model??_settings.WorkBuddyModel;var previousEffort=SelectedEffort;
            _model.Items.Clear();foreach(var model in catalog.Models)_model.Items.Add(model);
            _model.SelectedItem=catalog.Models.FirstOrDefault(item=>item.Model==previousModel)??catalog.Models.FirstOrDefault(item=>item.Model==catalog.CurrentModel)??catalog.Models[0];
            SetEfforts(catalog.Efforts,catalog.Efforts.Contains(previousEffort)?previousEffort:catalog.CurrentEffort);
            _status.Text=T($"已读取 {catalog.Models.Count} 个模型 · 可测试连接",$"Loaded {catalog.Models.Count} models · ready to test");
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex)when(ex is IOException or InvalidOperationException or System.ComponentModel.Win32Exception or System.Text.Json.JsonException or KeyNotFoundException or UnauthorizedAccessException){ShowError(ex);}
        finally{_detect.IsEnabled=true;_test.IsEnabled=true;}
    }
    private async Task TestAsync()
    {
        if(SelectedModel is not { } model){await DetectAsync();return;}
        _test.IsEnabled=false;_detect.IsEnabled=false;_model.IsEnabled=false;_effort.IsEnabled=false;
        _status.Foreground=Brushes.SlateGray;_status.Text=T("正在验证 WorkBuddy 回复…","Verifying a WorkBuddy response…");
        try
        {
            if(!await new WorkBuddyAiProvider(model.Model,SelectedEffort,model.SupportsImage).TestConnectionAsync(_token))throw new InvalidOperationException(T("WorkBuddy 未返回验证标记，请检查登录与额度。","WorkBuddy did not return the verification marker. Check sign-in and allowance."));
            _token.ThrowIfCancellationRequested();_status.Text=T("已连接 WorkBuddy","Connected to WorkBuddy");_status.Foreground=Brushes.SeaGreen;
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex)when(ex is IOException or InvalidOperationException or TimeoutException or System.ComponentModel.Win32Exception or System.Text.Json.JsonException or KeyNotFoundException or UnauthorizedAccessException){ShowError(ex);}
        finally{_test.IsEnabled=true;_detect.IsEnabled=true;_model.IsEnabled=true;_effort.IsEnabled=true;}
    }
    private void ShowError(Exception error)
    {
        if(_token.IsCancellationRequested)return;
        _status.Text=error is System.ComponentModel.Win32Exception or UnauthorizedAccessException?T("无法启动 WorkBuddy，请打开官方客户端并重试。","Cannot start WorkBuddy. Open the official client and retry."):error.Message;
        _status.Foreground=Brushes.Firebrick;
    }
    private static string T(string zh,string en)=>LocalizationService.T(zh,en);
    private sealed record EffortChoice(string Value)
    {
        public override string ToString()=>LocalizationService.IsEnglish?Value:Value switch{"disabled"=>"关闭","enabled"=>"默认","minimal"=>"极少","low"=>"较低","medium"=>"中等","high"=>"较高","xhigh"=>"很高","max"=>"最大",_=>Value};
    }
}
