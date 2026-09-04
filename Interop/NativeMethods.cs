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
    private const int GwlExStyle=-20;
    private const long WsExTransparent=0x00000020L;
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool RegisterHotKey(IntPtr hWnd,int id,uint modifiers,uint virtualKey);
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool UnregisterHotKey(IntPtr hWnd,int id);
    [DllImport("user32.dll",SetLastError=true)] private static extern bool SetWindowDisplayAffinity(IntPtr hWnd,uint affinity);
    [DllImport("user32.dll",SetLastError=true)] private static extern bool GetWindowDisplayAffinity(IntPtr hWnd,out uint affinity);
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool SetWindowPos(IntPtr hWnd,IntPtr insertAfter,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll")] internal static extern uint GetDpiForWindow(IntPtr hWnd);
    [DllImport("user32.dll",SetLastError=true)] internal static extern int SetWindowRgn(IntPtr hWnd,IntPtr hRgn,bool redraw);
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool GetWindowRect(IntPtr hWnd,out WindowRect rect);
    [DllImport("gdi32.dll",SetLastError=true)] internal static extern IntPtr CreateRectRgn(int left,int top,int right,int bottom);
    [DllImport("gdi32.dll",SetLastError=true)] internal static extern int CombineRgn(IntPtr destination,IntPtr source1,IntPtr source2,int mode);
    [DllImport("gdi32.dll",SetLastError=true)] internal static extern bool DeleteObject(IntPtr handle);
    [DllImport("dwmapi.dll",PreserveSig=true)] private static extern int DwmSetWindowAttribute(IntPtr windowHandle,int attribute,ref int value,int valueSize);
    [DllImport("user32.dll",EntryPoint="GetWindowLongPtrW",SetLastError=true)] private static extern IntPtr GetWindowLongPtr(IntPtr windowHandle,int index);
    [DllImport("user32.dll",EntryPoint="SetWindowLongPtrW",SetLastError=true)] private static extern IntPtr SetWindowLongPtr(IntPtr windowHandle,int index,IntPtr value);

    private const int DwmWindowCornerPreference=33;
    private const int DwmCornerRound=2;

    internal static bool TryUseSystemRoundedCorners(IntPtr windowHandle)
    {
        if(windowHandle==IntPtr.Zero||!OperatingSystem.IsWindowsVersionAtLeast(10,0,22000))return false;
        var preference=DwmCornerRound;
        try{return DwmSetWindowAttribute(windowHandle,DwmWindowCornerPreference,ref preference,sizeof(int))>=0;}
        catch(DllNotFoundException){return false;}
        catch(EntryPointNotFoundException){return false;}
    }

    internal static bool TrySetWindowMouseTransparent(IntPtr windowHandle,bool transparent)
    {
        if(windowHandle==IntPtr.Zero)return false;
        Marshal.SetLastPInvokeError(0);
        var current=GetWindowLongPtr(windowHandle,GwlExStyle);
        if(current==IntPtr.Zero&&Marshal.GetLastPInvokeError()!=0)return false;
        var bits=current.ToInt64();
        var next=transparent?bits|WsExTransparent:bits&~WsExTransparent;
        if(next==bits)return true;
        Marshal.SetLastPInvokeError(0);
        var previous=SetWindowLongPtr(windowHandle,GwlExStyle,new IntPtr(next));
        return previous!=IntPtr.Zero||Marshal.GetLastPInvokeError()==0;
    }

    internal static bool ExcludeFromCapture(IntPtr windowHandle,bool requireProtection=false)
    {
#if DEBUG
        // Visual QA needs to observe the real overlay hierarchy. This escape hatch is
        // compiled out of Release builds, so production privacy cannot be disabled.
        if (VisualQaCaptureEnabled&&!requireProtection)
            return SetWindowDisplayAffinity(windowHandle,0);
#endif
        return SetWindowDisplayAffinity(windowHandle, WdaExcludeFromCapture)&&(!requireProtection||IsExcludedFromCapture(windowHandle));
    }

    internal static bool IsExcludedFromCapture(IntPtr windowHandle)
        =>windowHandle!=IntPtr.Zero&&GetWindowDisplayAffinity(windowHandle,out var affinity)&&affinity==WdaExcludeFromCapture;

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
