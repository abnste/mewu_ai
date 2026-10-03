// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Threading;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private RightButtonPassThrough? _rightPassThrough;
    private DispatcherTimer? _rightPassThroughPolicyTimer;
    private bool _rightPassThroughVisual;

    private bool CanBeginRightPassThrough()=>_overlayReady&&!_closed&&!_recordingMode&&!_recordingCountdownActive&&
        !_videoTrimBar.IsInteracting&&!_drawingMode&&!_longCaptureMode&&!_applicationSnapshotActive&&!_drawingModalOpen&&_systemFileDialogDepth==0&&!_selecting&&!_moving;

    private async void StartRightPassThrough()
    {
        var service=new RightButtonPassThrough(new WindowInteropHelper(this).Handle,
            generation=>Dispatcher.BeginInvoke(new Action(()=>BeginRightPassThroughVisual(generation))),
            generation=>Dispatcher.BeginInvoke(new Action(()=>EndRightPassThroughVisual(generation))),
            ()=>Dispatcher.BeginInvoke(new Action(()=>{if(!_closed)PromptStatus.Text=L("无法穿透操作此窗口，请检查目标应用权限。","Cannot pass input to this window. Check the target application's permissions.");})));
        _rightPassThrough=service;
        if(!await service.StartAsync())
        {
            service.Dispose();if(_closed)return;
            _rightPassThrough=null;PromptStatus.Text=L("右键穿透未能启动，请重新截图。","Right-button pass-through could not start. Start a new capture.");return;
        }
        if(_closed){service.Dispose();return;}
        _rightPassThroughPolicyTimer=new DispatcherTimer(TimeSpan.FromMilliseconds(32),DispatcherPriority.Input,(_,_)=>UpdateRightPassThroughPolicy(),Dispatcher);
        UpdateRightPassThroughPolicy();
    }

    private PointerPassThroughPolicy BuildRightPassThroughPolicy()
    {
        var excluded=new List<ScreenRect>();
        _rightPassThrough?.ExcludeHigherWindows(excluded);
        void Exclude(FrameworkElement element)
        {
            if(!element.IsVisible||!element.IsHitTestVisible)return;
            var bounds=GetElementBounds(element);if(bounds.IsEmpty)return;
            excluded.Add(ScreenCoordinateService.ToScreenRect(ToPixelRect(bounds),_frame.OriginX,_frame.OriginY));
        }
        foreach(var element in new FrameworkElement[]{PromptBarHost,Toolbar,DrawingToolbar,RecordingBar,LongCaptureBar,_videoTrimBar})Exclude(element);
        foreach(var item in _selections)
        {
            Exclude(item.TextSelection);
            if(item.VideoPath is not null)Exclude(item.Host);
        }
        return new(CanBeginRightPassThrough(),excluded.ToArray(),Environment.TickCount64);
    }

    private void UpdateRightPassThroughPolicy()=>_rightPassThrough?.Update(BuildRightPassThroughPolicy());

    private async void BeginRightPassThroughVisual(long generation)
    {
        var service=_rightPassThrough;if(service is null||!service.IsCurrent(generation))return;
        var succeeded=false;
        try
        {
            var point=service.StartPoint;
            if(!BuildRightPassThroughPolicy().Allows(point.X,point.Y,Environment.TickCount64))return;
            if(!_rightPassThroughVisual)
            {
                // References keep their original pixels and annotations while
                // the desktop behind them changes during a forwarded gesture.
                foreach(var item in _selections.Where(item=>!item.IsImplicit&&item.VideoPath is null&&item.CapturedImageOverride is null))
                {
                    var pixels=ToPixelRect(item.Bounds);
                    if(!pixels.IsEmpty)item.CapturedImageOverride=ScreenCaptureService.Crop(_frame.Image,pixels);
                }
                if(!NativeMethods.TrySetWindowMouseTransparent(new WindowInteropHelper(this).Handle,true))return;
                _rightPassThroughVisual=true;Root.Opacity=0;
                if(Mouse.Captured is not null)Mouse.Capture(null);
            }
            await Dispatcher.InvokeAsync(()=>{},DispatcherPriority.ContextIdle);
            // WPF's first composition pass can rewrite the extended style.
            // Reapply after that pass before forwarding any native down.
            succeeded=!_closed&&service.IsCurrent(generation)&&
                NativeMethods.TrySetWindowMouseTransparent(new WindowInteropHelper(this).Handle,true);
            if(succeeded)NativeMethods.FlushComposition();
        }
        catch(Exception ex){new PrivacyLogger().Info("PointerPassThrough",ex.GetType().Name);}
        finally{service.Ready(generation,succeeded);}
    }

    private async void EndRightPassThroughVisual(long generation)
    {
        var service=_rightPassThrough;if(service is null)return;
        // Keep the hit-test hole until the injected up reaches the target.
        await Task.Delay(60);
        if(_closed||!service.IsCurrent(generation))return;
        if(_rightPassThroughVisual)RefreshDesktopFrameIncludingPinnedWindows();
        if(_closed)return;
        var restoreFailed=false;
        service.TryComplete(generation,()=>
        {
            if(!NativeMethods.TrySetWindowMouseTransparent(new WindowInteropHelper(this).Handle,false)){restoreFailed=true;return false;}
            Root.Opacity=1;_rightPassThroughVisual=false;return true;
        });
        if(restoreFailed){Close();return;}
        UpdateRightPassThroughPolicy();KeepOverlayBelowPinnedWindows();
    }

    private void StopRightPassThrough()
    {
        _rightPassThroughPolicyTimer?.Stop();_rightPassThroughPolicyTimer=null;
        _rightPassThrough?.Dispose();_rightPassThrough=null;
        _rightPassThroughVisual=false;Root.Opacity=1;
    }
}
