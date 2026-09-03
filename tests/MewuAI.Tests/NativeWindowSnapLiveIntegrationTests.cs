using System.Diagnostics;
using System.Runtime.ExceptionServices;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Interop;
using System.Windows.Media;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class NativeWindowSnapLiveIntegrationTests
{
    [Fact]
    public void OverlayStillResolvesARealCodexControl()
    {
        if(!string.Equals(Environment.GetEnvironmentVariable("MEWU_SNAP_LIVE"),"1",StringComparison.Ordinal))return;

        Exception? failure=null;
        var thread=new Thread(() =>
        {
            try{RunOverlayProbe();}
            catch(Exception ex){failure=ex;}
        });
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        Assert.True(thread.Join(TimeSpan.FromSeconds(15)),"Codex 覆盖层吸附实景测试超时");
        if(failure is not null)ExceptionDispatchInfo.Capture(failure).Throw();
    }

    private static void RunOverlayProbe()
    {
        var codex=Process.GetProcessesByName("ChatGPT")
            .Where(process=>process.MainWindowHandle!=IntPtr.Zero)
            .OrderByDescending(process=>process.MainWindowTitle.Length)
            .FirstOrDefault()??throw new InvalidOperationException("未找到可见的 Codex/ChatGPT 窗口");
        var root=AutomationElement.FromHandle(codex.MainWindowHandle);
        var rootBounds=root.Current.BoundingRectangle;
        var expected=FindSemanticTarget(rootBounds);
        var point=expected is { } semantic
            ?new System.Windows.Point(semantic.Left+semantic.Width/2,semantic.Top+semantic.Height/2)
            :new System.Windows.Point(rootBounds.Left+rootBounds.Width/2,rootBounds.Top+rootBounds.Height/2);
        var virtualScreen=System.Windows.Forms.SystemInformation.VirtualScreen;
        var overlay=new Window
        {
            WindowStyle=WindowStyle.None,
            ResizeMode=ResizeMode.NoResize,
            ShowInTaskbar=false,
            Topmost=true,
            Background=Brushes.Black,
            Left=virtualScreen.Left,
            Top=virtualScreen.Top,
            Width=Math.Max(1,virtualScreen.Width),
            Height=Math.Max(1,virtualScreen.Height)
        };
        try
        {
            overlay.Show();
            var overlayHandle=new WindowInteropHelper(overlay).Handle;
            NativeMethods.SetWindowPos(
                overlayHandle,new IntPtr(-1),
                virtualScreen.Left,virtualScreen.Top,
                virtualScreen.Width,virtualScreen.Height,0x0040);

            var service=new NativeWindowSnapService();
            var result=service.FindTopmostTargetAt(
                (int)Math.Round(point.X),(int)Math.Round(point.Y),overlayHandle);
            Assert.NotNull(result);
            if(expected is { } expectedBounds)
                Assert.True(IsSameBounds(expectedBounds,result!.Bounds),
                    $"覆盖层下命中了错误区域：期望 {Format(expectedBounds)}，实际 {result.Bounds}；{DescribeWindow(result.Handle)}；Codex HWND {codex.MainWindowHandle}");

            // Use the same service after it has populated both native and
            // semantic caches. A real full-screen overlay must not let those
            // caches make the taskbar fall through to the maximized app.
            var taskbar=FindWindow("Shell_TrayWnd",null);
            Assert.NotEqual(IntPtr.Zero,taskbar);
            Assert.True(GetWindowRect(taskbar,out var taskbarBounds));
            var taskbarX=taskbarBounds.Left+(taskbarBounds.Right-taskbarBounds.Left)/2;
            var taskbarY=taskbarBounds.Top+(taskbarBounds.Bottom-taskbarBounds.Top)/2;
            Assert.Null(service.FindFastTargetAt(taskbarX,taskbarY,overlayHandle));
            Assert.Null(service.FindTopmostTargetAt(taskbarX,taskbarY,overlayHandle));
        }
        finally{overlay.Close();}
    }

    private static Rect? FindSemanticTarget(Rect rootBounds)
    {
        for(var row=1;row<=10;row++)
        for(var column=1;column<=14;column++)
        {
            var point=new System.Windows.Point(
                rootBounds.Left+rootBounds.Width*column/15,
                rootBounds.Top+rootBounds.Height*row/11);
            AutomationElement? element;
            try{element=AutomationElement.FromPoint(point);}
            catch{continue;}
            for(var depth=0;element is not null&&depth<16;depth++)
            {
                try
                {
                    var info=element.Current;
                    var bounds=info.BoundingRectangle;
                    if((info.ControlType==ControlType.Button||info.ControlType==ControlType.Edit||info.ControlType==ControlType.MenuItem)&&
                       rootBounds.Contains(bounds.TopLeft)&&
                       bounds.Width>=16&&bounds.Height>=16&&bounds.Width<rootBounds.Width*.8&&bounds.Height<rootBounds.Height*.8)
                        return bounds;
                    element=TreeWalker.RawViewWalker.GetParent(element);
                }
                catch{break;}
            }
        }
        return null;
    }

    private static bool IsSameBounds(Rect expected,ScreenRect actual)
        =>Math.Abs(expected.Left-actual.X)<=3&&Math.Abs(expected.Top-actual.Y)<=3&&
          Math.Abs(expected.Width-actual.Width)<=4&&Math.Abs(expected.Height-actual.Height)<=4;

    private static string Format(Rect value)
        =>$"{value.Left:0},{value.Top:0} {value.Width:0}x{value.Height:0}";

    private static string DescribeWindow(IntPtr handle)
    {
        GetWindowThreadProcessId(handle,out var processId);
        var className=new StringBuilder(256);GetClassName(handle,className,className.Capacity);
        var title=new StringBuilder(256);GetWindowText(handle,title,title.Capacity);
        var processName="unknown";
        try{processName=Process.GetProcessById(unchecked((int)processId)).ProcessName;}catch{}
        DwmGetWindowAttribute(handle,14,out var cloaked,sizeof(int));
        return $"目标 HWND {handle}，进程 {processName}({processId})，类 {className}，标题 {title}，cloaked {cloaked}";
    }

    [DllImport("user32.dll")]private static extern uint GetWindowThreadProcessId(IntPtr handle,out uint processId);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)]private static extern int GetClassName(IntPtr handle,StringBuilder value,int count);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)]private static extern int GetWindowText(IntPtr handle,StringBuilder value,int count);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)]private static extern IntPtr FindWindow(string className,string? windowName);
    [DllImport("user32.dll")]private static extern bool GetWindowRect(IntPtr handle,out NativeRect rectangle);
    [DllImport("dwmapi.dll")]private static extern int DwmGetWindowAttribute(IntPtr handle,int attribute,out int value,int size);
    [StructLayout(LayoutKind.Sequential)]private struct NativeRect{public int Left,Top,Right,Bottom;}
}
