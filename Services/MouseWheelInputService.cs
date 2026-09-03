using System.Runtime.InteropServices;

namespace mewu_ai_Assistant.Services;

internal static class MouseWheelInputService
{
    internal static bool SetCursor(int x,int y)=>SetCursorPos(x,y);
    internal static bool ScrollDown(int amount=720){var input=new Input{Type=0,Mouse=new MouseInput{MouseData=unchecked((uint)-Math.Abs(amount)),Flags=0x0800}};return SendInput(1,[input],Marshal.SizeOf<Input>())==1;}
    internal static bool ScrollDown(IntPtr target,int screenX,int screenY,int amount=720)
    {
        return Scroll(target,screenX,screenY,-Math.Abs(amount));
    }
    internal static bool Scroll(IntPtr target,int screenX,int screenY,int delta)
    {
        if(delta==0)return true;
        if(target==IntPtr.Zero){var input=new Input{Type=0,Mouse=new MouseInput{MouseData=unchecked((uint)delta),Flags=0x0800}};return SendInput(1,[input],Marshal.SizeOf<Input>())==1;}
        var wheel=unchecked((ushort)(short)Math.Clamp(delta,short.MinValue,short.MaxValue));var wParam=new IntPtr((long)wheel<<16);var lParam=new IntPtr((screenY&0xffff)<<16|(screenX&0xffff));return PostMessage(target,0x020A,wParam,lParam);
    }
    [StructLayout(LayoutKind.Sequential)]private struct Input{public uint Type;public MouseInput Mouse;}
    [StructLayout(LayoutKind.Sequential)]private struct MouseInput{public int Dx,Dy;public uint MouseData,Flags,Time;public UIntPtr ExtraInfo;}
    [DllImport("user32.dll")]private static extern uint SendInput(uint count,Input[] inputs,int size);
    [DllImport("user32.dll")]private static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")]private static extern bool PostMessage(IntPtr window,uint message,IntPtr wParam,IntPtr lParam);
}
