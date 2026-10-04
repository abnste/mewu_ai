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
    private readonly ComboBox _model=new(),_effort=new();
    private readonly TextBlock _status=new(){TextWrapping=TextWrapping.Wrap,FontSize=12,Margin=new Thickness(0,12,0,12)};
    private readonly Button _detect=new();
    private readonly TextBox _path=new();
    internal string Path=>_path.Text.Trim();
    private readonly CancellationToken _token;
    private readonly AppSettings _settings;
    private bool _loaded;
    internal CodexModelOption? SelectedModel=>_model.SelectedItem as CodexModelOption;
    internal string SelectedEffort=>(_effort.SelectedItem as EffortChoice)?.Value??_settings.CodexReasoningEffort;
    internal bool ConnectionVerified {get;private set;}

    internal CodexSettingsPage(AppSettings settings,CancellationToken token)
    {
        _settings=settings;_token=token;
        var form=new AiSettingsForm("Codex",T("沿用本机 ChatGPT 登录，使用 Work / Codex 额度，消耗以账号套餐为准。","Uses your local ChatGPT sign-in and Work / Codex allowance. Usage depends on your plan."),_status);
        Children.Add(form);
        _status.Text=T("打开此页后自动检测登录与模型。","Sign-in and models are checked when this page opens.");
        form.Fields.Children.Add(AiSettingsForm.Field(T("模型","Model"),_model));
        _path.Text=settings.CodexExecutablePath;_path.IsReadOnly=true;
        var browse=new Button{Content=T("选择 codex.exe","Choose codex.exe"),Margin=new Thickness(8,0,0,0)};
        browse.Click+=(_,_)=>{var d=new Microsoft.Win32.OpenFileDialog{Title=T("选择 codex.exe","Choose codex.exe"),Filter=T("Codex 可执行文件|codex.exe","Codex executable|codex.exe")};if(d.ShowDialog()==true)_path.Text=d.FileName;};
        var row=new Grid();row.ColumnDefinitions.Add(new ColumnDefinition());row.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});row.Children.Add(_path);Grid.SetColumn(browse,1);row.Children.Add(browse);
        form.Fields.Children.Add(AiSettingsForm.Field(T("Codex 程序路径（可选）","Codex executable path (optional)"),row));
        form.Fields.Children.Add(AiSettingsForm.Field(T("思考程度","Reasoning effort"),_effort));
        _model.SelectionChanged+=(_,_)=>UpdateEfforts();
        if(!string.IsNullOrWhiteSpace(settings.CodexModel))
        {
            var saved=new CodexModelOption(settings.CodexModel,settings.CodexModel,false,settings.CodexSupportsImage,settings.CodexReasoningEffort,[settings.CodexReasoningEffort]);
            _model.Items.Add(saved);_model.SelectedItem=saved;
        }
        form.AddAction(_detect,T("测试连接","Test connection"));
        _detect.ToolTip=T("检测登录与模型，不发送问题或附件。","Checks sign-in and models without sending prompts or attachments.");
        _detect.Click+=async(_,_)=>await DetectAsync();
        Loaded+=async(_,_)=>{if(_loaded)return;_loaded=true;await DetectAsync();};
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
            await using var server=await CodexAppServer.StartAsync(_token,false,Path);
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
                _=>FormatErrorMessage(ex.Message)
            };
            _status.Foreground=Brushes.Firebrick;
        }
        finally{_detect.IsEnabled=true;}
    }

    internal static string FormatErrorMessage(string message)
    {
        var translated=message switch
        {
            "未找到本机 Codex，请选择原生 codex.exe。"=>T(message,"Codex was not found. Select the native codex.exe executable."),
            "无法启动本机 Codex。"=>T(message,"Codex could not start on this device."),
            "Codex 模型目录过大。"=>T(message,"The Codex model catalog exceeds the size limit."),
            "Codex 模型目录分页未结束，请升级官方客户端后重试。"=>T(message,"The Codex model catalog did not finish loading. Update the official client and retry."),
            "Codex 尚未登录，请在官方 ChatGPT 桌面应用或 Codex CLI 中完成登录，再重新检测。"=>T(message,"Codex is not signed in. Sign in to the official ChatGPT desktop app or Codex CLI, then check again."),
            "本机 Codex 的 MCP 名称无法安全隔离，请使用仅含字母、数字、下划线或短横线的名称。"=>T(message,"A local Codex MCP name cannot be safely isolated. Use only letters, digits, underscores, or hyphens in MCP names."),
            "Codex 后台已退出，请重新检测连接。"=>T(message,"The Codex background process has exited. Check the connection again."),
            "Codex 本机接口超时或已断开，请重新检测连接。"=>T(message,"The local Codex connection timed out or disconnected. Check the connection again."),
            "Codex 请求超过 64 MiB 限制。"=>T(message,"The Codex request exceeds the 64 MiB limit."),
            "Codex 返回了不完整的接口响应。"=>T(message,"Codex returned an incomplete protocol response."),
            "Codex 后台连接中断，未完成的回答已取消。"=>T(message,"The Codex connection was interrupted. Pending responses were canceled."),
            "响应缺少必要字段。"=>T(message,"The response is missing required fields."),
            "Codex 单条响应超过安全限制。"=>T(message,"A Codex response exceeds the size limit."),
            "Codex 响应被截断。"=>T(message,"The Codex response was truncated."),
            _=>message
        };
        if(!string.Equals(translated,message,StringComparison.Ordinal)||!LocalizationService.IsEnglish)return translated;
        var rejected=System.Text.RegularExpressions.Regex.Match(message,"^Codex 拒绝了接口请求（代码 (-?[0-9]+)），请检查登录、模型与官方客户端版本。$",System.Text.RegularExpressions.RegexOptions.CultureInvariant);
        return rejected.Success?T(message,$"Codex rejected the request (code {rejected.Groups[1].Value}). Check sign-in, the model, and the official client version."):message;
    }

    private static string T(string zh,string en)=>LocalizationService.T(zh,en);
    private sealed record EffortChoice(string Value)
    {
        public override string ToString()=>LocalizationService.IsEnglish?Value:Value switch
        {"none"=>"关闭","minimal"=>"极少","low"=>"较低","medium"=>"中等","high"=>"较高","xhigh"=>"很高","max"=>"最大","ultra"=>"极致",_=>Value};
    }
}
