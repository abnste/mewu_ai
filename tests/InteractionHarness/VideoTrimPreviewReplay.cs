// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application = System.Windows.Application;
using Image = System.Windows.Controls.Image;
using Size = System.Windows.Size;

internal static partial class VideoTrimReplay
{
    private static void RunDispatcherScenario(Func<Task> action)
    {
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown }; Exception? failure = null;
        app.Resources.MergedDictionaries.Add(new ResourceDictionary
        { Source = new Uri("/MewuAI;component/Themes/LightTheme.xaml", UriKind.Relative) });
        app.Dispatcher.BeginInvoke(new Action(async () =>
        {
            try { await action(); } catch (Exception error) { failure = error; }
            finally { app.Shutdown(); }
        }));
        app.Run(); if (failure is not null) System.Runtime.ExceptionServices.ExceptionDispatchInfo.Capture(failure).Throw();
    }

    private static async Task VerifyPreviewAsync(string directory, Evidence evidence)
    {
        // A silent fixture lets the real native decoder run without changing
        // either application audio policy or the user's system volume.
        var source = await evidence.Step("silent-fixture", token => VideoTrimSyntheticMedia.CreateAsync(Path.Combine(directory, "source"), false, token), 60);
        var sourceHash = Hash(source); var image = new Image();
        using var preview = new VideoPreviewSurface(image, Dispatcher.CurrentDispatcher);
        var frames = new List<double>(); var errors = new List<Exception>(); var openedCount = 0;
        preview.FramePresented += position => frames.Add(position.TotalSeconds);
        preview.Failed += error => errors.Add(error); preview.Opened += () => openedCount++;
        var range = new VideoClipRange(TimeSpan.FromSeconds(1.25), TimeSpan.FromSeconds(4.75));
        await evidence.Step("paused-bounded-load", token => LoadPreviewAsync(preview, source, range, token));
        evidence.Check(!preview.IsPlaying && Math.Abs(preview.Duration.TotalSeconds - 3.5) < .001, "paused native preview uses selected playback duration");
        evidence.Check(preview.PlaybackRange == range, "preview exposes selected source range");
        AssertPreviewFrame(image, 1.25, evidence, "paused range initial frame");
        evidence.Check(preview.LastPresentedPosition >= range.Start && preview.LastPresentedPosition < range.Start + TimeSpan.FromSeconds(.1),
            "initial presented frame is in absolute source time");
        foreach (var position in new[] { 2.1, 4.1, 1.6 })
        {
            var returned = await evidence.Step("absolute-seek-" + position, token => preview.SeekAsync(TimeSpan.FromSeconds(position), true, token));
            evidence.Check(Math.Abs(returned.TotalSeconds - position) < .1, $"seek {position:0.0}s reports source-absolute presented time");
            AssertPreviewFrame(image, position, evidence, $"seek {position:0.0}s actual pixels");
        }
        await evidence.Step("serialized-seeks", async token =>
        {
            var first = preview.SeekAsync(TimeSpan.FromSeconds(2.1), true, token);
            var second = preview.SeekAsync(TimeSpan.FromSeconds(4.1), true, token);
            var values = await Task.WhenAll(first, second);
            evidence.Check(Math.Abs(values[0].TotalSeconds - 2.1) < .1 && Math.Abs(values[1].TotalSeconds - 4.1) < .1,
                "simultaneous callers complete seeks in source order");
        });
        AssertPreviewFrame(image, 4.1, evidence, "serialized final frame");
        var lower = await evidence.Step("seek-before-start", token => preview.SeekAsync(TimeSpan.Zero, true, token));
        evidence.Check(lower >= range.Start && lower < range.Start + TimeSpan.FromSeconds(.1), "seek before selected start clamps to source start");
        AssertPreviewFrame(image, 1.25, evidence, "clamped start frame");
        var upper = await evidence.Step("seek-after-end", token => preview.SeekAsync(TimeSpan.FromSeconds(6), true, token));
        evidence.Check(upper <= range.End && upper > range.End - TimeSpan.FromSeconds(.1), "seek after selected end settles on final selected frame");
        AssertPreviewFrame(image, 4.7, evidence, "clamped final selected frame");

        // Replacing the source while an actual native seek is pending must
        // cancel its waiter, then only publish the new range's first frame.
        var obsolete = preview.SeekAsync(TimeSpan.FromSeconds(2.6), true);
        evidence.Check(!obsolete.IsCompleted, "replacement test has a genuinely pending native seek");
        var replacement = new VideoClipRange(TimeSpan.FromSeconds(4.25), TimeSpan.FromSeconds(5.25));
        var countBefore = openedCount;
        await evidence.Step("replace-pending-seek", async token =>
        {
            await LoadPreviewAsync(preview, source, replacement, token);
            await ExpectCanceledAsync(obsolete, evidence, "source replacement cancels obsolete seek");
        });
        evidence.Check(openedCount == countBefore + 1, "only replacement source announces Opened");
        AssertPreviewFrame(image, 4.25, evidence, "replacement first frame");
        frames.Clear(); preview.Play();
        await Task.Delay(1_350); preview.Pause();
        evidence.Check(frames.Count > 3 && frames.All(seconds => seconds >= 4.25 && seconds <= 5.25),
            "playing and looping native range never reports frames outside selected bounds");
        var pendingClose = preview.SeekAsync(TimeSpan.FromSeconds(4.4), true);
        evidence.Check(!pendingClose.IsCompleted, "close test has a genuinely pending seek");
        preview.CloseSource(); await ExpectCanceledAsync(pendingClose, evidence, "CloseSource cancels native seek waiter");
        await Dispatcher.CurrentDispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
        evidence.Check(image.Source is null && preview.Position == TimeSpan.Zero && !preview.IsPlaying, "CloseSource clears frame and playback state");
        evidence.Check(errors.Count == 0, "source replacement and close do not emit stale media failures");
        evidence.Check(Hash(source) == sourceHash, "preview and seek never modify original media");
        evidence.Check(Application.Current.Windows.Count == 0, "decoder regression never creates a visible or hidden window");
        VerifyTrimBar(evidence);
    }

    private static async Task LoadPreviewAsync(VideoPreviewSurface preview, string source, VideoClipRange range, CancellationToken token)
    {
        var ready = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        void Opened() => ready.TrySetResult(); void Failed(Exception error) => ready.TrySetException(error);
        preview.Opened += Opened; preview.Failed += Failed;
        try { preview.Load(source, false, range); await ready.Task.WaitAsync(TimeSpan.FromSeconds(25), token); }
        finally { preview.Opened -= Opened; preview.Failed -= Failed; }
    }

    private static async Task VerifyNearEndAsync(string directory, Evidence evidence, long offsetTicks = 1)
    {
        var source = await evidence.Step("near-end-fixture", token => VideoTrimSyntheticMedia.CreateAsync(Path.Combine(directory, "source"), false, token), 60);
        var image = new Image(); using var preview = new VideoPreviewSurface(image, Dispatcher.CurrentDispatcher);
        var range = new VideoClipRange(TimeSpan.FromSeconds(1.2573344), TimeSpan.FromSeconds(3.7833333));
        await evidence.Step("near-end-paused-load", token => LoadPreviewAsync(preview, source, range, token));
        AssertPreviewFrame(image, 1.2573344, evidence, "near-end fixture initial frame");
        var request = range.End - TimeSpan.FromTicks(offsetTicks);
        var native = (Windows.Media.Playback.MediaPlayer)typeof(VideoPreviewSurface).GetField("_player", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(preview)!;
        var trace = new System.Collections.Concurrent.ConcurrentQueue<object>();
        int FrameCode()
        {
            var bitmap = image.Source as BitmapSource; if (bitmap is null) return -1;
            var pixel = new byte[4]; var code = 0;
            for (var bit = 0; bit < 5; bit++) { bitmap.CopyPixels(new Int32Rect(18 + bit * 24, 160, 1, 1), pixel, 4, 0); if (pixel[0] > 160) code |= 1 << bit; }
            return code;
        }
        Windows.Foundation.TypedEventHandler<Windows.Media.Playback.MediaPlaybackSession, object> sought = (session, _) =>
            trace.Enqueue(new { kind = "native-seek-completed", utc = DateTime.UtcNow, relativePosition = session.Position.TotalSeconds, duration = session.NaturalDuration.TotalSeconds });
        void Presented(TimeSpan position) => trace.Enqueue(new { kind = "frame-presented", utc = DateTime.UtcNow, absolutePosition = position.TotalSeconds, code = FrameCode() });
        native.PlaybackSession.SeekCompleted += sought; preview.FramePresented += Presented;
        evidence.Stage("near-end-request", new { range.Start, range.End, request, offsetTicks, before = preview.LastPresentedPosition });
        try
        {
            var actual = await evidence.Step("near-end-seek", token => preview.SeekAsync(request, true, token), 5);
            evidence.Check(actual <= range.End && actual >= range.End - TimeSpan.FromSeconds(.08), "paused fractional-tail seek settles on final selected frame");
            AssertPreviewFrame(image, 3.77, evidence, "fractional near-end final frame");
            evidence.Check(!preview.IsPlaying, "near-end seek preserves paused playback");
        }
        finally
        {
            native.PlaybackSession.SeekCompleted -= sought; preview.FramePresented -= Presented;
            evidence.Stage("near-end-native-trace", new { request, offsetTicks, nativePosition = native.PlaybackSession.Position.TotalSeconds,
                nativeDuration = native.PlaybackSession.NaturalDuration.TotalSeconds, sourcePosition = preview.Position.TotalSeconds,
                presented = preview.LastPresentedPosition.TotalSeconds, presentedFrameCount = preview.PresentedFrameCount, code = FrameCode(), events = trace.ToArray() });
        }
    }

    private static async Task ExpectCanceledAsync(Task task, Evidence evidence, string name)
    {
        var canceled = false; try { await task.WaitAsync(TimeSpan.FromSeconds(3)); } catch (OperationCanceledException) { canceled = true; }
        evidence.Check(canceled, name);
    }

    private static async Task VerifyShortRangeAsync(string directory, Evidence evidence)
    {
        var source = await evidence.Step("short-range-fixture", token => VideoTrimSyntheticMedia.CreateAsync(Path.Combine(directory, "source"), false, token), 60);
        var image = new Image(); using var preview = new VideoPreviewSurface(image, Dispatcher.CurrentDispatcher);
        var range = new VideoClipRange(TimeSpan.FromSeconds(1.2573344), TimeSpan.FromSeconds(1.3573344));
        await evidence.Step("short-range-load", token => LoadPreviewAsync(preview, source, range, token));
        evidence.Check(Math.Abs(preview.Duration.TotalSeconds - .1) < .001, "short native range is 100 milliseconds");
        foreach (var (name, requested) in new[] { ("start", range.Start), ("end", range.End),
                     ("end-minus-tick", range.End - TimeSpan.FromTicks(1)), ("end-minus-1ms", range.End - TimeSpan.FromMilliseconds(1)),
                     ("end-minus-10ms", range.End - TimeSpan.FromMilliseconds(10)) })
        {
            evidence.Stage("short-range-request", new { name, requested, before = preview.LastPresentedPosition, preview.PresentedFrameCount });
            var actual = await evidence.Step("short-range-" + name, token => preview.SeekAsync(requested, true, token), 5);
            evidence.Stage("short-range-presented", new { name, requested, actual, preview.LastPresentedPosition, preview.PresentedFrameCount });
            evidence.Check(actual >= range.Start && actual <= range.End, "short range " + name + " stays within selected source bounds");
            evidence.Check(name == "start" ? actual < range.Start + TimeSpan.FromSeconds(.075) : actual > range.End - TimeSpan.FromSeconds(.067),
                "short range " + name + " displays the requested boundary frame");
            AssertPreviewFrame(image, 1.3, evidence, "short range " + name + " pixels");
            evidence.Check(!preview.IsPlaying, "short range " + name + " stays paused");
        }
    }

    private static void AssertPreviewFrame(Image image, double sourceSeconds, Evidence evidence, string name)
    {
        evidence.Check(image.Source is BitmapSource, name + " is materialized");
        var source = image.Source as BitmapSource ?? throw new InvalidOperationException("Missing decoded preview frame.");
        BitmapSource bitmap = source.Format == PixelFormats.Bgra32 || source.Format == PixelFormats.Pbgra32 ? source
            : new FormatConvertedBitmap(source, PixelFormats.Bgra32, null, 0);
        var pixels = new byte[bitmap.PixelWidth * bitmap.PixelHeight * 4]; bitmap.CopyPixels(pixels, bitmap.PixelWidth * 4, 0);
        var center = (90 * bitmap.PixelWidth + 160) * 4; var channel = sourceSeconds < 2 ? 2 : sourceSeconds < 4 ? 0 : 1;
        evidence.Check(pixels[center + channel] > 220 && pixels[center + (channel + 1) % 3] < 40, name + " color");
        var timecode = 0;
        for (var bit = 0; bit < 5; bit++) if (pixels[(160 * bitmap.PixelWidth + 18 + bit * 24) * 4] > 160) timecode |= 1 << bit;
        evidence.Check(timecode == (int)(sourceSeconds * 4), name + " encoded timecode");
        evidence.Stage("preview-pixels", new { name, sourceSeconds, timecode });
    }

    private static void VerifyTrimBar(Evidence evidence)
    {
        var bar = new VideoTrimBar(); bar.SetState(TimeSpan.FromSeconds(6), TimeSpan.FromSeconds(1), TimeSpan.FromSeconds(5), TimeSpan.FromSeconds(3), false, true);
        bar.Measure(new Size(320, 100)); bar.Arrange(new Rect(0, 0, 320, bar.DesiredSize.Height)); bar.UpdateLayout();
        var log = new List<string>(); var ranges = new List<(double Start, double End, bool Final)>(); var seeks = new List<(double Position, bool Final)>();
        bar.InteractionStarted += () => log.Add("start:" + bar.IsTrimInteraction);
        bar.RangeChanged += (start, end, final) => { ranges.Add((start.TotalSeconds, end.TotalSeconds, final)); log.Add("range:" + final); };
        bar.SeekRequested += (position, final) => seeks.Add((position.TotalSeconds, final));
        bar.InteractionCompleted += canceled => log.Add("complete:" + canceled + ":" + bar.IsTrimInteraction);
        const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic;
        var type = typeof(VideoTrimBar); var interaction = type.GetNestedType("Interaction", BindingFlags.NonPublic)!;
        object? Invoke(string method, params object?[] arguments) => type.GetMethod(method, flags)!.Invoke(bar, arguments);
        object Mode(string name) => Enum.Parse(interaction, name);
        evidence.Check((bool)Invoke("BeginInteraction", Mode("Start"), null)!, "trim gesture begins before any capture");
        Invoke("UpdateInteraction", TimeSpan.FromSeconds(2));
        bar.SetState(TimeSpan.FromSeconds(6), TimeSpan.Zero, TimeSpan.FromSeconds(6), TimeSpan.Zero, true, true);
        Invoke("CompleteInteraction", false);
        evidence.Check(ranges.Count == 2 && ranges[0] == (2d, 5d, false) && ranges[1] == (2d, 5d, true),
            "range drag emits preview then one final and ignores stale playback state");
        evidence.Check(log[0] == "start:True" && log[^1] == "complete:False:True" && !bar.IsInteracting,
            "gesture type remains available to final callback and clears afterwards");
        var priorFinals = ranges.Count(value => value.Final);
        Invoke("BeginInteraction", Mode("End"), null); Invoke("UpdateInteraction", TimeSpan.FromSeconds(3)); bar.CancelInteraction();
        evidence.Check(ranges.Count(value => value.Final) == priorFinals && log[^1] == "complete:True:True", "cancel emits no final range and reports cancellation");
        evidence.Check((TimeSpan)type.GetField("_end", flags)!.GetValue(bar)! == TimeSpan.FromSeconds(5), "cancel restores prior range endpoint");
        Invoke("BeginInteraction", Mode("Seek"), null); Invoke("UpdateInteraction", TimeSpan.FromSeconds(4)); Invoke("CompleteInteraction", false);
        evidence.Check(seeks.SequenceEqual(new[] { (4d, false), (4d, true) }), "seek emits preview and final source positions");
        bar.SetState(TimeSpan.FromSeconds(6), TimeSpan.FromSeconds(2), TimeSpan.FromSeconds(5), TimeSpan.FromSeconds(4), false, false);
        evidence.Check(!(bool)Invoke("BeginInteraction", Mode("Start"), null)! && (bool)Invoke("BeginInteraction", Mode("Seek"), null)!,
            "busy trim policy blocks endpoints but permits source seek");
        bar.CancelInteraction();
        evidence.Stage("trim-bar-scope", new { directGestureStateReplay = true, actualMouseInput = false });
    }
}
