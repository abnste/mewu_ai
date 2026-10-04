// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Security.Cryptography;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application = System.Windows.Application;
using Brushes = System.Windows.Media.Brushes;
using FlowDirection = System.Windows.FlowDirection;
using Point = System.Windows.Point;
using Size = System.Windows.Size;
using Panel = System.Windows.Controls.Panel;

/// <summary>Synthetic, no-window combination replay through real lift and history methods.</summary>
internal static class DrawingHighlightLiftReplay
{
    private const BindingFlags Flags = BindingFlags.Instance | BindingFlags.Static | BindingFlags.NonPublic | BindingFlags.Public;
    private const int Width = 640, Height = 288;
    private static readonly Rect LiftBounds = new(100, 152, 320, 64);
    private static readonly Vector MoveDelta = new(0, -112);

    internal static void Run()
    {
        var directory = ReplayOutputDirectory.PrepareWorkingDirectory("highlight-lift");
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(directory, "logs"));
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        app.Resources.MergedDictionaries.Add(new ResourceDictionary { Source = new Uri("/MewuAI;component/Themes/LightTheme.xaml", UriKind.Relative) });
        using var host = new AppHost(app, null, "MewuAI-HighlightLift-" + Guid.NewGuid().ToString("N"));
        host.Settings.EnableVoiceInput = false;
        host.Settings.AutomaticallyStartListening = false;
        host.Settings.SaveConversationHistory = false;
        var checks = new List<string>(); var failures = new List<string>(); var evidence = new List<object>();
        var originalFocus = Keyboard.FocusedElement; var originalCapture = Mouse.Captured;
        void Check(bool value, string label) { checks.Add(label); if (!value) failures.Add(label); }
        try
        {
            foreach (var scale in new[] { 1d, 1.75d, 2d })
            foreach (var kind in new[] { "region", "stroke" })
            foreach (var highlightFirst in new[] { true, false })
            {
                var name = $"{kind}-{(highlightFirst ? "highlight-first" : "lift-first")}-{(int)(96 * scale)}";
                var output = Path.Combine(directory, name); Directory.CreateDirectory(output);
                try { RunCase(host, scale, kind, highlightFirst, output, (ok, label) => Check(ok, name + " " + label), evidence); }
                catch (Exception error) { failures.Add(name + ": " + error); }
                WriteResult();
            }
            Check(ReferenceEquals(originalFocus, Keyboard.FocusedElement), "no system keyboard focus change");
            Check(ReferenceEquals(originalCapture, Mouse.Captured), "no mouse capture change");
        }
        finally { app.Shutdown(); WriteResult(); }
        Environment.ExitCode = failures.Count == 0 ? 0 : 1;

