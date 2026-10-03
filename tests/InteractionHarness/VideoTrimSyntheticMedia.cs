// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.IO;
using System.Text;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Windows.Media.Editing;
using Windows.Media.MediaProperties;
using Windows.Media.Transcoding;
using Windows.Storage;
using Brushes = System.Windows.Media.Brushes;
using BitmapDecoder = Windows.Graphics.Imaging.BitmapDecoder;
using FlowDirection = System.Windows.FlowDirection;
using Point = System.Windows.Point;

/// <summary>Entirely synthetic media: color, visible timecode, binary timecode, and optional PCM tones.</summary>
internal static class VideoTrimSyntheticMedia
{
    internal const int Width = 320, Height = 180, Rate = 44100;
    internal static async Task<string> CreateAsync(string directory, bool audio, CancellationToken token)
    {
        Directory.CreateDirectory(directory);
        // WPF drawing is completed before the first await, on the fresh STA worker.
        var frames = Enumerable.Range(0, 24).Select(index => WriteFrame(directory, index)).ToArray();
        var wave = Path.Combine(directory, "tones.wav");
        if (audio) WriteTones(wave);
        var composition = new MediaComposition();
        try
        {
            foreach (var frame in frames)
                composition.Clips.Add(await MediaClip.CreateFromImageFileAsync(
                    await StorageFile.GetFileFromPathAsync(frame).AsTask(token), TimeSpan.FromMilliseconds(250)).AsTask(token));
            if (audio) composition.BackgroundAudioTracks.Add(await BackgroundAudioTrack.CreateFromFileAsync(
                await StorageFile.GetFileFromPathAsync(wave).AsTask(token)).AsTask(token));
            var folder = await StorageFolder.GetFolderFromPathAsync(directory).AsTask(token);
            var output = await folder.CreateFileAsync("synthetic.mp4", CreationCollisionOption.FailIfExists).AsTask(token);
            var profile = MediaEncodingProfile.CreateMp4(VideoEncodingQuality.Wvga);
            profile.Video.Width = Width; profile.Video.Height = Height;
            profile.Video.FrameRate.Numerator = 30; profile.Video.FrameRate.Denominator = 1;
            if (!audio) profile.Audio = null;
            var result = await composition.RenderToFileAsync(output, MediaTrimmingPreference.Precise, profile).AsTask(token);
            if (result != TranscodeFailureReason.None) throw new InvalidDataException("Synthetic fixture encoding failed: " + result);
            return output.Path;
        }
        finally { composition.Clips.Clear(); composition.BackgroundAudioTracks.Clear(); }
    }

