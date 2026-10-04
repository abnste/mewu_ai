// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application = System.Windows.Application;
using Brushes = System.Windows.Media.Brushes;
using Color = System.Windows.Media.Color;
using Forms = System.Windows.Forms;
using FlowDirection = System.Windows.FlowDirection;
using Pen = System.Windows.Media.Pen;
using Point = System.Windows.Point;

/// <summary>
/// An opt-in, genuinely interactive product overlay over a supplied synthetic
/// frame. No recorded gestures, constructed annotations, input injection or
/// production AppHost.Start(). The operator uses the normal product UI.
/// </summary>
internal static partial class ManualDrawingDesktopSession
{
    private static readonly JsonSerializerOptions JsonOptions = new() { WriteIndented = true };
    private static string OutputDirectory = string.Empty;

    internal static void ConfigureOutput()
    {
        var cwd = ReplayOutputDirectory.PrepareWorkingDirectory("manual-drawing");
        OutputDirectory = Path.Combine(cwd, ".codex-build", "manual-drawing-desktop", "session-" + Environment.ProcessId);
        Directory.CreateDirectory(OutputDirectory);
        File.WriteAllText(Path.Combine(OutputDirectory, ".mini-temp"), "Synthetic manual drawing session\n", new UTF8Encoding(false));
        var processTemp = Path.Combine(OutputDirectory, "temporary");
        Directory.CreateDirectory(processTemp);
        Environment.SetEnvironmentVariable("TEMP", processTemp);
        Environment.SetEnvironmentVariable("TMP", processTemp);
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(OutputDirectory, "logs"));
    }

    internal static void Run(Application app)
    {
        if (string.IsNullOrEmpty(OutputDirectory)) throw new InvalidOperationException("Configure the output directory first.");
        app.ShutdownMode = ShutdownMode.OnExplicitShutdown;
        var assemblyPath = typeof(CaptureOverlayWindow).Assembly.Location;
        var assemblyHash = Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(assemblyPath))).ToLowerInvariant();
        var began = DateTimeOffset.UtcNow;
        var commandPath = Path.Combine(OutputDirectory, "command.txt");
        var failures = new List<object>();
        ClipboardSnapshot? clipboard = null;
        AppHost? host = null;
        CaptureOverlayWindow? overlay = null;
        ManualDrawingInputObserver? inputObserver = null;
        var overlayOpened = false;
        var overlayClosed = false;
        var clipboardRestored = false;
        var finished = false;
        var teardownComplete = false;
        var ready = false;
        var appLoopStarted = false;
        var handle = 0L;
        var copyNumber = 0;
        var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(200) };

        // Error types/stages are enough for a bounded failure report. Never put
        // exception messages, clipboard format names or clipboard values in it.
        void Failure(string stage, Exception error) => failures.Add(new { stage, errorType = error.GetType().Name });
        void WriteStatus(string phase)
        {
            WriteJson("session.json", new
            {
                phase, began, updated = DateTimeOffset.UtcNow, pid = Environment.ProcessId,
                assemblyPath, assemblySha256 = assemblyHash, outputDirectory = OutputDirectory,
                commandPath, windowHandle = handle, windowTitle = "MewuAI Synthetic Manual Drawing",
                ready, overlayOpened, overlayClosed, clipboardBackupSupported = clipboard is not null,
                clipboardFormatCount = clipboard?.FormatCount, clipboardRestored, clipboardStoredOnDisk = false,
                unreadableKnownBitmapDerivativeCount = clipboard?.UnreadableKnownBitmapDerivativeCount ?? 0,
                reconstructedImageRepresentations = clipboard?.ReconstructedImageRepresentations ?? [],
                clipboardRestoreVerification = clipboardRestored ? "materialized-values-and-image-pixels" : null,
                syntheticFrameOnly = true, frameWidth = 2560, frameHeight = 1440,
                productionSettingsLoaded = false, productionHostStarted = false,
                globalInputInjected = false, scriptedDrawingActions = false,
                passiveInputDiagnostics = true, inputDiagnosticFailure = inputObserver?.FailureType,
                teachingModeInMemory = true, copyNumber, failures,
                refreshBoundary = "The product replayFrame supplies synthetic pixels. Real Pin/activation handlers still invoke the product refresh and ReanchorRegionMarksAsync."
            });
        }
        void Finish()
        {
            if (finished) return;
            timer.Stop();
            if (!teardownComplete)
            {
                inputObserver?.Dispose();
                foreach (var window in app.Windows.Cast<Window>().Where(window => !ReferenceEquals(window, overlay)).ToArray())
                {
                    try { window.Close(); }
                    catch (Exception error) { Failure("close-session-owned-window", error); }
                }
                try { host?.Dispose(); }
                catch (Exception error) { Failure("dispose-isolated-host", error); }
                teardownComplete = true;
            }
            if (clipboard is not null)
            {
                try { clipboard.Restore(); clipboardRestored = true; }
                catch (Exception error)
                {
                    Failure("restore-clipboard", error);
                    WriteStatus("clipboard-restore-required");
                    // Keep the only complete snapshot alive in this process.
                    // The bounded command reader can retry without a UI window.
                    timer.Start();
                    return;
                }
                clipboard.Dispose();
            }
            finished = true;
            try { WriteStatus("closed"); }
            finally { app.Shutdown(failures.Count == 0 && overlayClosed && clipboardRestored ? 0 : 1); }
        }
        void SaveClipboard()
        {
            try
            {
                // Only an image owned by this test process after startup is
                // eligible. A stale/original/other-application clipboard is not
                // exported merely because a command file exists.
                var owner = GetClipboardOwner();
                _ = GetWindowThreadProcessId(owner, out var ownerPid);
                if (teardownComplete || ownerPid != Environment.ProcessId || clipboard is null ||
                    GetClipboardSequenceNumber() == clipboard.Sequence || !Forms.Clipboard.ContainsImage())
                {
                    WriteJson("copy-status.json", new { ok = false, reason = "no-new-session-owned-image" });
                    return;
                }
                using var image = Forms.Clipboard.GetImage();
                if (image is null)
                {
                    WriteJson("copy-status.json", new { ok = false, reason = "no-image" });
                    return;
                }
                var path = Path.Combine(OutputDirectory, "copied.png");
                image.Save(path, System.Drawing.Imaging.ImageFormat.Png);
                copyNumber++;
                WriteJson("copy-status.json", new { ok = true, width = image.Width, height = image.Height, copyNumber, pid = Environment.ProcessId });
                WriteStatus("ready");
            }
            catch (Exception error)
            {
                Failure("save-synthetic-clipboard-image", error);
                WriteJson("copy-status.json", new { ok = false, reason = "clipboard-read-failed", errorType = error.GetType().Name });
            }
        }
        timer.Tick += (_, _) =>
        {
            try
            {
                inputObserver?.UpdateState();
                if (!File.Exists(commandPath)) return;
                var info = new FileInfo(commandPath);
                if (info.Length > 64) throw new InvalidOperationException("Invalid command length.");
                var command = File.ReadAllText(commandPath, Encoding.UTF8).Trim();
                File.Delete(commandPath);
                switch (command)
                {
                    case "save-clipboard": SaveClipboard(); break;
                    case "close": if (teardownComplete || overlayClosed || overlay is null) Finish(); else overlay.Close(); break;
                    case "retry-clipboard-restore": if (teardownComplete || overlayClosed || !overlayOpened) Finish(); break;
                    default: WriteJson("command-status.json", new { ok = false, reason = "unknown-command" }); break;
                }
            }
            catch (Exception error)
            {
                Failure("command-read", error);
                timer.Stop(); // No unbounded repeating error loop.
                WriteJson("command-status.json", new { ok = false, reason = "command-reader-stopped", errorType = error.GetType().Name });
            }
        };
        app.DispatcherUnhandledException += (_, args) =>
        {
            Failure("dispatcher", args.Exception);
            args.Handled = true;
            try { overlay?.Close(); }
            finally { Finish(); }
        };
        try
        {
            // Materialize every advertised format before permitting any test
            // input. An unsupported format aborts without changing clipboard.
            clipboard = ClipboardSnapshot.Capture();
            host = new AppHost(app, null, "MewuAI-IsolatedManual-" + Guid.NewGuid().ToString("N"));
            host.Settings.EnableVoiceInput = false;
            host.Settings.AutomaticallyStartListening = false;
            host.Settings.SaveConversationHistory = false;
            host.Settings.TeachingMode = true;
            var area = Forms.SystemInformation.VirtualScreen;
            var frame = new CaptureFrame(area.Left, area.Top, CreateSyntheticFrame());
            overlay = new CaptureOverlayWindow(host, null, frame)
            {
                Title = "MewuAI Synthetic Manual Drawing", ShowInTaskbar = true,
                ShowActivated = true, IsHitTestVisible = true
            };
            inputObserver = new ManualDrawingInputObserver(overlay, WriteJson);
            overlay.SourceInitialized += (_, _) => handle = new WindowInteropHelper(overlay).Handle.ToInt64();
            overlay.Loaded += (_, _) =>
            {
                overlayOpened = true;
                // Ordinary initial window activation, never a drawing action.
                overlay.Activate();
                ready = true;
                WriteStatus("ready");
                timer.Start();
            };
            overlay.Closed += (_, _) =>
            {
                overlayClosed = true;
                ready = false;
                app.Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(Finish));
            };
            WriteStatus("starting");
            appLoopStarted = true;
            app.Run(overlay);
        }
        catch (Exception error)
        {
            Failure("session-start-or-run", error);
            try { overlay?.Close(); }
            catch (Exception closeError) { Failure("close-overlay", closeError); }
        }
        finally
        {
            Finish();
            if (!finished && !appLoopStarted) app.Run();
        }
    }

    private static void WriteJson(string name, object value) =>
        File.WriteAllText(Path.Combine(OutputDirectory, name), JsonSerializer.Serialize(value, JsonOptions), new UTF8Encoding(false));

    private static BitmapSource CreateSyntheticFrame()
    {
        var visual = new DrawingVisual();
        using (var drawing = visual.RenderOpen())
        {
            drawing.DrawRectangle(new SolidColorBrush(Color.FromRgb(234, 237, 241)), null, new Rect(0, 0, 2560, 1440));
            drawing.DrawRectangle(Brushes.White, new Pen(Brushes.SlateGray, 2), new Rect(280, 220, 1260, 860));
            var grid = new Pen(new SolidColorBrush(Color.FromRgb(222, 229, 237)), 1);
            for (var x = 320; x <= 1500; x += 40) drawing.DrawLine(grid, new Point(x, 240), new Point(x, 1060));
            for (var y = 260; y <= 1040; y += 40) drawing.DrawLine(grid, new Point(300, y), new Point(1520, y));
            var colors = new[] { Color.FromRgb(247, 215, 192), Color.FromRgb(191, 218, 248), Color.FromRgb(198, 236, 208), Color.FromRgb(228, 207, 243) };
            var names = new[] { "ALPHA", "BRAVO", "CHARLIE", "DELTA" };
            for (var index = 0; index < 4; index++)
            {
                var x = 360 + (index % 2) * 570;
                var y = 340 + (index / 2) * 330;
                drawing.DrawRectangle(new SolidColorBrush(colors[index]), new Pen(Brushes.SlateGray, 2), new Rect(x, y, 420, 240));
                DrawText(drawing, names[index], x + 25, y + 25, 42);
                DrawText(drawing, "Synthetic local content 123", x + 25, y + 105, 23);
                // Deterministic small detail makes region-template matching
                // meaningful without ever reading another window's pixels.
                for (var cell = 0; cell < 18; cell++)
                    drawing.DrawRectangle((cell + index) % 3 == 0 ? Brushes.SlateGray : Brushes.White, null,
                        new Rect(x + 25 + cell * 19, y + 168, 12, 12 + (cell % 4) * 8));
            }
            DrawText(drawing, "MEWU SYNTHETIC DRAWING QA", 300, 135, 46);
            DrawText(drawing, "All pixels are generated locally. Draw using the normal product controls.", 300, 1130, 27);
            DrawText(drawing, "Close the overlay to end this session and restore its starting clipboard.", 300, 1190, 27);
        }
        var image = new RenderTargetBitmap(2560, 1440, 96, 96, PixelFormats.Pbgra32);
        image.Render(visual);
        image.Freeze();
        return image;
    }

    private static void DrawText(DrawingContext drawing, string text, double x, double y, double size) =>
        drawing.DrawText(new FormattedText(text, CultureInfo.InvariantCulture, FlowDirection.LeftToRight,
            new Typeface("Segoe UI"), size, Brushes.DarkSlateGray, 1), new Point(x, y));

    private sealed partial class ClipboardSnapshot : IDisposable
    {
        private readonly Forms.DataObject data = new();
        private readonly List<IDisposable> owned = [];
        private readonly List<(string Format, object Value)> saved = [];
        private string[] formats = [];
        internal int FormatCount => formats.Length;
        internal uint Sequence { get; private set; }
        internal int UnreadableKnownBitmapDerivativeCount { get; private set; }
        internal string[] ReconstructedImageRepresentations { get; private set; } = [];

        internal static ClipboardSnapshot Capture()
        {
            Exception? last = null;
            for (var attempt = 0; attempt < 3; attempt++)
            {
                var snapshot = new ClipboardSnapshot();
                try
                {
                    snapshot.Sequence = GetClipboardSequenceNumber();
                    var source = Forms.Clipboard.GetDataObject();
                    snapshot.formats = source?.GetFormats(false) ?? [];
                    foreach (var format in snapshot.formats)
                    {
                        object? value;
                        try { value = source!.GetData(format, false); }
                        catch (NotSupportedException) when (IsKnownBitmapDerivative(format)) { value = null; }
                        if (value is null && IsKnownBitmapDerivative(format))
                        {
                            snapshot.UnreadableKnownBitmapDerivativeCount++;
                            continue;
                        }
                        if (value is null && format == Forms.DataFormats.Bitmap)
                        {
                            value = ReadNativeClipboardBitmap();
                            snapshot.owned.Add((IDisposable)value);
                        }
                        if (value is null) throw new NotSupportedException("Clipboard format cannot be materialized.");
                        var clone = snapshot.CloneValue(value);
                        snapshot.data.SetData(format, false, clone);
                        snapshot.saved.Add((format, clone));
                    }
                    if (snapshot.UnreadableKnownBitmapDerivativeCount > 0) snapshot.ReconstructKnownBitmapDerivative();
                    if (snapshot.Sequence != GetClipboardSequenceNumber()) throw new InvalidOperationException("Clipboard changed during backup.");
                    return snapshot;
                }
                catch (Exception error)
                {
                    snapshot.Dispose();
                    last = error;
                    if (error is NotSupportedException) break;
                    if (attempt < 2) Thread.Sleep(80);
                }
            }
            throw new InvalidOperationException("Complete clipboard backup unavailable; manual session was not opened.", last);
        }

        private object CloneValue(object value)
        {
            object clone = value switch
            {
                string text => text,
                byte[] bytes => bytes.ToArray(),
                string[] strings => strings.ToArray(),
                System.Drawing.Image image => image.Clone(),
                MemoryStream stream => new MemoryStream(stream.ToArray(), writable: false),
                int or uint or long or ulong or short or ushort or byte or sbyte or bool or char or float or double or decimal => value,
                _ => throw new NotSupportedException("Clipboard format cannot be safely cloned in memory.")
            };
            if (clone is IDisposable resource) owned.Add(resource);
            return clone;
        }

        internal void Restore()
        {
            Exception? last = null;
            for (var attempt = 0; attempt < 3; attempt++)
            {
                try
                {
                    if (formats.Length == 0) Forms.Clipboard.Clear();
                    else Forms.Clipboard.SetDataObject(data, true, 3, 60);
                    VerifyRestoredValues();
                    return;
                }
                catch (Exception error)
                {
                    last = error;
                    if (attempt < 2) Thread.Sleep(80);
                }
            }
            throw new InvalidOperationException("Clipboard restore failed after bounded retries.", last);
        }

        public void Dispose()
        {
            foreach (var resource in owned) resource.Dispose();
            owned.Clear();
            saved.Clear();
        }
    }

    [DllImport("user32.dll")] private static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll")] private static extern IntPtr GetClipboardOwner();
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
}
