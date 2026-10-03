// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.ComponentModel;
using System.IO;
using System.Windows;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Image = System.Windows.Controls.Image;

internal static partial class VideoTrimReplay
{
    private static async Task VerifyFlickerAsync(string directory, Evidence evidence)
    {
        var source = await evidence.Step("flicker-fixture", token => VideoTrimSyntheticMedia.CreateAsync(Path.Combine(directory, "source"), false, token), 60);
        var sourceHash = Hash(source);
        var view = new Image(); using var preview = new VideoPreviewSurface(view, Dispatcher.CurrentDispatcher);
        var originalRange = new VideoClipRange(TimeSpan.FromSeconds(1.25), TimeSpan.FromSeconds(4.75));
        await evidence.Step("initial-bounded-load", token => LoadPreviewAsync(preview, source, originalRange, token));
        await evidence.Step("establish-middle-frame", token => preview.SeekAsync(TimeSpan.FromSeconds(2.75), true, token), 5);
        AssertPreviewFrame(view, 2.75, evidence, "baseline blue frame");

        string phase = "idle";
        double? target = null;
        var retainedCode = ReadTimecode();
        var nullTransitions = 0;
        var observations = 0;
        var violations = new List<string>();
        var frames = new List<(string Phase, double Position, int Code)>();
        var opened = 0;
        void Observe(string cause)
        {
            if (phase == "idle") return;
            observations++;
            var code = ReadTimecode();
            if (code < 0) { nullTransitions++; violations.Add(phase + ": blank pixels observed by " + cause); return; }
            if (target is { } requested && code != retainedCode && code != (int)(requested * 4))
                violations.Add(phase + ": unexpected displayed timecode " + code + " during " + cause);
        }
        void Presented(TimeSpan position)
        {
            Observe("FramePresented");
            if (phase == "idle") return;
            var code = ReadTimecode(); frames.Add((phase, position.TotalSeconds, code));
            if (target is { } requested && (Math.Abs(position.TotalSeconds - requested) > .10 || code != (int)(requested * 4)))
                violations.Add(phase + ": intermediate frame event " + position.TotalSeconds + "/code" + code);
        }
        void Opened() { opened++; Observe("Opened"); }
        EventHandler changed = (_, _) => Observe("Image.Source changed");
        var descriptor = DependencyPropertyDescriptor.FromProperty(Image.SourceProperty, typeof(Image))
            ?? throw new InvalidOperationException("Image.Source descriptor unavailable.");
        descriptor.AddValueChanged(view, changed);
        preview.FramePresented += Presented; preview.Opened += Opened;
        var sampler = new DispatcherTimer(DispatcherPriority.Render) { Interval = TimeSpan.FromMilliseconds(1) };
        sampler.Tick += (_, _) => Observe("dispatcher sample"); sampler.Start();
        int ReadTimecode()
        {
            if (view.Source is not BitmapSource image) return -1;
            var pixel = new byte[4]; var code = 0;
            for (var bit = 0; bit < 5; bit++)
            {
                image.CopyPixels(new Int32Rect(18 + bit * 24, 160, 1, 1), pixel, 4, 0);
                if (pixel[0] > 160) code |= 1 << bit;
            }
            return code;
        }
        void Begin(string name, double requested)
        {
            retainedCode = ReadTimecode(); phase = name; target = requested;
            evidence.Stage(name + ":before", new { retainedCode, presented = preview.LastPresentedPosition.TotalSeconds });
        }
        async Task WaitOpenedAsync(Action load, CancellationToken token)
        {
            var ready = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            void Ready() => ready.TrySetResult(); void Failed(Exception error) => ready.TrySetException(error);
            preview.Opened += Ready; preview.Failed += Failed;
            try { load(); await ready.Task.WaitAsync(TimeSpan.FromSeconds(8), token); }
            finally { preview.Opened -= Ready; preview.Failed -= Failed; }
        }
        void Settled(string name, double requested)
        {
            Observe("settled");
            evidence.Check(violations.Count == 0, name + ": no blank or intermediate GOP/start pixels (" + string.Join("; ", violations.Take(3)) + ")");
            evidence.Check(Math.Abs(preview.LastPresentedPosition.TotalSeconds - requested) < .10, name + ": published source time matches requested frame");
            AssertPreviewFrame(view, requested, evidence, name + " final pixels");
            phase = "idle"; target = null;
        }
        try
        {
            foreach (var (name, range, requested) in new (string, VideoClipRange?, double)[]
            {
                ("bounded-to-full-at-middle", null, 2.75),
                ("full-to-bounded-at-middle", originalRange, 2.75),
                ("changed-start-at-later-frame", new VideoClipRange(TimeSpan.FromSeconds(1.75), TimeSpan.FromSeconds(4.75)), 3.25),
                ("cancel-restores-old-range-at-middle", originalRange, 2.75)
            })
            {
                Begin(name, requested); var previousPixels = view.Source; var previousPosition = preview.LastPresentedPosition;
                var priorOpened = opened;
                await evidence.Step(name, token => WaitOpenedAsync(() =>
                {
                    preview.Load(source, false, range, TimeSpan.FromSeconds(requested));
                    evidence.Check(ReferenceEquals(view.Source, previousPixels) && ReadTimecode() == retainedCode,
                        name + ": same-source reload retains previously displayed pixels synchronously");
                    evidence.Check(preview.LastPresentedPosition == previousPosition,
                        name + ": retained pixels keep their truthful previous source timestamp");
                }, token), 10);
                evidence.Check(opened == priorOpened + 1, name + ": announces readiness exactly once after target frame");
                Settled(name, requested);
            }
            // A new generation must not claim that retained pixels belong to
            // its range. Rapid replacement should publish only the last load.
            Begin("rapid-load-replacement", 4.25); var beforeRapid = opened;
            await evidence.Step("rapid-load-replacement", token => WaitOpenedAsync(() =>
            {
                preview.Load(source, false, null, TimeSpan.FromSeconds(.75));
                preview.Load(source, false, originalRange, TimeSpan.FromSeconds(4.25));
            }, token), 10);
            await Task.Delay(180);
            evidence.Check(opened == beforeRapid + 1, "rapid replacement ignores obsolete Opened callback");
            Settled("rapid replacement", 4.25);

            // Ordinary cross-GOP seeks must suppress decoder preroll too.
            await evidence.Step("expand-for-source-seeks", token => WaitOpenedAsync(() => preview.Load(source, false, null, TimeSpan.FromSeconds(4.25)), token), 10);
            foreach (var requested in new[] { .75, 4.25, 2.75 })
            {
                Begin("cross-gop-seek-" + requested, requested);
                await evidence.Step(phase, token => preview.SeekAsync(TimeSpan.FromSeconds(requested), true, token), 5);
                Settled(phase, requested);
            }
            evidence.Check(nullTransitions == 0 && observations > 0, "all observed reload/seek intervals preserve nonempty visible pixels");
            evidence.Check(frames.Count > 0 && frames.All(frame => frame.Code != 0), "no operation publishes a transient source-zero frame");
            phase = "idle";
            using (var cancelSeek = new CancellationTokenSource())
            {
                var pending = preview.SeekAsync(TimeSpan.FromSeconds(.75), true, cancelSeek.Token);
                evidence.Check(!pending.IsCompleted, "cancel/play race starts with a genuinely pending seek");
                cancelSeek.Cancel(); preview.Play();
                await ExpectCanceledAsync(pending, evidence, "canceled seek releases its waiter after Play takes ownership");
                var count = preview.PresentedFrameCount;
                await Task.Delay(450);
                evidence.Check(preview.IsPlaying && preview.PresentedFrameCount >= count + 2,
                    "canceled seek cannot restore a presentation hold after Play");
                var position = preview.LastPresentedPosition; var nextCount = preview.PresentedFrameCount;
                await Task.Delay(250);
                evidence.Check(preview.PresentedFrameCount > nextCount && preview.LastPresentedPosition > position,
                    "playback continues to advance after canceled seek finally completes");
                preview.Pause();
            }
            var playFrames = new List<(double Position, int Code)>();
            void PlayingFrame(TimeSpan position) => playFrames.Add((position.TotalSeconds, ReadTimecode()));
            preview.FramePresented += PlayingFrame;
            try
            {
                var pending = preview.SeekAsync(TimeSpan.FromSeconds(4.25), true);
                evidence.Check(!pending.IsCompleted, "pending seek/play race reaches native asynchronous work");
                preview.Play();
                // Play may invalidate the seek's presentation ownership, but
                // must never leave its waiter hung or reapply a paused hold.
                try { await pending.WaitAsync(TimeSpan.FromSeconds(5)); } catch (OperationCanceledException) { }
                await Task.Delay(350);
                evidence.Check(preview.IsPlaying && playFrames.Count >= 2, "Play during pending seek continues producing real frames");
                evidence.Check(playFrames.All(frame => frame.Position > 4 && frame.Code >= 16),
                    "Play during pending seek does not publish preroll source-start pixels");
                preview.Pause();
            }
            finally { preview.FramePresented -= PlayingFrame; }
            evidence.Check(Hash(source) == sourceHash, "reload and seek leave synthetic source unchanged");
            phase = "idle"; preview.CloseSource();
            evidence.Check(view.Source is null && preview.LastPresentedPosition == TimeSpan.Zero, "explicit CloseSource releases retained pixels and timestamp");
            evidence.Check(System.Windows.Application.Current.Windows.Count == 0, "flicker replay creates no window");
        }
        finally
        {
            sampler.Stop(); descriptor.RemoveValueChanged(view, changed);
            preview.FramePresented -= Presented; preview.Opened -= Opened;
            evidence.Stage("flicker-observations", new { observations, nullTransitions, violations,
                frames = frames.Select(frame => new { phase = frame.Phase, position = frame.Position, timecode = frame.Code }) });
        }
    }
}
