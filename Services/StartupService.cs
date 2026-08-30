using Microsoft.Win32;
namespace mewu_ai_Assistant.Services;
public static class StartupService
{
    private const string Name="MewuAI";
    public static void SetEnabled(bool enabled)
    {
        using var key=Registry.CurrentUser.CreateSubKey(@"Software\Microsoft\Windows\CurrentVersion\Run")??throw new InvalidOperationException("无法打开当前用户的开机启动配置");
        if(enabled)
        {
            var executable=Environment.ProcessPath;
            if(string.IsNullOrWhiteSpace(executable)||!File.Exists(executable))throw new InvalidOperationException("无法确定当前应用程序路径，未启用开机启动");
            key.SetValue(Name,BuildCommand(executable),RegistryValueKind.String);
        }
        else key.DeleteValue(Name,false);
    }

    internal static string BuildCommand(string executablePath)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(executablePath);
        var fullPath=Path.GetFullPath(executablePath);
        if(fullPath.Contains('"'))throw new ArgumentException("应用程序路径不能包含引号",nameof(executablePath));
        return $"\"{fullPath}\"";
    }
}
