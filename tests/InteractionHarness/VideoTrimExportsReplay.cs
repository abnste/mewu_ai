// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;

internal static partial class VideoTrimReplay
{
    private static async Task VerifyExportsAsync(string directory, Evidence evidence)
    {
        var source = Path.GetFullPath(Path.Combine(directory, "..", "media", "trim-evidence.mp4"));
        evidence.Check(File.Exists(source), "export worker uses actual prepared clip evidence"); var before = Hash(source);
        var temp = new TempFileService(Path.Combine(directory, "export-temp"));
        var copied = Path.Combine(directory, "clean.mp4");
        AtomicFileService.Copy(source, copied);
        evidence.Check(Hash(copied) == before, "clean MP4 export preserves the prepared clip bytes");

        var original = new AiAnnotation(.75, .3, .2, .4, "synthetic annotation", 0, 2, 4,
            [new(2, .75, .3, .2, .4), new(4, .75, .3, .2, .4)], Kind: AiAnnotationKind.Rectangle,
            Style: new("#FFFFFF", .05));
        var range = new VideoClipRange(TimeSpan.FromSeconds(1.25), TimeSpan.FromSeconds(4.75));
        var projection = VideoClipTimeline.Project([original], range);
        evidence.Check(projection.Annotations[0].StartTime == .75 && projection.Annotations[0].EndTime == 2.75,
            "export annotation times are relative to the selected clip");
        evidence.Check(original.StartTime == 2 && original.EndTime == 4, "projection preserves source-time annotation");
        var annotated = Path.Combine(directory, "annotated.mp4");
        await evidence.Step("annotated-export", token => AnnotatedVideoExportService.ExportAsync(source, annotated, null,
            projection.Annotations, token, projection.CardPositions, temp), 60);
        var metadata = await evidence.Step("annotated-metadata", token => VideoTrimSyntheticMedia.MetadataAsync(annotated, token));
        evidence.Check(Math.Abs(metadata.Seconds - 3.5) < .12 && metadata.Audio, "annotated MP4 retains trimmed duration and audio");
        var beforeMark = await evidence.Step("annotation-before", token => VideoTrimSyntheticMedia.SampleAsync(annotated, .3, token, 240, 90));
        var onMark = await evidence.Step("annotation-during", token => VideoTrimSyntheticMedia.SampleAsync(annotated, 1.3, token, 240, 90));
        var afterMark = await evidence.Step("annotation-after", token => VideoTrimSyntheticMedia.SampleAsync(annotated, 3.1, token, 240, 90));
        evidence.Check(beforeMark.Red > 220 && beforeMark.Blue < 40, "temporal annotation is absent before selected interval");
        evidence.Check(onMark.Red > 220 && onMark.Green > 220 && onMark.Blue > 220, "temporal annotation appears on selected interval pixels");
        evidence.Check(afterMark.Green > 220 && afterMark.Red < 40, "temporal annotation is absent after selected interval");

        var audio = Path.Combine(directory, "trim.mp3");
        await evidence.Step("mp3-export", token => Mp3ExportService.ExportAsync(source, audio, token, temp));
        var pcm = await evidence.Step("mp3-decode", token => VideoTrimSyntheticMedia.DecodeAudioAsync(audio, Path.Combine(directory, "mp3.wav"), token));
        evidence.Check(Math.Abs(pcm.Length / (double)VideoTrimSyntheticMedia.Rate - 3.5) < .15, "MP3 contains selected audio duration");
        foreach (var (seconds, frequency) in new[] { (.2, 440), (.95, 660), (3.05, 880) })
        {
            var selected = VideoTrimSyntheticMedia.ToneEnergy(pcm, seconds, frequency);
            var other = new[] { 440, 660, 880 }.Where(value => value != frequency).Select(value => VideoTrimSyntheticMedia.ToneEnergy(pcm, seconds, value)).Max();
            evidence.Check(selected > other * 20, $"MP3 preserves selected {frequency}Hz audio content");
        }

        var gif = Path.Combine(directory, "trim.gif");
        var gifResult = await evidence.Step("gif-export", token => GifExportService.ExportFromVideoAsync(source, gif, 4, token, temp: temp));
        using (var stream = File.OpenRead(gif))
        {
            var decoder = new GifBitmapDecoder(stream, BitmapCreateOptions.PreservePixelFormat, BitmapCacheOption.OnLoad);
            var centiseconds = decoder.Frames.Sum(frame => Convert.ToInt32(((BitmapMetadata)frame.Metadata).GetQuery("/grctlext/Delay")));
            evidence.Check(Math.Abs(centiseconds / 100d - 3.5) < .12, "GIF timing represents only selected duration");
            evidence.Check(decoder.Frames.Count == gifResult.FrameCount, "GIF frame count matches export report");
        }

        var existing = Path.Combine(directory, "keep.mp3"); await File.WriteAllTextAsync(existing, "synthetic-existing-output");
        using (var canceled = new CancellationTokenSource())
        {
            canceled.Cancel(); var rejected = false;
            try { await Mp3ExportService.ExportAsync(source, existing, canceled.Token, temp); }
            catch (OperationCanceledException) { rejected = true; }
            evidence.Check(rejected && await File.ReadAllTextAsync(existing) == "synthetic-existing-output", "canceled export preserves existing destination bytes");
        }
        evidence.Check(Hash(source) == before, "all exports preserve prepared media bytes");
        evidence.Check(TempMediaRegistry.Shared.ActiveLeaseCount == 0, "exports release every source and intermediate lease");
        evidence.Stage("export-cleanup:begin"); var cleanup = temp.Cleanup(TimeSpan.Zero); evidence.Stage("export-cleanup:end", cleanup);
        evidence.Check(cleanup.FailureCount == 0 && Directory.GetFiles(temp.DirectoryPath).Length == 0, "export intermediate files clean after lease release");
    }
}
