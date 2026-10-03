// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows.Controls;
using System.Windows;
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
    private readonly TextBox _path=new();
    internal string Path=>_path.Text.Trim();
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
        _test.ToolTip=T("只检查 WorkBuddy 后台连接、会话、模型和思考选项，不发送对话。","Checks the WorkBuddy bridge, session, model and reasoning options without sending a turn.");
        form.Fields.Children.Add(AiSettingsForm.Field(T("模型","Model"),_model));
        _path.Text=settings.WorkBuddyExecutablePath;_path.IsReadOnly=true;
        var browse=new Button{Content=T("选择 WorkBuddy.exe","Choose WorkBuddy.exe"),Margin=new Thickness(8,0,0,0)};
        browse.Click+=(_,_)=>{var d=new Microsoft.Win32.OpenFileDialog{Title=T("选择 WorkBuddy.exe","Choose WorkBuddy.exe"),Filter=T("WorkBuddy 可执行文件|WorkBuddy.exe","WorkBuddy executable|WorkBuddy.exe")};if(d.ShowDialog()==true)_path.Text=d.FileName;};
        var row=new Grid();row.ColumnDefinitions.Add(new ColumnDefinition());row.ColumnDefinitions.Add(new ColumnDefinition{Width=GridLength.Auto});row.Children.Add(_path);Grid.SetColumn(browse,1);row.Children.Add(browse);
        form.Fields.Children.Add(AiSettingsForm.Field(T("WorkBuddy 程序路径（可选）","WorkBuddy executable path (optional)"),row));
        form.Fields.Children.Add(AiSettingsForm.Field(T("思考程度","Reasoning effort"),_effort));
        if(!string.IsNullOrWhiteSpace(settings.WorkBuddyModel))
        {
            var saved=new WorkBuddyModelOption(settings.WorkBuddyModel,settings.WorkBuddyModel,settings.WorkBuddySupportsImage);
            _model.Items.Add(saved);_model.SelectedItem=saved;
        }
        SetEfforts([settings.WorkBuddyReasoningEffort],settings.WorkBuddyReasoningEffort);
        _status.Text=T("打开此页后自动读取模型，测试连接只做后台协议检查。","Models load when this page opens. Test connection performs a bridge check without sending a turn.");
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
            await using var server=await WorkBuddyAcpServer.StartAsync(_token,false,Path);
            var catalog=await server.NewSessionAsync(_token);_token.ThrowIfCancellationRequested();
            var previousModel=SelectedModel?.Model??_settings.WorkBuddyModel;var previousEffort=SelectedEffort;
            _model.Items.Clear();foreach(var model in catalog.Models)_model.Items.Add(model);
            _model.SelectedItem=catalog.Models.FirstOrDefault(item=>item.Model==previousModel)??catalog.Models.FirstOrDefault(item=>item.Model==catalog.CurrentModel)??catalog.Models[0];
            SetEfforts(catalog.Efforts,catalog.Efforts.Contains(previousEffort)?previousEffort:catalog.CurrentEffort);
            // Persist the discovered executable so later launches skip the
            // filesystem scan and probe this install directly.
            if(!string.IsNullOrWhiteSpace(server.ExecutablePath)&&!string.Equals(server.ExecutablePath,Path,StringComparison.OrdinalIgnoreCase))_path.Text=server.ExecutablePath;
            _status.Text=T($"已读取 {catalog.Models.Count} 个模型 · 可测试连接",$"Loaded {catalog.Models.Count} models · ready to test");
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex){new PrivacyLogger().Error("WorkBuddySettingsDetect",ex);ShowError(ex);EnsureFallbackModel();}
        finally{_detect.IsEnabled=true;_test.IsEnabled=true;}
    }
    private async Task TestAsync()
    {
        if(SelectedModel is not { } model){await DetectAsync();return;}
        _test.IsEnabled=false;_detect.IsEnabled=false;_model.IsEnabled=false;_effort.IsEnabled=false;
        _status.Foreground=Brushes.SlateGray;_status.Text=T("正在验证 WorkBuddy 回复…","Verifying a WorkBuddy response…");
        try
        {
            if(!await new WorkBuddyAiProvider(model.Model,SelectedEffort,model.SupportsImage,Path).TestConnectionAsync(_token))throw new InvalidOperationException(T("WorkBuddy 未返回验证标记，请检查登录与额度。","WorkBuddy did not return the verification marker. Check sign-in and allowance."));
            _token.ThrowIfCancellationRequested();_status.Text=T("已连接 WorkBuddy","Connected to WorkBuddy");_status.Foreground=Brushes.SeaGreen;
        }
        catch(OperationCanceledException)when(_token.IsCancellationRequested){}
        catch(Exception ex){new PrivacyLogger().Error("WorkBuddySettingsTest",ex);ShowError(ex);}
        finally{_test.IsEnabled=true;_detect.IsEnabled=true;_model.IsEnabled=true;_effort.IsEnabled=true;}
    }
    private void ShowError(Exception error)
    {
        if(_token.IsCancellationRequested)return;
        _status.Text=error is System.ComponentModel.Win32Exception or UnauthorizedAccessException?T("无法启动 WorkBuddy，请打开官方客户端并重试。","Cannot start WorkBuddy. Open the official client and retry."):FormatErrorMessage(error.Message);
        _status.Foreground=Brushes.Firebrick;
    }

    // Detection fallback: when the bridge cannot be reached, still offer a
    // usable model choice (the previously saved one, or "auto") so the user
    // can save settings and let the send path retry the connection.
    private void EnsureFallbackModel()
    {
        if(_model.SelectedItem is not null)return;
        var model=string.IsNullOrWhiteSpace(_settings.WorkBuddyModel)?"auto":_settings.WorkBuddyModel;
        var option=new WorkBuddyModelOption(model,model=="auto"?T("自动（兜底）","Auto (fallback)"):model,_settings.WorkBuddySupportsImage);
        _model.Items.Add(option);_model.SelectedItem=option;
        _status.Text+=T("　已提供兜底模型，保存后发送时会重新连接验证。"," A fallback model is offered; sending will reconnect and verify.");
    }

    internal static string FormatErrorMessage(string message)
    {
        var translated=message switch
        {
            "未找到本机 WorkBuddy，请选择 WorkBuddy.exe。"=>T(message,"WorkBuddy was not found. Select WorkBuddy.exe."),
            "无法启动 WorkBuddy 本机接口。"=>T(message,"The local WorkBuddy connection could not start."),
            "WorkBuddy ACP 版本不兼容，请更新官方客户端。"=>T(message,"The WorkBuddy ACP version is incompatible. Update the official client."),
            "WorkBuddy 连接响应缺少 connectionId。"=>T(message,"The WorkBuddy connection response is missing connectionId."),
            "WorkBuddy 连接响应格式无效。"=>T(message,"The WorkBuddy connection response has an invalid format."),
            "WorkBuddy 未应用安全的默认权限模式，已停止。"=>T(message,"WorkBuddy did not apply the required default permissions and was stopped."),
            "WorkBuddy 未启用本机视频工具的隔离环境。"=>T(message,"WorkBuddy did not enable the isolated environment for local video tools."),
            "WorkBuddy 未返回有效会话。"=>T(message,"WorkBuddy did not return a valid session."),
            "WorkBuddy 模型目录格式无效。"=>T(message,"The WorkBuddy model catalog has an invalid format."),
            "WorkBuddy 没有返回可用模型，请检查官方客户端。"=>T(message,"WorkBuddy did not return any available models. Check the official client."),
            "WorkBuddy 思考选项不兼容。"=>T(message,"The WorkBuddy reasoning options are incompatible."),
            "WorkBuddy 模型或思考程度已不可用，请重新检测并选择。"=>T(message,"The WorkBuddy model or reasoning effort is no longer available. Refresh and select them again."),
            "WorkBuddy 未应用所选思考程度。"=>T(message,"WorkBuddy did not apply the selected reasoning effort."),
            "WorkBuddy 连接已关闭。"=>T(message,"The WorkBuddy connection has closed."),
            "WorkBuddy 本机接口超时或已断开，请重新检测连接。"=>T(message,"The local WorkBuddy connection timed out or disconnected. Check the connection again."),
            "WorkBuddy 本机接口在响应前关闭，请重新检测连接。"=>T(message,"The local WorkBuddy connection closed before responding. Check the connection again."),
            "WorkBuddy 返回了不完整的接口响应。"=>T(message,"WorkBuddy returned an incomplete protocol response."),
            "响应缺少必要字段。"=>T(message,"The response is missing required fields."),
            "WorkBuddy 单条响应超过安全限制。"=>T(message,"A WorkBuddy response exceeds the size limit."),
            "WorkBuddy 响应被截断。"=>T(message,"The WorkBuddy response was truncated."),
            "WorkBuddy 连接检查超过 30 秒，已停止。请确认官方客户端已登录后重试。"=>T(message,"The WorkBuddy connection check exceeded 30 seconds and was stopped. Sign in to the official client and retry."),
            "请在 WorkBuddy 页重新选择可用模型和思考程度。"=>T(message,"Select an available model and reasoning effort on the WorkBuddy page."),
            _=>message
        };
        if(!string.Equals(translated,message,StringComparison.Ordinal)||!LocalizationService.IsEnglish)return translated;
        var rejected=System.Text.RegularExpressions.Regex.Match(message,"^WorkBuddy 拒绝了接口请求（代码 (-?[0-9]+)），请在官方 WorkBuddy 中确认登录、额度和所选模型后重试。$",System.Text.RegularExpressions.RegexOptions.CultureInvariant);
        if(rejected.Success)return T(message,$"WorkBuddy rejected the request (code {rejected.Groups[1].Value}). Check sign-in, allowance, and the selected model in the official app, then retry.");
        var http=System.Text.RegularExpressions.Regex.Match(message,"^WorkBuddy HTTP 接口返回 ([0-9]+)。$",System.Text.RegularExpressions.RegexOptions.CultureInvariant);
        if(http.Success)return T(message,$"The WorkBuddy HTTP endpoint returned {http.Groups[1].Value}.");
        var ready=System.Text.RegularExpressions.Regex.Match(message,"^WorkBuddy 本机接口在 ([0-9]+) 秒内未就绪，请确认官方客户端已登录后重试。([\\s\\S]*)$",System.Text.RegularExpressions.RegexOptions.CultureInvariant);
        return ready.Success?T(message,$"The local WorkBuddy connection was not ready within {ready.Groups[1].Value} seconds. Sign in to the official client and retry."+(ready.Groups[2].Length>0?" "+ready.Groups[2].Value:string.Empty)):message;
    }

    private static string T(string zh,string en)=>LocalizationService.T(zh,en);
    private sealed record EffortChoice(string Value)
    {
        public override string ToString()=>LocalizationService.IsEnglish?Value:Value switch{"disabled"=>"关闭","enabled"=>"默认","minimal"=>"极少","low"=>"较低","medium"=>"中等","high"=>"较高","xhigh"=>"很高","max"=>"最大",_=>Value};
    }
}