    private static string WriteFrame(string directory, int index)
    {
        var visual = new DrawingVisual();
        using (var drawing = visual.RenderOpen())
        {
            drawing.DrawRectangle(index < 8 ? Brushes.Red : index < 16 ? Brushes.Blue : Brushes.Lime, null, new Rect(0, 0, Width, Height));
            drawing.DrawRectangle(Brushes.Black, null, new Rect(0, 0, Width, 36));
            drawing.DrawText(new FormattedText($"SOURCE {index / 4d:0.00}s / {index:00}", CultureInfo.InvariantCulture,
                FlowDirection.LeftToRight, new Typeface("Consolas"), 20, Brushes.White, 1), new Point(8, 5));
            // A compression-tolerant, machine-readable quarter-second index.
            for (var bit = 0; bit < 5; bit++) drawing.DrawRectangle((index & (1 << bit)) != 0 ? Brushes.White : Brushes.Black,
                null, new Rect(8 + bit * 24, 148, 20, 24));
        }
        var bitmap = new RenderTargetBitmap(Width, Height, 96, 96, PixelFormats.Pbgra32); bitmap.Render(visual); bitmap.Freeze();
        var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(bitmap));
        var path = Path.Combine(directory, $"time-{index:00}.png"); using var stream = File.Create(path); encoder.Save(stream); return path;
    }

    internal static async Task<(double Seconds, bool Audio)> MetadataAsync(string path, CancellationToken token)
    {
        var clip = await MediaClip.CreateFromFileAsync(await StorageFile.GetFileFromPathAsync(path).AsTask(token)).AsTask(token);
        return (clip.OriginalDuration.TotalSeconds, clip.EmbeddedAudioTracks.Count > 0);
    }

    internal static async Task<(int Blue, int Green, int Red, int Timecode)> SampleAsync(string path, double seconds, CancellationToken token, int x = 160, int y = 90)
    {
        var composition = new MediaComposition();
        try
        {
            composition.Clips.Add(await MediaClip.CreateFromFileAsync(await StorageFile.GetFileFromPathAsync(path).AsTask(token)).AsTask(token));
            using var thumbnail = await composition.GetThumbnailAsync(TimeSpan.FromSeconds(seconds), Width, Height, VideoFramePrecision.NearestFrame).AsTask(token);
            var decoder = await BitmapDecoder.CreateAsync(thumbnail).AsTask(token);
            var pixels = (await decoder.GetPixelDataAsync(Windows.Graphics.Imaging.BitmapPixelFormat.Bgra8,
                Windows.Graphics.Imaging.BitmapAlphaMode.Ignore, new Windows.Graphics.Imaging.BitmapTransform(),
                Windows.Graphics.Imaging.ExifOrientationMode.IgnoreExifOrientation,
                Windows.Graphics.Imaging.ColorManagementMode.DoNotColorManage).AsTask(token)).DetachPixelData();
            var center = (y * Width + x) * 4; var code = 0;
            for (var bit = 0; bit < 5; bit++) if (pixels[(160 * Width + 18 + bit * 24) * 4] > 160) code |= 1 << bit;
            return (pixels[center], pixels[center + 1], pixels[center + 2], code);
        }
        finally { composition.Clips.Clear(); }
    }

    private static void WriteTones(string path)
    {
        const int samples = Rate * 6;
        using var writer = new BinaryWriter(File.Create(path));
        writer.Write(Encoding.ASCII.GetBytes("RIFF")); writer.Write(36 + samples * 2); writer.Write(Encoding.ASCII.GetBytes("WAVEfmt "));
        writer.Write(16); writer.Write((short)1); writer.Write((short)1); writer.Write(Rate); writer.Write(Rate * 2); writer.Write((short)2); writer.Write((short)16);
        writer.Write(Encoding.ASCII.GetBytes("data")); writer.Write(samples * 2);
        for (var i = 0; i < samples; i++)
        {
            var frequency = i < Rate * 2 ? 440 : i < Rate * 4 ? 660 : 880;
            writer.Write((short)(Math.Sin(2 * Math.PI * frequency * i / Rate) * 5000));
        }
    }

    internal static async Task<short[]> DecodeAudioAsync(string video, string outputPath, CancellationToken token)
    {
        var source = await StorageFile.GetFileFromPathAsync(video).AsTask(token);
        var folder = await StorageFolder.GetFolderFromPathAsync(Path.GetDirectoryName(outputPath)!).AsTask(token);
        var output = await folder.CreateFileAsync(Path.GetFileName(outputPath), CreationCollisionOption.FailIfExists).AsTask(token);
        var profile = MediaEncodingProfile.CreateWav(AudioEncodingQuality.High); profile.Audio = AudioEncodingProperties.CreatePcm(Rate, 1, 16);
        using (var input = await source.OpenAsync(FileAccessMode.Read).AsTask(token))
        using (var destination = await output.OpenAsync(FileAccessMode.ReadWrite).AsTask(token))
        {
            var prepared = await new MediaTranscoder().PrepareStreamTranscodeAsync(input, destination, profile).AsTask(token);
            if (!prepared.CanTranscode) throw new InvalidDataException("Synthetic audio decoding failed: " + prepared.FailureReason);
            await prepared.TranscodeAsync().AsTask(token);
        }
        using var reader = new BinaryReader(File.OpenRead(outputPath));
        if (Encoding.ASCII.GetString(reader.ReadBytes(4)) != "RIFF") throw new InvalidDataException("Missing RIFF header.");
        reader.ReadUInt32(); if (Encoding.ASCII.GetString(reader.ReadBytes(4)) != "WAVE") throw new InvalidDataException("Missing WAVE form.");
        short[]? samples = null; var formatValid = false;
        while (reader.BaseStream.Position + 8 <= reader.BaseStream.Length)
        {
            var id = Encoding.ASCII.GetString(reader.ReadBytes(4)); var length = reader.ReadUInt32(); var end = reader.BaseStream.Position + length;
            if (end > reader.BaseStream.Length) throw new InvalidDataException("Invalid RIFF chunk length.");
            if (id == "fmt ")
            {
                var format = reader.ReadUInt16(); var channels = reader.ReadUInt16(); var rate = reader.ReadUInt32();
                reader.ReadUInt32(); reader.ReadUInt16(); var bits = reader.ReadUInt16();
                formatValid = (format == 1 || format == 0xfffe) && channels == 1 && rate == Rate && bits == 16;
            }
            if (id == "data")
            {
                if (length > Rate * 20 * 2 || (length & 1) != 0) throw new InvalidDataException("Unexpected synthetic PCM length.");
                samples = new short[length / 2]; for (var i = 0; i < samples.Length; i++) samples[i] = reader.ReadInt16();
            }
            reader.BaseStream.Position = end + (length & 1);
        }
        if (!formatValid || samples is null) throw new InvalidDataException("Unexpected decoded audio format.");
        return samples;
    }

    internal static double ToneEnergy(short[] samples, double startSeconds, int frequency)
    {
        var start = (int)(startSeconds * Rate); const int count = Rate / 10;
        if (start < 0 || start + count > samples.Length) throw new ArgumentOutOfRangeException(nameof(startSeconds));
        double real = 0, imaginary = 0;
        for (var i = 0; i < count; i++)
        {
            var angle = 2 * Math.PI * frequency * i / Rate;
            real += samples[start + i] * Math.Cos(angle); imaginary += samples[start + i] * Math.Sin(angle);
        }
        return real * real + imaginary * imaginary;
    }
}
