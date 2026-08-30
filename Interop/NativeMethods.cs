using System.Runtime.InteropServices;
namespace mewu_ai_Assistant.Interop;
internal static class NativeMethods
{
    internal const int WmHotkey=0x0312; internal const uint WdaExcludeFromCapture=0x11;
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool RegisterHotKey(IntPtr hWnd,int id,uint modifiers,uint virtualKey);
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool UnregisterHotKey(IntPtr hWnd,int id);
    [DllImport("user32.dll",SetLastError=true)] private static extern bool SetWindowDisplayAffinity(IntPtr hWnd,uint affinity);
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool SetWindowPos(IntPtr hWnd,IntPtr insertAfter,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll")] internal static extern uint GetDpiForWindow(IntPtr hWnd);

    internal static bool ExcludeFromCapture(IntPtr windowHandle)
    {
#if DEBUG
        // Visual QA needs to observe the real overlay hierarchy. This escape hatch is
        // compiled out of Release builds, so production privacy cannot be disabled.
        if (VisualQaCaptureEnabled)
            return true;
#endif
        return SetWindowDisplayAffinity(windowHandle, WdaExcludeFromCapture);
    }

    internal static bool VisualQaCaptureEnabled
    {
        get
        {
#if DEBUG
            return string.Equals(Environment.GetEnvironmentVariable("MEWU_QA_CAPTURE_WINDOWS"), "1", StringComparison.Ordinal);
#else
            return false;
#endif
        }
    }
}
