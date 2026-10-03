// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Point = System.Windows.Point;

internal static class DrawingHealReplay
{
    internal static async Task Verify(CaptureOverlayWindow overlay, object item, Action<bool, string> require, string directory)
    {
        const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public;
        object? Invoke(string name, params object?[] args) => typeof(CaptureOverlayWindow).GetMethod(name, flags)!.Invoke(overlay, args);
        object? Field(string name) => typeof(CaptureOverlayWindow).GetField(name, flags)!.GetValue(overlay);
        void SetField(string name, object value) => typeof(CaptureOverlayWindow).GetField(name, flags)!.SetValue(overlay, value);
        T Property<T>(object value, string name) => (T)value.GetType().GetProperty(name, flags)!.GetValue(value)!;
        var canvas = Property<InkCanvas>(item, "Markup"); var elements = Property<IList>(item, "DrawingElements"); var order = Property<IList>(item, "DrawingOrder");
        var snapshot = Invoke("CaptureOverlaySnapshot")!; var originalFrame = (CaptureFrame)Field("_frame")!; var originalWidth = (double)Field("_drawHealWidth")!;
        var crop = (Rect)item.GetType().GetField("Bounds")!.GetValue(item)!;
        BitmapSource Saved(bool manual = true) => (BitmapSource)Invoke("RenderSelectionImage", item, manual, false, false)!;
        void Tool() => Invoke("SetDrawTool", Enum.Parse(typeof(CaptureOverlayWindow).GetNestedType("DrawTool", BindingFlags.NonPublic)!, "Heal"));
        void Undo() => Invoke("DrawUndo", overlay, new RoutedEventArgs());
        void Redo() => Invoke("DrawRedo", overlay, new RoutedEventArgs());
        async Task WaitFor(Func<bool> finished)
        {
            var clock = Stopwatch.StartNew();
            while (!finished())
            {
                if (clock.Elapsed > TimeSpan.FromSeconds(8)) throw new TimeoutException("Healing replay did not settle.");
                await overlay.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.Background); await Task.Delay(5);
            }
        }
        try
        {
            Invoke("DrawClear", overlay, new RoutedEventArgs()); SetField("_drawHealWidth", 30d);
            var region = (Int32Rect)Invoke("ToPixelRect", crop)!; var scaleX = region.Width / crop.Width; var scaleY = region.Height / crop.Height;
            var width = originalFrame.Image.PixelWidth; var height = originalFrame.Image.PixelHeight; var bytes = new byte[width * height * 4];
            for (var y = 0; y < height; y++) for (var x = 0; x < width; x++)
            {
                var localX = (x - region.X) / scaleX; var localY = (y - region.Y) / scaleY;
                var black = localX >= 81 && localX < 89 && localY >= 65 && localY <= 145 || localY >= 141 && localY < 149 && localX >= 85 && localX < 155;
                var sentinel = localX >= 120 && localX < 124 && localY >= 90 && localY < 94; var at = (y * width + x) * 4;
                bytes[at] = black ? (byte)0 : sentinel ? (byte)220 : (byte)80;
                bytes[at + 1] = black ? (byte)0 : sentinel ? (byte)20 : (byte)140;
                bytes[at + 2] = black ? (byte)0 : sentinel ? (byte)220 : (byte)210; bytes[at + 3] = 255;
            }
            var source = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgra32, null, bytes, width * 4); source.Freeze();
            SetField("_frame", new CaptureFrame(originalFrame.OriginX, originalFrame.OriginY, source)); Invoke("UpdateSelection", item);
            var original = Pixels(source); var clean = Saved(false); var before = order.Count; var blackPoint = new Point(85 * scaleX, 100 * scaleY); var sentinelPoint = new Point(122 * scaleX, 92 * scaleY);
            require(Pixel(clean, blackPoint).Take(3).All(c => c == 0), "healing fixture black path is inside the actual physical screenshot crop");
            Tool(); require((bool)Invoke("BeginDrawingGesture", item, new Point(85, 65), canvas, 1, canvas)!, "healing starts through normal tool dispatcher");
            Invoke("UpdateHealingDrawingPreview", item, new Point(85, 145));
            require(Field("_drawingHealPreview") is not null && elements.Count == 0 && order.Count == before, "healing drag shows only an uncommitted path selection");
            Invoke("CancelHealingOperation"); canvas.ReleaseMouseCapture();
            require(Field("_drawingHealPreview") is null && elements.Count == 0 && order.Count == before, "canceling path preview creates no repair action");

