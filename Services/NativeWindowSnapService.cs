using System.Runtime.InteropServices;
using System.Windows;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal sealed class NativeWindowSnapService
{
    internal ScreenRect? FindTopmostWindowAt(int screenX,int screenY,IntPtr excludedWindow)
        =>FindTopmostTargetAt(screenX,screenY,excludedWindow)?.Bounds;

    internal WindowSnapTarget? FindTopmostTargetAt(int screenX,int screenY,IntPtr excludedWindow)
    {
        WindowSnapTarget? match=null;var currentProcess=(uint)Environment.ProcessId;
        EnumWindows((handle,_)=>
        {
            if(handle==excludedWindow||!IsWindowVisible(handle)||IsIconic(handle))return true;
            GetWindowThreadProcessId(handle,out var processId);if(processId==currentProcess)return true;
            if(!TryGetBounds(handle,out var bounds)||bounds.Width<24||bounds.Height<24)return true;
            if(screenX<bounds.X||screenX>=bounds.X+bounds.Width||screenY<bounds.Y||screenY>=bounds.Y+bounds.Height)return true;
            var target=DeepestChildAt(handle,screenX,screenY);var selectedBounds=bounds;if(target!=handle&&TryGetBounds(target,out var childBounds)&&childBounds.Width>=24&&childBounds.Height>=24&&screenX>=childBounds.X&&screenX<childBounds.X+childBounds.Width&&screenY>=childBounds.Y&&screenY<childBounds.Y+childBounds.Height)selectedBounds=childBounds;match=new WindowSnapTarget(target,selectedBounds);return false;
        },IntPtr.Zero);
        return match;
    }

    private static bool TryGetBounds(IntPtr handle,out ScreenRect bounds)
    {
        bounds=default;NativeRect rectangle;
        try{if(DwmGetWindowAttribute(handle,9,out rectangle,Marshal.SizeOf<NativeRect>())<0&&!GetWindowRect(handle,out rectangle))return false;}
        catch(DllNotFoundException){if(!GetWindowRect(handle,out rectangle))return false;}
        var width=rectangle.Right-rectangle.Left;var height=rectangle.Bottom-rectangle.Top;if(width<=0||height<=0)return false;bounds=new ScreenRect(rectangle.Left,rectangle.Top,width,height);return true;
    }
    private static IntPtr DeepestChildAt(IntPtr root,int screenX,int screenY)
    {
        var current=root;for(var depth=0;depth<12;depth++){var point=new NativePoint{X=screenX,Y=screenY};if(!ScreenToClient(current,ref point))break;var child=ChildWindowFromPointEx(current,point,0x0001|0x0002|0x0004);if(child==IntPtr.Zero||child==current)break;current=child;}return current;
    }

    private delegate bool EnumWindowsCallback(IntPtr handle,IntPtr parameter);
    [StructLayout(LayoutKind.Sequential)]private struct NativeRect{public int Left,Top,Right,Bottom;}
    [StructLayout(LayoutKind.Sequential)]private struct NativePoint{public int X,Y;}
    [DllImport("user32.dll")]private static extern bool EnumWindows(EnumWindowsCallback callback,IntPtr parameter);
    [DllImport("user32.dll")]private static extern bool IsWindowVisible(IntPtr handle);
    [DllImport("user32.dll")]private static extern bool IsIconic(IntPtr handle);
    [DllImport("user32.dll")]private static extern uint GetWindowThreadProcessId(IntPtr handle,out uint processId);
    [DllImport("user32.dll")]private static extern bool GetWindowRect(IntPtr handle,out NativeRect rectangle);
    [DllImport("user32.dll")]private static extern bool ScreenToClient(IntPtr handle,ref NativePoint point);
    [DllImport("user32.dll")]private static extern IntPtr ChildWindowFromPointEx(IntPtr parent,NativePoint point,uint flags);
    [DllImport("dwmapi.dll")]private static extern int DwmGetWindowAttribute(IntPtr handle,int attribute,out NativeRect value,int valueSize);
}

internal sealed record WindowSnapTarget(IntPtr Handle,ScreenRect Bounds);

internal static class SelectionSnapPolicy
{
    internal static Rect SnapResize(Rect value,string directions,Rect target,double threshold)
    {
        if(value.IsEmpty||target.IsEmpty||threshold<0)return value;var left=value.Left;var top=value.Top;var right=value.Right;var bottom=value.Bottom;
        if(directions.Contains('W'))left=Nearest(left,target.Left,target.Right,threshold);
        if(directions.Contains('E'))right=Nearest(right,target.Left,target.Right,threshold);
        if(directions.Contains('N'))top=Nearest(top,target.Top,target.Bottom,threshold);
        if(directions.Contains('S'))bottom=Nearest(bottom,target.Top,target.Bottom,threshold);
        return right-left>=12&&bottom-top>=12?new Rect(new Point(left,top),new Point(right,bottom)):value;
    }
    private static double Nearest(double value,double first,double second,double threshold){var firstDistance=Math.Abs(value-first);var secondDistance=Math.Abs(value-second);if(firstDistance<=threshold&&firstDistance<=secondDistance)return first;if(secondDistance<=threshold)return second;return value;}
}
