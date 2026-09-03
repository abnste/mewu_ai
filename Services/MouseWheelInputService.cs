using System.Runtime.InteropServices;

namespace mewu_ai_Assistant.Services;

internal static class MouseWheelInputService
{
    private const uint WmMouseWheel=0x020A;
    private const uint GetAncestorRoot=2;
    private const uint ChildWindowSkipInvisible=0x0001;
    private const uint ChildWindowSkipDisabled=0x0002;
    private const uint ChildWindowSkipTransparent=0x0004;
    private const uint ChildWindowFlags=ChildWindowSkipInvisible|ChildWindowSkipDisabled|ChildWindowSkipTransparent;

    internal static bool SetCursor(int x,int y)=>SetCursorPos(x,y);
    internal static bool InjectWheel(int delta)
    {
        if(delta==0)return true;
        var input=new Input{Type=0,Mouse=new MouseInput{MouseData=unchecked((uint)delta),Flags=0x0800}};
        return SendInput(1,[input],Marshal.SizeOf<Input>())==1;
    }
    internal static bool ScrollDown(int amount=720){var input=new Input{Type=0,Mouse=new MouseInput{MouseData=unchecked((uint)-Math.Abs(amount)),Flags=0x0800}};return SendInput(1,[input],Marshal.SizeOf<Input>())==1;}
    internal static bool ScrollDown(IntPtr target,int screenX,int screenY,int amount=720)
    {
        return Scroll(target,screenX,screenY,-Math.Abs(amount));
    }
    internal static bool Scroll(IntPtr target,int screenX,int screenY,int delta)
    {
        if(delta==0)return true;
        // Sending synthetic input while the capture overlay owns the pointer
        // only delivers the wheel back to the overlay.  Require a real lower
        // target so a missing hit cannot recurse without scrolling the page.
        if(target==IntPtr.Zero)return false;

        var root=GetAncestor(target,GetAncestorRoot);
        if(root==IntPtr.Zero)root=target;
        var deepest=DeepestChildAt(root,screenX,screenY);
        var recipient=deepest!=IntPtr.Zero?deepest:target;
        var wParam=PackWheelWParam(delta);
        var lParam=PackScreenPointLParam(screenX,screenY);
        if(PostMessage(recipient,WmMouseWheel,wParam,lParam))return true;

        // WM_MOUSEWHEEL naturally propagates from a child to its parents via
        // DefWindowProc.  Only fall back when the child could not be queued;
        // posting to both would make a single notch scroll twice.
        return recipient!=target&&PostMessage(target,WmMouseWheel,wParam,lParam);
    }

    internal static IntPtr PackWheelWParam(int delta)
    {
        var wheel=unchecked((ushort)(short)Math.Clamp(delta,short.MinValue,short.MaxValue));
        return new IntPtr(unchecked((int)((uint)wheel<<16)));
    }

    internal static IntPtr PackScreenPointLParam(int screenX,int screenY)
        =>new(unchecked((int)(((uint)(ushort)screenY<<16)|(ushort)screenX)));

    private static IntPtr DeepestChildAt(IntPtr root,int screenX,int screenY)
    {
        var current=root;
        for(var depth=0;depth<16;depth++)
        {
            var point=new NativePoint{X=screenX,Y=screenY};
            if(!ScreenToClient(current,ref point))break;
            var child=ChildWindowFromPointEx(current,point,ChildWindowFlags);
            if(child==IntPtr.Zero||child==current)break;
            current=child;
        }
        return current;
    }

    [StructLayout(LayoutKind.Sequential)]private struct Input{public uint Type;public MouseInput Mouse;}
    [StructLayout(LayoutKind.Sequential)]private struct MouseInput{public int Dx,Dy;public uint MouseData,Flags,Time;public UIntPtr ExtraInfo;}
    [StructLayout(LayoutKind.Sequential)]private struct NativePoint{public int X,Y;}
    [DllImport("user32.dll")]private static extern uint SendInput(uint count,Input[] inputs,int size);
    [DllImport("user32.dll")]private static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")]private static extern bool PostMessage(IntPtr window,uint message,IntPtr wParam,IntPtr lParam);
    [DllImport("user32.dll")]private static extern IntPtr GetAncestor(IntPtr handle,uint flags);
    [DllImport("user32.dll")]private static extern bool ScreenToClient(IntPtr handle,ref NativePoint point);
    [DllImport("user32.dll")]private static extern IntPtr ChildWindowFromPointEx(IntPtr parent,NativePoint point,uint flags);
}
