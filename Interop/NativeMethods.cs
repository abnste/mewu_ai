using System.Runtime.InteropServices;
namespace mewu_ai_Assistant.Interop;
internal static class NativeMethods
{
    internal const int WmHotkey=0x0312; internal const uint WdaExcludeFromCapture=0x11;
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool RegisterHotKey(IntPtr hWnd,int id,uint modifiers,uint virtualKey);
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool UnregisterHotKey(IntPtr hWnd,int id);
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool SetWindowDisplayAffinity(IntPtr hWnd,uint affinity);
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool SetWindowPos(IntPtr hWnd,IntPtr insertAfter,int x,int y,int width,int height,uint flags);
}
