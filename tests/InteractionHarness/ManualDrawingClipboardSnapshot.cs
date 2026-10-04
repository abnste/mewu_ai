// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using Forms = System.Windows.Forms;

internal static partial class ManualDrawingDesktopSession
{
    private sealed partial class ClipboardSnapshot
    {
        private static bool IsKnownBitmapDerivative(string format) =>
            string.Equals(format, typeof(Bitmap).FullName, StringComparison.Ordinal);

        private void ReconstructKnownBitmapDerivative()
        {
            // This narrowly recognized registered representation has no payload.
            // Preserve the authoritative native image; never ignore an unknown
            // unreadable format or claim byte identity for the missing derivative.
            var bitmap = ReadNativeClipboardBitmap();
            owned.Add(bitmap);
            foreach (var entry in saved.Where(entry => entry.Format == Forms.DataFormats.Bitmap))
                if (entry.Value is not Image image || !SamePixels(image, bitmap))
                    throw new InvalidOperationException("Native and managed clipboard images disagree.");
            if (!saved.Any(entry => entry.Format == Forms.DataFormats.Bitmap))
            {
                data.SetData(Forms.DataFormats.Bitmap, false, bitmap);
                saved.Add((Forms.DataFormats.Bitmap, bitmap));
            }
            if (!saved.Any(entry => entry.Format == "PNG"))
            {
                var png = new MemoryStream();
                bitmap.Save(png, ImageFormat.Png);
                png.Position = 0;
                owned.Add(png);
                data.SetData("PNG", false, png);
                saved.Add(("PNG", png));
            }
            ReconstructedImageRepresentations = ["native-bitmap", "png"];
        }

        private void VerifyRestoredValues()
        {
            var current = Forms.Clipboard.GetDataObject();
            if (formats.Length == 0)
            {
                if (current?.GetFormats(false).Length > 0) throw new InvalidOperationException("Empty clipboard restore failed.");
                return;
            }
            if (current is null) throw new InvalidOperationException("Restored clipboard is not readable.");
            foreach (var entry in saved)
            {
                if (entry.Format == Forms.DataFormats.Bitmap && entry.Value is Image expectedBitmap)
                {
                    using var restoredBitmap = ReadNativeClipboardBitmap();
                    if (!SamePixels(expectedBitmap, restoredBitmap)) throw new InvalidOperationException("Restored image pixels differ.");
                    continue;
                }
                if (entry.Format == "PNG" && ReconstructedImageRepresentations.Length > 0)
                {
                    var bytes = ReadNativeClipboardBytes(Forms.DataFormats.GetFormat("PNG").Id);
                    try
                    {
                        using var stream = new MemoryStream(bytes, writable: false);
                        using var restoredPng = Image.FromStream(stream);
                        using var restoredBitmap = ReadNativeClipboardBitmap();
                        if (!SamePixels(restoredPng, restoredBitmap)) throw new InvalidOperationException("Reconstructed image pixels differ.");
                    }
                    finally { CryptographicOperations.ZeroMemory(bytes); }
                    continue;
                }
                var value = current.GetData(entry.Format, false);
                if (value is null || !SameValue(entry.Value, value))
                    throw new InvalidOperationException("Restored clipboard value is unavailable or different.");
            }
        }

        private static bool SameValue(object expected, object actual) => (expected, actual) switch
        {
            (Image first, Image second) => SamePixels(first, second),
            (MemoryStream first, MemoryStream second) => SameStreamBytes(first, second),
            (byte[] first, byte[] second) => first.AsSpan().SequenceEqual(second),
            (string[] first, string[] second) => first.SequenceEqual(second, StringComparer.Ordinal),
            _ => expected.Equals(actual)
        };

        private static bool SameStreamBytes(MemoryStream first, MemoryStream second)
        {
            var a = first.ToArray(); var b = second.ToArray();
            try { return a.AsSpan().SequenceEqual(b); }
            finally { CryptographicOperations.ZeroMemory(a); CryptographicOperations.ZeroMemory(b); }
        }

        private static bool SamePixels(Image first, Image second)
        {
            if (first.Width != second.Width || first.Height != second.Height) return false;
            var a = ImagePixels(first); var b = ImagePixels(second);
            try { return a.AsSpan().SequenceEqual(b); }
            finally { CryptographicOperations.ZeroMemory(a); CryptographicOperations.ZeroMemory(b); }
        }

        private static byte[] ImagePixels(Image image)
        {
            using var normalized = new Bitmap(image.Width, image.Height, PixelFormat.Format32bppArgb);
            using (var graphics = Graphics.FromImage(normalized))
            {
                graphics.CompositingMode = System.Drawing.Drawing2D.CompositingMode.SourceCopy;
                graphics.DrawImageUnscaled(image, 0, 0);
            }
            var locked = normalized.LockBits(new Rectangle(0, 0, normalized.Width, normalized.Height),
                ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            try
            {
                var stride = checked(normalized.Width * 4);
                var bytes = new byte[checked(stride * normalized.Height)];
                for (var row = 0; row < normalized.Height; row++)
                    Marshal.Copy(IntPtr.Add(locked.Scan0, row * locked.Stride), bytes, row * stride, stride);
                return bytes;
            }
            finally { normalized.UnlockBits(locked); }
        }

        private static Bitmap ReadNativeClipboardBitmap()
        {
            if (!OpenClipboard(IntPtr.Zero)) throw new InvalidOperationException("Clipboard could not be opened for verification.");
            try
            {
                var handle = GetClipboardData(2); // CF_BITMAP, a native GDI handle.
                if (handle == IntPtr.Zero) throw new NotSupportedException("Native clipboard image is unavailable.");
                using var image = Image.FromHbitmap(handle);
                return (Bitmap)image.Clone();
            }
            finally { CloseClipboard(); }
        }

        private static byte[] ReadNativeClipboardBytes(int format)
        {
            if (!OpenClipboard(IntPtr.Zero)) throw new InvalidOperationException("Clipboard could not be opened for verification.");
            try
            {
                var handle = GetClipboardData((uint)format);
                if (handle == IntPtr.Zero) throw new NotSupportedException("Native clipboard storage is unavailable.");
                var size = GlobalSize(handle).ToUInt64();
                if (size == 0 || size > 128UL * 1024 * 1024) throw new NotSupportedException("Native clipboard storage is outside the supported bounds.");
                var address = GlobalLock(handle);
                if (address == IntPtr.Zero) throw new NotSupportedException("Native clipboard storage cannot be read.");
                try { var bytes = new byte[(int)size]; Marshal.Copy(address, bytes, 0, bytes.Length); return bytes; }
                finally { GlobalUnlock(handle); }
            }
            finally { CloseClipboard(); }
        }

        [DllImport("user32.dll", SetLastError = true)] private static extern bool OpenClipboard(IntPtr owner);
        [DllImport("user32.dll")] private static extern bool CloseClipboard();
        [DllImport("user32.dll", SetLastError = true)] private static extern IntPtr GetClipboardData(uint format);
        [DllImport("kernel32.dll", SetLastError = true)] private static extern UIntPtr GlobalSize(IntPtr handle);
        [DllImport("kernel32.dll", SetLastError = true)] private static extern IntPtr GlobalLock(IntPtr handle);
        [DllImport("kernel32.dll")] private static extern bool GlobalUnlock(IntPtr handle);
    }
}