        void WriteResult() => File.WriteAllText(Path.Combine(directory, "result.json"), JsonSerializer.Serialize(new
        {
            passed = failures.Count == 0, checks, failures, evidence,
            productSha256 = Hash(typeof(AppHost).Assembly.Location), harnessSha256 = Hash(typeof(DrawingHighlightLiftReplay).Assembly.Location),
            syntheticOnly = true, shownWindows = false, inputInjected = false, clipboardUsed = false, productionSettingsUsed = false,
            gestureBoundary = "Real lift Begin/Update/Commit and object move/commit; legal private gesture state seeded without mouse capture."
        }, new JsonSerializerOptions { WriteIndented = true }));
    }

    private static void RunCase(AppHost host, double scale, string kind, bool highlightFirst, string directory, Action<bool, string> check, List<object> evidence)
    {
        var (source, glyph) = CreateSource(scale); var original = Pixels(source);
        using var session = new Session(host, source, directory);
        var overlay = session.Overlay; var item = session.Item; var markup = session.Markup;
        var elements = Get<IList>(item, "DrawingElements"); var order = Get<IList>(item, "DrawingOrder");
        var clean = (BitmapSource)session.Invoke("RenderSelectionImage", item, false, false, false)!;
        var fullSource = clean.PixelWidth == source.PixelWidth && clean.PixelHeight == source.PixelHeight && Pixels(clean).SequenceEqual(original);
        check(fullSource, "clean selection is the complete synthetic source with identical pixels");
        if (!fullSource) throw new InvalidOperationException("Synthetic layout does not map the full source into its selection; see layout.json.");
        void Call(string method, params object?[] arguments) => session.Invoke(method, arguments);
        BitmapSource Saved() => (BitmapSource)session.Invoke("RenderSelectionImage", item, true, false, false)!;
        void Undo() => Call("DrawUndo", overlay, new RoutedEventArgs());
        void Redo() => Call("DrawRedo", overlay, new RoutedEventArgs());
        void BeginLift()
        {
            session.Set("_drawStart", LiftBounds.TopLeft);
            Call("BeginSeamlessEraseDrawingPreview", item);
            Call("UpdateSeamlessEraseDrawingPreview", item, LiftBounds.BottomRight);
        }
        void CommitLift() => Call("CommitSeamlessEraseDrawingPreview", item, LiftBounds.BottomRight);
        void AddHighlight()
        {
            session.Set("_drawColor", Colors.Red);
            if (kind == "region") { Call("AddRegionMark", item, new Rect(40, 24, 560, 224)); return; }
            var protection = (BackgroundHighlightSource)session.Invoke("GetBackgroundHighlightSource", item)!;
            var bounds = (Rect)session.Invoke("BackgroundHighlightSourceBounds", item)!;
            var stroke = new BackgroundHighlightStroke(new StylusPointCollection { new StylusPoint(100, 136), new StylusPoint(540, 136) },
                new DrawingAttributes { Color = Colors.Red, Width = 224, Height = 224, FitToCurve = false, IsHighlighter = false }, protection, bounds);
            markup.Strokes.Add(stroke);
            // The same committed preview state as BeginBackgroundHighlightPreview, omitting only its CaptureMouse call.
            var previewField = typeof(CaptureOverlayWindow).GetField("_backgroundHighlightPreview", Flags)!;
            var tuple = Activator.CreateInstance(Nullable.GetUnderlyingType(previewField.FieldType)!, item, stroke);
            previewField.SetValue(overlay, tuple);
            Call("CommitBackgroundHighlightPreview", item, new Point(540, 136));
        }

        Save(source, Path.Combine(directory, "00-source.png"));
        if (highlightFirst) AddHighlight();
        var before = Saved(); Save(before, Path.Combine(directory, "01-before-lift.png")); var beforeOrder = order.Count;
        BeginLift();
        check(elements.Count == 0 && order.Count == beforeOrder, "outline preview creates no raster or history");
        Call("CancelMosaicDrawingPreview");
        check(Pixels(Saved()).SequenceEqual(Pixels(before)) && session.Field("_drawingSeamlessErasePreview") is null, "cancel restores exact pre-lift composition");
        BeginLift(); CommitLift();
        check(elements.Count == 2 && order.Count == beforeOrder + 1 && order[order.Count - 1]!.GetType().Name == "LiftDrawingAction", "real commit adds one paired history action");
        if (elements.Count != 2) throw new InvalidOperationException("Real lift did not produce its background and foreground pair.");
        var background = elements[0]!; var content = elements[1]!;
        var id = Get<Guid>(content, "Id"); var backgroundId = Get<Guid>(background, "Id");
        var cached = Get<BitmapSource>(content, "Pixels"); var repair = Get<BitmapSource>(background, "Pixels");
        var boundsBefore = Bounds(content); var backgroundBounds = Bounds(background);
        check(Get<bool>(content, "IsLiftedContent") && Get<bool>(background, "SeamlessErase"), "pair identifies transparent content and fixed repair");
        check(Equals(session.Field("_selectedDrawingElementId"), id) && session.Field("_drawTool")!.ToString() == "Select", "commit selects the foreground for editing");
        var cachedPixels = Pixels(cached);
        var coloredForeground = 0; var transparent = 0; var opaque = 0;
        for (var at = 0; at < cachedPixels.Length; at += 4)
        {
            if (cachedPixels[at + 3] == 0) { transparent++; continue; }
            if (cachedPixels[at + 3] == 255) opaque++;
            if (Math.Max(cachedPixels[at], Math.Max(cachedPixels[at + 1], cachedPixels[at + 2])) - Math.Min(cachedPixels[at], Math.Min(cachedPixels[at + 1], cachedPixels[at + 2])) > 1) coloredForeground++;
        }
        check(cached.IsFrozen && transparent > 0 && opaque > 0 && coloredForeground == 0, "black foreground remains transparent and has no baked red highlight");
        Undo(); check(elements.Count == 0 && Pixels(Saved()).SequenceEqual(Pixels(before)), "one creation undo removes the whole pair and restores prior pixels");
        Redo(); check(elements.Count == 2 && Get<Guid>(elements[0]!, "Id") == backgroundId && Get<Guid>(elements[1]!, "Id") == id, "creation redo restores pair order");
        if (!highlightFirst) AddHighlight();
        var created = Saved(); Save(created, Path.Combine(directory, "02-created.png"));
        check(order.Count == 2, "combined setup has exactly one highlighter and one lift action");

        // Seed only the pre-capture selection state; all movement, constraints, history and redraw are product code.
        content = elements.Cast<object>().Single(value => Get<Guid>(value, "Id") == id);
        var center = new Point(boundsBefore.X + boundsBefore.Width / 2, boundsBefore.Y + boundsBefore.Height / 2);
        session.Set("_selectedDrawingElementId", id); session.Set("_drawingMoveOriginalElement", content);
        session.Set("_drawingMovePointerStart", center);
        Call("MoveSelectedDrawingObject", item, center + MoveDelta * .5, markup);
        var sourceAfterFirstMove = session.Invoke("GetBackgroundHighlightSource", item);
        Call("MoveSelectedDrawingObject", item, center + MoveDelta, markup);
        var sourceReusedDuringMove = ReferenceEquals(sourceAfterFirstMove, session.Invoke("GetBackgroundHighlightSource", item));
        check(sourceReusedDuringMove, "successive drag updates reuse the same derived highlight source without an export refresh");
        // Capture actual presentation before both commit and export: export refreshes
        // derived highlight sources and must not conceal stale pixels during a drag.
        var duringMove = RenderLive(session, source.PixelWidth, source.PixelHeight);
        Save(duringMove, Path.Combine(directory, "03-during-move-live.png"));
        var duringMoveExport = Saved(); Save(duringMoveExport, Path.Combine(directory, "03-during-move-export.png"));
        var duringMoveExportError = Pixels(duringMove).Zip(Pixels(duringMoveExport)).Max(pair => Math.Abs(pair.First - pair.Second));
        check(duringMoveExportError <= 2, "during move export agrees with the already captured live layer order");
        Call("CommitSelectedDrawingMove");
        var rasterLayer = Get<Panel>(item, "RasterLayer"); var previewLayer = Get<Panel>(item, "RasterPreviewLayer");
        check(session.Field("_rasterObjectDrawingPreview") is null && previewLayer.Children.Count == 0 &&
            rasterLayer.Children.OfType<FrameworkElement>().Any(child => Equals(child.Tag, id)),
            "move commit clears transient preview state and returns the foreground to the raster layer");
        content = elements.Cast<object>().Single(value => Get<Guid>(value, "Id") == id);
        background = elements.Cast<object>().Single(value => Get<Guid>(value, "Id") == backgroundId);
        check(Bounds(content).TopLeft == boundsBefore.TopLeft + MoveDelta && order.Count == 3, "real move commits the requested translation exactly once");
        check(ReferenceEquals(Get<BitmapSource>(content, "Pixels"), cached) && ReferenceEquals(Get<BitmapSource>(background, "Pixels"), repair) && Bounds(background) == backgroundBounds,
            "moving keeps foreground pixels immutable and repair fixed");
        var live = RenderLive(session, source.PixelWidth, source.PixelHeight); Save(live, Path.Combine(directory, "04-moved-live.png"));
        var moved = Saved(); Save(moved, Path.Combine(directory, "03-moved-export.png"));
        var a = Pixels(moved); var b = Pixels(live); var parityError = a.Zip(b).Max(pair => Math.Abs(pair.First - pair.Second));
        var during = Pixels(duringMove);
        check(parityError <= 2, "live layers and saved composition agree within WPF source-over quantization");
        var sampleX = (int)(480 * scale); var sampleY = (int)(180 * scale); var reference = Pixel(moved, sampleX, sampleY);
        check(reference[1] < 200 && reference[2] > reference[1] + 40, "reference background visibly receives red highlight");
        var originError = 0; var duringMoveOriginError = 0;
        for (var y = (int)(LiftBounds.Top * scale) + 3; y < (int)(LiftBounds.Bottom * scale) - 3; y++)
        for (var x = (int)(LiftBounds.Left * scale) + 3; x < (int)(LiftBounds.Right * scale) - 3; x++)
        for (var c = 0; c < 3; c++)
        {
            var at = (y * moved.PixelWidth + x) * 4 + c;
            originError = Math.Max(originError, Math.Abs(a[at] - reference[c]));
            duringMoveOriginError = Math.Max(duringMoveOriginError, Math.Abs(during[at] - reference[c]));
        }
        check(originError <= 2, "entire vacated rectangle is continuously highlighted without old text or a pale block");
        check(duringMoveOriginError <= 2, "during move before commit or export the vacated rectangle has no text ghost or pale block");
        var coreCount = 0; var coreError = 0; var duringMoveCoreError = 0; var shiftX = (int)(MoveDelta.X * scale); var shiftY = (int)(MoveDelta.Y * scale);
        for (var y = (int)(LiftBounds.Top * scale); y < (int)(LiftBounds.Bottom * scale); y++)
        for (var x = (int)(LiftBounds.Left * scale); x < (int)(LiftBounds.Right * scale); x++)
        {
            var at = (y * moved.PixelWidth + x) * 4; if (glyph[at + 3] != 255) continue;
            coreCount++; var destination = ((y + shiftY) * moved.PixelWidth + x + shiftX) * 4;
            for (var c = 0; c < 3; c++)
            {
                coreError = Math.Max(coreError, a[destination + c]);
                duringMoveCoreError = Math.Max(duringMoveCoreError, during[destination + c]);
            }
        }
        check(coreCount > 100 && coreError == 0, "moved real text cores remain black at the highlighted destination");
        check(coreCount > 100 && duringMoveCoreError == 0, "during move before commit or export destination text cores remain exactly black");
        var padding = Pixel(moved, (int)((boundsBefore.Left + 8) * scale), (int)((boundsBefore.Top + MoveDelta.Y + 8) * scale));
        check(padding.Take(3).Zip(reference).All(pair => Math.Abs(pair.First - pair.Second) <= 2), "transparent destination padding exposes continuous highlight");
        Undo(); check(Pixels(Saved()).SequenceEqual(Pixels(created)), "move undo restores exact combined pixels");
        Redo(); check(Pixels(Saved()).SequenceEqual(a), "move redo restores exact combined pixels");
        session.Set("_selectedDrawingElementId", id);
        check((bool)session.Invoke("DeleteSelectedDrawingObject")! && elements.Count == 1, "Delete removes only lifted foreground");
        var deleted = Saved(); Save(deleted, Path.Combine(directory, "05-deleted.png"));
        var deletedPixels = Pixels(deleted); var remainingText = 0;
        for (var y = (int)(45 * scale); y < (int)(210 * scale); y++)
        for (var x = (int)(110 * scale); x < (int)(410 * scale); x++)
            if (deletedPixels[(y * deleted.PixelWidth + x) * 4 + 2] < 100) remainingText++;
        check(remainingText == 0, "deleting content leaves no original or destination text ghost");
        Undo(); check(Pixels(Saved()).SequenceEqual(a), "delete undo restores original z-order and pixels");
        Redo(); check(Pixels(Saved()).SequenceEqual(deletedPixels), "delete redo reproduces clean highlighted background"); Undo();
        var retainedCount = order.Count;
        BeginLift(); CommitLift();
        check(elements.Count == 2 && order.Count == retainedCount, "lifting vacated background neither resurrects old text nor bakes highlight into a new pair");
        Call("DrawClear", overlay, new RoutedEventArgs()); Undo();
        check(Pixels(Saved()).SequenceEqual(a), "clear undo restores the composed highlighter and both lift layers");
        check(Pixels(source).SequenceEqual(original), "all edits leave original screenshot bytes unchanged");
        // Later raster records must not steal hits from editable labels visually above them.
        foreach (var numbered in new[] { false, true })
        {
            var annotationId = Guid.NewGuid(); var overlap = Bounds(content);
            var annotationType = typeof(CaptureOverlayWindow).GetNestedType(numbered ? "NumberDrawingElement" : "TextDrawingElement", BindingFlags.NonPublic)!;
            var annotation = numbered
                ? Activator.CreateInstance(annotationType, annotationId, overlap.X, overlap.Y, 40d, 7, Colors.Blue)!
                : Activator.CreateInstance(annotationType, annotationId, overlap.X, overlap.Y, 100d, "editable", "Microsoft YaHei UI", 22d, Colors.Blue, false)!;
            elements.Insert(0, annotation); Call("RebuildDrawingElements", item);
            var hit = session.Invoke("HitTestDrawingElement", item, new Point(overlap.X + 12, overlap.Y + 12));
            check(hit is not null && Get<Guid>(hit, "Id") == annotationId, (numbered ? "number" : "text") + " above a later raster layer keeps hit-test priority");
            elements.Remove(annotation); Call("RebuildDrawingElements", item);
        }
        if (scale == 1 && kind == "region" && highlightFirst)
        {
            var overlap = Bounds(content); session.Set("_drawStart", overlap.TopLeft);
            Call("BeginMosaicDrawingPreview", item); Call("UpdateMosaicDrawingPreview", item, overlap.BottomRight);
            Call("CommitMosaicDrawingPreview", item, overlap.BottomRight);
            check(previewLayer.Children.Count == 0 && session.Field("_drawingMosaicPreview") is null,
                "ordinary mosaic commit removes all temporary preview visuals and state");
            var occluded = Saved(); Save(occluded, Path.Combine(directory, "06-later-mosaic.png"));
            var occludedPixels = Pixels(occluded); var ids = elements.Cast<object>().Select(value => Get<Guid>(value, "Id")).ToArray();
            check(elements.Count == 3 && !occludedPixels.SequenceEqual(a), "later ordinary mosaic visibly occludes the lifted foreground");
            session.Set("_selectedDrawingElementId", id); Call("DeleteSelectedDrawingObject"); Undo();
            check(elements.Cast<object>().Select(value => Get<Guid>(value, "Id")).SequenceEqual(ids) && Pixels(Saved()).SequenceEqual(occludedPixels),
                "undo deleting a lower foreground restores its exact index beneath the later mosaic");
            var occludedLive = Pixels(RenderLive(session, occluded.PixelWidth, occluded.PixelHeight));
            check(occludedPixels.Zip(occludedLive).Max(pair => Math.Abs(pair.First - pair.Second)) <= 2, "ordinary mosaic overlap uses the same live and export order");
            Undo(); check(Pixels(Saved()).SequenceEqual(a), "undo later mosaic exposes the unchanged lifted text and highlight");
        }
        check(new WindowInteropHelper(overlay).Handle == IntPtr.Zero && !overlay.IsLoaded && !overlay.IsVisible && !markup.IsMouseCaptureWithin,
            "no window or mouse capture was created");
        SaveComparison([source, created, moved, deleted], Path.Combine(directory, "comparison.png"));
        evidence.Add(new { scenario = Path.GetFileName(directory), scale, kind, highlightFirst, sourceReusedDuringMove, parityError, duringMoveExportError, originError, duringMoveOriginError, coreCount, coreError, duringMoveCoreError, remainingText,
            coloredForeground, transparent, opaque, foregroundBounds = Bounds(content), physicalWidth = moved.PixelWidth, physicalHeight = moved.PixelHeight });
    }

    private sealed class Session : IDisposable
    {
        internal CaptureOverlayWindow Overlay { get; }
        internal object Item { get; }
        internal InkCanvas Markup { get; }
        internal Session(AppHost host, BitmapSource source, string directory)
        {
            Overlay = new CaptureOverlayWindow(host, null, new CaptureFrame(0, 0, source));
            var root = (Canvas)Overlay.FindName("Root"); root.Width = Width; root.Height = Height;
            root.Measure(new Size(Width, Height)); root.Arrange(new Rect(0, 0, Width, Height)); root.UpdateLayout();
            Item = Invoke("CreateSelection", false)!;
            ((IList)Field("_selections")!).Add(Item); Set("_activeIndex", 0);
            Item.GetType().GetField("Bounds", Flags)!.SetValue(Item, new Rect(0, 0, Width, Height));
            Invoke("UpdateSelection", Item); Markup = Get<InkCanvas>(Item, "Markup");
            var selection = Get<Grid>(Item, "Host"); selection.Measure(new Size(Width, Height)); selection.Arrange(new Rect(0, 0, Width, Height)); selection.UpdateLayout();
            var before = Layout();
            // A Window's DPI/layout callbacks use the real virtual desktop dimensions.
            // Detach its actual content for this no-HWND synthetic layout, so child
            // UpdateLayout cannot make ToPixelRect crop against the physical desktop.
            Overlay.Content = null;
            root.Width = Width; root.Height = Height;
            root.Measure(new Size(Width, Height)); root.Arrange(new Rect(0, 0, Width, Height)); root.UpdateLayout();
            Invoke("UpdateSelection", Item);
            selection.Measure(new Size(Width, Height)); selection.Arrange(new Rect(0, 0, Width, Height)); selection.UpdateLayout();
            File.WriteAllText(Path.Combine(directory, "layout.json"), JsonSerializer.Serialize(new { before, after = Layout(), sourceWidth = source.PixelWidth, sourceHeight = source.PixelHeight },
                new JsonSerializerOptions { WriteIndented = true }));

            object Layout() => new { rootWidth = root.Width, rootHeight = root.Height, rootActualWidth = root.ActualWidth, rootActualHeight = root.ActualHeight,
                windowWidth = Overlay.Width, windowHeight = Overlay.Height, windowActualWidth = Overlay.ActualWidth, windowActualHeight = Overlay.ActualHeight,
                pixelRect = (Int32Rect)Invoke("ToPixelRect", new Rect(0, 0, Width, Height))!, hwnd = new WindowInteropHelper(Overlay).Handle.ToInt64(), windowLoaded = Overlay.IsLoaded };
        }
        internal object? Invoke(string name, params object?[] args) => typeof(CaptureOverlayWindow).GetMethod(name, Flags)!.Invoke(Overlay, args);
        internal object? Field(string name) => typeof(CaptureOverlayWindow).GetField(name, Flags)!.GetValue(Overlay);
        internal void Set(string name, object? value) => typeof(CaptureOverlayWindow).GetField(name, Flags)!.SetValue(Overlay, value);
        public void Dispose() { Invoke("CancelMosaicDrawingPreview"); Overlay.Close(); }
    }

    private static BitmapSource RenderLive(Session session, int width, int height)
    {
        var host = Get<Grid>(session.Item, "Host"); var parent = (Panel)VisualTreeHelper.GetParent(host); var index = parent.Children.IndexOf(host);
        var hidden = new[] { Get<FrameworkElement>(session.Item, "Outline"), session.Field("_drawingSelectionOutline") as FrameworkElement }.OfType<FrameworkElement>().Distinct().Select(value => (Value: value, value.Visibility)).ToArray();
        var root = new Grid { Width = Width, Height = Height };
        foreach (var value in hidden) value.Value.Visibility = Visibility.Collapsed;
        parent.Children.Remove(host);
        try
        {
            root.Children.Add(host); root.Measure(new Size(Width, Height)); root.Arrange(new Rect(0, 0, Width, Height)); root.UpdateLayout();
            var result = new RenderTargetBitmap(width, height, 96d * width / Width, 96d * height / Height, PixelFormats.Pbgra32);
            result.Render(root); result.Freeze(); return result;
        }
        finally
        {
            root.Children.Remove(host); parent.Children.Insert(index, host);
            foreach (var value in hidden) value.Value.Visibility = value.Visibility;
            parent.UpdateLayout();
        }
    }

    private static (BitmapSource Source, byte[] Glyph) CreateSource(double scale)
    {
        var width = (int)(Width * scale); var height = (int)(Height * scale);
        var visual = new DrawingVisual();
        using (var drawing = visual.RenderOpen())
        {
            drawing.PushTransform(new ScaleTransform(scale, scale));
            drawing.DrawText(new FormattedText("提取文字 Lift 123", CultureInfo.InvariantCulture, FlowDirection.LeftToRight,
                new Typeface("Microsoft YaHei UI"), 24, Brushes.Black, scale), new Point(120, 166)); drawing.Pop();
        }
        var glyph = new RenderTargetBitmap(width, height, 96, 96, PixelFormats.Pbgra32); glyph.Render(visual);
        var alpha = Pixels(glyph); var data = new byte[width * height * 4];
        for (var at = 0; at < data.Length; at += 4)
        {
            for (var c = 0; c < 3; c++) data[at + c] = (byte)(alpha[at + c] + (243 * (255 - alpha[at + 3]) + 127) / 255);
            data[at + 3] = 255;
        }
        var source = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgra32, null, data, width * 4); source.Freeze(); return (source, alpha);
    }

    private static T Get<T>(object value, string name) => (T)(value.GetType().GetProperty(name, Flags)?.GetValue(value) ?? value.GetType().GetField(name, Flags)!.GetValue(value))!;
    private static Rect Bounds(object value) => new(Get<double>(value, "X"), Get<double>(value, "Y"), Get<double>(value, "Width"), Get<double>(value, "Height"));
    private static byte[] Pixels(BitmapSource value) { var result = new byte[value.PixelWidth * value.PixelHeight * 4]; value.CopyPixels(result, value.PixelWidth * 4, 0); return result; }
    private static byte[] Pixel(BitmapSource value, int x, int y) { var result = new byte[4]; value.CopyPixels(new Int32Rect(x, y, 1, 1), result, 4, 0); return result; }
    private static string Hash(string path) { using var input = File.OpenRead(path); return Convert.ToHexString(SHA256.HashData(input)); }
    private static void Save(BitmapSource value, string path) { var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(value)); using var output = File.Create(path); encoder.Save(output); }
    private static void SaveComparison(BitmapSource[] images, string path)
    {
        var width = images.Max(image => image.PixelWidth); var height = images.Sum(image => image.PixelHeight + 12);
        var visual = new DrawingVisual(); using (var drawing = visual.RenderOpen())
        {
            drawing.DrawRectangle(Brushes.White, null, new Rect(0, 0, width, height)); var y = 0;
            foreach (var image in images) { drawing.DrawImage(image, new Rect(0, y, image.PixelWidth, image.PixelHeight)); y += image.PixelHeight + 12; }
        }
        var output = new RenderTargetBitmap(width, height, 96, 96, PixelFormats.Pbgra32); output.Render(visual); Save(output, path);
    }
}
