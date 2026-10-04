// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Security.Cryptography;
using System.Text.Json;
using System.Collections;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.OCR;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application = System.Windows.Application;
using Size = System.Windows.Size;
using Brushes = System.Windows.Media.Brushes;
using Point = System.Windows.Point;
using FlowDirection = System.Windows.FlowDirection;

internal static class OcrMemoryReplay
{
    private sealed record MemorySample(string stage, int iteration, int lines, long milliseconds, long privateBytes,
        long workingSet, long managedBytes, int handles, string? recognitionHash);

    internal static void Run(string[] args)
    {
        var directory = ReplayOutputDirectory.PrepareWorkingDirectory("ocr-memory");
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(directory, "logs"));
        var samples = new List<MemorySample>();
        var failures = new List<string>();
        var windows = new List<WeakReference>();
        var overlayMode = args.Contains("--overlay");
        var completed = false;
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext(app.Dispatcher));
        app.Resources.MergedDictionaries.Add(new ResourceDictionary { Source = new Uri("/MewuAI;component/Themes/LightTheme.xaml", UriKind.Relative) });
        using var host = new AppHost(app, null, "MewuAI-OcrMemory-" + Guid.NewGuid().ToString("N"));
        host.Settings.EnableVoiceInput = false;
        host.Settings.AutomaticallyStartListening = false;
        host.Settings.SaveConversationHistory = false;
        using var timeout = new CancellationTokenSource(TimeSpan.FromMinutes(4));
        try
        {
            Sample("cold", 0, 0, 0);
            for (var i = 0; i < 64; i++)
            {
                // Same-size warmup/repetition followed by different screenshot sizes.
                var shape = i % 32;
                var width = shape < 8 ? 1000 : 800 + (shape - 8) * 37;
                var height = shape < 8 ? 600 : 400 + (shape - 8) * 23;
                var image = CreateImage(width, height);
                var watch = Stopwatch.StartNew();
                mewu_ai_Assistant.Models.OcrDocument result;
                if (overlayMode) result = RecognizeAndClose(host, image, windows, timeout.Token);
                else
                {
                    result = new WindowsOcrService(null).RecognizeAsync(image, timeout.Token).GetAwaiter().GetResult();
                    if (result.Engine != "PP-OCRv6 本地 OCR" || !result.Text.Contains("SCREEN", StringComparison.OrdinalIgnoreCase))
                        failures.Add($"iteration {i}: local OCR did not recognize fixture");
                }
                DrainDispatcher();
                Sample("recognized", i + 1, result.Lines.Count, watch.ElapsedMilliseconds,
                    Convert.ToHexString(SHA256.HashData(JsonSerializer.SerializeToUtf8Bytes(result))));
                if (Process.GetCurrentProcess().PrivateMemorySize64 > 2500L * 1024 * 1024)
                    throw new InvalidOperationException("Memory safety limit exceeded");
            }
            // Diagnostic only: never add forced GC to the application's OCR path.
            GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();
            DrainDispatcher();
            GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();
            Sample("after-diagnostic-gc", 64, 0, 0);
            if (windows.Any(window => window.IsAlive)) failures.Add("Closed OCR overlays remain rooted after dispatcher drain and diagnostic GC");
            // Generous opt-in process budget, not a timing/working-set unit test.
            // The unmodified CPU arena exceeds this with these changing sizes.
            if (samples[^1].privateBytes > 768L * 1024 * 1024) failures.Add("Retained private memory exceeds 768 MiB after 64 synthetic OCR operations");
            var firstCycle = samples.Where(sample => sample.stage == "recognized" && sample.iteration <= 32).Max(sample => sample.privateBytes);
            var secondCycle = samples.Where(sample => sample.stage == "recognized" && sample.iteration > 32).Max(sample => sample.privateBytes);
            if (secondCycle - firstCycle > 192L * 1024 * 1024) failures.Add("Repeated size cycle retains more than 192 MiB of additional private memory");
            completed = true;
        }
        catch (Exception error) { failures.Add(error.ToString()); }
        finally { Save(); app.Shutdown(); }
        Environment.ExitCode = failures.Count == 0 ? 0 : 1;

        void Sample(string stage, int iteration, int lines, long milliseconds, string? recognitionHash = null)
        {
            using var process = Process.GetCurrentProcess(); process.Refresh();
            samples.Add(new MemorySample(stage, iteration, lines, milliseconds, process.PrivateMemorySize64,
                process.WorkingSet64, GC.GetTotalMemory(false), process.HandleCount, recognitionHash));
            Save();
        }
        void Save() => File.WriteAllText(Path.Combine(directory, "result.json"), JsonSerializer.Serialize(new
        {
            passed = completed && failures.Count == 0, completed, failures, samples, overlayMode, closedWindowsAlive = windows.Count(window => window.IsAlive), syntheticOnly = true, shownWindows = false,
            productSha256 = Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(typeof(WindowsOcrService).Assembly.Location)))
        }, new JsonSerializerOptions { WriteIndented = true }));
    }

    private const BindingFlags Private = BindingFlags.Instance | BindingFlags.NonPublic;

    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.NoInlining)]
    private static mewu_ai_Assistant.Models.OcrDocument RecognizeAndClose(AppHost host, BitmapSource image, List<WeakReference> windows, CancellationToken token)
    {
        var overlay = new CaptureOverlayWindow(host, null, new CaptureFrame(0, 0, image));
        try
        {
            var root = (Canvas)overlay.FindName("Root");
            overlay.Content = null;
            root.Width = image.PixelWidth; root.Height = image.PixelHeight;
            root.Measure(new Size(image.PixelWidth, image.PixelHeight)); root.Arrange(new Rect(0, 0, image.PixelWidth, image.PixelHeight));
            var item = typeof(CaptureOverlayWindow).GetMethod("CreateSelection", Private)!.Invoke(overlay, [false])!;
            ((IList)typeof(CaptureOverlayWindow).GetField("_selections", Private)!.GetValue(overlay)!).Add(item);
            typeof(CaptureOverlayWindow).GetField("_activeIndex", Private)!.SetValue(overlay, 0);
            item.GetType().GetField("Bounds")!.SetValue(item, new Rect(0, 0, image.PixelWidth, image.PixelHeight));
            typeof(CaptureOverlayWindow).GetMethod("UpdateSelection", Private)!.Invoke(overlay, [item]);
            typeof(CaptureOverlayWindow).GetMethod("Ocr", Private)!.Invoke(overlay, [overlay, new RoutedEventArgs()]);
            while (typeof(CaptureOverlayWindow).GetField("_overlayRequest", Private)!.GetValue(overlay) is not null)
            {
                token.ThrowIfCancellationRequested(); DrainDispatcher(); Thread.Sleep(10);
            }
            var textState = item.GetType().GetProperty("TextLayer")!.GetValue(item)!;
            var document = (mewu_ai_Assistant.Models.OcrDocument?)textState.GetType().GetProperty("Document")?.GetValue(textState);
            if (document is null || document.Engine != "PP-OCRv6 本地 OCR" || !document.Text.Contains("SCREEN", StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("Real overlay OCR did not recognize fixture");
            if (new WindowInteropHelper(overlay).Handle != IntPtr.Zero) throw new InvalidOperationException("Replay must not create a window");
            return document;
        }
        finally { windows.Add(new WeakReference(overlay)); overlay.Close(); }
    }

    private static void DrainDispatcher()
    {
        var frame = new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }

    private static BitmapSource CreateImage(int width, int height)
    {
        var visual = new DrawingVisual();
        using (var drawing = visual.RenderOpen())
        {
            drawing.DrawRectangle(Brushes.White, null, new Rect(0, 0, width, height));
            for (var y = 24; y < height - 50; y += 68)
                drawing.DrawText(new FormattedText("HELLO SCREEN 123 喵呜文字识别", CultureInfo.GetCultureInfo("zh-CN"),
                    FlowDirection.LeftToRight, new Typeface("Microsoft YaHei UI"), 26, Brushes.Black, 1), new Point(24, y));
        }
        var image = new RenderTargetBitmap(width, height, 96, 96, PixelFormats.Pbgra32);
        image.Render(visual); image.Freeze(); return image;
    }
}
