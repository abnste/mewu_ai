// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;
using Windows.Media.Editing;
using Windows.Media.MediaProperties;
using Windows.Media.Transcoding;
using Windows.Storage;
using Application = System.Windows.Application;
using Image = System.Windows.Controls.Image;

/// <summary>
/// Fresh bounded workers encode synthetic detail charts, then exercise the real
/// preview surface. No window, capture, settings, audio, provider or clipboard.
/// </summary>
internal static class VideoPreviewResolutionReplay
{
    private static readonly JsonSerializerOptions Json = new() { WriteIndented = true };
    private const BindingFlags Private = BindingFlags.Instance | BindingFlags.NonPublic;
    private sealed record Result(int Width, bool Passed, int ProcessId, string ProductSha256, string HarnessSha256,
        string[] Checks, string? Failure, double ElapsedSeconds);

    internal static void Run(string[] args)
    {
        var worker = args.FirstOrDefault(arg => arg.StartsWith("--preview-resolution-width=", StringComparison.Ordinal));
        var cwd = worker is null ? ReplayOutputDirectory.PrepareWorkingDirectory("video-preview-resolution")
            : ReplayOutputDirectory.ValidateDirectory(Environment.CurrentDirectory);
        if (worker is not null)
        {
            if (!int.TryParse(worker["--preview-resolution-width=".Length..], out var width) || width is not (1920 or 2560 or 3840))
                throw new ArgumentException("Unsupported synthetic preview size.");
            RunWorker(cwd, width); return;
        }
        var output = Path.Combine(cwd, ".codex-build", "video-preview-resolution");
        Directory.CreateDirectory(output);
        var results = new List<Result>(); var failures = new List<string>();
        foreach (var width in new[] { 1920, 2560, 3840 })
        {
            var directory = Path.Combine(output, width.ToString(System.Globalization.CultureInfo.InvariantCulture));
            Directory.CreateDirectory(directory);
            var executable = Environment.ProcessPath ?? throw new InvalidOperationException("Missing replay executable.");
            var start = new ProcessStartInfo(executable)
            { WorkingDirectory = directory, UseShellExecute = false, CreateNoWindow = true, WindowStyle = ProcessWindowStyle.Hidden };
            if (string.Equals(Path.GetFileNameWithoutExtension(executable), "dotnet", StringComparison.OrdinalIgnoreCase))
                start.ArgumentList.Add(typeof(VideoPreviewResolutionReplay).Assembly.Location);
            start.ArgumentList.Add("--verify-video-preview-resolution");
            start.ArgumentList.Add("--preview-resolution-width=" + width);
            start.Environment["TEMP"] = directory; start.Environment["TMP"] = directory;
            using var child = Process.Start(start) ?? throw new InvalidOperationException("Could not start isolated preview replay.");
            if (!child.WaitForExit(150_000))
            {
                child.Kill(entireProcessTree: true); child.WaitForExit(5_000);
                failures.Add(width + ": exact worker exceeded its deadline; diagnostics retained"); break;
            }
            var path = Path.Combine(directory, "result.json");
            if (!File.Exists(path)) { failures.Add(width + ": missing worker result"); continue; }
            var result = JsonSerializer.Deserialize<Result>(File.ReadAllText(path)) ?? throw new InvalidDataException("Invalid worker result.");
            results.Add(result);
            if (child.ExitCode != 0 || !result.Passed) failures.Add(width + ": " + result.Failure);
        }
        var passed = failures.Count == 0 && results.Count == 3;
        File.WriteAllText(Path.Combine(output, "result.json"), JsonSerializer.Serialize(new
        {
            passed, checks = results.Sum(result => result.Checks.Length), results, failures,
            syntheticMediaOnly = true, productionSettingsUsed = false, desktopCaptureUsed = false,
            windowsCreated = false, microphoneUsed = false, clipboardUsed = false,
            observationsArePerformanceGuarantees = false
        }, Json), new UTF8Encoding(false));
        Environment.ExitCode = passed ? 0 : 1;
    }

