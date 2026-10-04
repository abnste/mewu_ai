// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Reflection;
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
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application = System.Windows.Application;
using Image = System.Windows.Controls.Image;
using Brushes = System.Windows.Media.Brushes;
using FlowDirection = System.Windows.FlowDirection;
using Forms = System.Windows.Forms;
using Point = System.Windows.Point;

/// <summary>Opt-in manual timeline session. Only fixture setup is scripted; all editing uses the real product UI.</summary>
internal static class VideoTrimDesktopSession
{
    private const BindingFlags Fields = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;
    private const string Title = "MewuAI Synthetic Video Trim TEST";

    internal static void Run()
    {
        var directory = ConfigureOutput();
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        LocalizationService.Initialize("zh-CN", null);
        app.Resources.MergedDictionaries.Add(new ResourceDictionary { Source = new Uri("/MewuAI;component/Themes/LightTheme.xaml", UriKind.Relative) });
        var product = typeof(CaptureOverlayWindow).Assembly.Location;
        var productHash = Hash(product);
        var harnessHash = Hash(typeof(VideoTrimDesktopSession).Assembly.Location);
        var began = DateTimeOffset.UtcNow;
        var started = Process.GetCurrentProcess().StartTime.ToUniversalTime();
        var commandPath = Path.Combine(directory, "command.txt");
        var samples = new List<object>();
        var failures = new List<object>();
        using var lifetime = new CancellationTokenSource();
        AppHost? host = null;
        CaptureOverlayWindow? overlay = null;
        object? item = null;
        VideoTrimBar? bar = null;
        VideoPreviewSurface? preview = null;
        DateTimeOffset? readyAt = null;
        var finished = false;
        var closed = false;
        var deadlineExpired = false;
        var frames = 0;
        var blankTransitions = 0;
        var closing = false;
        Image? monitoredVideo = null;
        DependencyPropertyDescriptor? sourceDescriptor = null;
        EventHandler? sourceChanged = null;
        var hwnd = 0L;
        string? source = null;
        string? sourceHash = null;
        var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(200) };

