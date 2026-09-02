using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Automation;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal sealed class NativeWindowSnapService
{
    internal ScreenRect? FindTopmostWindowAt(int screenX,int screenY,IntPtr excludedWindow)
        =>FindTopmostTargetAt(screenX,screenY,excludedWindow)?.Bounds;

    internal WindowSnapTarget? FindTopmostTargetAt(int screenX,int screenY,IntPtr excludedWindow)
    {
        var pointTarget=FindAutomationElementAtPoint(screenX,screenY,excludedWindow);
        if(pointTarget is not null)return pointTarget;
        WindowSnapTarget? match=null;var currentProcess=(uint)Environment.ProcessId;
        EnumWindows((handle,_)=>
        {
            if(handle==excludedWindow||!IsWindowVisible(handle)||IsIconic(handle))return true;
            if(IsDesktopShellWindow(handle))return true;
            GetWindowThreadProcessId(handle,out var processId);if(processId==currentProcess)return true;
            if(!TryGetBounds(handle,out var bounds)||bounds.Width<24||bounds.Height<24)return true;
            if(screenX<bounds.X||screenX>=bounds.X+bounds.Width||screenY<bounds.Y||screenY>=bounds.Y+bounds.Height)return true;
            // UIA exposes controls inside a browser or modern app even when
            // they are not separate Win32 child HWNDs. Walk only the branch
            // containing the pointer (bounded depth/siblings), then fall back
            // to the native child HWND hierarchy for classic windows.
            var automationTarget=FindAutomationDescendantAt(handle,screenX,screenY,currentProcess);
            if(automationTarget is not null){match=automationTarget;return false;}
            var target=DeepestChildAt(handle,screenX,screenY);
            if(target!=handle&&TryGetBounds(target,out var childBounds)&&childBounds.Width>=24&&childBounds.Height>=24&&screenX>=childBounds.X&&screenX<childBounds.X+childBounds.Width&&screenY>=childBounds.Y&&screenY<childBounds.Y+childBounds.Height)
            {
                var childArea=(long)childBounds.Width*childBounds.Height;var windowArea=(long)bounds.Width*bounds.Height;
                // Chromium/Edge render-host HWNDs commonly cover the whole
                // client area; they are not a concrete UI element. Do not
                // present those as a fake full-screen snap candidate.
                if(childArea*100<windowArea*88){match=new WindowSnapTarget(target,childBounds);return false;}
            }
            // This is a real top-level application window (desktop shell
            // windows were filtered above). Do not treat a nearly full-monitor
            // client as a useful snap target when no concrete UIA child was
            // exposed; that was the source of the old "snap to full screen"
            // behaviour.
            var monitor=System.Windows.Forms.Screen.FromPoint(new System.Drawing.Point(screenX,screenY)).Bounds;
            var topWindowArea=(long)bounds.Width*bounds.Height;var monitorArea=(long)monitor.Width*monitor.Height;
            if(topWindowArea*100<monitorArea*97){match=new WindowSnapTarget(handle,bounds);return false;}
            return true;
        },IntPtr.Zero);
        return match;
    }

    private static WindowSnapTarget? FindAutomationElementAtPoint(int screenX,int screenY,IntPtr excludedWindow)
    {
        try
        {
            var element=AutomationElement.FromPoint(new System.Windows.Point(screenX,screenY));
            var walker=TreeWalker.RawViewWalker;var currentProcess=(int)Environment.ProcessId;
            for(var depth=0;element is not null&&depth<8;depth++)
            {
                var info=element.Current;var rectangle=info.BoundingRectangle;
                if(info.ProcessId!=currentProcess&&info.NativeWindowHandle!=excludedWindow.ToInt32()&&IsUsefulElement(element,info,rectangle,screenX,screenY))
                {
                    return new WindowSnapTarget(info.NativeWindowHandle==0?IntPtr.Zero:new IntPtr(info.NativeWindowHandle),new ScreenRect((int)Math.Round(rectangle.Left),(int)Math.Round(rectangle.Top),(int)Math.Round(rectangle.Width),(int)Math.Round(rectangle.Height)));
                }
                element=walker.GetParent(element);
            }
        }
        catch(COMException){}
        catch(InvalidOperationException){}
        catch(ArgumentException){}
        return null;
    }

    private static WindowSnapTarget? FindAutomationDescendantAt(IntPtr windowHandle,int screenX,int screenY,uint currentProcess)
    {
        try
        {
            var root=AutomationElement.FromHandle(windowHandle);var walker=TreeWalker.RawViewWalker;WindowSnapTarget? best=null;
            var pending=new Stack<(AutomationElement Element,int Depth)>();pending.Push((root,0));var visited=0;
            while(pending.Count>0&&visited++<1024)
            {
                var (element,depth)=pending.Pop();var info=element.Current;var rectangle=info.BoundingRectangle;
                if(info.ProcessId!=(int)currentProcess&&IsUsefulElement(element,info,rectangle,screenX,screenY))
                {
                    var candidate=new ScreenRect((int)Math.Round(rectangle.Left),(int)Math.Round(rectangle.Top),(int)Math.Round(rectangle.Width),(int)Math.Round(rectangle.Height));
                    var area=(long)candidate.Width*candidate.Height;if(best is null||area<(long)best.Bounds.Width*best.Bounds.Height)best=new WindowSnapTarget(info.NativeWindowHandle==0?windowHandle:new IntPtr(info.NativeWindowHandle),candidate);
                }
                if(depth>=10)continue;
                var child=walker.GetFirstChild(element);var siblings=0;while(child is not null&&siblings++<64){pending.Push((child,depth+1));child=walker.GetNextSibling(child);}
            }
            return best;
        }
        catch(COMException){return null;}
        catch(InvalidOperationException){return null;}
        catch(ArgumentException){return null;}
    }

    private static bool IsUsefulElement(AutomationElement element,AutomationElement.AutomationElementInformation info,Rect rectangle,int screenX,int screenY)
    {
        if(rectangle.Width<24||rectangle.Height<18||rectangle.Width>4096||rectangle.Height>4096||!rectangle.Contains(screenX,screenY))return false;
        var type=info.ControlType;
        if(type is not null&&(
            type==ControlType.Button||type==ControlType.CheckBox||type==ControlType.ComboBox||
            type==ControlType.Hyperlink||type==ControlType.Image||type==ControlType.ListItem||
            type==ControlType.MenuItem||type==ControlType.RadioButton||type==ControlType.TabItem||
            type==ControlType.Edit||type==ControlType.Slider||type==ControlType.SplitButton))return true;
        // Custom providers (notably Electron/Chromium accessibility trees)
        // often expose no standard ControlType. Their action patterns are a
        // stronger signal than the type name, but still require a compact
        // rectangle so a document/window cannot become the snap target.
        if(rectangle.Width<=900&&rectangle.Height<=180&&rectangle.Width*rectangle.Height<=300_000&&info.IsKeyboardFocusable)return true;
        try
        {
            return rectangle.Width<=900&&rectangle.Height<=180&&rectangle.Width*rectangle.Height<=300_000&&(
                element.TryGetCurrentPattern(InvokePattern.Pattern,out _)||
                element.TryGetCurrentPattern(TogglePattern.Pattern,out _)||
                element.TryGetCurrentPattern(SelectionItemPattern.Pattern,out _)||
                element.TryGetCurrentPattern(ValuePattern.Pattern,out _));
        }
        catch(InvalidOperationException){return false;}
    }

    private static bool TryGetBounds(IntPtr handle,out ScreenRect bounds)
    {
        bounds=default;NativeRect rectangle;
        try{if(DwmGetWindowAttribute(handle,9,out rectangle,Marshal.SizeOf<NativeRect>())<0&&!GetWindowRect(handle,out rectangle))return false;}
        catch(DllNotFoundException){if(!GetWindowRect(handle,out rectangle))return false;}
        var width=rectangle.Right-rectangle.Left;var height=rectangle.Bottom-rectangle.Top;if(width<=0||height<=0)return false;bounds=new ScreenRect(rectangle.Left,rectangle.Top,width,height);return true;
    }

    private static bool IsDesktopShellWindow(IntPtr handle)
    {
        Span<char> buffer=stackalloc char[128];var length=GetClassName(handle,ref MemoryMarshal.GetReference(buffer),buffer.Length);if(length<=0)return false;
        var name=new string(buffer[..length]);
        return name is "Progman" or "WorkerW" or "Shell_TrayWnd" or "Shell_SecondaryTrayWnd" or "Windows.UI.Core.CoreWindow";
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
    [DllImport("user32.dll",CharSet=CharSet.Unicode)]private static extern int GetClassName(IntPtr handle,ref char className,int maxCount);
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
