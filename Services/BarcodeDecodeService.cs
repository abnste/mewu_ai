// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using ZXing;
using ZXing.Windows.Compatibility;
namespace mewu_ai_Assistant.Services;
internal static class BarcodeDecodeService
{
    internal static IReadOnlyList<Result> Decode(BitmapSource source)
    {
        var reader = new BarcodeReaderGeneric { AutoRotate = true, Options = new ZXing.Common.DecodingOptions { TryHarder = true, TryInverted = true, PossibleFormats = [BarcodeFormat.QR_CODE, BarcodeFormat.DATA_MATRIX, BarcodeFormat.AZTEC, BarcodeFormat.PDF_417, BarcodeFormat.EAN_13, BarcodeFormat.EAN_8, BarcodeFormat.UPC_A, BarcodeFormat.CODE_128, BarcodeFormat.CODE_39] } };
        var scale = Math.Min(3d, 1400d / Math.Max(source.PixelWidth, source.PixelHeight));
        BitmapSource working = source;
        if (scale > 1.05) { var resized = new TransformedBitmap(source, new ScaleTransform(scale, scale)); resized.Freeze(); working = resized; }
        if (working.Format != PixelFormats.Pbgra32) { var converted = new FormatConvertedBitmap(working, PixelFormats.Pbgra32, null, 0); converted.Freeze(); working = converted; }
        using var bitmap = new System.Drawing.Bitmap(working.PixelWidth, working.PixelHeight, System.Drawing.Imaging.PixelFormat.Format32bppPArgb);
        var data = bitmap.LockBits(new System.Drawing.Rectangle(0, 0, bitmap.Width, bitmap.Height), System.Drawing.Imaging.ImageLockMode.WriteOnly, bitmap.PixelFormat);
        try { working.CopyPixels(new Int32Rect(0, 0, working.PixelWidth, working.PixelHeight), data.Scan0, checked(data.Stride * bitmap.Height), data.Stride); }
        finally { bitmap.UnlockBits(data); }
        return reader.DecodeMultiple(bitmap) ?? (reader.Decode(bitmap) is { } one ? [one] : []);
    }
}
