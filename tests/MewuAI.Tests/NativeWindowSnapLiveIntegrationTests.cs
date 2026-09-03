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
        var expected=FindSemanticTarget(rootBounds,codex.Id)
            ??throw new InvalidOperationException("Codex 窗口内未找到可用于实景测试的按钮或输入框");
        var point=new System.Windows.Point(expected.Left+expected.Width/2,expected.Top+expected.Height/2);
        var overlay=new Window
        {
            WindowStyle=WindowStyle.None,
            ResizeMode=ResizeMode.NoResize,
            ShowInTaskbar=false,
            Topmost=true,
            Background=Brushes.Black,
            Left=rootBounds.Left,
            Top=rootBounds.Top,
            Width=Math.Max(1,rootBounds.Width),
            Height=Math.Max(1,rootBounds.Height)
        };
        try
        {
            overlay.Show();
            var overlayHandle=new WindowInteropHelper(overlay).Handle;
            NativeMethods.SetWindowPos(
                overlayHandle,new IntPtr(-1),
                (int)Math.Round(rootBounds.Left),(int)Math.Round(rootBounds.Top),
                (int)Math.Round(rootBounds.Width),(int)Math.Round(rootBounds.Height),0x0040);

            var result=new NativeWindowSnapService().FindTopmostTargetAt(
                (int)Math.Round(point.X),(int)Math.Round(point.Y),overlayHandle);
            Assert.NotNull(result);
            Assert.True(IsSameBounds(expected,result!.Bounds),
                $"覆盖层下命中了错误区域：期望 {Format(expected)}，实际 {result.Bounds}；{DescribeWindow(result.Handle)}；Codex HWND {codex.MainWindowHandle}");
        }
        finally{overlay.Close();}
    }

    private static Rect? FindSemanticTarget(Rect rootBounds,int processId)
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
                    if(info.ProcessId==processId&&
                       (info.ControlType==ControlType.Button||info.ControlType==ControlType.Edit||info.ControlType==ControlType.MenuItem)&&
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
    [DllImport("dwmapi.dll")]private static extern int DwmGetWindowAttribute(IntPtr handle,int attribute,out int value,int size);
}
