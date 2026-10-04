// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Runtime.ExceptionServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application = System.Windows.Application;
using Point = System.Windows.Point;
using Size = System.Windows.Size;

/// <summary>
/// Actual overlay attachment, timeline and history methods on synthetic media.
/// The window is never shown, no HWND/input injection is created, and no
/// provider, production settings, clipboard or screen capture is used.
/// </summary>
internal static class VideoTrimOverlayReplay
{
    private const BindingFlags Flags = BindingFlags.Instance | BindingFlags.Static | BindingFlags.NonPublic | BindingFlags.Public | BindingFlags.DeclaredOnly;

    internal static async Task RunAsync(Application app, string sourcePath, string directory, List<string> checks, CancellationToken token)
    {
        void Check(bool condition, string name)
        {
            if (!condition) throw new InvalidOperationException(name);
            checks.Add(name);
        }
        void Stage(string name, object? data = null) => File.AppendAllText(Path.Combine(directory, "overlay-stages.jsonl"),
            JsonSerializer.Serialize(new { utc = DateTime.UtcNow, stage = name, data }) + "\n", new UTF8Encoding(false));
        var originalHash = Hash(sourcePath);
        var metadata = await VideoTrimSyntheticMedia.MetadataAsync(sourcePath, token);
        var duration = TimeSpan.FromSeconds(metadata.Seconds);
        var range = new VideoClipRange(TimeSpan.FromSeconds(1.25), TimeSpan.FromSeconds(4.75));
        var temp = new TempFileService(Path.Combine(directory, "prepared"));
        using var host = new AppHost(app, null, "MewuAI-IsolatedVideoTrim-" + Guid.NewGuid().ToString("N"));
        host.Settings.EnableVoiceInput = false;
        host.Settings.AutomaticallyStartListening = false;
        host.Settings.SaveConversationHistory = false;
        host.Settings.TeachingMode = false;
        var pixels = new byte[800 * 600 * 4];
        for (var i = 0; i < pixels.Length; i += 4) { pixels[i] = 231; pixels[i + 1] = 239; pixels[i + 2] = 247; pixels[i + 3] = 255; }
        var bitmap = BitmapSource.Create(800, 600, 96, 96, PixelFormats.Bgra32, null, pixels, 800 * 4);
        bitmap.Freeze(); Array.Clear(pixels);
        var frame = new CaptureFrame(0, 0, bitmap);
        var overlay = new CaptureOverlayWindow(host, null, frame) { ShowActivated = false, IsHitTestVisible = false };
        var attachmentLeases = new List<TempMediaLease>();
        var previewErrors = new List<Exception>();
        object item = null!;
        object? Invoke(string name, params object?[] args) => Call(overlay, name, args);
        object? Field(string name) => Read(overlay, name);
        void SetField(string name, object? value) => Write(overlay, name, value);
        try
        {
            Stage("construct-isolated-overlay");
            SetField("_videoTrimFiles", temp);
            var root = (Canvas)overlay.FindName("Root");
            root.Width = 800; root.Height = 600;
            root.Measure(new Size(800, 600)); root.Arrange(new Rect(0, 0, 800, 600)); root.UpdateLayout();
            Check(ReferenceEquals(Field("_frame"), frame) && Field("_rightPassThrough") is null,
                "overlay uses only supplied synthetic frame and installs no desktop input hook");
            Check(host.IsIsolatedReplay && Read(host, "_settingsService") is null && host.GetConversationChannels().Count == 0,
                "overlay host has no loaded settings or available provider channels");
            item = Invoke("CreateSelection", false)!;
            var selections = (IList)Field("_selections")!;
            selections.Add(item);
            Write(item, "Bounds", new Rect(80, 80, 320, 180));
            Write(item, "VideoPath", sourcePath);
            Write(item, "VideoDuration", duration);
            Write(item, "VideoLease", TempMediaRegistry.Shared.AcquireExistingFile(sourcePath));
            SetField("_activeIndex", 0);
            var referenceHandle = Property<string>(item, "ReferenceHandle");
            var notes = Property<List<AiAnnotation>>(item, "AnnotationNotes");
            var cardPositions = Property<Dictionary<AiAnnotation, Point>>(item, "AnnotationCardPositions");
            var sourceNote = new AiAnnotation(.1, .2, .15, .2, "synthetic source marker", StartTime: .25, EndTime: 5.75,
                Keyframes: [new(.25, .1, .2, .15, .2), new(5.75, .65, .2, .15, .2)], ReferenceHandle: referenceHandle,
                Kind: AiAnnotationKind.Rectangle, Style: new AiAnnotationStyle(Color: "#FF0000"));
            notes.Add(sourceNote); cardPositions[sourceNote] = new Point(.65, .7);
            Invoke("UpdateSelection", item);
            var preview = (VideoPreviewSurface)Invoke("EnsureVideoPreview", item)!;
            preview.Failed += previewErrors.Add;
            var bar = Field("_videoTrimBar")!;
            Stage("paused-controls-follow-prompt-layout");
            var promptHost = (FrameworkElement)overlay.FindName("PromptBarHost");
            var promptBar = (FrameworkElement)overlay.FindName("PromptBar");
            var toolbar = (FrameworkElement)overlay.FindName("Toolbar");
            var promptState = (promptBar.Width, promptBar.Height, promptBar.MinHeight, promptBar.MaxHeight,
                promptHost.Visibility, left: Canvas.GetLeft(promptHost), top: Canvas.GetTop(promptHost));
            var oldConversation = Field("_conversationAiAvailable");
            var oldHidden = Field("_promptBarHidden");
            var oldDetached = Field("_promptDetached");
            var oldDragScreen = Field("_promptDragScreen");
            var oldDragMonitor = Field("_promptDragMonitor");
            var oldDragOrigin = Field("_promptDragOrigin");
            var oldDragOffset = Field("_promptDragOffset");
            var oldDragging = Field("_promptDragging");
            var trimControl = (FrameworkElement)bar;
            void LayoutControls()
            {
                root.Measure(new Size(800, 600)); root.Arrange(new Rect(0, 0, 800, 600)); root.UpdateLayout();
            }
            Rect TrimBounds() => new(Canvas.GetLeft(trimControl), Canvas.GetTop(trimControl), trimControl.ActualWidth, trimControl.ActualHeight);
            try
            {
                // Exercise the real visibility/placement methods with no HWND,
                // media frames or main-toolbar refresh to hide a missing hook.
                SetField("_conversationAiAvailable", true); SetField("_promptBarHidden", true);
                SetField("_promptDetached", false); toolbar.Visibility = Visibility.Collapsed;
                promptHost.Visibility = Visibility.Visible;
                promptBar.Width = 574; promptBar.Height = 180; promptBar.MinHeight = 180;
                Canvas.SetLeft(promptHost, 80); Canvas.SetTop(promptHost, 220);
                LayoutControls(); Invoke("UpdateVideoTrimBar"); LayoutControls();
                var withoutPrompt = TrimBounds();
                Check(withoutPrompt.IntersectsWith((Rect)Invoke("GetPromptInteractionBounds")!),
                    "paused placement fixture puts the hidden prompt over the preferred video controls position");
                Invoke("SetPromptBarHidden", false, true); LayoutControls();
                Check(!TrimBounds().IntersectsWith((Rect)Invoke("GetPromptInteractionBounds")!) && TrimBounds().Top < withoutPrompt.Top,
                    "revealing the prompt immediately repositions paused controls without a video frame or visible main toolbar");
                Invoke("SetPromptBarHidden", true, true); LayoutControls();
                Check(Math.Abs(TrimBounds().Top - withoutPrompt.Top) < .01,
                    "hiding the prompt restores paused controls to their preferred position");
                Invoke("SetPromptBarHidden", false, true);
                SetField("_promptDetached", true); SetField("_promptDragScreen", new Rect(0, 0, 800, 600));
                Canvas.SetTop(promptHost, 400);
                Invoke("PositionPromptBar"); LayoutControls();
                Check(Math.Abs(TrimBounds().Top - withoutPrompt.Top) < .01,
                    "moving the visible prompt away repositions paused controls without playback");
                SetField("_promptDragging", true); SetField("_promptDragOrigin", new Point(Canvas.GetLeft(promptHost), Canvas.GetTop(promptHost)));
                SetField("_promptDragMonitor", new Rect(0, 0, 800, 600));
                Invoke("PromptDragDelta", promptHost, new DragDeltaEventArgs(0, -180)); LayoutControls();
                Check(!TrimBounds().IntersectsWith((Rect)Invoke("GetPromptInteractionBounds")!) && TrimBounds().Top < withoutPrompt.Top,
                    "actual prompt drag handler repositions paused controls as the prompt crosses them");
                Invoke("PromptDragDelta", promptHost, new DragDeltaEventArgs(0, 180)); LayoutControls();
                Check(Math.Abs(TrimBounds().Top - withoutPrompt.Top) < .01,
                    "dragging the prompt away restores paused controls without a decoded frame");
                SetField("_promptDragging", false);
                promptBar.MinHeight = 420;
                Invoke("PositionPromptBar"); LayoutControls();
                // Real SizeChanged invokes this same positioning pass after
                // arrangement in a shown overlay; this replay stays unshown.
                Invoke("PositionPromptBar"); LayoutControls();
                Check(promptBar.ActualHeight >= 420 && !TrimBounds().IntersectsWith((Rect)Invoke("GetPromptInteractionBounds")!) &&
                    TrimBounds().Top < withoutPrompt.Top,
                    "growing prompt content keeps the paused controls clear of its final arranged bounds");
                Check(preview.PresentedFrameCount == 0 && !preview.IsPlaying && toolbar.Visibility == Visibility.Collapsed,
                    "paused geometry refresh does not start decoding, playback or the main toolbar");
            }
            finally
            {
                SetField("_conversationAiAvailable", oldConversation); SetField("_promptBarHidden", oldHidden);
                SetField("_promptDetached", oldDetached); SetField("_promptDragScreen", oldDragScreen);
                SetField("_promptDragging", oldDragging); SetField("_promptDragMonitor", oldDragMonitor);
                SetField("_promptDragOrigin", oldDragOrigin); SetField("_promptDragOffset", oldDragOffset);
                ((FrameworkElement)overlay.FindName("PromptDockHint")).Visibility = Visibility.Collapsed;
                promptBar.Width = promptState.Width; promptBar.Height = promptState.Height;
                promptBar.MinHeight = promptState.MinHeight; promptBar.MaxHeight = promptState.MaxHeight;
                promptHost.Visibility = promptState.Visibility;
                Canvas.SetLeft(promptHost, promptState.left); Canvas.SetTop(promptHost, promptState.top);
                Invoke("UpdatePromptBarHiddenTransform", false); LayoutControls(); Invoke("UpdateVideoTrimBar");
            }
            var history = Field("_overlayHistory")!;
            int UndoCount() => Property<int>(history, "UndoCount");
            int RedoCount() => Property<int>(history, "RedoCount");
            VideoClipRange CurrentRange() => (VideoClipRange)Invoke("GetVideoRange", item)!;
            long Revision() => (long)Read(item, "VideoClipRevision")!;
            PreparedVideoClip? Cached() => (PreparedVideoClip?)Read(item, "PreparedVideoClip");
            Task<PreparedVideoClip> Prepare() => (Task<PreparedVideoClip>)Invoke("PrepareSelectionVideoAsync", item, token)!;
            void BarState() => Call(bar, "SetState", duration, CurrentRange().Start, CurrentRange().End, CurrentRange().Start, false, true);
            void ChangeRange(VideoClipRange next) { BarState(); Call(bar, "ChangeRange", next.Start, next.End); }
            async Task SettlePreviewAsync(int seconds = 25, bool requirePaused = true)
            {
                var clock = Stopwatch.StartNew();
                while (true)
                {
                    token.ThrowIfCancellationRequested();
                    if (previewErrors.Count > 0) throw new InvalidOperationException("Overlay preview failed.", previewErrors[0]);
                    var ready = (Task)Read(item, "VideoPreviewReady")!;
                    if (ready.IsFaulted) await ready;
                    var seek = (Task?)Read(item, "VideoSeekWork");
                    if (!(bool)Read(item, "VideoPreviewLoading")! && Read(item, "VideoSeekRequest") is null && (seek is null || seek.IsCompleted)) break;
                    if (clock.Elapsed > TimeSpan.FromSeconds(seconds)) throw new TimeoutException("Overlay preview did not settle after range operation.");
                    await Task.Delay(10, token);
                }
                await app.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
                if (requirePaused) Check(!preview.IsPlaying, "range operation leaves the isolated synthetic preview paused");
            }
            void FreezeSentTargets()
            {
                var targets = (IList)Field("_lastSentAnnotationTargets")!;
                targets.Clear(); targets.Add(Invoke("CaptureSentAnnotationTarget", item)!);
            }
            var capabilities = new AiProviderCapabilities(true, true, false, 10 * 1024 * 1024, 50 * 1024 * 1024,
                TimeSpan.FromMinutes(10), new HashSet<string> { "image/png", "image/jpeg", "video/mp4" });
            Task<List<AiAttachment>> Build() => (Task<List<AiAttachment>>)Invoke("BuildAttachmentsAsync", selections, capabilities, token, attachmentLeases)!;
            async Task<object> Map(AiAnnotation note)
            {
                var task = (Task)Invoke("MapAnnotationsAsync", new[] { note }, token)!;
                await task; return Property<object>(task, "Result");
            }
            IReadOnlyList<AiAnnotation> Mapped(object mapping) => (IReadOnlyList<AiAnnotation>)Property<IDictionary>(mapping, "BySelection")[item]!;

            Stage("load-source-paused");
            Invoke("LoadVideoRange", item, null, TimeSpan.Zero, false);
            await SettlePreviewAsync();
            Stage("full-range-cache");
            var full = await Prepare();
            Check(full.IsOriginal && SamePath(full.Path, sourcePath) && full.Range.IsFull(duration), "overlay full-range cache returns original media without transcoding");
            Check(ReferenceEquals(full, await Prepare()) && Directory.GetFiles(temp.DirectoryPath).Length == 0,
                "repeated full-range preparation reuses one cache handle and produces no file");

            Stage("commit-trim");
            ChangeRange(range);
            await SettlePreviewAsync();
            Check(CurrentRange() == range && UndoCount() == 1 && RedoCount() == 0,
                "actual trim control commit records one overlay history operation");
            Check(Cached() is null && Revision() == 1, "range commit invalidates prepared media and advances revision");
            Check((TimeSpan)Read(item, "VideoDuration")! == duration && preview.PlaybackRange is null && Math.Abs(preview.Duration.TotalSeconds - duration.TotalSeconds) < .001,
                "paused range editing keeps the complete decoder open while retaining selected export bounds");
            Check(notes.Count == 1 && ReferenceEquals(notes[0], sourceNote) && cardPositions[sourceNote] == new Point(.65, .7),
                "trim retains source-absolute annotations and manual card positions");

            Stage("prepare-and-build-attachments");
            var prepared = await Prepare();
            var cachedFileCount = Directory.GetFiles(temp.DirectoryPath).Length;
            FreezeSentTargets();
            var attachments = await Build();
            Check(attachments is [{ Type: AiAttachmentType.Video, MimeType: "video/mp4", Data: null }] &&
                attachments[0].FilePath == prepared.Path && attachments[0].Duration == range.Duration,
                "BuildAttachmentsAsync sends the real prepared clip with selected logical duration");
            Check(!SamePath(prepared.Path, sourcePath) && ReferenceEquals(prepared, Cached()) && ReferenceEquals(prepared, await Prepare()),
                "attachments and shared preparation use the same cached partial clip");
            var repeated = await Build();
            Check(repeated[0].FilePath == prepared.Path && Directory.GetFiles(temp.DirectoryPath).Length == cachedFileCount && attachmentLeases.Count == 2,
                "repeated attachment construction reuses cached bytes and acquires independent request leases");
            var clipMetadata = await VideoTrimSyntheticMedia.MetadataAsync(attachments[0].FilePath!, token);
            Check(Math.Abs(clipMetadata.Seconds - 3.5) < .08 && clipMetadata.Audio,
                "actual attachment file is the selected duration with source audio retained");
            var sample = await VideoTrimSyntheticMedia.SampleAsync(attachments[0].FilePath!, .9, token);
            Check(sample.Blue > 220 && sample.Red < 40 && sample.Green < 40 && sample.Timecode == 8,
                "attachment pixels at relative 0.9s contain blue source 2.15s machine timecode");
            var clones = (List<AiAttachment>)Invoke("CloneAttachmentsForFollowUp", attachments)!;
            Check(!ReferenceEquals(clones[0], attachments[0]) && clones[0].FilePath == prepared.Path && clones[0].Duration == range.Duration,
                "follow-up validation attachment preserves frozen prepared path and duration");

            Stage("seek-request-keeps-retained-range");
            var seekMode = bar.GetType().GetNestedType("Interaction", BindingFlags.NonPublic)!;
            var seekRequest = (Action<TimeSpan, bool>)Read(bar, "SeekRequested")!;
            var seekRevision = Revision(); var seekUndo = UndoCount();
            foreach (var requested in new[] { TimeSpan.Zero, duration })
            {
                BarState();
                Check((bool)Call(bar, "BeginInteraction", Enum.Parse(seekMode, "Seek"), null)!,
                    "overlay accepts a paused seek interaction inside the retained range");
                Call(bar, "UpdateInteraction", requested);
                seekRequest(requested, false);
                var expected = requested < range.Start ? range.Start : range.End;
                Check((TimeSpan)Read(item, "VideoLastRequestedPosition")! == expected && CurrentRange() == range,
                    "an out-of-range SeekRequested is clamped without expanding the selected clip");
                Call(bar, "CompleteInteraction", false);
                await SettlePreviewAsync(5);
                Check(CurrentRange() == range && Revision() == seekRevision && UndoCount() == seekUndo && ReferenceEquals(Cached(), prepared),
                    "seek-only interaction preserves selected bounds, history and prepared export cache");
                Check(Math.Abs((preview.LastPresentedPosition - expected).TotalSeconds) < .1,
                    "out-of-range seek presents the actual retained endpoint frame");
            }

            Stage("history-undo-redo");
            Invoke("UndoOverlayOperation");
            await SettlePreviewAsync();
            Check(CurrentRange().IsFull(duration) && UndoCount() == 0 && RedoCount() == 1 && Cached() is null,
                "overlay undo restores full source range and invalidates trimmed cache");
            Check(TempMediaRegistry.Shared.IsLeased(prepared.Path), "request leases preserve a sent clip after history invalidates its cache");
            Check(notes.SequenceEqual(new[] { sourceNote }) && cardPositions[sourceNote] == new Point(.65, .7),
                "undo preserves source annotations and their card positions");
            Invoke("RedoOverlayOperation");
            await SettlePreviewAsync();
            Check(CurrentRange() == range && UndoCount() == 1 && RedoCount() == 0 && notes.SequenceEqual(new[] { sourceNote }),
                "overlay redo restores selected range without rebasing or deleting source annotations");
            Check((TimeSpan)Read(item, "VideoDuration")! == duration, "undo and redo never replace original duration with clipped duration");

            var staleNote = new AiAnnotation(.2, .2, .2, .2, "returned synthetic marker", StartTime: .5, EndTime: 2.5,
                Keyframes: [new(.5, .2, .2, .2, .2), new(2.5, .4, .2, .2, .2)], ReferenceHandle: referenceHandle,
                Kind: AiAnnotationKind.Rectangle, Style: new AiAnnotationStyle(Color: "#00FF00"));
            // Undo/redo reconstructs the selection snapshot's sent targets. A
            // real new send likewise freezes them immediately before building.
            prepared = await Prepare(); FreezeSentTargets(); attachments = await Build();
            var frozenTarget = ((IList)Field("_lastSentAnnotationTargets")!)[0]!;
            Check(Property<VideoClipRange?>(frozenTarget, "VideoRange") == range && Property<long>(frozenTarget, "VideoRevision") == Revision(),
                "send target freezes the prepared source range and current selection revision");

            Stage("gesture-cancel-and-frozen-ai-map");
            var savedRevision = Revision(); var savedUndo = UndoCount(); var savedRedo = RedoCount();
            BarState();
            var interaction = bar.GetType().GetNestedType("Interaction", BindingFlags.NonPublic)!;
            Check((bool)Call(bar, "BeginInteraction", Enum.Parse(interaction, "Start"), null)!, "actual trim control begins a cancellable start-boundary gesture");
            Call(bar, "UpdateInteraction", TimeSpan.FromSeconds(.5));
            Check(CurrentRange().Start == TimeSpan.FromSeconds(.5) && Property<VideoClipRange?>(frozenTarget, "VideoRange") == range,
                "active trim preview does not mutate the already-sent range snapshot");
            var mapping = await Map(staleNote);
            var mapped = Mapped(mapping);
            Check(mapped.Count == 1 && mapped[0].StartTime == 1.75 && mapped[0].EndTime == 3.75 &&
                mapped[0].Keyframes![0].Time == 1.75 && mapped[0].Keyframes![^1].Time == 3.75,
                "MapAnnotationsAsync offsets relative AI timeline by frozen sent start during a later preview gesture");
            Check(mapped[0].Text == staleNote.Text && mapped[0].ReferenceHandle == referenceHandle && mapped[0].EffectiveStyle == staleNote.EffectiveStyle &&
                staleNote.StartTime == .5 && staleNote.Keyframes![0].Time == .5,
                "AI mapping preserves user-visible content, reference, style and immutable relative response");
            Call(bar, "CancelInteraction");
            await SettlePreviewAsync();
            Check(!Property<bool>(bar, "IsInteracting") && CurrentRange() == range && Revision() == savedRevision &&
                UndoCount() == savedUndo && RedoCount() == savedRedo && ReferenceEquals(Cached(), prepared),
                "cancel restores retained range without a revision, history entry or cache invalidation");
            Check(notes.SequenceEqual(new[] { sourceNote }), "canceled trim leaves all existing source timeline notes unchanged");
            Invoke("ApplyAnnotationMapping", mapping, AiAnnotationUpdateMode.Replace, false);
            Check(notes.Count == 1 && notes[0].StartTime == 1.75 && notes[0].EndTime == 3.75,
                "actual annotation application stores accepted clip-relative response in source time");

            Stage("reject-stale-ai-and-prepare");
            var changed = new VideoClipRange(TimeSpan.FromSeconds(1.5), TimeSpan.FromSeconds(4.5));
            ChangeRange(changed);
            await SettlePreviewAsync();
            var rejected = await Map(staleNote);
            Check(Mapped(rejected).Count == 0 && Property<int>(rejected, "DurationRejectedCount") == 1,
                "committed range revision rejects a late response from the previous sent clip");
            Check(notes.Count == 1 && notes[0].StartTime == 1.75,
                "rejecting a late response does not erase accepted source annotations");
            var pending = Prepare();
            Check(!pending.IsCompleted, "real media preparation is pending before a subsequent range edit");
            ChangeRange(VideoClipRange.Full(duration));
            var canceled = false;
            try { await pending; } catch (OperationCanceledException) { canceled = true; }
            Check(canceled && Cached() is null, "obsolete prepared result is discarded when range changes during asynchronous preparation");
            await SettlePreviewAsync();
            var final = await Prepare();
            Check(final.IsOriginal && SamePath(final.Path, sourcePath), "restoring full range resumes original media instead of stale trimmed output");

            Stage("rapid-alternating-endpoints");
            var originalBounds = (Rect)Read(item, "Bounds")!;
            var rapidUndo = UndoCount(); var rapidRevision = Revision();
            void Begin(string mode)
            {
                BarState();
                Check((bool)Call(bar, "BeginInteraction", Enum.Parse(interaction, mode), null)!, mode + " accepts another gesture without waiting for decoder settlement");
            }
            void End() => Call(bar, "CompleteInteraction", false);
            int CurrentTimecode()
            {
                var image = Property<System.Windows.Controls.Image>(item, "Video");
                var source = image.Source as BitmapSource ?? throw new InvalidOperationException("Preview has no real frame.");
                var pixel = new byte[4]; var code = 0;
                for (var bit = 0; bit < 5; bit++) { source.CopyPixels(new Int32Rect(18 + bit * 24, 160, 1, 1), pixel, 4, 0); if (pixel[0] > 160) code |= 1 << bit; }
                return code;
            }
            Invoke("LoadVideoRange", item, range, range.Start, false);
            Check((bool)Read(item, "VideoPreviewLoading")!, "rapid replay starts during a real pending bounded decoder load");
            long editingGeneration = -1;
            var expectedRange = VideoClipRange.Full(duration);
            for (var gesture = 0; gesture < 12; gesture++)
            {
                var start = gesture % 2 == 0; Begin(start ? "Start" : "End");
                if (gesture == 0) editingGeneration = (long)Read(preview, "_generation")!;
                var targetSeconds = (start ? 1.125 : 4.125) + (gesture / 2 % 2) * .5;
                var beforeSeconds = (start ? CurrentRange().Start : CurrentRange().End).TotalSeconds;
                for (var move = 1; move <= 24; move++) Call(bar, "UpdateInteraction", TimeSpan.FromSeconds(beforeSeconds + (targetSeconds - beforeSeconds) * move / 24));
                End();
                expectedRange = start ? new VideoClipRange(TimeSpan.FromSeconds(targetSeconds), expectedRange.End)
                    : new VideoClipRange(expectedRange.Start, TimeSpan.FromSeconds(targetSeconds));
                Check(CurrentRange() == expectedRange && UndoCount() == rapidUndo + gesture + 1 && Revision() == rapidRevision + gesture + 1,
                    "alternating gesture " + gesture + " commits its latest endpoint with exactly one history operation");
                Check((long)Read(preview, "_generation")! == editingGeneration,
                    "alternating gesture " + gesture + " reuses the same full-source decoder");
            }
            Begin("Start"); Call(bar, "UpdateInteraction", TimeSpan.FromSeconds(.625)); Call(bar, "CancelInteraction");
            await SettlePreviewAsync(5);
            Check(CurrentRange() == expectedRange && UndoCount() == rapidUndo + 12 && Revision() == rapidRevision + 12,
                "cancel after rapid alternating drags restores the last committed range without another history entry");
            Check(preview.PlaybackRange is null && Math.Abs((preview.LastPresentedPosition - expectedRange.End).TotalSeconds) < .1 && CurrentTimecode() == 18,
                "rapid endpoint queue settles on the latest actual source frame and remains editable");

            Stage("edit-supersedes-pending-play-load");
            Invoke("ToggleVideoPlayback", overlay, new RoutedEventArgs());
            Check((bool)Read(item, "VideoPreviewLoading")!, "play after paused trim starts an actual bounded load");
            Begin("Start"); Call(bar, "UpdateInteraction", TimeSpan.FromSeconds(2.125)); End();
            await SettlePreviewAsync(5);
            Check(!preview.IsPlaying && preview.PlaybackRange is null && CurrentRange().Start == TimeSpan.FromSeconds(2.125) && CurrentTimecode() == 8,
                "a new endpoint gesture cancels pending autoplay and keeps its own latest frame");
            Invoke("ToggleVideoPlayback", overlay, new RoutedEventArgs());
            await SettlePreviewAsync(5, requirePaused: false);
            Check(preview.IsPlaying && preview.PlaybackRange == CurrentRange() && Math.Abs((preview.Duration - CurrentRange().Duration).TotalSeconds) < .001,
                "explicit Play applies the selected native bounds after editing");
            Invoke("ToggleVideoPlayback", overlay, new RoutedEventArgs());
            Begin("End"); Call(bar, "UpdateInteraction", TimeSpan.FromSeconds(4.125)); End();
            await SettlePreviewAsync(5);
            Check(preview.PlaybackRange is null && !preview.IsPlaying && CurrentTimecode() == 16,
                "pause then edit returns to full-source seeking without stale bounded pixels");

            Stage("quick-seek-play-and-edit-ownership");
            ChangeRange(VideoClipRange.Full(duration)); await SettlePreviewAsync(5);
            Begin("Seek"); Call(bar, "UpdateInteraction", TimeSpan.FromSeconds(2.125)); End();
            Check(Read(item, "VideoSeekRequest") is not null, "quick Play follows a genuinely pending overlay seek");
            Invoke("ToggleVideoPlayback", overlay, new RoutedEventArgs());
            await SettlePreviewAsync(5, requirePaused: false);
            Check(preview.IsPlaying && preview.PlaybackRange is null && preview.LastPresentedPosition.TotalSeconds is >= 2.125 and < 2.4 && CurrentTimecode() == 8,
                "same-range quick Play retains the latest requested position before advancing");
            Invoke("ToggleVideoPlayback", overlay, new RoutedEventArgs());
            Begin("Seek"); Call(bar, "UpdateInteraction", TimeSpan.FromSeconds(4.125)); End();
            Invoke("ToggleVideoPlayback", overlay, new RoutedEventArgs());
            Begin("Start"); Call(bar, "UpdateInteraction", TimeSpan.FromSeconds(1.125)); End();
            await SettlePreviewAsync(5);
            var finalCode = CurrentTimecode();
            Stage("quick-play-edit-final-state", new { preview.IsPlaying, currentCode = finalCode, preview.LastPresentedPosition,
                lastRequested = (TimeSpan)Read(item, "VideoLastRequestedPosition")!, range = CurrentRange(),
                requestCleared = Read(item, "VideoSeekRequest") is null, seekWorkCompleted = (Read(item, "VideoSeekWork") as Task)?.IsCompleted,
                sourceGeneration = (long)Read(preview, "_generation")!, nativeRange = preview.PlaybackRange,
                pendingAutoplay = (bool)Read(item, "VideoPreviewPlayWhenReady")! });
            Check(!preview.IsPlaying && finalCode == 4 && Math.Abs(preview.LastPresentedPosition.TotalSeconds - 1.125) < .1,
                "editing during pending quick Play owns the final paused frame without late autoplay");
            Check((Rect)Read(item, "Bounds")! == originalBounds && !Property<bool>(bar, "IsInteracting") && Read(item, "VideoSeekRequest") is null,
                "rapid replay releases gesture and seek state without moving the video selection");
            Check(Hash(sourcePath) == originalHash, "overlay attachment, AI mapping, history and cancellation preserve original media SHA-256");
            Check(new WindowInteropHelper(overlay).Handle == IntPtr.Zero && !overlay.IsVisible && !overlay.IsLoaded && Field("_rightPassThrough") is null,
                "complete overlay regression uses no native window, desktop input or Loaded activation");
            Check(Read(host, "_settingsService") is null, "overlay regression never starts production settings service");
        }
        finally
        {
            Stage("overlay-close-and-release");
            overlay.Close();
            foreach (var lease in attachmentLeases) lease.Dispose();
            await app.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
        }
        Check((bool)Field("_closed")! && item is not null && Read(item, "PreparedVideoClip") is null && Read(item, "VideoPreview") is null,
            "normal overlay close releases prepared cache and native preview");
        Check(!TempMediaRegistry.Shared.IsLeased(sourcePath) && Directory.GetFiles(temp.DirectoryPath).All(path => !TempMediaRegistry.Shared.IsLeased(path)),
            "normal overlay and request teardown release all source and prepared media leases");
        Stage("overlay-complete");
    }

    private static object? Read(object instance, string name) => instance.GetType().GetField(name, Flags)!.GetValue(instance);
    private static void Write(object instance, string name, object? value) => instance.GetType().GetField(name, Flags)!.SetValue(instance, value);
    private static T Property<T>(object instance, string name) => (T)instance.GetType().GetProperty(name, Flags & ~BindingFlags.DeclaredOnly)!.GetValue(instance)!;
    private static object? Call(object instance, string name, params object?[] args)
    {
        try { return instance.GetType().GetMethod(name, Flags)!.Invoke(instance, args); }
        catch (TargetInvocationException error) when (error.InnerException is not null)
        {
            ExceptionDispatchInfo.Capture(error.InnerException).Throw(); throw;
        }
    }
    private static bool SamePath(string first, string second) => string.Equals(Path.GetFullPath(first), Path.GetFullPath(second), StringComparison.OrdinalIgnoreCase);
    private static string Hash(string path) { using var stream = File.OpenRead(path); return Convert.ToHexString(SHA256.HashData(stream)); }
}