            Tool(); Invoke("BeginDrawingGesture", item, new Point(85, 65), canvas, 1, canvas); Invoke("UpdateHealingDrawingPreview", item, new Point(85, 145));
            Invoke("CommitHealingDrawingPreview", item, new Point(155, 145));
            require(Field("_drawingHealRequest") is not null && !((FrameworkElement)overlay.FindName("DrawingToolbar")).IsEnabled, "asynchronous healing disables toolbar while its request is active");
            canvas.ReleaseMouseCapture(); await WaitFor(() => Field("_drawingHealRequest") is null);
            require(elements.Count == 1 && order.Count == before + 1 && order[order.Count - 1]!.GetType().Name == "ElementDrawingAction", "release during worker completion commits exactly one repair action");
            require(((FrameworkElement)overlay.FindName("DrawingToolbar")).IsEnabled && !canvas.IsMouseCaptured, "completed healing restores toolbar and releases pointer capture");
            var element = elements[0]!; var cache = Property<BitmapSource>(element, "Pixels");
            require(Property<bool>(element, "SeamlessErase") && !Property<bool>(element, "IsLiftedContent") && cache.IsFrozen, "healing produces one fixed immutable repair layer without lifted foreground");
            var alpha = Pixels(cache).Where((_, i) => i % 4 == 3).ToArray();
            require(alpha.Any(a => a == 0) && alpha.Any(a => a == 255), "curved healing output keeps transparent pixels inside its rectangular bounding box");
            var repaired = Saved(); require(Pixel(repaired, blackPoint).Take(3).SequenceEqual(new byte[] { 80, 140, 210 }), "covered black content becomes the known background color");
            require(Pixel(repaired, sentinelPoint).SequenceEqual(Pixel(clean, sentinelPoint)), "unpainted sentinel inside the path bounding rectangle remains intact");
            Save(repaired, Path.Combine(directory, "healing-curved-path.png"));
            var id = Property<Guid>(element, "Id");
            Invoke("SetDrawTool", Enum.Parse(typeof(CaptureOverlayWindow).GetNestedType("DrawTool", BindingFlags.NonPublic)!, "Select"));
            Invoke("BeginDrawingGesture", item, new Point(85, 100), canvas, 1, canvas); Invoke("CommitSelectedDrawingMove"); canvas.ReleaseMouseCapture();
            require(!Equals(Field("_selectedDrawingElementId"), id), "healing background patch cannot be moved as foreground content");
            Undo(); require(elements.Count == 0 && Pixels(Saved()).SequenceEqual(Pixels(clean)), "one undo restores the complete pre-stroke screenshot");
            Redo(); require(elements.Count == 1 && ReferenceEquals(Property<BitmapSource>(elements[0]!, "Pixels"), cache), "one redo reuses the immutable curved repair pixels");
            var retained = Invoke("CaptureOverlaySnapshot")!; Invoke("DrawClear", overlay, new RoutedEventArgs()); require(elements.Count == 0, "Clear removes repair layers"); Undo();
            require(elements.Count == 1 && Pixels(Saved()).SequenceEqual(Pixels(repaired)), "Clear undo restores exact repaired composition");
            Invoke("ApplyOverlaySnapshot", retained); require(ReferenceEquals(Property<BitmapSource>(elements[0]!, "Pixels"), cache), "snapshot preserves frozen healing pixels");
            Invoke("SetSelectionBoundsPreservingManualContent", item, new Rect(crop.X + 50, crop.Y + 40, crop.Width, crop.Height)); Invoke("UpdateSelection", item);
            Invoke("SetSelectionBoundsPreservingManualContent", item, crop); Invoke("UpdateSelection", item);
            require(Pixels(Saved()).SequenceEqual(Pixels(repaired)), "reframe away and back preserves anchored healing patch");

            var beforeCancel = order.Count; Tool(); Invoke("BeginDrawingGesture", item, new Point(280, 210), canvas, 1, canvas);
            Invoke("CommitHealingDrawingPreview", item, new Point(400, 310)); var cancelled = (CancellationTokenSource)Field("_drawingHealRequest")!;
            require(cancelled is not null, "cancellation scenario reaches a real asynchronous repair request"); Invoke("CancelHealingOperation"); canvas.ReleaseMouseCapture();
            await WaitFor(() => { try { _ = cancelled!.Token; return false; } catch (ObjectDisposedException) { return true; } });
            require(elements.Count == 1 && order.Count == beforeCancel && Field("_drawingHealRequest") is null, "canceled worker finishes without late repair or history commit");
            require(((FrameworkElement)overlay.FindName("DrawingToolbar")).IsEnabled && Pixels(source).SequenceEqual(original), "canceled repair restores toolbar and never mutates screenshot source");
            File.WriteAllText(Path.Combine(directory, "healing-pixels.json"), JsonSerializer.Serialize(new { pixelWidth = repaired.PixelWidth, pixelHeight = repaired.PixelHeight,
                transparentMaskPixels = alpha.Count(a => a == 0), opaqueMaskPixels = alpha.Count(a => a == 255), blackPoint = Pixel(repaired, blackPoint), sentinel = Pixel(repaired, sentinelPoint) }));
        }
        finally
        {
            Invoke("CancelHealingOperation"); if (canvas.IsMouseCaptured) canvas.ReleaseMouseCapture();
            SetField("_drawHealWidth", originalWidth); SetField("_frame", originalFrame); Invoke("ApplyOverlaySnapshot", snapshot);
        }
    }
    private static byte[] Pixel(BitmapSource source, Point point) { var bytes = new byte[4]; source.CopyPixels(new Int32Rect((int)point.X, (int)point.Y, 1, 1), bytes, 4, 0); return bytes; }
    private static byte[] Pixels(BitmapSource source) { var bytes = new byte[source.PixelWidth * source.PixelHeight * 4]; source.CopyPixels(bytes, source.PixelWidth * 4, 0); return bytes; }
    private static void Save(BitmapSource image, string path) { var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(image)); using var output = File.Create(path); encoder.Save(output); }
}
