using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Recording;
using Xunit;

namespace MewuAI.Tests;

public sealed class GifExportTests
{
    [Fact]
    public void Export_WritesAnimatedFramesWithDelayMetadata()
    {
        var root = Path.Combine(Path.GetTempPath(), "MewuAI.Tests", Guid.NewGuid().ToString("N"));
        var frames = Path.Combine(root, "frames");
        var output = Path.Combine(root, "result.gif");
        Directory.CreateDirectory(frames);
        try
        {
            WritePng(Path.Combine(frames, "00000.png"), Colors.Red);
            WritePng(Path.Combine(frames, "00001.png"), Colors.Blue);
            GifExportService.Export(frames, output, 10);
            using var stream = File.OpenRead(output);
            var decoder = new GifBitmapDecoder(stream, BitmapCreateOptions.PreservePixelFormat, BitmapCacheOption.OnLoad);
            Assert.Equal(2, decoder.Frames.Count);
            var metadata = Assert.IsType<BitmapMetadata>(decoder.Frames[0].Metadata);
            Assert.Equal((ushort)10, metadata.GetQuery("/grctlext/Delay"));
        }
        finally { if (Directory.Exists(root)) Directory.Delete(root, true); }
    }

    private static void WritePng(string path, Color color)
    {
        var pixels = Enumerable.Repeat(new[] { color.B, color.G, color.R, color.A }, 16).SelectMany(x => x).ToArray();
        var bitmap = BitmapSource.Create(4, 4, 96, 96, PixelFormats.Bgra32, null, pixels, 16);
        var encoder = new PngBitmapEncoder();
        encoder.Frames.Add(BitmapFrame.Create(bitmap));
        using var output = File.Create(path);
        encoder.Save(output);
    }
}
