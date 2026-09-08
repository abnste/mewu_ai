// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    // Runs only while another window owns activation. No hook, focus stealing,
    // or global shortcut reservation; only the Escape down-edge is observed.
    private readonly DispatcherTimer _inactiveEscapeTimer=new(){Interval=TimeSpan.FromMilliseconds(60)};
    private bool _inactiveEscapeHeld;

    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window,out uint processId);
    private static bool IsEscapePressed()=>(GetAsyncKeyState(0x1B)&0x8000)!=0;

    private void CheckInactiveEscape(object? sender,EventArgs e)
    {
        var pressed=IsEscapePressed();var rising=pressed&&!_inactiveEscapeHeld;_inactiveEscapeHeld=pressed;
        if(!rising||_closed||IsActive||!IsVisible||_systemFileDialogDepth>0||_drawingModalOpen||_longCaptureMode)return;
        var foreground=GetForegroundWindow();
        if(foreground==IntPtr.Zero)return;
        GetWindowThreadProcessId(foreground,out var processId);
        // Our settings, color/save dialogs and pinned windows handle their own
        // Escape. A key dismissing one must not also close this window.
        if(processId==0||processId==(uint)Environment.ProcessId)return;
        HandleEscape();
    }

    private void CopyCurrentScreenshotAndClose()
    {
        if(Active is not {IsImplicit:false} item)return;
        if(item.VideoPath is not null){PromptStatus.Text="视频请使用工具条复制或保存";return;}
        try
        {
            if(_drawingMode)ExitDrawingMode();
            if(!ClipboardService.TrySetImage(RenderSelectionImage(item,true,true,true),out var error))
            {
                PromptStatus.Text=error;return;
            }
            Close();
        }
        catch(Exception ex)
        {
            new PrivacyLogger().Error("OverlayCopyAndClose",ex);
            PromptStatus.Text="截图未能复制，请重试或使用保存";
        }
    }
}
