// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private CancellationTokenSource? _thinkingGlowRequest;
    private CancellationTokenRegistration _thinkingGlowCancellation;
    private void StartThinkingGlow(CancellationTokenSource request)
    {
        StopThinkingGlow();
        if(_closed||request.IsCancellationRequested||!_host.Settings.ThinkingGlowEnabled)return;
        _thinkingGlowRequest=request;
        PositionThinkingGlow();
        BottomThinkingGlow.Start(_host.Settings.ThinkingGlowColor);
        _thinkingGlowCancellation=request.Token.Register(()=>
        {
            if(!Dispatcher.HasShutdownStarted)Dispatcher.BeginInvoke(new Action(()=>StopThinkingGlow(request)));
        });
    }
    private void StopThinkingGlow(CancellationTokenSource? request=null)
    {
        if(request is not null&&!ReferenceEquals(_thinkingGlowRequest,request))return;
        _thinkingGlowRequest=null;_thinkingGlowCancellation.Dispose();_thinkingGlowCancellation=default;
        BottomThinkingGlow.Stop();
    }
    private void PositionThinkingGlow()
    {
        var monitor=PromptMonitorBounds();if(monitor.IsEmpty)return;
        BottomThinkingGlow.Width=monitor.Width;
        BottomThinkingGlow.Height=Math.Min(220,monitor.Height*.24);
        Canvas.SetLeft(BottomThinkingGlow,monitor.Left);
        Canvas.SetTop(BottomThinkingGlow,monitor.Bottom-BottomThinkingGlow.Height);
    }
}
