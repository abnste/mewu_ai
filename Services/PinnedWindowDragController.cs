using System.Windows;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using mewu_ai_Assistant.Interop;

namespace mewu_ai_Assistant.Services;

internal sealed class PinnedWindowDragController(Window window)
{
    private const uint MoveWithoutResizeOrZOrder=0x0001|0x0004|0x0010;
    private Point _cursorOrigin;
    private NativeMethods.WindowRect _windowOrigin;
    private bool _armed,_moving;

    internal void Begin(Point pointer)
    {
        var handle=new WindowInteropHelper(window).Handle;
        if(handle==IntPtr.Zero||!NativeMethods.GetWindowRect(handle,out _windowOrigin))return;
        _cursorOrigin=window.PointToScreen(pointer);
        // Do not capture on the first button-down.  Capturing immediately can
        // steal the second click in a double-click sequence and makes the
        // pinned window's unpin gesture unreliable.
        _armed=true;_moving=false;
    }

    internal void Move(MouseButtonState leftButton,Point pointer)
    {
        if(!_armed)return;
        if(leftButton!=MouseButtonState.Pressed){End();return;}
        var cursor=window.PointToScreen(pointer);
        var deltaX=cursor.X-_cursorOrigin.X;var deltaY=cursor.Y-_cursorOrigin.Y;
        if(!_moving)
        {
            var dpi=VisualTreeHelper.GetDpi(window);
            if(!PinnedWindowInteractionPolicy.ShouldBeginDrag(
                new Point(0,0),new Point(deltaX,deltaY),
                SystemParameters.MinimumHorizontalDragDistance*dpi.DpiScaleX,
                SystemParameters.MinimumVerticalDragDistance*dpi.DpiScaleY))return;
            _moving=true;Mouse.Capture(window);
        }
        var handle=new WindowInteropHelper(window).Handle;
        if(handle!=IntPtr.Zero)NativeMethods.SetWindowPos(handle,IntPtr.Zero,_windowOrigin.Left+(int)Math.Round(deltaX),_windowOrigin.Top+(int)Math.Round(deltaY),0,0,MoveWithoutResizeOrZOrder);
    }

    internal void End()
    {
        _armed=_moving=false;
        if(Mouse.Captured==window)Mouse.Capture(null);
    }
}
