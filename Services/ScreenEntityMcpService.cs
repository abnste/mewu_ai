// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Diagnostics;

namespace mewu_ai_Assistant.Services;

internal static class ScreenEntityMcpService
{
    internal static bool OpenUrl(string value)
        =>OpenUrl(value,Process.Start,LogFailure);

    internal static bool OpenUrl(string value,Func<ProcessStartInfo,Process?> start,Action<string>? logFailure=null)
    {
        if(!Uri.TryCreate(value,UriKind.Absolute,out var uri)||uri.Scheme is not ("http" or "https"))return false;
        return Start(uri.AbsoluteUri,start,logFailure);
    }
    internal static bool OpenMailProvider(ScreenEntity entity)
        =>OpenMailProvider(entity,out _);

    internal static bool OpenMailProvider(ScreenEntity entity,out bool addressCopied)
        =>OpenMailProvider(entity,out addressCopied,static value=>ClipboardService.TrySetText(value,out _),Process.Start,LogFailure);

    internal static bool OpenMailProvider(ScreenEntity entity,out bool addressCopied,Func<string,bool> copyAddress,
        Func<ProcessStartInfo,Process?> start,Action<string>? logFailure=null)
    {
        addressCopied=false;
        if(entity.Type!=ScreenEntityType.Email)return false;
        try{addressCopied=copyAddress(entity.Value);}catch{}
        var url=entity.MailProvider switch { "qq"=>"https://mail.qq.com/", "netease"=>"https://mail.163.com/", _=>"mailto:"+Uri.EscapeDataString(entity.Value) };
        return Start(url,start,logFailure);
    }
    private static bool Start(string target,Func<ProcessStartInfo,Process?> start,Action<string>? logFailure)
    {
        try
        {
            // Shell dispatch may reuse an existing browser or mail client and
            // succeed without returning a new Process object.
            using var process=start(new ProcessStartInfo(target){UseShellExecute=true});
            return true;
        }
        catch(Exception ex)
        {
            // Shell exceptions can include the complete screen-derived URL or
            // mail address. Keep only the exception type in diagnostics.
            try{logFailure?.Invoke(ex.GetType().Name);}catch{}
            return false;
        }
    }
    private static void LogFailure(string exceptionType)=>new PrivacyLogger().Info("ScreenEntityMcp",exceptionType);
}
