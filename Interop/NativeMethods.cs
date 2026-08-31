using System.Runtime.InteropServices;
namespace mewu_ai_Assistant.Interop;
internal static class NativeMethods
{
    [StructLayout(LayoutKind.Sequential)]
    internal struct WindowRect
    {
        internal int Left;
        internal int Top;
        internal int Right;
        internal int Bottom;
    }

    internal const int WmHotkey=0x0312; internal const int WmNcHitTest=0x0084; internal const int HtTransparent=-1; internal const uint WdaExcludeFromCapture=0x11; internal const int RgnOr=2; internal const int RgnDiff=4;
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool RegisterHotKey(IntPtr hWnd,int id,uint modifiers,uint virtualKey);
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool UnregisterHotKey(IntPtr hWnd,int id);
    [DllImport("user32.dll",SetLastError=true)] private static extern bool SetWindowDisplayAffinity(IntPtr hWnd,uint affinity);
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool SetWindowPos(IntPtr hWnd,IntPtr insertAfter,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll")] internal static extern uint GetDpiForWindow(IntPtr hWnd);
    [DllImport("user32.dll",SetLastError=true)] internal static extern int SetWindowRgn(IntPtr hWnd,IntPtr hRgn,bool redraw);
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool GetWindowRect(IntPtr hWnd,out WindowRect rect);
    [DllImport("gdi32.dll",SetLastError=true)] internal static extern IntPtr CreateRectRgn(int left,int top,int right,int bottom);
    [DllImport("gdi32.dll",SetLastError=true)] internal static extern int CombineRgn(IntPtr destination,IntPtr source1,IntPtr source2,int mode);
    [DllImport("gdi32.dll",SetLastError=true)] internal static extern bool DeleteObject(IntPtr handle);

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
