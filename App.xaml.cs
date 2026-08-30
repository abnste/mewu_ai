using System.Windows;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant;

public partial class App : System.Windows.Application
{
    private AppHost? _host;private readonly PrivacyLogger _logger=new();
    protected override async void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        if(e.Args.Contains("--import-env-providers",StringComparer.OrdinalIgnoreCase))
        {
            var service=new SettingsService();var settings=service.Load();
            if(new EnvironmentProviderBootstrap().Import(settings))service.Save(settings);
            if(e.Args.Contains("--verify",StringComparer.OrdinalIgnoreCase))await new ProviderVerificationService().VerifyAsync(settings,CancellationToken.None);
            Shutdown();return;
        }
        DispatcherUnhandledException+=(_,args)=>{_logger.Error("UI",args.Exception);args.Handled=true;System.Windows.MessageBox.Show("喵呜AI 遇到错误并已安全记录。基础功能仍可继续使用。","喵呜AI");};TaskScheduler.UnobservedTaskException+=(_,args)=>{_logger.Error("Task",args.Exception);args.SetObserved();};_host = new AppHost(this);
        if (!_host.Start()) Shutdown();
    }
    protected override void OnExit(ExitEventArgs e)
    {
        _host?.Dispose();
        base.OnExit(e);
    }
}
