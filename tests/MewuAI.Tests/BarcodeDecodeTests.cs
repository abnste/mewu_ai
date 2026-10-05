// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Xml.Linq;
using mewu_ai_Assistant.Services;
using QRCoder;
using Xunit;

namespace MewuAI.Tests;

public sealed class BarcodeDecodeTests
{
    private const string Content = "https://example.invalid/qr?text=中文&value=42";

    [Fact]
    public void NewRecognitionCommandsAreReachableFromTheExistingOcrButtonMenu()
    {
        var document = XDocument.Load(Path.Combine(AppContext.BaseDirectory, "Fixtures", "CaptureOverlayWindow.xaml.xml"));
        XNamespace xaml = "http://schemas.microsoft.com/winfx/2006/xaml";
        var button = document.Descendants().Single(element => (string?)element.Attribute(xaml + "Name") == "OcrButton");
        var menu = Assert.Single(button.Descendants().Where(element => element.Name.LocalName == "ContextMenu"));
        Assert.Equal(new[] { "DecodeBarcode", "ScanFill" }, menu.Elements().Select(element => (string?)element.Attribute("Click")).ToArray());
        Assert.Equal("Ocr", (string?)button.Attribute("Click"));
        Assert.Equal("RecognitionMenuOpened", (string?)menu.Attribute("Opened"));
    }

    [Theory]
    [InlineData("Bgr24", 0)]
    [InlineData("Gray8", 0)]
    [InlineData("Pbgra32", 0)]
    [InlineData("Bgra32", 90)]
    public void ReadsQrContentFromDifferentPixelFormatsAndRotation(string format, int angle)
    {
        BitmapSource image = Qr(Content);
        if (angle != 0) image = new TransformedBitmap(image, new RotateTransform(angle));
        var pixelFormat = format switch { "Bgr24" => PixelFormats.Bgr24, "Gray8" => PixelFormats.Gray8, "Bgra32" => PixelFormats.Bgra32, _ => PixelFormats.Pbgra32 };
        image = new FormatConvertedBitmap(image, pixelFormat, null, 0);
        image.Freeze();
        Assert.Equal(Content, Assert.Single(BarcodeDecodeService.Decode(image)).Text);
    }

    [Fact]
    public void ReadsBothCodesWithoutDiscardingTheSecondOne()
    {
        var first = Qr("first-code");
        var second = Qr("second-code");
        var width = first.PixelWidth + second.PixelWidth + 60;
        var height = Math.Max(first.PixelHeight, second.PixelHeight);
        var pixels = Enumerable.Repeat((byte)255, width * height * 4).ToArray();
        foreach (var (image, offset) in new[] { (first, 0), (second, first.PixelWidth + 60) })
        {
            var converted = new FormatConvertedBitmap(image, PixelFormats.Bgra32, null, 0);
            var row = new byte[image.PixelWidth * image.PixelHeight * 4];
            converted.CopyPixels(row, image.PixelWidth * 4, 0);
            for (var y = 0; y < image.PixelHeight; y++) Array.Copy(row, y * image.PixelWidth * 4, pixels, (y * width + offset) * 4, image.PixelWidth * 4);
        }
        var canvas = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgra32, null, pixels, width * 4);
        canvas.Freeze();
        Assert.Equal(new[] { "first-code", "second-code" }, BarcodeDecodeService.Decode(canvas).Select(result => result.Text).Order().ToArray());
    }

    [Fact]
    public void BlankImageDoesNotInventAResult()
    {
        var pixels = Enumerable.Repeat((byte)255, 120 * 100).ToArray();
        var image = BitmapSource.Create(120, 100, 96, 96, PixelFormats.Gray8, null, pixels, 120);
        image.Freeze();
        Assert.Empty(BarcodeDecodeService.Decode(image));
    }

    private static BitmapSource Qr(string text)
    {
        using var generator = new QRCodeGenerator();
        using var data = generator.CreateQrCode(text, QRCodeGenerator.ECCLevel.M);
        using var code = new PngByteQRCode(data);
        using var stream = new MemoryStream(code.GetGraphic(5));
        var image = new BitmapImage();
        image.BeginInit(); image.CacheOption = BitmapCacheOption.OnLoad; image.StreamSource = stream; image.EndInit(); image.Freeze();
        return image;
    }
}
