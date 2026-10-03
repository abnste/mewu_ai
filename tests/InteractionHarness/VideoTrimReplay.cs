// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Diagnostics;
using System.IO;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;

/// <summary>
/// Opt-in synthetic, process-isolated video trimming regression. No production
/// settings, providers, desktop input, clipboard, microphone or screen capture.
/// Every worker has an outer deadline covering native calls and synchronous teardown.
/// </summary>
internal static partial class VideoTrimReplay
{
    private static readonly JsonSerializerOptions Json = new() { WriteIndented = true };
    private sealed record Result(string Scenario, bool Passed, int ProcessId, string ProductSha256,
        string HarnessSha256, string[] Checks, string? Failure, double ElapsedSeconds);
    private sealed class Evidence(string directory)
    {
        internal readonly List<string> Checks = [];
        internal void Check(bool condition, string name)
        {
            if (!condition) throw new InvalidOperationException(name);
            Checks.Add(name);
        }
        internal void Stage(string stage, object? data = null) => File.AppendAllText(Path.Combine(directory, "stages.jsonl"),
            JsonSerializer.Serialize(new { utc = DateTime.UtcNow, processId = Environment.ProcessId, stage, data }) + "\n", new UTF8Encoding(false));
        internal async Task<T> Step<T>(string name, Func<CancellationToken, Task<T>> action, int seconds = 40)
        {
            Stage(name + ":begin"); using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(seconds));
            var value = await action(deadline.Token); Stage(name + ":end"); return value;
        }
        internal async Task Step(string name, Func<CancellationToken, Task> action, int seconds = 40)
        {
            Stage(name + ":begin"); using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(seconds));
            await action(deadline.Token); Stage(name + ":end");
        }
    }

    internal static void Run(string[] args)
    {
        var worker = args.FirstOrDefault(arg => arg.StartsWith("--video-trim-case=", StringComparison.Ordinal));
        var cwd = worker is null ? ReplayOutputDirectory.PrepareWorkingDirectory("video-trim")
            : ReplayOutputDirectory.ValidateDirectory(Environment.CurrentDirectory);
        if (worker is not null) { RunWorker(cwd, worker["--video-trim-case=".Length..]); return; }
        var output = Path.Combine(cwd, ".codex-build", "video-trim"); Directory.CreateDirectory(output);
        var results = new List<Result>(); var failures = new List<string>();
        var scenarios = new[] { "media", "cancellation", "exports", "preview", "near-end", "near-end-exact", "near-end-1ms", "near-end-10ms", "short-range", "overlay" };
        foreach (var scenario in scenarios)
        {
            var directory = Path.Combine(output, scenario); Directory.CreateDirectory(directory);
            var start = new ProcessStartInfo(Environment.ProcessPath ?? throw new InvalidOperationException("Missing replay executable."))
            { WorkingDirectory = directory, UseShellExecute = false, CreateNoWindow = true, WindowStyle = ProcessWindowStyle.Hidden };
            start.ArgumentList.Add("--verify-isolated-video-trim"); start.ArgumentList.Add("--video-trim-case=" + scenario);
            start.Environment["TEMP"] = directory; start.Environment["TMP"] = directory;
            using var child = Process.Start(start) ?? throw new InvalidOperationException("Could not start isolated video replay.");
            if (!child.WaitForExit(150_000))
            {
                // Exact process object created here: never search/terminate a production process.
                child.Kill(entireProcessTree: true); child.WaitForExit(5_000);
                failures.Add(scenario + ": worker timed out; stage log and media retained"); break;
            }
            var resultPath = Path.Combine(directory, "result.json");
            if (!File.Exists(resultPath)) { failures.Add(scenario + ": missing worker result"); continue; }
            var result = JsonSerializer.Deserialize<Result>(File.ReadAllText(resultPath)) ?? throw new InvalidDataException("Invalid video worker report.");
            results.Add(result); if (child.ExitCode != 0 || !result.Passed) failures.Add(scenario + ": " + result.Failure);
        }
        File.WriteAllText(Path.Combine(output, "result.json"), JsonSerializer.Serialize(new
        {
            passed = failures.Count == 0 && results.Count == scenarios.Length, checks = results.Sum(result => result.Checks.Length), results, failures,
            syntheticMediaOnly = true, realProviderInvoked = false, productionSettingsUsed = false, desktopInputInjected = false,
            clipboardUsed = false, cleanupStrategy = "Stage-logged isolated workers; fixtures retained. Outer timeout includes synchronous native cleanup."
        }, Json), new UTF8Encoding(false));
        Environment.ExitCode = failures.Count == 0 && results.Count == scenarios.Length ? 0 : 1;
    }

    private static void RunWorker(string directory, string scenario)
    {
        var evidence = new Evidence(directory); var watch = Stopwatch.StartNew(); string? failure = null;
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(directory, "logs"));
        try
        {
            evidence.Stage("worker-start", new { scenario, source = typeof(TempFileService).Assembly.Location });
            switch (scenario)
            {
                case "media": VerifyMediaAsync(directory, evidence).GetAwaiter().GetResult(); break;
                case "cancellation": VerifyCancellationAsync(directory, evidence).GetAwaiter().GetResult(); break;
                case "exports": VerifyExportsAsync(directory, evidence).GetAwaiter().GetResult(); break;
                case "preview": RunDispatcherScenario(() => VerifyPreviewAsync(directory, evidence)); break;
                case "near-end": RunDispatcherScenario(() => VerifyNearEndAsync(directory, evidence)); break;
                case "near-end-exact": RunDispatcherScenario(() => VerifyNearEndAsync(directory, evidence, 0)); break;
                case "near-end-1ms": RunDispatcherScenario(() => VerifyNearEndAsync(directory, evidence, TimeSpan.TicksPerMillisecond)); break;
                case "near-end-10ms": RunDispatcherScenario(() => VerifyNearEndAsync(directory, evidence, TimeSpan.TicksPerMillisecond * 10)); break;
                case "short-range": RunDispatcherScenario(() => VerifyShortRangeAsync(directory, evidence)); break;
                case "flicker": RunDispatcherScenario(() => VerifyFlickerAsync(directory, evidence)); break;
                case "overlay": RunDispatcherScenario(() => evidence.Step("overlay-integration", token => VideoTrimOverlayReplay.RunAsync(
                    System.Windows.Application.Current, Path.GetFullPath(Path.Combine(directory, "..", "media", "source", "synthetic.mp4")),
                    directory, evidence.Checks, token), 90)); break;
                default: throw new ArgumentException("Unknown video trim scenario.");
            }
            evidence.Check(TempMediaRegistry.Shared.ActiveLeaseCount == 0, "all worker-owned media leases released");
        }
        catch (Exception error) { failure = error.ToString(); evidence.Stage("failure", new { failure }); }
        finally
        {
            evidence.Stage("worker-finished");
            File.WriteAllText(Path.Combine(directory, "result.json"), JsonSerializer.Serialize(new Result(scenario, failure is null,
                Environment.ProcessId, Hash(typeof(TempFileService).Assembly.Location), Hash(typeof(VideoTrimReplay).Assembly.Location),
                evidence.Checks.ToArray(), failure, watch.Elapsed.TotalSeconds), Json), new UTF8Encoding(false));
            Environment.ExitCode = failure is null ? 0 : 1;
        }
    }

    private static async Task VerifyMediaAsync(string directory, Evidence evidence)
    {
        var source = await evidence.Step("fixture-encode", token => VideoTrimSyntheticMedia.CreateAsync(Path.Combine(directory, "source"), true, token), 60);
        var sourceHash = Hash(source);
        var metadata = await evidence.Step("source-metadata", token => VideoTrimSyntheticMedia.MetadataAsync(source, token));
        evidence.Check(metadata.Seconds is > 5.95 and < 6.1 && metadata.Audio, "source is six-second synthetic video with an audio track");
        var temp = new TempFileService(Path.Combine(directory, "prepared"));
        using (var untouched = await evidence.Step("null-full-range", token => VideoClipPreparationService.PrepareAsync(source, null, token, temp)))
        {
            evidence.Check(untouched.IsOriginal && SamePath(untouched.Path, source) && Hash(untouched.Path) == sourceHash,
                "null full range keeps exact original bytes and avoids recorder elapsed-time trimming");
            evidence.Check(Math.Abs(untouched.Duration.TotalSeconds - metadata.Seconds) < .001,
                "null full range derives actual duration from source metadata");
        }
        using (var full = await evidence.Step("full-range", token => VideoClipPreparationService.PrepareAsync(source, VideoClipRange.Full(TimeSpan.FromSeconds(metadata.Seconds)), token, temp)))
        {
            evidence.Check(full.IsOriginal && SamePath(full.Path, source), "full range retains source without transcoding");
            evidence.Check(Math.Abs(full.Duration.TotalSeconds - metadata.Seconds) < .001, "full range uses real source duration");
            evidence.Check(TempMediaRegistry.Shared.IsLeased(source), "full-range handle holds source lease");
        }
        evidence.Check(!TempMediaRegistry.Shared.IsLeased(source), "full-range disposal releases source lease");
        var range = new VideoClipRange(TimeSpan.FromSeconds(1.25), TimeSpan.FromSeconds(4.75));
        string preparedPath;
        using (var prepared = await evidence.Step("trim-prepare", token => VideoClipPreparationService.PrepareAsync(source, range, token, temp), 60))
        {
            preparedPath = prepared.Path;
            evidence.Check(!prepared.IsOriginal && !SamePath(preparedPath, source), "partial range creates independent media");
            evidence.Check(prepared.Range == range && Math.Abs(prepared.Duration.TotalSeconds - 3.5) < .001, "prepared range and logical duration match selection");
            evidence.Check(TempMediaRegistry.Shared.IsLeased(preparedPath), "prepared clip holds lease");
            evidence.Stage("leased-cleanup:begin"); var cleanup = temp.Cleanup(TimeSpan.Zero); evidence.Stage("leased-cleanup:end");
            evidence.Check(cleanup.SkippedLeasedCount >= 1 && File.Exists(preparedPath), "cleanup preserves leased prepared clip");
            var outputMetadata = await evidence.Step("trim-metadata", token => VideoTrimSyntheticMedia.MetadataAsync(preparedPath, token));
            evidence.Check(Math.Abs(outputMetadata.Seconds - 3.5) < .08 && outputMetadata.Audio, "encoded clip has selected duration and retained audio");
            foreach (var seconds in new[] { .15, .6, .9, 1.4, 2.6, 2.9, 3.15 })
            {
                var sample = await evidence.Step("frame-" + seconds, token => VideoTrimSyntheticMedia.SampleAsync(preparedPath, seconds, token));
                var expectedSource = seconds + range.Start.TotalSeconds;
                var channel = expectedSource < 2 ? 2 : expectedSource < 4 ? 0 : 1;
                var channels = new[] { sample.Blue, sample.Green, sample.Red };
                evidence.Check(channels[channel] > 220 && channels[(channel + 1) % 3] < 40 && channels[(channel + 2) % 3] < 40,
                    $"output {seconds:0.00}s contains correct source color");
                evidence.Check(sample.Timecode == (int)(expectedSource * 4), $"output {seconds:0.00}s contains correct encoded source timecode");
                evidence.Stage("decoded-frame", new { seconds, sourceSeconds = expectedSource, sample.Timecode });
            }
            var samples = await evidence.Step("decode-trim-audio", token => VideoTrimSyntheticMedia.DecodeAudioAsync(preparedPath, Path.Combine(directory, "trim-audio.wav"), token));
            evidence.Check(Math.Abs(samples.Length / (double)VideoTrimSyntheticMedia.Rate - 3.5) < .12, "decoded audio duration matches selected video");
            foreach (var (seconds, frequency) in new[] { (.2, 440), (.9, 660), (2.9, 880) })
            {
                var selected = VideoTrimSyntheticMedia.ToneEnergy(samples, seconds, frequency);
                var others = new[] { 440, 660, 880 }.Where(candidate => candidate != frequency).Select(candidate => VideoTrimSyntheticMedia.ToneEnergy(samples, seconds, candidate)).Max();
                evidence.Check(selected > others * 20 && selected > 1e8, $"audio at {seconds:0.0}s has expected {frequency}Hz source interval");
            }
            File.Copy(preparedPath, Path.Combine(directory, "trim-evidence.mp4"), false);
        }
        evidence.Check(!TempMediaRegistry.Shared.IsLeased(preparedPath), "prepared disposal releases output lease");
        evidence.Stage("unleased-cleanup:begin"); var finalCleanup = temp.Cleanup(TimeSpan.Zero); evidence.Stage("unleased-cleanup:end", finalCleanup);
        evidence.Check(finalCleanup.FailureCount == 0 && !File.Exists(preparedPath), "released prepared output can be cleaned");
        evidence.Check(Hash(source) == sourceHash, "original video SHA-256 unchanged after trim, decode and cleanup");
        evidence.Stage("source-hash", new { sourceHash });
    }

    private static async Task VerifyCancellationAsync(string directory, Evidence evidence)
    {
        var source = Path.GetFullPath(Path.Combine(directory, "..", "media", "source", "synthetic.mp4"));
        evidence.Check(File.Exists(source), "cancellation uses preceding synthetic fixture"); var sourceHash = Hash(source);
        var temp = new TempFileService(Path.Combine(directory, "prepared"));
        var range = new VideoClipRange(TimeSpan.FromSeconds(.25), TimeSpan.FromSeconds(5.75));
        using (var canceled = new CancellationTokenSource())
        {
            canceled.Cancel(); var rejected = false;
            try { using var ignored = await VideoClipPreparationService.PrepareAsync(source, range, canceled.Token, temp); }
            catch (OperationCanceledException) { rejected = true; }
            evidence.Check(rejected && Directory.GetFiles(temp.DirectoryPath).Length == 0, "pre-canceled prepare produces no output");
        }
        using (var cancel = new CancellationTokenSource(TimeSpan.FromSeconds(15)))
        {
            evidence.Stage("inflight-cancel:begin");
            var pending = VideoClipPreparationService.PrepareAsync(source, range, cancel.Token, temp);
            while (!pending.IsCompleted && Directory.GetFiles(temp.DirectoryPath).Length == 0) await Task.Delay(1);
            var observedInFlight = !pending.IsCompleted && Directory.GetFiles(temp.DirectoryPath).Length > 0;
            evidence.Stage("inflight-cancel:signal", new { observedInFlight }); cancel.Cancel(); var rejected = false;
            try { using var ignored = await pending; } catch (OperationCanceledException) { rejected = true; }
            evidence.Check(observedInFlight, "cancellation signaled after temporary encoding output appeared while preparation was pending");
            evidence.Check(rejected, "in-flight preparation responds to cancellation");
            evidence.Check(Directory.GetFiles(temp.DirectoryPath).All(path => !TempMediaRegistry.Shared.IsLeased(path)), "canceled preparation releases every incomplete output lease");
            evidence.Stage("cancel-cleanup:begin"); var cleanup = temp.Cleanup(TimeSpan.Zero); evidence.Stage("cancel-cleanup:end", cleanup);
            evidence.Check(cleanup.FailureCount == 0 && Directory.GetFiles(temp.DirectoryPath).Length == 0, "normal cleanup removes canceled unleased outputs");
        }
        evidence.Check(Hash(source) == sourceHash, "cancellation leaves source bytes unchanged");
    }

    private static bool SamePath(string first, string second) => string.Equals(Path.GetFullPath(first), Path.GetFullPath(second), StringComparison.OrdinalIgnoreCase);
    private static string Hash(string path) { using var stream = File.OpenRead(path); return Convert.ToHexString(SHA256.HashData(stream)); }
}
