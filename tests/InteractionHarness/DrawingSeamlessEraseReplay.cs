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
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Image = System.Windows.Controls.Image;
using Point = System.Windows.Point;
using Size = System.Windows.Size;
using Panel = System.Windows.Controls.Panel;

/// <summary>Synthetic source, real creation/edit/history/export paths; no input injection.</summary>
internal static class DrawingSeamlessEraseReplay
{
    internal static void Verify(CaptureOverlayWindow overlay, object item, Action<bool, string> require, string directory)
    {
        const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public | BindingFlags.DeclaredOnly;
        object? Invoke(string name, params object?[] args) => typeof(CaptureOverlayWindow).GetMethod(name, flags)!.Invoke(overlay, args);
        object? Field(string name) => typeof(CaptureOverlayWindow).GetField(name, flags)!.GetValue(overlay);
        void SetField(string name, object? value) => typeof(CaptureOverlayWindow).GetField(name, flags)!.SetValue(overlay, value);
        T Property<T>(object value, string name) => (T)value.GetType().GetProperty(name)!.GetValue(value)!;
        var type = item.GetType(); var elements = Property<IList>(item, "DrawingElements"); var order = Property<IList>(item, "DrawingOrder");
        var canvas = Property<InkCanvas>(item, "Markup"); var raster = Property<InkCanvas>(item, "RasterLayer"); var snapshot = Invoke("CaptureOverlaySnapshot")!;
        var originalFrame = (CaptureFrame)Field("_frame")!; Guid id = default, backgroundId = default;
        Rect Crop() => (Rect)type.GetField("Bounds")!.GetValue(item)!;
        object Element() => elements.Cast<object>().Single(element => Property<Guid>(element, "Id") == id);
        Rect Bounds() { var element = Element(); return new(Property<double>(element, "X"), Property<double>(element, "Y"), Property<double>(element, "Width"), Property<double>(element, "Height")); }
        BitmapSource Cache() => Property<BitmapSource>(Element(), "Pixels");
        void Tool(string name) => Invoke("SetDrawTool", Enum.Parse(typeof(CaptureOverlayWindow).GetNestedType("DrawTool", BindingFlags.NonPublic)!, name));
        void Commit() { Invoke("CommitSelectedDrawingMove"); canvas.ReleaseMouseCapture(); }
        void Undo() => Invoke("DrawUndo", overlay, new RoutedEventArgs());
        void Redo() => Invoke("DrawRedo", overlay, new RoutedEventArgs());
        Point Center() { var bounds = Bounds(); return new(bounds.X + bounds.Width / 2, bounds.Y + bounds.Height / 2); }
        void Select() { Tool("Select"); require((bool)Invoke("BeginDrawingGesture", item, Center(), canvas, 1, canvas)! && Equals(Field("_selectedDrawingElementId"), id), "seamless erase Select targets the actual patch object"); }
        BitmapSource Clean() => (BitmapSource)Invoke("RenderSelectionImage", item, false, false, false)!;
        BitmapSource Export() { var clean = Clean(); return (BitmapSource)Invoke("RenderManualOverlay", item, clean.PixelWidth, clean.PixelHeight)!; }
        void Reframe(Rect bounds)
        {
            var before = Invoke("CaptureOverlaySnapshot")!;
            Invoke("SetSelectionBoundsPreservingManualContent", item, bounds); Invoke("InvalidateImageDerivedLayers", item); Invoke("UpdateSelection", item);
            overlay.UpdateLayout();
            Invoke("RecordGeometryOperationIfChanged", before, "seamless erase replay reframe");
        }
        object Background() => elements.Cast<object>().Single(element => Property<Guid>(element, "Id") == backgroundId);
        Rect BackgroundBounds() { var e = Background(); return new(Property<double>(e, "X"), Property<double>(e, "Y"), Property<double>(e, "Width"), Property<double>(e, "Height")); }
        void CheckMode(string stage) => require(Property<bool>(Element(), "IsLiftedContent") && !Property<bool>(Element(), "SeamlessErase") && Cache().IsFrozen,
            stage + " retains transparent foreground mode and immutable original pixels");
        try
        {
            Invoke("DrawClear", overlay, new RoutedEventArgs());
            var crop = Crop(); var width = originalFrame.Image.PixelWidth; var height = originalFrame.Image.PixelHeight;
            var cropPixels = (Int32Rect)Invoke("ToPixelRect", crop)!;
            var scaleX = cropPixels.Width / crop.Width; var scaleY = cropPixels.Height / crop.Height;
            var target = new Int32Rect(cropPixels.X + (int)Math.Ceiling(70 * scaleX), cropPixels.Y + (int)Math.Ceiling(60 * scaleY),
                (int)Math.Floor(60 * scaleX), (int)Math.Floor(30 * scaleY));
            var bytes = new byte[width * height * 4];
            for (var y = 0; y < height; y++) for (var x = 0; x < width; x++)
            {
                var at = (y * width + x) * 4; var blue = x >= cropPixels.X + 200 * scaleX;
                bytes[at] = blue ? (byte)200 : (byte)60; bytes[at + 1] = blue ? (byte)110 : (byte)150; bytes[at + 2] = blue ? (byte)45 : (byte)220; bytes[at + 3] = 255;
                if (x >= target.X && x < target.X + target.Width && y >= target.Y && y < target.Y + target.Height) bytes[at] = bytes[at + 1] = bytes[at + 2] = 0;
            }
            var source = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgra32, null, bytes, width * 4); source.Freeze();
            SetField("_frame", new CaptureFrame(originalFrame.OriginX, originalFrame.OriginY, source)); Invoke("UpdateSelection", item);
            var sourceBefore = Pixels(source); var countBefore = order.Count;
            var cleanBeforeErase = Clean();
            var targetCenter = new Int32Rect(target.X - cropPixels.X + target.Width / 2, target.Y - cropPixels.Y + target.Height / 2, 1, 1);
            var beforeTarget = new byte[4]; cleanBeforeErase.CopyPixels(targetCenter, beforeTarget, 4, 0);
            require(beforeTarget.Take(3).All(channel => channel == 0), "synthetic black target is actually inside the product's physical clean crop before erasing");
            var performanceRegion = new Int32Rect(32, 32, 1024, 512); var watch = Stopwatch.StartNew();
            var performancePreview = SeamlessEraseService.CreatePreviewPatch(source, performanceRegion); var previewMs = watch.Elapsed.TotalMilliseconds;
            watch.Restart(); var performancePatch = SeamlessEraseService.CreatePatch(source, performanceRegion); var commitMs = watch.Elapsed.TotalMilliseconds;
            watch.Restart(); var performanceContent = SeamlessEraseService.ExtractContent(source, performanceRegion, performancePatch); var extractMs = watch.Elapsed.TotalMilliseconds;
            require(performanceContent.IsFrozen, "large synthetic extraction returns immutable pixels");
            File.WriteAllText(Path.Combine(directory, "seamless-timing.json"), JsonSerializer.Serialize(new { previewMs, commitMs, extractMs, region = performanceRegion,
                previewWidth = performancePreview.PixelWidth, previewHeight = performancePreview.PixelHeight, commitWidth = performancePatch.PixelWidth, commitHeight = performancePatch.PixelHeight }, new JsonSerializerOptions { WriteIndented = true }));
            Tool("SeamlessErase");
            Invoke("BeginDrawingGesture", item, new Point(0, 0), canvas, 1, canvas);
            Invoke("UpdateSeamlessEraseDrawingPreview", item, new Point(crop.Width, crop.Height));
            Invoke("CommitSeamlessEraseDrawingPreview", item, new Point(crop.Width, crop.Height)); canvas.ReleaseMouseCapture();
            require(elements.Count == 0 && order.Count == countBefore && Field("_drawingSeamlessErasePreview") is null,
                "full-selection erase without external background fails without object, history or retained preview");
            require(Field("_seamlessEraseFailureTip") is System.Windows.Controls.ToolTip { IsOpen: true },
                "seamless failure feedback remains visible after mouse capture is released");
            require((bool)Invoke("BeginDrawingGesture", item, new Point(60, 50), canvas, 1, canvas)!, "seamless creation uses the normal gesture dispatcher");
            require(Field("_seamlessEraseFailureTip") is null, "the next seamless gesture dismisses old failure feedback");
            Invoke("UpdateSeamlessEraseDrawingPreview", item, new Point(140, 100));
            require(elements.Count == 0 && order.Count == countBefore && !canvas.Children.OfType<Image>().Any() && !raster.Children.OfType<Image>().Any(),
                "lift drag preview is a selection outline without an image patch or history");
            require(Pixels(source).SequenceEqual(sourceBefore), "lift outline preview leaves screenshot bytes unchanged");
            Invoke("CancelMosaicDrawingPreview"); canvas.ReleaseMouseCapture();
            require(elements.Count == 0 && order.Count == countBefore, "canceling seamless preview creates neither object nor history");
            Tool("SeamlessErase"); Invoke("BeginDrawingGesture", item, new Point(250, 150), canvas, 1, canvas);
            Invoke("UpdateSeamlessEraseDrawingPreview", item, new Point(300, 200)); Invoke("CommitSeamlessEraseDrawingPreview", item, new Point(300, 200)); canvas.ReleaseMouseCapture();
            require(elements.Count == 0 && order.Count == countBefore, "background-only selection creates no empty transparent pair");
            Tool("SeamlessErase");
            require((bool)Invoke("BeginDrawingGesture", item, new Point(60, 50), canvas, 1, canvas)!, "seamless creation can restart after cancellation");
            Invoke("UpdateSeamlessEraseDrawingPreview", item, new Point(140, 100));
            Invoke("CommitSeamlessEraseDrawingPreview", item, new Point(140, 100)); canvas.ReleaseMouseCapture();
            require(elements.Count == 2 && order.Count == countBefore + 1 && order[order.Count - 1]!.GetType().Name == "LiftDrawingAction", "lift commit records a background/content pair as one action");
            backgroundId = Property<Guid>(elements[0]!, "Id"); id = Property<Guid>(elements[1]!, "Id"); CheckMode("created foreground");
            require(Property<bool>(Background(), "SeamlessErase") && !Property<bool>(Background(), "IsLiftedContent"), "fixed background repair is below transparent content");
            require(Field("_drawTool")!.ToString() == "Select" && Equals(Field("_selectedDrawingElementId"), id), "lift automatically selects foreground in Select mode");
            var initialBounds = Bounds(); var initialCache = Cache(); var initialPixels = Pixels(initialCache);
            var backgroundBounds = BackgroundBounds(); var backgroundCache = Property<BitmapSource>(Background(), "Pixels");
            var alpha = initialPixels.Where((_, index) => index % 4 == 3).ToArray();
            require(alpha.Any(value => value == 0) && alpha.Any(value => value == 255), "extracted content has transparent padding and opaque real pixels");
            Undo(); require(elements.Count == 0, "one undo removes both lift layers");
            Redo(); require(elements.Count == 2 && Property<Guid>(elements[0]!, "Id") == backgroundId && Property<Guid>(elements[1]!, "Id") == id,
                "one redo restores the complete pair in correct order");
            require(ReferenceEquals(Cache(), initialCache) && ReferenceEquals(Property<BitmapSource>(Background(), "Pixels"), backgroundCache), "pair redo shares both frozen buffers");
            var visual = raster.Children.OfType<Image>().Single(image => Equals(image.Tag, id));
            require(ReferenceEquals(visual.Source, initialCache), "actual visible seamless image uses its committed pixel cache");
            var exported = Export(); var visible = RenderActualManualLayers(item, exported.PixelWidth, exported.PixelHeight);
            var exportedPixels = Pixels(exported); var visiblePixels = Pixels(visible);
            var mismatches = exportedPixels.Zip(visiblePixels).Count(pair => pair.First != pair.Second);
            File.WriteAllText(Path.Combine(directory, "seamless-pixel-parity.json"), JsonSerializer.Serialize(new { cacheWidth = initialCache.PixelWidth, cacheHeight = initialCache.PixelHeight,
                bounds = initialBounds, exportWidth = exported.PixelWidth, exportHeight = exported.PixelHeight, visibleWidth = visible.PixelWidth, visibleHeight = visible.PixelHeight, mismatches }));
            var saved = (BitmapSource)Invoke("RenderSelectionImage", item, true, false, false)!;
            var savedTarget = new byte[4]; saved.CopyPixels(targetCenter, savedTarget, 4, 0);
            require(savedTarget.Take(3).All(channel => channel <= 3), "initial physical export retains lifted content at its original position");
            var actualSaved = Pixels(saved); var expectedSaved = Pixels(cleanBeforeErase); var maximumBackgroundError = 0;
            var patchLeft = (int)Math.Ceiling(initialBounds.Left * scaleX) + 1; var patchTop = (int)Math.Ceiling(initialBounds.Top * scaleY) + 1;
            var patchRight = (int)Math.Floor(initialBounds.Right * scaleX) - 1; var patchBottom = (int)Math.Floor(initialBounds.Bottom * scaleY) - 1;
            for (var y = patchTop; y < patchBottom; y++) for (var x = patchLeft; x < patchRight; x++)
            {
                var at = (y * saved.PixelWidth + x) * 4;
                for (var channel = 0; channel < 3; channel++) maximumBackgroundError = Math.Max(maximumBackgroundError, Math.Abs(actualSaved[at + channel] - expectedSaved[at + channel]));
            }
            File.WriteAllText(Path.Combine(directory, "seamless-background-verification.json"), JsonSerializer.Serialize(new { cropPixels, target, targetCenter, beforeTarget, savedTarget,
                outputWidth = saved.PixelWidth, outputHeight = saved.PixelHeight, maximumBackgroundError }));
            SaveImage(saved, Path.Combine(directory, "seamless-erase-composite.png"));
            require(exported.PixelWidth == visible.PixelWidth && exported.PixelHeight == visible.PixelHeight && mismatches == 0,
                "manual export exactly matches actual visible seamless rendering at the same output scale");
            require(maximumBackgroundError <= 3, "initial lifted composition reproduces source content within three channel values");

            Select(); var center = Center(); var beforeMove = order.Count;
            Invoke("MoveSelectedDrawingObject", item, new Point(center.X + 160, center.Y + 40), canvas); Commit();
            var movedBounds = Bounds(); var movedCache = Cache();
            require(movedBounds.X == initialBounds.X + 160 && movedBounds.Y == initialBounds.Y + 40 && order.Count == beforeMove + 1, "explicit seamless move changes geometry in one action");
            require(ReferenceEquals(movedCache, initialCache), "foreground move retains original pixels without destination resampling"); CheckMode("moved foreground");
            require(BackgroundBounds() == backgroundBounds && ReferenceEquals(Property<BitmapSource>(Background(), "Pixels"), backgroundCache), "foreground move leaves background repair fixed");
            var movedSaved = (BitmapSource)Invoke("RenderSelectionImage", item, true, false, false)!;
            var exposed = new byte[4]; movedSaved.CopyPixels(targetCenter, exposed, 4, 0);
            require(exposed.Take(3).SequenceEqual(new byte[] { 60, 150, 220 }), "moving exposes reconstructed original orange background");
            var destinationPadding = new byte[4]; movedSaved.CopyPixels(new Int32Rect((int)((movedBounds.X + 3) * scaleX), (int)((movedBounds.Y + 3) * scaleY), 1, 1), destinationPadding, 4, 0);
            require(destinationPadding.Take(3).SequenceEqual(new byte[] { 200, 110, 45 }), "transparent foreground adopts blue destination without an orange rectangle");
            SaveImage(movedSaved, Path.Combine(directory, "seamless-lift-moved.png"));
            Undo(); require(Bounds() == initialBounds && ReferenceEquals(Cache(), initialCache), "move undo restores original seamless geometry and exact immutable cache"); CheckMode("move undo");
            Redo(); require(Bounds() == movedBounds && ReferenceEquals(Cache(), movedCache), "move redo restores destination seamless geometry and exact cache");
            Select(); Commit();
            var corner = ((IEnumerable)Field("_drawingObjectHandles")!).Cast<Point>().OrderByDescending(point => point.X + point.Y).First();
            require((bool)Invoke("TryBeginDrawingResize", item, corner, canvas)!, "seamless patch uses standard resize handles");
            Invoke("ResizeSelectedDrawingObjectWithConstraint", item, new Point(corner.X + 24, corner.Y + 18), canvas, false); Commit();
            var resizedBounds = Bounds(); var resizedCache = Cache();
            require(resizedBounds.Width == movedBounds.Width + 24 && resizedBounds.Height == movedBounds.Height + 18, "seamless resize changes both dimensions"); CheckMode("resized patch");
            require(ReferenceEquals(resizedCache, initialCache) && BackgroundBounds() == backgroundBounds, "foreground resize changes only geometry and leaves background fixed");
            Undo(); require(Bounds() == movedBounds && ReferenceEquals(Cache(), movedCache), "resize undo restores the smaller cached patch");
            Redo(); require(Bounds() == resizedBounds && ReferenceEquals(Cache(), resizedCache), "resize redo restores resampled cache");
            Select(); Commit(); require((bool)Invoke("DeleteSelectedDrawingObject")! && elements.Count == 1 && Property<Guid>(elements[0]!, "Id") == backgroundId, "Delete removes only foreground and keeps repair");
            Tool("Select"); Invoke("BeginDrawingGesture", item, new Point(backgroundBounds.X + 3, backgroundBounds.Y + 3), canvas, 1, canvas); Commit();
            require(!Equals(Field("_selectedDrawingElementId"), backgroundId), "fixed background repair is not selectable");
            Undo(); CheckMode("delete undo"); require(ReferenceEquals(Cache(), resizedCache), "delete undo restores cached pixels without resampling");
            Redo(); require(elements.Count == 1, "delete redo removes foreground while retaining repair"); Undo();

            var baseline = Pixels(Export()); var world = new Point(Bounds().X + crop.X, Bounds().Y + crop.Y);
            Reframe(new Rect(crop.X + 400, crop.Y + 300, crop.Width, crop.Height)); CheckMode("reframed patch");
            require(new Point(Bounds().X + Crop().X, Bounds().Y + Crop().Y) == world && ReferenceEquals(Cache(), resizedCache), "reframing keeps desktop anchor and does not resample hidden patch");
            Reframe(crop); require(Pixels(Export()).SequenceEqual(baseline), "reframing away and back restores exact seamless export pixels");
            var retained = Invoke("CaptureOverlaySnapshot")!; Invoke("DrawClear", overlay, new RoutedEventArgs());
            require(elements.Count == 0, "clear removes seamless patch");
            require(Property<long>(order[order.Count - 1]!, "RetainedBytes") >= (long)(resizedCache.PixelWidth * resizedCache.PixelHeight + backgroundCache.PixelWidth * backgroundCache.PixelHeight) * 4,
                "clear history accounts for both lift buffers in its memory budget");
            Undo(); CheckMode("clear undo");
            require(Pixels(Export()).SequenceEqual(baseline), "clear undo retains seamless patch export");
            Invoke("ApplyOverlaySnapshot", retained); CheckMode("snapshot restore");
            require(ReferenceEquals(Cache(), resizedCache) && Pixels(Export()).SequenceEqual(baseline), "snapshot restore shares frozen seamless cache and preserves export");
            Undo(); CheckMode("older history after snapshot"); require(elements.Count == 2, "older drawing history remains usable after snapshot restore");

            Redo(); overlay.UpdateLayout(); var beforeOrdinary = Cache(); var layerBounds = Bounds();
            Tool("Mosaic"); Invoke("BeginDrawingGesture", item, new Point(layerBounds.X + 20, layerBounds.Y + 10), canvas, 1, canvas);
            var mosaicEnd = new Point(layerBounds.Right + 20, layerBounds.Bottom + 10);
            Invoke("UpdateMosaicDrawingPreview", item, mosaicEnd); Invoke("CommitMosaicDrawingPreview", item, mosaicEnd); canvas.ReleaseMouseCapture();
            var ordinary = elements.Cast<object>().Single(element => Property<Guid>(element, "Id") != id && Property<Guid>(element, "Id") != backgroundId);
            require(!Property<bool>(ordinary, "SeamlessErase") && !Property<bool>(ordinary, "IsLiftedContent"), "ordinary mosaic and lift retain distinct modes on the same canvas");
            var layerIds = elements.Cast<object>().Select(element => Property<Guid>(element, "Id")).ToArray(); var occluded = Pixels(Export());
            Tool("Select"); Invoke("BeginDrawingGesture", item, new Point(layerBounds.X + 3, layerBounds.Y + 3), canvas, 1, canvas); Commit();
            require(Equals(Field("_selectedDrawingElementId"), id) && (bool)Invoke("DeleteSelectedDrawingObject")!, "exposed foreground edge permits deleting the lower overlapping layer");
            Undo(); require(elements.Cast<object>().Select(element => Property<Guid>(element, "Id")).SequenceEqual(layerIds) && Pixels(Export()).SequenceEqual(occluded),
                "delete undo restores original layer index and exact occlusion beneath later objects");
            Redo(); Undo(); require(Pixels(Export()).SequenceEqual(occluded), "repeated deletion preserves overlap order");
            Undo(); require(elements.Count == 2 && ReferenceEquals(Cache(), beforeOrdinary), "ordinary mosaic undo leaves paired lift pixels intact");
            var pairCount = order.Count; Tool("SeamlessErase"); Invoke("BeginDrawingGesture", item, new Point(60, 50), canvas, 1, canvas);
            Invoke("UpdateSeamlessEraseDrawingPreview", item, new Point(140, 100)); Invoke("CommitSeamlessEraseDrawingPreview", item, new Point(140, 100)); canvas.ReleaseMouseCapture();
            require(elements.Count == 2 && order.Count == pairCount, "reselecting repaired original location does not resurrect source text");
            var current = Bounds(); Tool("SeamlessErase"); Invoke("BeginDrawingGesture", item, current.TopLeft, canvas, 1, canvas);
            Invoke("UpdateSeamlessEraseDrawingPreview", item, current.BottomRight); Invoke("CommitSeamlessEraseDrawingPreview", item, current.BottomRight); canvas.ReleaseMouseCapture();
            require(elements.Count == 4 && order.Count == pairCount + 1, "second lift captures currently visible moved content as an independent pair");
            Undo(); require(elements.Count == 2 && ReferenceEquals(Cache(), beforeOrdinary), "one undo removes only the newest complete pair");
            Redo(); require(elements.Count == 4, "redo restores second pair without disturbing the older pair");
            var pairSnapshot = Invoke("CaptureOverlaySnapshot")!; Invoke("DrawClear", overlay, new RoutedEventArgs()); Undo();
            require(elements.Count == 4, "clear undo restores multiple paired layers");
            Invoke("ApplyOverlaySnapshot", pairSnapshot); Undo(); require(elements.Count == 2, "snapshot restores grouped Lift history with live layer identities");
            Redo(); require(elements.Count == 4, "snapshot grouped redo restores both layer identities exactly once");
            SaveImage((BitmapSource)Invoke("RenderSelectionImage", item, true, false, false)!, Path.Combine(directory, "seamless-lift-composite.png"));
            require(Pixels(source).SequenceEqual(sourceBefore), "seamless drawing and history never mutate original screenshot bytes");
        }
        finally
        {
            Invoke("CancelMosaicDrawingPreview"); if (canvas.IsMouseCaptured) canvas.ReleaseMouseCapture();
            SetField("_frame", originalFrame); Invoke("ApplyOverlaySnapshot", snapshot);
        }
    }

    private static byte[] Pixels(BitmapSource image) { var bytes = new byte[image.PixelWidth * image.PixelHeight * 4]; image.CopyPixels(bytes, image.PixelWidth * 4, 0); return bytes; }
    private static BitmapSource RenderActualManualLayers(object item, int pixelWidth, int pixelHeight)
    {
        var type = item.GetType(); var markup = (InkCanvas)type.GetProperty("Markup")!.GetValue(item)!;
        var raster = (InkCanvas)type.GetProperty("RasterLayer")!.GetValue(item)!;
        var region = (Canvas?)type.GetField("RegionMarkLayer")!.GetValue(item);
        var parent = (Panel)VisualTreeHelper.GetParent(markup);
        var layers = parent.Children.OfType<FrameworkElement>().Where(child => ReferenceEquals(child, markup) || ReferenceEquals(child, raster) || ReferenceEquals(child, region))
            .Select(child => (Child: child, Index: parent.Children.IndexOf(child))).ToArray();
        var size = new Size(markup.Width, markup.Height); var renderHost = new Grid { Width = size.Width, Height = size.Height };
        foreach (var layer in layers) parent.Children.Remove(layer.Child);
        try
        {
            foreach (var layer in layers) renderHost.Children.Add(layer.Child);
            renderHost.Measure(size); renderHost.Arrange(new Rect(size)); renderHost.UpdateLayout();
            var rendered = new RenderTargetBitmap(pixelWidth, pixelHeight, 96 * pixelWidth / size.Width, 96 * pixelHeight / size.Height, PixelFormats.Pbgra32); rendered.Render(renderHost); rendered.Freeze(); return rendered;
        }
        finally { renderHost.Children.Clear(); foreach (var layer in layers) parent.Children.Insert(layer.Index, layer.Child); parent.UpdateLayout(); }
    }
    private static void SaveImage(BitmapSource bitmap, string path)
    {
        var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(bitmap)); using var output = File.Create(path); encoder.Save(output);
    }
}