    private static void RunWorker(string directory, int width)
    {
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(directory, "logs"));
        var checks = new List<string>(); var clock = Stopwatch.StartNew(); Exception? failure = null;
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext(app.Dispatcher));
        app.Dispatcher.BeginInvoke(new Action(async () =>
        {
            try
            {
                using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(120));
                await VerifyAsync(directory, width, checks, deadline.Token);
            }
            catch (Exception error) { failure = error; Stage(directory, "failure", new { error = error.ToString() }); }
            finally { app.Shutdown(); }
        }));
        try { app.Run(); }
        catch (Exception error) { failure ??= error; }
        File.WriteAllText(Path.Combine(directory, "result.json"), JsonSerializer.Serialize(new Result(width, failure is null,
            Environment.ProcessId, Hash(typeof(VideoPreviewSurface).Assembly.Location), Hash(typeof(VideoPreviewResolutionReplay).Assembly.Location),
            checks.ToArray(), failure?.ToString(), clock.Elapsed.TotalSeconds), Json), new UTF8Encoding(false));
        Environment.ExitCode = failure is null ? 0 : 1;
    }

    private static async Task VerifyAsync(string directory, int width, List<string> checks, CancellationToken token)
    {
        void Check(bool value, string name) { if (!value) throw new InvalidOperationException(name); checks.Add(name); }
        var height = width * 9 / 16;
        Stage(directory, "encode-synthetic-source", new { width, height });
        var source = await CreateSourceAsync(directory, width, height, token);
        var hash = Hash(source);
        var storage = await StorageFile.GetFileFromPathAsync(source).AsTask(token);
        var metadata = await storage.Properties.GetVideoPropertiesAsync().AsTask(token);
        Check(metadata.Width == width && metadata.Height == height && metadata.Duration.TotalSeconds is >= 2.95 and < 3.2,
            "synthetic MP4 retains exact high-resolution dimensions and three-second duration");
        Stage(directory, "source-ready", new { width = metadata.Width, height = metadata.Height, metadata.Duration, hash });

        foreach (var percent in new[] { 100, 75, 50 })
        {
            var image = new Image();
            // Deliberately omit the optional argument for the default case.
            var preview = percent == 100 ? new VideoPreviewSurface(image, Application.Current.Dispatcher)
                : new VideoPreviewSurface(image, Application.Current.Dispatcher, percent);
            var errors = new List<Exception>();
            preview.Failed += errors.Add;
            var identities = new HashSet<object>(ReferenceEqualityComparer.Instance);
            var largestPool = 0;
            void ObservePool()
            {
                lock (Read(preview, "_frameGate")!)
                {
                    var buffers = ((IEnumerable)Read(preview, "_frameBuffers")!).Cast<object?>().Where(buffer => buffer is not null).ToArray();
                    largestPool = Math.Max(largestPool, buffers.Length);
                    foreach (var buffer in buffers) identities.Add(buffer!);
                }
            }
            preview.FramePresented += _ => ObservePool();
            var expectedWidth = width * percent / 100; var expectedHeight = height * percent / 100;
            try
            {
                Stage(directory, "load-preview", new { percent, expectedWidth, expectedHeight });
                await LoadAsync(preview, source, token);
                Check(!preview.IsPlaying && preview.PresentedFrameCount > 0, percent + "% paused load waits for a truly presented frame");
                if (percent == 100) SavePng(image, Path.Combine(directory, "native-preview.png"));
                AssertImage(image, expectedWidth, expectedHeight, width, height, percent, false, Check);

                var count = preview.PresentedFrameCount;
                var initialTime = preview.LastPresentedPosition;
                using var process = Process.GetCurrentProcess(); process.Refresh();
                var before = new { managedBytes = GC.GetTotalMemory(false), allocatedBytes = GC.GetTotalAllocatedBytes(),
                    workingSetBytes = process.WorkingSet64, gen2Collections = GC.CollectionCount(2), frames = count };
                var playbackClock = Stopwatch.StartNew();
                preview.Play();
                if (width == 1920 && percent == 100) await Task.Delay(1_000, token);
                await WaitForAsync(() => preview.PresentedFrameCount >= count + 2, TimeSpan.FromSeconds(8), token);
                preview.Pause();
                process.Refresh();
                Stage(directory, "playback-observation", new { percent, elapsedMilliseconds = playbackClock.Elapsed.TotalMilliseconds, before,
                    after = new { managedBytes = GC.GetTotalMemory(false), allocatedBytes = GC.GetTotalAllocatedBytes(),
                        workingSetBytes = process.WorkingSet64, gen2Collections = GC.CollectionCount(2), frames = preview.PresentedFrameCount },
                    preview.LastPresentedPosition, largestPool, distinctBuffers = identities.Count });
                Check(!preview.IsPlaying && preview.PresentedFrameCount >= count + 2 && preview.LastPresentedPosition > initialTime,
                    percent + "% native playback advances actual frames and pauses normally");

                using var seekDeadline = CancellationTokenSource.CreateLinkedTokenSource(token);
                seekDeadline.CancelAfter(TimeSpan.FromSeconds(12));
                var actual = await preview.SeekAsync(TimeSpan.FromSeconds(1.9), true, seekDeadline.Token);
                Check(Math.Abs(actual.TotalSeconds - 1.9) < .1, percent + "% seek reports the actual source time");
                AssertImage(image, expectedWidth, expectedHeight, width, height, percent, true, Check);
                actual = await preview.SeekAsync(TimeSpan.FromSeconds(.4), true, seekDeadline.Token);
                Check(Math.Abs(actual.TotalSeconds - .4) < .1, percent + "% backward seek reports the actual source time");
                AssertImage(image, expectedWidth, expectedHeight, width, height, percent, false, Check);
                ObservePool();
                Check(largestPool is > 0 and <= 3 && identities.Count <= 3,
                    percent + "% preview reuses at most three pixel buffers across playback and bidirectional seeks");
                Check(errors.Count == 0 && Hash(source) == hash, percent + "% preview and seeks preserve source bytes and report no decoder failure");
            }
            finally
            {
                Stage(directory, "close-preview", new { percent });
                preview.CloseSource(); preview.Dispose();
            }
            await Application.Current.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
            Check(image.Source is null && !preview.IsPlaying && Read(preview, "_player") is null && Read(preview, "_surface") is null &&
                Read(preview, "_latestPixels") is null && Read(preview, "_stagedFrame") is null && Read(preview, "_deliveringPixels") is null,
                percent + "% close releases native source, texture and queued/staged/delivering pixels");
            Check(!((IEnumerable)Read(preview, "_frameBuffers")!).Cast<object?>().Any(buffer => buffer is not null),
                percent + "% close clears reusable pixel buffers");
        }
        Check(Hash(source) == hash && Application.Current.Windows.Count == 0 && TempMediaRegistry.Shared.ActiveLeaseCount == 0,
            "all resolution cases retain the original MP4 and finish with no window or media lease");
    }

    private static void AssertImage(Image image, int width, int height, int sourceWidth, int sourceHeight, int percent,
        bool secondHalf, Action<bool, string> check)
    {
        var bitmap = image.Source as BitmapSource ?? throw new InvalidOperationException("Missing presented preview pixels.");
        check(bitmap.PixelWidth == width && bitmap.PixelHeight == height, percent + "% presented bitmap has exact requested physical dimensions");
        byte[] Pixel(int x, int y)
        {
            var bytes = new byte[4];
            bitmap.CopyPixels(new Int32Rect(Math.Clamp(x * percent / 100, 0, width - 1),
                Math.Clamp(y * percent / 100, 0, height - 1), 1, 1), bytes, 4, 0);
            return bytes;
        }
        var corners = new[] { (48, 48, 2), (sourceWidth - 48, 48, 1), (48, sourceHeight - 48, 0) };
        foreach (var (x, y, channel) in corners)
        {
            var pixel = Pixel(x, y);
            check(pixel[channel] >= 200 && pixel[(channel + 1) % 3] < 65 && pixel[(channel + 2) % 3] < 65,
                percent + "% preview preserves corner marker " + channel + " without crop or coordinate drift");
        }
        var temporal = Pixel(sourceWidth / 2 + 48, sourceHeight / 2 + 48);
        var activeChannel = secondHalf ? 0 : 2;
        check(temporal[activeChannel] >= 200 && temporal[2 - activeChannel] < 65,
            percent + "% frame pixels correspond to the requested " + (secondHalf ? "second" : "first") + " source segment");
        for (var stripe = 0; stripe < 8; stripe++)
        {
            var pixel = Pixel(320 + stripe * 8 + 4, 192);
            check((stripe % 2 == 0 ? pixel[0] < 65 : pixel[0] > 190), percent + "% eight-pixel detail stripe " + stripe + " is correctly mapped");
        }
        if (percent == 100)
        {
            double dark = 0, light = 0;
            for (var x = 0; x < 64; x++)
            {
                var value = Pixel(128 + x, 192)[0];
                if ((x & 1) == 0) dark += value; else light += value;
            }
            check(light / 32 - dark / 32 > 150,
                "default native preview resolves one-pixel alternating detail; measured contrast=" + (light / 32 - dark / 32));
        }
    }

    private static async Task<string> CreateSourceAsync(string directory, int width, int height, CancellationToken token)
    {
        var composition = new MediaComposition();
        try
        {
            for (var half = 0; half < 2; half++)
            {
                var pixels = new byte[checked(width * height * 4)];
                for (var offset = 0; offset < pixels.Length; offset += 4)
                { pixels[offset] = pixels[offset + 1] = pixels[offset + 2] = 240; pixels[offset + 3] = 255; }
                void Rectangle(int left, int top, int w, int h, byte blue, byte green, byte red)
                {
                    for (var y = top; y < top + h; y++) for (var x = left; x < left + w; x++)
                    { var p = (y * width + x) * 4; pixels[p] = blue; pixels[p + 1] = green; pixels[p + 2] = red; }
                }
                Rectangle(16, 16, 96, 96, 0, 0, 255);
                Rectangle(width - 112, 16, 96, 96, 0, 255, 0);
                Rectangle(16, height - 112, 96, 96, 255, 0, 0);
                Rectangle(width / 2, height / 2, 96, 96, half == 0 ? (byte)0 : (byte)255, 0, half == 0 ? (byte)255 : (byte)0);
                for (var x = 0; x < 128; x++)
                { var color = (x & 1) == 0 ? (byte)0 : (byte)255; Rectangle(128 + x, 128, 1, 128, color, color, color); }
                for (var stripe = 0; stripe < 16; stripe++)
                { var color = (stripe & 1) == 0 ? (byte)0 : (byte)255; Rectangle(320 + stripe * 8, 128, 8, 128, color, color, color); }
                var bitmap = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgra32, null, pixels, width * 4); bitmap.Freeze();
                var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(bitmap));
                var path = Path.Combine(directory, "detail-" + half + ".png");
                using (var stream = File.Create(path)) encoder.Save(stream);
                Array.Clear(pixels);
                composition.Clips.Add(await MediaClip.CreateFromImageFileAsync(await StorageFile.GetFileFromPathAsync(path).AsTask(token),
                    TimeSpan.FromSeconds(1.5)).AsTask(token));
            }
            var folder = await StorageFolder.GetFolderFromPathAsync(directory).AsTask(token);
            var output = await folder.CreateFileAsync("synthetic-detail.mp4", CreationCollisionOption.FailIfExists).AsTask(token);
            var profile = MediaEncodingProfile.CreateMp4(VideoEncodingQuality.HD1080p);
            profile.Video.Width = (uint)width; profile.Video.Height = (uint)height;
            profile.Video.FrameRate.Numerator = 30; profile.Video.FrameRate.Denominator = 1;
            profile.Video.Bitrate = 50_000_000; profile.Audio = null;
            var result = await composition.RenderToFileAsync(output, MediaTrimmingPreference.Precise, profile).AsTask(token);
            if (result != TranscodeFailureReason.None) throw new InvalidDataException("Synthetic detail encoding failed: " + result);
            return output.Path;
        }
        finally { composition.Clips.Clear(); }
    }

    private static async Task LoadAsync(VideoPreviewSurface preview, string path, CancellationToken token)
    {
        var ready = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        void Opened() => ready.TrySetResult(); void Failed(Exception error) => ready.TrySetException(error);
        preview.Opened += Opened; preview.Failed += Failed;
        try { preview.Load(path, false, initialPosition: TimeSpan.Zero); await ready.Task.WaitAsync(TimeSpan.FromSeconds(25), token); }
        finally { preview.Opened -= Opened; preview.Failed -= Failed; }
    }

    private static async Task WaitForAsync(Func<bool> condition, TimeSpan duration, CancellationToken token)
    {
        var clock = Stopwatch.StartNew();
        while (!condition())
        {
            if (clock.Elapsed > duration) throw new TimeoutException("Native preview did not advance its frame count.");
            await Task.Delay(20, token);
        }
    }
    private static void SavePng(Image image, string path)
    {
        var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create((BitmapSource)image.Source));
        using var stream = File.Create(path); encoder.Save(stream);
    }
    private static object? Read(object instance, string name) => instance.GetType().GetField(name, Private)!.GetValue(instance);
    private static string Hash(string path) { using var stream = File.OpenRead(path); return Convert.ToHexString(SHA256.HashData(stream)); }
    private static void Stage(string directory, string stage, object? data = null) => File.AppendAllText(Path.Combine(directory, "stages.jsonl"),
        JsonSerializer.Serialize(new { utc = DateTime.UtcNow, processId = Environment.ProcessId, stage, data }) + "\n", new UTF8Encoding(false));
}