        void Write(string name, object value) => File.WriteAllText(Path.Combine(directory, name), JsonSerializer.Serialize(value,
            new JsonSerializerOptions { WriteIndented = true }), new UTF8Encoding(false));
        void Failure(string stage, Exception error) => failures.Add(new { stage, errorType = error.GetType().Name });
        object State(string reason)
        {
            var bounds = item is null ? Rect.Empty : (Rect)Get(item, "Bounds")!;
            var range = item is null ? null : (VideoClipRange?)Get(item, "VideoRange");
            return new
            {
                utc = DateTimeOffset.UtcNow, reason,
                duration = item is null ? 0 : ((TimeSpan)Get(item, "VideoDuration")!).TotalSeconds,
                rangeStart = range?.Start.TotalSeconds ?? 0,
                rangeEnd = range?.End.TotalSeconds ?? (item is null ? 0 : ((TimeSpan)Get(item, "VideoDuration")!).TotalSeconds),
                position = preview?.LastPresentedPosition.TotalSeconds ?? 0,
                requestedPosition = item is null ? 0 : ((TimeSpan)Get(item, "VideoLastRequestedPosition")!).TotalSeconds,
                playing = preview?.IsPlaying ?? false, interacting = bar?.IsInteracting ?? false,
                trimInteraction = bar?.IsTrimInteraction ?? false,
                bounds = bounds.IsEmpty ? null : new { x = bounds.X, y = bounds.Y, width = bounds.Width, height = bounds.Height },
                barVisible = bar?.IsVisible ?? false, barLeft = Finite(bar is null ? 0 : Canvas.GetLeft(bar)),
                barTop = Finite(bar is null ? 0 : Canvas.GetTop(bar)), frames, blankTransitions
            };
        }
        void Record(string reason)
        {
            if (item is not null && samples.Count < 1800)
            {
                var state = State(reason); samples.Add(state);
                File.AppendAllText(Path.Combine(directory, "timeline.jsonl"), JsonSerializer.Serialize(state) + "\n", new UTF8Encoding(false));
            }
        }
        void Status(string phase) => Write("session.json", new
        {
            phase, began, updated = DateTimeOffset.UtcNow, pid = Environment.ProcessId, processStartUtc = started,
            productPath = product, productSha256 = productHash, harnessSha256 = harnessHash,
            outputDirectory = directory, sourcePath = source, sourceSha256 = sourceHash,
            commandPath, windowTitle = Title, windowHandle = hwnd,
            ready = readyAt is not null && !closed, readyAt, automaticCloseSeconds = 240,
            deadlineExpired, closed, frames, blankTransitions, activeMediaLeases = TempMediaRegistry.Shared.ActiveLeaseCount,
            syntheticPixelsOnly = true, syntheticSilentVideoOnly = true,
            productionSettingsLoaded = false, productionHostStarted = false,
            clipboardAccessedByHelper = false, globalInputInjected = false, failures,
            current = item is null || closed ? null : State("status")
        });
        void Finish()
        {
            if (finished) return;
            finished = true; timer.Stop(); lifetime.Cancel();
            closing = true;
            if(monitoredVideo is not null&&sourceChanged is not null)sourceDescriptor?.RemoveValueChanged(monitoredVideo,sourceChanged);
            foreach (var window in app.Windows.Cast<Window>().ToArray())
                try { window.Close(); } catch (Exception error) { Failure("close-session-window", error); }
            try { host?.Dispose(); } catch (Exception error) { Failure("dispose-isolated-host", error); }
            closed = true;
            Write("timeline.json", samples);
            Status("closed");
            app.Shutdown(failures.Count == 0 && TempMediaRegistry.Shared.ActiveLeaseCount == 0 ? 0 : 1);
        }
        timer.Tick += (_, _) =>
        {
            try
            {
                Record("sample");
                if (readyAt is { } ready && DateTimeOffset.UtcNow - ready >= TimeSpan.FromSeconds(240))
                {
                    deadlineExpired = true; Record("automatic-close"); overlay?.Close(); return;
                }
                if (File.Exists(commandPath))
                {
                    if (new FileInfo(commandPath).Length > 64) throw new InvalidDataException("Command too long.");
                    var command = File.ReadAllText(commandPath, Encoding.UTF8).Trim(); File.Delete(commandPath);
                    if (command == "close") { Record("command-close"); overlay?.Close(); return; }
                    if (command == "snapshot") { Record("requested-snapshot"); Write("snapshot.json", State("requested-snapshot")); }
                    else throw new InvalidDataException("Unknown command.");
                }
                Status("ready");
            }
            catch (Exception error) { Failure("session-monitor", error); Finish(); }
        };
        app.Dispatcher.BeginInvoke(new Action(async () =>
        {
            try
            {
                Status("creating-synthetic-video");
                using var startup = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
                startup.CancelAfter(TimeSpan.FromSeconds(90));
                source = await VideoTrimSyntheticMedia.CreateAsync(Path.Combine(directory, "source"), false, startup.Token);
                sourceHash = Hash(source);
                var metadata = await VideoTrimSyntheticMedia.MetadataAsync(source, startup.Token);
                host = new AppHost(app, null, "MewuAI-IsolatedVideoTrim-" + Guid.NewGuid().ToString("N"));
                host.Settings.EnableVoiceInput = false; host.Settings.AutomaticallyStartListening = false;
                host.Settings.SaveConversationHistory = false; host.Settings.TeachingMode = true;
                var screen = Forms.SystemInformation.VirtualScreen;
                overlay = new CaptureOverlayWindow(host, null, new CaptureFrame(screen.Left, screen.Top, CreateFrame(screen.Width, screen.Height)))
                { Title = Title, ShowInTaskbar = true, ShowActivated = true, IsHitTestVisible = true };
                Set(overlay, "_videoTrimFiles", new TempFileService(Path.Combine(directory, "prepared")));
                overlay.SourceInitialized += (_, _) => hwnd = new WindowInteropHelper(overlay).Handle.ToInt64();
                overlay.Closing += (_, _) => { closing=true; Record("before-close"); Write("before-close.json", State("before-close")); };
                overlay.Closed += (_, _) => { closed = true; app.Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(Finish)); };
                overlay.Show();
                await app.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
                if (closed) return;
                var root = (Canvas)overlay.FindName("Root");
                item = Invoke(overlay, "CreateSelection", false)!;
                var width = Math.Max(100, Math.Min(720, Math.Min(root.ActualWidth - 80, (root.ActualHeight - 220) * 16 / 9)));
                Set(item, "Bounds", new Rect(Math.Max(20, (root.ActualWidth - width) / 2), Math.Max(70, (root.ActualHeight - width * 9 / 16) / 2 - 40), width, width * 9 / 16));
                Set(item, "VideoPath", source); Set(item, "VideoDuration", TimeSpan.FromSeconds(metadata.Seconds));
                Set(item, "VideoLease", TempMediaRegistry.Shared.AcquireExistingFile(source));
                ((IList)Get(overlay, "_selections")!).Add(item);
                Invoke(overlay, "Select", 0); Invoke(overlay, "RefreshSelectionNumbers");
                bar = (VideoTrimBar)Get(overlay, "_videoTrimBar")!;
                bar.InteractionStarted += () => Record("interaction-started");
                bar.InteractionCompleted += cancelled => Record(cancelled ? "interaction-cancelled" : "interaction-completed");
                bar.RangeChanged += (_, _, final) => { if (final) Record("range-final"); };
                bar.SeekRequested += (_, final) => { if (final) Record("seek-final"); };
                preview = (VideoPreviewSurface)Invoke(overlay, "EnsureVideoPreview", item)!;
                monitoredVideo=(Image)item.GetType().GetProperty("Video",Fields)!.GetValue(item)!;
                sourceDescriptor=DependencyPropertyDescriptor.FromProperty(Image.SourceProperty,typeof(Image));
                sourceChanged=(_,_)=>
                {
                    if(!closing&&frames>0&&monitoredVideo.Source is null)
                    {blankTransitions++;Record("unexpected-blank-preview");}
                };
                sourceDescriptor.AddValueChanged(monitoredVideo,sourceChanged);
                var presented = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
                preview.FramePresented += _ => { frames++; presented.TrySetResult(); };
                preview.Failed += error => { Failure("video-preview", error); presented.TrySetException(error); };
                Invoke(overlay, "StartVideoPreview", item);
                await presented.Task.WaitAsync(TimeSpan.FromSeconds(25), startup.Token);
                if (closed) return;
                overlay.Activate(); root.Focus();
                readyAt = DateTimeOffset.UtcNow; Record("ready"); Write("initial.json", State("ready"));
                Status("ready"); timer.Start();
            }
            catch (Exception error) { if (!finished) { Failure("session-start", error); Finish(); } }
        }));
        try { app.Run(); }
        finally { Finish(); }
    }

    private static object? Get(object target, string name) => target.GetType().GetField(name, Fields)!.GetValue(target);
    private static void Set(object target, string name, object? value) => target.GetType().GetField(name, Fields)!.SetValue(target, value);
    private static object? Invoke(object target, string name, params object?[] args) => target.GetType().GetMethod(name, Fields)!.Invoke(target, args);
    private static string Hash(string path) => Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(path)));
    private static double Finite(double value) => double.IsFinite(value) ? value : 0;

    private static string ConfigureOutput()
    {
        var cwd = ReplayOutputDirectory.PrepareWorkingDirectory("manual-video-trim");
        var output = Path.Combine(cwd, ".codex-build", "manual-video-trim-desktop", "session-" + Environment.ProcessId);
        Directory.CreateDirectory(output); File.WriteAllText(Path.Combine(output, ".mini-temp"), "Synthetic manual video timeline session\n", new UTF8Encoding(false));
        var temporary = Path.Combine(output, "temporary"); Directory.CreateDirectory(temporary);
        Environment.SetEnvironmentVariable("TEMP", temporary); Environment.SetEnvironmentVariable("TMP", temporary);
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(output, "logs"));
        return output;
    }

    private static BitmapSource CreateFrame(int width, int height)
    {
        var visual = new DrawingVisual();
        using (var drawing = visual.RenderOpen())
        {
            drawing.DrawRectangle(Brushes.WhiteSmoke, null, new Rect(0, 0, width, height));
            drawing.DrawText(new FormattedText("MEWU SYNTHETIC VIDEO TRIM TEST — generated pixels only", CultureInfo.InvariantCulture,
                FlowDirection.LeftToRight, new Typeface("Segoe UI"), 26, Brushes.DarkSlateGray, 1), new Point(45, 35));
        }
        var image = new RenderTargetBitmap(width, height, 96, 96, PixelFormats.Pbgra32); image.Render(visual); image.Freeze(); return image;
    }
}
