using Microsoft.Win32;
namespace mewu_ai_Assistant.Services;
public static class StartupService
{
    private const string Name="MewuAI";
    public static void SetEnabled(bool enabled){using var key=Registry.CurrentUser.CreateSubKey(@"Software\Microsoft\Windows\CurrentVersion\Run");if(enabled)key.SetValue(Name,$"\"{Environment.ProcessPath}\"");else key.DeleteValue(Name,false);}
}
