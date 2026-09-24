using System.Diagnostics;

namespace mewu_ai_Assistant.Services;

internal static class ScreenEntityMcpService
{
    internal static bool OpenUrl(string value)
    {
        if(!Uri.TryCreate(value,UriKind.Absolute,out var uri)||uri.Scheme is not ("http" or "https"))return false;
        return Start(uri.AbsoluteUri);
    }
    internal static bool OpenMailProvider(ScreenEntity entity)
    {
        if(entity.Type!=ScreenEntityType.Email)return false;
        try{ClipboardService.TrySetText(entity.Value,out _);}catch{}
        var url=entity.MailProvider switch { "qq"=>"https://mail.qq.com/", "netease"=>"https://mail.163.com/", _=>"mailto:"+Uri.EscapeDataString(entity.Value) };
        return Start(url);
    }
    private static bool Start(string target)
    {
        try{return Process.Start(new ProcessStartInfo(target){UseShellExecute=true}) is not null;}
        catch(Exception ex){try{new PrivacyLogger().Error("ScreenEntityMcp",ex);}catch{}return false;}
    }
}
