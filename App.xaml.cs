// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant;

public partial class App : System.Windows.Application
{
    private AppHost? _host;
    private readonly Lazy<PrivacyLogger> _logger=new(()=>new());

    protected override void OnSessionEnding(SessionEndingCancelEventArgs e)
    {
        base.OnSessionEnding(e);
        if(!e.Cancel)_host?.BeginShutdown();
    }

    protected override async void OnStartup(StartupEventArgs e)
    {
        // This isolated worker has no windows, configuration, credentials,
        // single-instance side effects, diagnostic marker or log rotation.
        if(e.Args.Length==1&&e.Args[0]==TeachingPdfProcess.Argument)
        {
            Shutdown(await TeachingPdfProcess.RunWorkerAsync());return;
        }
        System.Globalization.CultureInfo? qaUiCulture=null;
#if DEBUG
        var qaCultureName=Environment.GetEnvironmentVariable("MEWU_QA_UI_CULTURE");
        if(!string.IsNullOrWhiteSpace(qaCultureName))qaUiCulture=System.Globalization.CultureInfo.GetCultureInfo(qaCultureName);
#endif
        base.OnStartup(e);
        DispatcherUnhandledException+=(_,args)=>
        {
            CrashDiagnosticsService.MarkOperation("UI 未处理异常");
            _logger.Value.Error("UI",args.Exception);
            try{LocalizedMessageBox.Show(LocalizationService.T("喵呜AI 遇到无法安全恢复的错误，即将退出。错误信息已写入本地日志，请重新启动应用。","MewuAI encountered an error it cannot safely recover from and will close. Details were written to the local log. Please restart the app."),"MewuAI");}catch{}
            // Unknown UI-thread exceptions can leave capture, recording, or credential state
            // partially mutated. Let WPF terminate instead of pretending the process is safe.
            args.Handled=false;
        };
        TaskScheduler.UnobservedTaskException+=(_,args)=>{CrashDiagnosticsService.MarkOperation("后台任务未观察异常");_logger.Value.Error("Task",args.Exception);args.SetObserved();};
        AppDomain.CurrentDomain.UnhandledException+=(_,args)=>{CrashDiagnosticsService.MarkOperation("进程未处理异常");if(args.ExceptionObject is Exception exception)_logger.Value.Error("Process",exception);else _logger.Value.Error("Process",new InvalidOperationException("进程发生非托管未处理错误"));};
        if(e.Args.Contains("--import-env-providers",StringComparer.OrdinalIgnoreCase))
        {
            var exitCode=0;
            try
            {
                using var instance=new SingleInstanceService();
                if(!instance.IsPrimary)throw new InvalidOperationException("喵呜AI 正在运行，已拒绝并发导入 Provider；请退出主程序后重试");
                var service=new SettingsService();
                await new EnvironmentProviderBootstrap().ImportAndCommitAsync(
                    service,
                    e.Args.Contains("--verify",StringComparer.OrdinalIgnoreCase),
                    CancellationToken.None);
            }
            catch(Exception ex)
            {
                _logger.Value.Error("ProviderBootstrap",ex);
                exitCode=1;
            }
            finally{Shutdown(exitCode);}
            return;
        }
        try
        {
            _host = new AppHost(this,qaUiCulture);
            if (!_host.Start()) { Shutdown(); return; }
#if DEBUG
            if (e.Args.Contains("--settings", StringComparer.OrdinalIgnoreCase))
                _host.ShowSettings();
#endif
        }
        catch(Exception ex)
        {
            _logger.Value.Error("Startup",ex);
            try{LocalizedMessageBox.Show(LocalizationService.T("喵呜AI 启动失败，错误已安全记录。请重启应用；若问题持续，请查看本地日志。","MewuAI could not start. The error was recorded safely. Restart the app, and check the local log if the problem continues."),"MewuAI");}catch{}
            Shutdown(1);
        }
    }
    protected override void OnExit(ExitEventArgs e)
    {
        try{_host?.Dispose();}
        catch(Exception ex){_logger.Value.Error("Shutdown",ex);}
        base.OnExit(e);
    }
}
