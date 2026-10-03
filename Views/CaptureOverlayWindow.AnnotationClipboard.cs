// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private readonly DispatcherTimer _annotationCopyTimer = new() { Interval = TimeSpan.FromMilliseconds(180) };
    private SelectionItem? _annotationCopyItem;
    private uint _annotationCopySequence;
    private bool _annotationCopyInitialized;

    [DllImport("user32.dll")]
    private static extern uint GetClipboardSequenceNumber();

    private void MarkDrawingChanged(SelectionItem item)
    {
        TrimDrawingHistory(item);
        ReleaseInactiveBackgroundHighlightSources(item);
        RefreshBackgroundHighlightSources(item);
        _drawingOperationChanged = true;
        UpdateDrawingNumberControls(item);
    }

    private void QueueAnnotatedImageCopy(SelectionItem item)
    {
        if (_closed || _drawingMode || item.IsImplicit || item.VideoPath is not null || !_selections.Contains(item)) return;
        if (!_annotationCopyInitialized)
        {
            _annotationCopyTimer.Tick += (_, _) => TryFlushAnnotatedImageCopy();
            _annotationCopyInitialized = true;
        }
        _annotationCopyItem = item;
        _annotationCopySequence = GetClipboardSequenceNumber();
        _annotationCopyTimer.Stop();
        _annotationCopyTimer.Start();
    }

    private void CancelAnnotatedImageCopy()
    {
        _annotationCopyTimer.Stop();
        _annotationCopyItem = null;
    }

    private void TryFlushAnnotatedImageCopy()
    {
        _annotationCopyTimer.Stop();
        var item = _annotationCopyItem;
        if (item is null) return;
        if (_closed || item.IsImplicit || item.VideoPath is not null || !_selections.Contains(item) ||
            _recordingMode || _recordingCountdownActive || _longCaptureMode || _applicationSnapshotActive ||
            GetClipboardSequenceNumber() != _annotationCopySequence)
        {
            _annotationCopyItem = null;
            return;
        }
        // A pause while the pointer is still down is not a completed stroke.
        // Keep the pending item; mouse-up/focus-loss or Done will flush it.
        if (item.Markup.IsMouseCaptureWithin ||
            _drawPreview is not null || _drawingMosaicPreview is not null || _drawingSeamlessErasePreview is not null || _drawingHealPreview is not null || _drawingHealRequest is not null || _drawingObjectMoving) return;
        _annotationCopyItem = null;
        try
        {
            if (ClipboardService.TrySetImage(RenderSelectionImage(item, true, true, true), out var error))
            {
                // Do not overwrite AI progress or speech status with a background copy.
                if (_request is null && _overlayRequest is null)
                    PromptStatus.Text = LocalizationService.T("已自动复制标注后的图片", "Annotated image copied automatically");
            }
            else PromptStatus.Text = LocalizationService.T($"自动复制失败：{error}，可点击工具条复制重试。", $"Automatic copy failed: {error}. Retry with Copy on the toolbar.");
        }
        catch (Exception ex)
        {
            new PrivacyLogger().Error("AnnotationAutoCopy", ex);
            PromptStatus.Text = LocalizationService.T("自动复制失败，标注已保留，可点击复制重试。", "Automatic copy failed. Annotations are preserved; use Copy to retry.");
        }
    }

    private void ResumeAnnotatedImageCopy()
    {
        if (_annotationCopyItem is not null && !_closed)
        {
            _annotationCopyTimer.Stop();
            _annotationCopyTimer.Start();
        }
    }
}
