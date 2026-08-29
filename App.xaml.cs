using System.Windows;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant;

public partial class App : System.Windows.Application
{
    private AppHost? _host;
    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        _host = new AppHost(this);
        if (!_host.Start()) Shutdown();
    }
    protected override void OnExit(ExitEventArgs e)
    {
        _host?.Dispose();
        base.OnExit(e);
    }
}
