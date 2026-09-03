using System.Runtime.InteropServices;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Observes physical mouse-wheel input without consuming it. This is used only
/// while the long-screenshot window is mouse-transparent, where ordinary WPF
/// wheel events correctly reach the application underneath the overlay.
/// </summary>
internal sealed class LowLevelMouseWheelMonitor : IDisposable
{
    private const int WhMouseLowLevel=14;
    private const uint WmMouseWheel=0x020A;
    private const uint LowLevelMouseInjected=0x00000001;
    private readonly Action<int,int,int> _onPhysicalWheel;
    private readonly HookProcedure _procedure;
    private IntPtr _hook;
    private bool _disposed;

    internal LowLevelMouseWheelMonitor(Action<int,int,int> onPhysicalWheel)
    {
        _onPhysicalWheel=onPhysicalWheel??throw new ArgumentNullException(nameof(onPhysicalWheel));
        _procedure=HookCallback;
    }

    internal bool Start()
    {
        ObjectDisposedException.ThrowIf(_disposed,this);
        if(_hook!=IntPtr.Zero)return true;
        _hook=SetWindowsHookEx(WhMouseLowLevel,_procedure,GetModuleHandle(null),0);
        return _hook!=IntPtr.Zero;
    }

    internal static int DecodeWheelDelta(uint mouseData)
        =>unchecked((short)(mouseData>>16));

    internal static bool IsInjected(uint flags)=>(flags&LowLevelMouseInjected)!=0;

    private IntPtr HookCallback(int code,IntPtr message,IntPtr data)
    {
        if(code>=0&&unchecked((uint)message.ToInt64())==WmMouseWheel)
        {
            var input=Marshal.PtrToStructure<MouseHookData>(data);
            var delta=DecodeWheelDelta(input.MouseData);
            if(delta!=0&&!IsInjected(input.Flags))
            {
                try{_onPhysicalWheel(input.Point.X,input.Point.Y,delta);}
                catch
                {
                    // Never disrupt the system input chain from a hook.
                }
            }
        }
        return CallNextHookEx(_hook,code,message,data);
    }

    public void Dispose()
    {
        if(_disposed)return;
        _disposed=true;
        var hook=_hook;
        _hook=IntPtr.Zero;
        if(hook!=IntPtr.Zero)UnhookWindowsHookEx(hook);
        GC.SuppressFinalize(this);
    }

    private delegate IntPtr HookProcedure(int code,IntPtr message,IntPtr data);

    [StructLayout(LayoutKind.Sequential)]
    private struct NativePoint
    {
        public int X;
        public int Y;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MouseHookData
    {
        public NativePoint Point;
        public uint MouseData;
        public uint Flags;
        public uint Time;
        public UIntPtr ExtraInfo;
    }

    [DllImport("user32.dll",SetLastError=true)]
    private static extern IntPtr SetWindowsHookEx(int hookId,HookProcedure procedure,IntPtr module,uint threadId);

    [DllImport("user32.dll",SetLastError=true)]
    [return:MarshalAs(UnmanagedType.Bool)]
    private static extern bool UnhookWindowsHookEx(IntPtr hook);

    [DllImport("user32.dll")]
    private static extern IntPtr CallNextHookEx(IntPtr hook,int code,IntPtr message,IntPtr data);

    [DllImport("kernel32.dll",CharSet=CharSet.Unicode)]
    private static extern IntPtr GetModuleHandle(string? moduleName);
}
