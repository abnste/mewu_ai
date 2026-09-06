// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

internal sealed class CodexSettingsPage : StackPanel
{
    internal CheckBox EnabledChoice {get;}=new();
    private readonly ComboBox _model=new(),_effort=new();
    private readonly TextBlock _status=new(){TextWrapping=TextWrapping.Wrap,FontSize=12,Margin=new Thickness(0,12,0,12)};
    private readonly Button _detect=new();
    private readonly CancellationToken _token;
    private readonly AppSettings _settings;
    private bool _loaded;
    internal CodexModelOption? SelectedModel=>_model.SelectedItem as CodexModelOption;
    internal string SelectedEffort=>(_effort.SelectedItem as EffortChoice)?.Value??_settings.CodexReasoningEffort;
    internal bool IsEnabledForConversation=>EnabledChoice.IsChecked==true;
    internal bool ConnectionVerified {get;private set;}

    internal CodexSettingsPage(AppSettings settings,CancellationToken token)
    {
        _settings=settings;_token=token;Margin=new Thickness(6,0,6,6);
        Children.Add(new TextBlock{Text="ChatGPT Work · Codex",FontSize=21,FontWeight=FontWeights.SemiBold,Margin=new Thickness(0,8,0,10)});
        Children.Add(new TextBlock{Text=T("沿用本机 ChatGPT 登录，无需 API Key。使用 Work / Codex 额度，具体积分消耗以账号套餐为准。","Uses your local ChatGPT sign-in without an API key. Uses Work / Codex allowance; credit charges depend on your plan."),TextWrapping=TextWrapping.Wrap,Foreground=Brushes.SlateGray,FontSize=12,Margin=new Thickness(0,0,0,18)});
        EnabledChoice.Content=T("用于屏幕 AI 对话","Use for screen AI conversations");EnabledChoice.IsChecked=settings.CodexEnabled;Children.Add(EnabledChoice);
        Children.Add(_status);
        AddField(T("模型","Model"),_model);AddField(T("思考程度","Reasoning effort"),_effort);
        _model.SelectionChanged+=(_,_)=>UpdateEfforts();
        if(!string.IsNullOrWhiteSpace(settings.CodexModel))
        {
            var saved=new CodexModelOption(settings.CodexModel,settings.CodexModel,false,settings.CodexSupportsImage,settings.CodexReasoningEffort,[settings.CodexReasoningEffort]);
            _model.Items.Add(saved);_model.SelectedItem=saved;
        }
        _detect.Content=T("检测连接","Check connection");_detect.HorizontalAlignment=HorizontalAlignment.Left;_detect.Margin=new Thickness(0,10,0,8);
        _detect.Click+=async(_,_)=>await DetectAsync();Children.Add(_detect);
        Children.Add(new TextBlock{Text=T("支持文字、截图和本机视频分析。视频由 Codex 调用本机工具处理；检测连接不会发送问题或附件。","Supports text, screenshots and local video analysis. Codex uses local tools for video. Connection checks do not send prompts or attachments."),TextWrapping=TextWrapping.Wrap,Foreground=Brushes.SlateGray,FontSize=12,Margin=new Thickness(0,8,0,0)});
        Loaded+=async(_,_)=>{if(_loaded)return;_loaded=true;await DetectAsync();};
    }

    private void AddField(string label,Control control)
    {
        Children.Add(new TextBlock{Text=label,Margin=new Thickness(0,12,0,6)});control.MinWidth=0;control.HorizontalAlignment=HorizontalAlignment.Stretch;Children.Add(control);
    }
    private void UpdateEfforts()
    {
        var previous=SelectedEffort;_effort.Items.Clear();
        if(SelectedModel is not { } model)return;
        foreach(var value in model.Efforts)_effort.Items.Add(new EffortChoice(value));
        var selected=model.Efforts.Contains(previous)?previous:model.DefaultEffort;
        _effort.SelectedItem=_effort.Items.Cast<EffortChoice>().FirstOrDefault(item=>item.Value==selected);
    }
    internal async Task DetectAsync()
    {
        if(!_detect.IsEnabled||_token.IsCancellationRequested)return;
        _detect.IsEnabled=false;ConnectionVerified=false;_status.Foreground=Brushes.SlateGray;_status.Text=T("正在检测本机登录和可用模型…","Checking local sign-in and available models…");
        try
        {
            await using var server=await CodexAppServer.StartAsync(_token);
            var models=await server.ReadModelsAsync(_token);_token.ThrowIfCancellationRequested();
            var selected=SelectedModel?.Model??_settings.CodexModel;
            _model.Items.Clear();foreach(var model in models)_model.Items.Add(model);
            _model.SelectedItem=models.FirstOrDefault(item=>item.Model==selected)??models.FirstOrDefault(item=>item.IsDefault)??models.FirstOrDefault();
            ConnectionVerified=SelectedModel is not null;
            _status.Text=ConnectionVerified?T($"已连接 ChatGPT · {models.Count} 个可用模型",$"Connected to ChatGPT · {models.Count} available models"):T("账号未返回可用模型。","No models are available for this account.");
            _status.Foreground=ConnectionVerified?Brushes.SeaGreen:Brushes.Firebrick;
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex)when(ex is IOException or InvalidOperationException or System.ComponentModel.Win32Exception or System.Text.Json.JsonException or KeyNotFoundException or UnauthorizedAccessException)
        {
            _status.Text=ex switch
            {
                System.ComponentModel.Win32Exception=>T("Codex 启动失败，请重新安装官方客户端后重试。","Codex could not start. Reinstall the official client and retry."),
                KeyNotFoundException or System.Text.Json.JsonException=>T("Codex 接口格式不兼容，请升级官方客户端后重试。","The Codex protocol is incompatible. Update the official client and retry."),
                UnauthorizedAccessException=>T("无法访问 Codex 工作目录，请检查文件夹权限。","Cannot access the Codex workspace. Check folder permissions."),
                _=>ex.Message
            };
            _status.Foreground=Brushes.Firebrick;
        }
        finally{_detect.IsEnabled=true;}
    }
    private static string T(string zh,string en)=>LocalizationService.T(zh,en);
    private sealed record EffortChoice(string Value)
    {
        public override string ToString()=>LocalizationService.IsEnglish?Value:Value switch
        {"none"=>"关闭","minimal"=>"极少","low"=>"较低","medium"=>"中等","high"=>"较高","xhigh"=>"很高","max"=>"最大","ultra"=>"极致",_=>Value};
    }
}
