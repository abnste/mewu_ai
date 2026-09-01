using System.Windows;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant;

public partial class App : System.Windows.Application
{
    private AppHost? _host;
    private readonly PrivacyLogger _logger=new();

    protected override async void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        DispatcherUnhandledException+=(_,args)=>
        {
            CrashDiagnosticsService.MarkOperation("UI 未处理异常");
            _logger.Error("UI",args.Exception);
            try{System.Windows.MessageBox.Show("喵呜AI 遇到无法安全恢复的错误，即将退出。错误信息已写入本地日志，请重新启动应用。","喵呜AI");}catch{}
            // Unknown UI-thread exceptions can leave capture, recording, or credential state
            // partially mutated. Let WPF terminate instead of pretending the process is safe.
            args.Handled=false;
        };
        TaskScheduler.UnobservedTaskException+=(_,args)=>{CrashDiagnosticsService.MarkOperation("后台任务未观察异常");_logger.Error("Task",args.Exception);args.SetObserved();};
        AppDomain.CurrentDomain.UnhandledException+=(_,args)=>{CrashDiagnosticsService.MarkOperation("进程未处理异常");if(args.ExceptionObject is Exception exception)_logger.Error("Process",exception);else _logger.Error("Process",new InvalidOperationException("进程发生非托管未处理错误"));};
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
                _logger.Error("ProviderBootstrap",ex);
                exitCode=1;
            }
            finally{Shutdown(exitCode);}
            return;
        }
        try
        {
            _host = new AppHost(this);
            if (!_host.Start()) Shutdown();
        }
        catch(Exception ex)
        {
            _logger.Error("Startup",ex);
            try{System.Windows.MessageBox.Show("喵呜AI 启动失败，错误已安全记录。请重启应用；若问题持续，请查看本地日志。","喵呜AI");}catch{}
            Shutdown(1);
        }
    }
    protected override void OnExit(ExitEventArgs e)
    {
        try{_host?.Dispose();}
        catch(Exception ex){_logger.Error("Shutdown",ex);}
        base.OnExit(e);
    }
}
