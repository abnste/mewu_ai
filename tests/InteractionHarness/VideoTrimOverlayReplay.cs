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
        void Stage(string name) => File.AppendAllText(Path.Combine(directory, "overlay-stages.jsonl"),
            JsonSerializer.Serialize(new { utc = DateTime.UtcNow, stage = name }) + "\n", new UTF8Encoding(false));
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
            var history = Field("_overlayHistory")!;
            int UndoCount() => Property<int>(history, "UndoCount");
            int RedoCount() => Property<int>(history, "RedoCount");
            VideoClipRange CurrentRange() => (VideoClipRange)Invoke("GetVideoRange", item)!;
            long Revision() => (long)Read(item, "VideoClipRevision")!;
            PreparedVideoClip? Cached() => (PreparedVideoClip?)Read(item, "PreparedVideoClip");
            Task<PreparedVideoClip> Prepare() => (Task<PreparedVideoClip>)Invoke("PrepareSelectionVideoAsync", item, token)!;
            void BarState() => Call(bar, "SetState", duration, CurrentRange().Start, CurrentRange().End, CurrentRange().Start, false, true);
            void ChangeRange(VideoClipRange next) { BarState(); Call(bar, "ChangeRange", next.Start, next.End); }
            async Task SettlePreviewAsync()
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
                    if (clock.Elapsed > TimeSpan.FromSeconds(25)) throw new TimeoutException("Overlay preview did not settle after range operation.");
                    await Task.Delay(10, token);
                }
                await app.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
                Check(!preview.IsPlaying, "range operation leaves the isolated synthetic preview paused");
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
            Check((TimeSpan)Read(item, "VideoDuration")! == duration && preview.PlaybackRange == range && Math.Abs(preview.Duration.TotalSeconds - 3.5) < .001,
                "bounded preview retains full source metadata while native playback duration is clipped");
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
