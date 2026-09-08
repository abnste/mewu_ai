// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private const int TeachingCaptureFinishHotkeyId=0x6D38;
    private bool _teachingCaptureFinishRegistered;

    private bool PrepareTeachingLiveCapture()
    {
        if(!IsTeachingMode)return true;
        var handle=new WindowInteropHelper(this).Handle;
        if(!NativeMethods.IsVisibleToCapture(handle))
        {SetPromptBarHidden(false);PromptStatus.Text=LocalizationService.T("教学共享状态不可用，请重新截图后重试","Screen sharing is unavailable. Start a new capture and try again.");return false;}
        if(!_teachingCaptureFinishRegistered)
            _teachingCaptureFinishRegistered=NativeMethods.RegisterHotKey(handle,TeachingCaptureFinishHotkeyId,0x4000,0x77);
        if(!_teachingCaptureFinishRegistered)
        {SetPromptBarHidden(false);PromptStatus.Text=LocalizationService.T("F8 完成键被其他软件占用，请释放后重试","Another app is using F8. Release that shortcut and try again.");return false;}
        return true;
    }

    private void FinishTeachingLiveCapture()
    {
        if(_closed)return;
        if(_longCaptureMode)FinishLongCapture(this,new RoutedEventArgs());
        else if(_recordingCountdownActive)CancelRecordingCountdown();
        else if(_recordingMode)StopRecording(this,new RoutedEventArgs());
    }

    private void ReleaseTeachingLiveCapture()
    {
        TeachingLiveOutline.Visibility=Visibility.Collapsed;
        if(!_teachingCaptureFinishRegistered)return;
        NativeMethods.UnregisterHotKey(new WindowInteropHelper(this).Handle,TeachingCaptureFinishHotkeyId);
        _teachingCaptureFinishRegistered=false;
    }

    private void UpdateTeachingLiveOutline(SelectionItem item)
    {
        if(!IsTeachingMode)return;
        // Keep the stroke outside the exact pixel acquisition rectangle.
        Canvas.SetLeft(TeachingLiveOutline,item.Bounds.Left-3);
        Canvas.SetTop(TeachingLiveOutline,item.Bounds.Top-3);
        TeachingLiveOutline.Width=item.Bounds.Width+6;
        TeachingLiveOutline.Height=item.Bounds.Height+6;
        TeachingLiveOutline.Visibility=Visibility.Visible;
    }

    private bool IsTeachingAcquisitionClear(SelectionItem item)
    {
        var handle=new WindowInteropHelper(this).Handle;
        var pixels=ToPixelRect(item.Bounds);
        return NativeMethods.IsVisibleToCapture(handle)&&NativeMethods.IsCaptureRegionClear(handle,
            ScreenCoordinateService.ToScreenRect(pixels,_frame.OriginX,_frame.OriginY));
    }
}
