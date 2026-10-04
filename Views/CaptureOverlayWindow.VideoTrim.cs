// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private readonly VideoTrimBar _videoTrimBar = new() { Visibility = Visibility.Collapsed };
    private SelectionItem? _videoTrimGestureItem;
    private VideoClipRange _videoTrimGestureRange;
    private TimeSpan _videoTrimGesturePosition;
    private bool _videoTrimGesturePlaying;
    private OverlaySnapshot? _videoTrimBefore;
    private TempFileService? _videoTrimFiles = null;

    private static VideoClipRange GetVideoRange(SelectionItem item)
        => item.VideoRange ?? VideoClipRange.Full(item.VideoDuration);
    private static TimeSpan GetVideoOutputDuration(SelectionItem item)
        => item.VideoRange?.Duration ?? item.VideoDuration;

    private void InitializeVideoTrim()
    {
        Root.Children.Add(_videoTrimBar);
        Panel.SetZIndex(_videoTrimBar, 90);
        _videoTrimBar.InteractionStarted += BeginVideoTrimInteraction;
        _videoTrimBar.RangeChanged += (start, end, final) =>
        {
            if (_videoTrimGestureItem is not { } item) return;
            var previous = GetVideoRange(item);
            var next = VideoClipRange.Normalize(item.VideoDuration, start, end);
            if (next == previous) return;
            item.VideoRange = next;
            // Inspect the boundary being moved, including frames outside the
            // previous retained range. Rendering is deferred until export.
            QueueVideoSeek(item, previous.Start != start ? start : end);
        };
        _videoTrimBar.SeekRequested += (position, final) =>
        {
            if (_videoTrimGestureItem is { } item)
                QueueVideoSeek(item, VideoClipTimeline.ClampPosition(position, GetVideoRange(item)));
        };
        _videoTrimBar.InteractionCompleted += CompleteVideoTrimInteraction;
        _videoTrimBar.TogglePlaybackRequested += () => ToggleVideoPlayback(this, new RoutedEventArgs());
        Root.SizeChanged += (_, _) => UpdateVideoTrimBar();
        Closed += (_, _) =>
        {
            _videoTrimGestureItem = null;
            _videoTrimBefore = null;
            _videoTrimBar.CancelInteraction();
        };
    }

    private void UpdateVideoTrimBar()
    {
        if (_closed || _recordingMode || _recordingCountdownActive || _longCaptureMode || _drawingMode ||
            _selecting || _moving || Active is not { VideoPath: not null } item || item.VideoDuration <= TimeSpan.Zero)
        {
            _videoTrimBar.Visibility = Visibility.Collapsed;
            return;
        }
        var range = GetVideoRange(item);
        _videoTrimBar.Visibility = Visibility.Visible;
        _videoTrimBar.SetState(item.VideoDuration, range.Start, range.End,
            item.VideoSeekTarget ?? item.VideoPreview?.LastPresentedPosition ?? range.Start,
            item.VideoPreview?.IsPlaying == true, _request is null && _overlayRequest is null);
        _videoTrimBar.IsEnabled = _overlayRequest is null;
        var monitor = MonitorBounds(item.Bounds);
        var width = Math.Min(Math.Max(300, item.Bounds.Width), Math.Min(760, Math.Max(1, monitor.Width - 16)));
        _videoTrimBar.MinWidth = Math.Min(300, width);
        _videoTrimBar.Width = width;
        _videoTrimBar.Measure(new Size(width, double.PositiveInfinity));
        var height = _videoTrimBar.DesiredSize.Height;
        var left = Math.Clamp(item.Bounds.Left, monitor.Left + 8, Math.Max(monitor.Left + 8, monitor.Right - width - 8));
        var top = item.Bounds.Bottom + 6;
        if (top + height > monitor.Bottom - 8) top = Math.Max(monitor.Top + 8, item.Bounds.Bottom - height - 8);
        var prompt = GetPromptInteractionBounds();
        if (!_promptBarHidden && PromptBarHost.Visibility == Visibility.Visible && prompt.IntersectsWith(new Rect(left, top, width, height)))
            top = Math.Max(monitor.Top + 8, Math.Min(item.Bounds.Bottom - height - 8, prompt.Top - height - 6));
        Canvas.SetLeft(_videoTrimBar, left);
        Canvas.SetTop(_videoTrimBar, top);
    }

    private bool PointerOverVideoTrim(Point point)
    {
        if (_videoTrimBar.Visibility != Visibility.Visible) return false;
        if (_videoTrimBar.IsInteracting) return true;
        var bounds = GetElementBounds(_videoTrimBar);
        if (bounds.IsEmpty) return false;
        bounds.Inflate(5, 6);
        return bounds.Contains(point);
    }

    private void BeginVideoTrimInteraction()
    {
        if (Active is not { VideoPath: not null } item || _overlayRequest is not null ||
            (_videoTrimBar.IsTrimInteraction && _request is not null))
        {
            _videoTrimBar.CancelInteraction();
            return;
        }
        _videoTrimGestureItem = item;
        _videoTrimGestureRange = GetVideoRange(item);
        _videoTrimGesturePosition = item.VideoSeekRequest is not null || item.VideoPreviewLoading
            ? item.VideoLastRequestedPosition
            : item.VideoPreview?.LastPresentedPosition ?? _videoTrimGestureRange.Start;
        item.VideoLastRequestedPosition = _videoTrimGesturePosition;
        _videoTrimGesturePlaying = item.VideoPreview?.IsPlaying == true;
        _videoTrimBefore = _videoTrimBar.IsTrimInteraction ? CaptureOverlaySnapshot() : null;
        item.VideoPreviewPlayWhenReady = false;
        CancelVideoAnnotationPlayback(item);
        item.VideoPreview?.Pause();
        item.VideoPlaying = false;
        SetVideoPlaybackVisual(false);
        // Keep the full source open throughout paused editing. Reopening a
        // bounded decoder on every mouse-up made the next drag wait for it,
        // then immediately reopen the full source again. A pending open can
        // accept the latest target without rejecting the user's next gesture.
        if ((_videoTrimBar.IsTrimInteraction && item.VideoPreview?.PlaybackRange is not null) ||
            item.VideoPreviewReady.IsFaulted || item.VideoPreviewReady.IsCanceled)
            LoadVideoRange(item, null, _videoTrimGesturePosition, false);
    }

    private void CompleteVideoTrimInteraction(bool cancelled)
    {
        var item = _videoTrimGestureItem;
        var before = _videoTrimBefore;
        _videoTrimGestureItem = null;
        _videoTrimBefore = null;
        if (item is null || _closed || !_selections.Contains(item)) return;
        if (cancelled) item.VideoRange = _videoTrimGestureRange;
        var range = GetVideoRange(item);
        if (range != _videoTrimGestureRange)
        {
            InvalidateVideoClip(item);
            if (before is not null) RecordOverlayOperation(before, "裁切录屏");
            ApplyVideoAnswerActions(AnswerText.Markdown);
            UpdateReferenceChips();
        }
        var target = cancelled ? _videoTrimGesturePosition : item.VideoLastRequestedPosition;
        target = TimeSpan.FromTicks(Math.Clamp(target.Ticks, range.Start.Ticks, range.End.Ticks));
        // Range selection is independent of the paused preview's source.
        // Apply native clip bounds only when playback resumes; export and AI
        // attachments already use the committed VideoRange directly.
        if (cancelled && _videoTrimGesturePlaying) LoadVideoRange(item, range, target, true);
        else QueueVideoSeek(item, target);
        UpdateSelection(item);
        _ = Dispatcher.BeginInvoke(DispatcherPriority.Input, new Action(UpdateVideoTrimBar));
    }

    private static void InvalidateVideoClip(SelectionItem item)
    {
        item.VideoClipRevision++;
        item.PreparedVideoClip?.Dispose();
        item.PreparedVideoClip = null;
    }

    private void RestoreVideoRange(SelectionItem item, VideoClipRange? range)
    {
        if (item.VideoRange == range || item.VideoPath is null) return;
        item.VideoRange = range;
        InvalidateVideoClip(item);
        var retained = GetVideoRange(item);
        LoadVideoRange(item, retained, retained.Start, false);
    }

    private async void LoadVideoRange(SelectionItem item, VideoClipRange? range, TimeSpan position, bool play)
    {
        CancelVideoSeek(item);
        item.VideoLastRequestedPosition = position;
        item.VideoPreviewLoad?.Cancel();
        var load = new CancellationTokenSource();
        item.VideoPreviewLoad = load;
        item.VideoPreviewLoading = true;
        item.VideoPreviewPlayWhenReady = play;
        var ready = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        item.VideoPreviewReady = ready.Task;
        var preview = EnsureVideoPreview(item);
        void Opened() => ready.TrySetResult();
        void Failed(Exception error) => ready.TrySetException(error);
        preview.Opened += Opened;
        preview.Failed += Failed;
        try
        {
            preview.Load(item.VideoPath!, autoplay: false, playbackRange: range, initialPosition: position);
            await ready.Task.WaitAsync(TimeSpan.FromSeconds(20), load.Token);
            if (!ReferenceEquals(item.VideoPreviewLoad, load) || _closed) return;
            if (item.VideoSeekWork is { } seek) await seek;
            if (!ReferenceEquals(item.VideoPreviewLoad, load) || _closed) return;
            if (item.VideoPreviewPlayWhenReady) preview.Play();
            item.VideoPlaying = preview.IsPlaying;
            if (ReferenceEquals(item, Active)) SetVideoPlaybackVisual(item.VideoPlaying);
        }
        catch (OperationCanceledException) { ready.TrySetCanceled(); }
        catch (Exception error)
        {
            ready.TrySetException(error);
            if (!_closed && ReferenceEquals(item.VideoPreviewLoad, load))
            {
                new PrivacyLogger().Error("VideoTrimPreview", error);
                PromptStatus.Text = L("视频定位失败，可重试或保存视频。", "Video seeking failed. Retry or save the video.");
            }
        }
        finally
        {
            preview.Opened -= Opened;
            preview.Failed -= Failed;
            if (ReferenceEquals(item.VideoPreviewLoad, load))
            {
                item.VideoPreviewLoad = null;
                item.VideoPreviewLoading = false;
                item.VideoPreviewPlayWhenReady = false;
                if (!_closed) UpdateVideoTrimBar();
            }
            load.Dispose();
        }
    }

    private void QueueVideoSeek(SelectionItem item, TimeSpan position)
    {
        item.VideoSeekTarget = position;
        item.VideoLastRequestedPosition = position;
        if (item.VideoSeekRequest is not null) return;
        var request = new CancellationTokenSource();
        item.VideoSeekRequest = request;
        item.VideoSeekWork = SeekVideoQueueAsync(item, request);
    }

    private async Task SeekVideoQueueAsync(SelectionItem item, CancellationTokenSource request)
    {
        try
        {
            while (ReferenceEquals(item.VideoSeekRequest, request) && item.VideoSeekTarget is { } target)
            {
                await item.VideoPreviewReady.WaitAsync(request.Token);
                request.Token.ThrowIfCancellationRequested();
                // Coalesce pointer moves received while the decoder was busy.
                target = item.VideoSeekTarget ?? target;
                item.VideoSeekTarget = null;
                await EnsureVideoPreview(item).SeekAsync(target, true, request.Token);
            }
        }
        catch (OperationCanceledException) { }
        catch (Exception error)
        {
            if (!_closed && ReferenceEquals(item.VideoSeekRequest, request))
            {
                item.VideoPreviewPlayWhenReady = false;
                new PrivacyLogger().Error("VideoTrimSeek", error);
                PromptStatus.Text = L("视频定位失败，请重试。", "Video seeking failed. Please retry.");
            }
        }
        finally
        {
            if (ReferenceEquals(item.VideoSeekRequest, request))
            {
                item.VideoSeekRequest = null;
                item.VideoSeekTarget = null;
                if (!_closed) UpdateVideoTrimBar();
            }
            request.Dispose();
        }
    }

    private static void CancelVideoSeek(SelectionItem item)
    {
        var request = item.VideoSeekRequest;
        item.VideoSeekRequest = null;
        item.VideoSeekTarget = null;
        request?.Cancel();
    }

    private async Task PlayVideoSelectionAsync(SelectionItem item)
    {
        var preview = EnsureVideoPreview(item);
        var range = GetVideoRange(item);
        var pendingSeek = item.VideoSeekRequest is not null;
        var target = pendingSeek
            ? item.VideoLastRequestedPosition : preview.LastPresentedPosition;
        target = TimeSpan.FromTicks(Math.Clamp(target.Ticks, range.Start.Ticks, range.End.Ticks));
        var sameNativeRange = preview.PlaybackRange == range ||
            (preview.PlaybackRange is null && range.IsFull(item.VideoDuration));
        if (sameNativeRange)
        {
            if (pendingSeek)
            {
                // Let the existing coalescing queue finish the latest target.
                // Canceling and immediately seeking on the same native player
                // would overlap untagged SeekCompleted/frame callbacks.
                item.VideoPreviewPlayWhenReady = true;
                var seek = item.VideoSeekWork!;
                await seek;
                if (_closed || !_selections.Contains(item) || !item.VideoPreviewPlayWhenReady ||
                    !ReferenceEquals(item.VideoSeekWork, seek) || !ReferenceEquals(item.VideoPreview, preview)) return;
                item.VideoPreviewPlayWhenReady = false;
            }
            preview.Play();
            item.VideoPlaying = true;
            SetVideoPlaybackVisual(true);
        }
        else LoadVideoRange(item, range, target, true);
    }

    private async Task<PreparedVideoClip> PrepareSelectionVideoAsync(SelectionItem item, CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        var source = item.VideoPath ?? throw new InvalidOperationException("录屏文件已不可用");
        var range = GetVideoRange(item);
        var revision = item.VideoClipRevision;
        var cached = item.PreparedVideoClip;
        var fullSource = item.VideoRange is null || range.IsFull(item.VideoDuration);
        if (cached is not null && (cached.Range == range || fullSource && cached.IsOriginal) && File.Exists(cached.Path)) return cached;
        // A just-finished recording initially carries wall-clock duration.
        // Null means the actual complete media, never an accidental tail cut
        // caused by that provisional duration differing by an encoded frame.
        var prepared = await VideoClipPreparationService.PrepareAsync(source, fullSource ? null : range, token, _videoTrimFiles);
        if (_closed || !_selections.Contains(item) || item.VideoClipRevision != revision || item.VideoPath != source || GetVideoRange(item) != range)
        {
            prepared.Dispose();
            throw new OperationCanceledException("视频范围已改变，请重试", token);
        }
        item.PreparedVideoClip?.Dispose();
        item.PreparedVideoClip = prepared;
        if (prepared.IsOriginal)
        {
            item.VideoDuration = prepared.Duration;
            if (item.VideoRange is not null) item.VideoRange = prepared.Range;
        }
        return prepared;
    }

    private SentAnnotationTarget CaptureSentAnnotationTarget(SelectionItem item)
        => new(item.ReferenceHandle, item.VideoPath is null ? AiAttachmentType.Image : AiAttachmentType.Video, item,
            item.VideoPath is null || item.VideoDuration <= TimeSpan.Zero ? null : GetVideoRange(item), item.VideoClipRevision);

    private bool HandleVideoTrimKey(KeyEventArgs e)
    {
        if (_videoTrimBar.IsInteracting)
        {
            if (e.Key == Key.Escape) _videoTrimBar.CancelInteraction();
            e.Handled = true;
            return true;
        }
        if (!_videoTrimBar.IsKeyboardFocusWithin) return false;
        // Let native Button/Thumb key handlers own timeline navigation; the
        // overlay's arrows must not move the selection behind a focused handle.
        if (e.Key is Key.Left or Key.Right or Key.Up or Key.Down or Key.Home or Key.End or Key.Space or Key.Enter or Key.Tab) return true;
        return false;
    }
}
