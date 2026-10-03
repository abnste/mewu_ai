// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Animation;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private void ShowRecordingCountdownStep(int value)
    {
        ClearRecordingCountdownAnimations();
        RecordingCountdownText.Text=value.ToString(CultureInfo.InvariantCulture);
        RecordingCountdownLabel.Text=L("即将录制","Recording soon");
        RecordingCountdownHint.Text=L("Esc 取消","Esc to cancel");
        RecordingCountdownArc1.Opacity=value>=3?1:.16;
        RecordingCountdownArc2.Opacity=value>=2?1:.16;
        RecordingCountdownArc3.Opacity=value>=1?1:.16;
        if(!SystemParameters.ClientAreaAnimation)return;

        // Only the numeral settles into place. The card and ring stay still,
        // and these short visual clocks never drive the recording deadline.
        RecordingCountdownText.BeginAnimation(OpacityProperty,new DoubleAnimation(.5,1,TimeSpan.FromMilliseconds(160)){FillBehavior=FillBehavior.Stop});
        var settle=new DoubleAnimation(.94,1,TimeSpan.FromMilliseconds(180))
        {EasingFunction=new CubicEase{EasingMode=EasingMode.EaseOut},FillBehavior=FillBehavior.Stop};
        RecordingCountdownScale.BeginAnimation(ScaleTransform.ScaleXProperty,settle);
        RecordingCountdownScale.BeginAnimation(ScaleTransform.ScaleYProperty,settle);
    }

    private void ClearRecordingCountdownAnimations()
    {
        RecordingCountdown.BeginAnimation(OpacityProperty,null);
        RecordingCountdownText.BeginAnimation(OpacityProperty,null);
        RecordingCountdownScale.BeginAnimation(ScaleTransform.ScaleXProperty,null);
        RecordingCountdownScale.BeginAnimation(ScaleTransform.ScaleYProperty,null);
        RecordingCountdown.Opacity=RecordingCountdownText.Opacity=1;
        RecordingCountdownScale.ScaleX=RecordingCountdownScale.ScaleY=1;
    }

    private void PositionRecordingCountdown(Rect selectionBounds)
    {
        var monitor=MonitorBounds(selectionBounds);
        var left=selectionBounds.Left+(selectionBounds.Width-RecordingCountdown.Width)/2;
        var top=selectionBounds.Top+(selectionBounds.Height-RecordingCountdown.Height)/2;
        Canvas.SetLeft(RecordingCountdown,Math.Clamp(left,monitor.Left+8,Math.Max(monitor.Left+8,monitor.Right-RecordingCountdown.Width-8)));
        Canvas.SetTop(RecordingCountdown,Math.Clamp(top,monitor.Top+8,Math.Max(monitor.Top+8,monitor.Bottom-RecordingCountdown.Height-8)));
    }
}
