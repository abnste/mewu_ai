// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
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
    [DllImport("user32.dll")] internal static extern IntPtr GetWindow(IntPtr hWnd,uint command);
    [DllImport("user32.dll")] internal static extern uint GetDpiForWindow(IntPtr hWnd);
    [DllImport("user32.dll",SetLastError=true)] internal static extern int SetWindowRgn(IntPtr hWnd,IntPtr hRgn,bool redraw);
    [DllImport("user32.dll",SetLastError=true)] private static extern int GetWindowRgn(IntPtr hWnd,IntPtr hRgn);
    [DllImport("gdi32.dll")] private static extern bool RectInRegion(IntPtr region,ref WindowRect rectangle);
    [DllImport("user32.dll",SetLastError=true)] internal static extern bool GetWindowRect(IntPtr hWnd,out WindowRect rect);
    [DllImport("user32.dll")] internal static extern bool IsWindow(IntPtr hWnd);
    [DllImport("gdi32.dll",SetLastError=true)] internal static extern IntPtr CreateRectRgn(int left,int top,int right,int bottom);
    [DllImport("gdi32.dll",SetLastError=true)] internal static extern int CombineRgn(IntPtr destination,IntPtr source1,IntPtr source2,int mode);
    [DllImport("gdi32.dll",SetLastError=true)] internal static extern bool DeleteObject(IntPtr handle);
    [DllImport("dwmapi.dll",PreserveSig=true)] private static extern int DwmSetWindowAttribute(IntPtr windowHandle,int attribute,ref int value,int valueSize);
    [DllImport("dwmapi.dll",PreserveSig=true)] private static extern int DwmGetWindowAttribute(IntPtr windowHandle,int attribute,out int value,int valueSize);
    [DllImport("user32.dll",EntryPoint="GetWindowLongPtrW",SetLastError=true)] private static extern IntPtr GetWindowLongPtr(IntPtr windowHandle,int index);
    [DllImport("user32.dll",EntryPoint="SetWindowLongPtrW",SetLastError=true)] private static extern IntPtr SetWindowLongPtr(IntPtr windowHandle,int index,IntPtr value);

    private const int DwmWindowCornerPreference=33;
    private const int DwmCornerRound=2;
    private const int DwmWindowCloak=13,DwmWindowCloaked=14,DwmCloakedApp=1;

    // Hide only for a synchronous desktop snapshot. Changing display affinity,
    // even briefly, can stop NVIDIA Instant Replay for the whole desktop.
    internal static T WithWindowCloaked<T>(IntPtr windowHandle,Func<T> capture)
    {
        ArgumentNullException.ThrowIfNull(capture);
        Marshal.ThrowExceptionForHR(DwmGetWindowAttribute(windowHandle,DwmWindowCloaked,out var original,sizeof(int)));
        var cloaked=1;
        Marshal.ThrowExceptionForHR(DwmSetWindowAttribute(windowHandle,DwmWindowCloak,ref cloaked,sizeof(int)));
        try
        {
            FlushComposition();
            return capture();
        }
        finally
        {
            // Preserve an existing app cloak, including nested snapshot calls.
            // Shell/inherited cloaks are controlled independently by Windows.
            var restore=(original&DwmCloakedApp)!=0?1:0;
            Marshal.ThrowExceptionForHR(DwmSetWindowAttribute(windowHandle,DwmWindowCloak,ref restore,sizeof(int)));
            FlushComposition();
        }
    }

    internal static bool IsWindowUncloaked(IntPtr windowHandle)
        =>DwmGetWindowAttribute(windowHandle,DwmWindowCloaked,out var value,sizeof(int))>=0&&(value&DwmCloakedApp)==0;

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
        // compiled out of Release builds. Product teaching visibility uses a separate, explicit API.
        if (VisualQaCaptureEnabled&&!requireProtection)
            return SetWindowDisplayAffinity(windowHandle,0);
#endif
        return SetWindowDisplayAffinity(windowHandle, WdaExcludeFromCapture)&&(!requireProtection||IsExcludedFromCapture(windowHandle));
      }

      internal static bool SetWindowCaptureVisibleForDiagnostics(IntPtr windowHandle)
          =>windowHandle!=IntPtr.Zero&&SetWindowDisplayAffinity(windowHandle,0);

    // Presentation windows follow the explicit teaching/screen-sharing setting.
    internal static bool ApplyPresentationCaptureVisibility(IntPtr windowHandle,bool teachingMode)
        =>teachingMode
            ?windowHandle!=IntPtr.Zero&&SetWindowDisplayAffinity(windowHandle,0)&&IsVisibleToCapture(windowHandle)
            :ExcludeFromCapture(windowHandle);

    // Dialogs follow their actual owner's capture policy, including settings
    // and nested dialogs. Missing/unknown owners retain capture protection.
    internal static bool ApplyOwnedWindowCaptureVisibility(IntPtr windowHandle,IntPtr ownerHandle)
        =>IsVisibleToCapture(ownerHandle)
            ?ApplyPresentationCaptureVisibility(windowHandle,true)
            :ExcludeFromCapture(windowHandle,requireProtection:true);

    internal static bool IsVisibleToCapture(IntPtr windowHandle)
        =>windowHandle!=IntPtr.Zero&&GetWindowDisplayAffinity(windowHandle,out var affinity)&&affinity==0;

    [DllImport("dwmapi.dll",PreserveSig=true)]
    private static extern int DwmFlush();

    internal static void FlushComposition()=>Marshal.ThrowExceptionForHR(DwmFlush());

    internal static bool IsExcludedFromCapture(IntPtr windowHandle)
        =>windowHandle!=IntPtr.Zero&&GetWindowDisplayAffinity(windowHandle,out var affinity)&&affinity==WdaExcludeFromCapture;

    // A teaching overlay stays visible to screen sharing. Its native region
    // must exclude every pixel we acquire; WPF transparency alone is not proof.
    internal static bool IsCaptureRegionClear(IntPtr windowHandle,mewu_ai_Assistant.Models.ScreenRect capture)
    {
        if(windowHandle==IntPtr.Zero||capture.IsEmpty||!GetWindowRect(windowHandle,out var bounds))return false;
        var relative=mewu_ai_Assistant.Services.ScreenCoordinateService.ToWindowRelativePixelRect(
            new System.Windows.Int32Rect(0,0,capture.Width,capture.Height),capture.X,capture.Y,
            new mewu_ai_Assistant.Models.ScreenRect(bounds.Left,bounds.Top,bounds.Right-bounds.Left,bounds.Bottom-bounds.Top));
        if(relative.IsEmpty)return true;
        var region=CreateRectRgn(0,0,0,0);if(region==IntPtr.Zero)return false;
        try
        {
            if(GetWindowRgn(windowHandle,region)==0)return false;
            var rectangle=new WindowRect{Left=relative.X,Top=relative.Y,Right=relative.Right,Bottom=relative.Bottom};
            return !RectInRegion(region,ref rectangle);
        }
        finally{DeleteObject(region);}
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
