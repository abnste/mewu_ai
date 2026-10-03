// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Color = System.Windows.Media.Color;
using Point = System.Windows.Point;
using TextBox = System.Windows.Controls.TextBox;

/// <summary>Real product editing/history methods on patterned synthetic pixels; no global input.</summary>
internal static class DrawingRegionMarkReplay
{
    internal static async Task Verify(CaptureOverlayWindow overlay, object item, Action<bool, string> require)
    {
        const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public | BindingFlags.DeclaredOnly;
        object? Invoke(string name, params object?[] args) => typeof(CaptureOverlayWindow).GetMethod(name, flags)!.Invoke(overlay, args);
        object? Field(string name) => typeof(CaptureOverlayWindow).GetField(name, flags)!.GetValue(overlay);
        void SetField(string name, object? value) => typeof(CaptureOverlayWindow).GetField(name, flags)!.SetValue(overlay, value);
        T Property<T>(object value, string name) => (T)value.GetType().GetProperty(name)!.GetValue(value)!;
        var itemType = item.GetType();
        var marks = (IList)itemType.GetField("RegionMarks")!.GetValue(item)!;
        var order = (IList)itemType.GetProperty("DrawingOrder")!.GetValue(item)!;
        var canvas = (InkCanvas)itemType.GetProperty("Markup")!.GetValue(item)!;
        var snapshot = Invoke("CaptureOverlaySnapshot")!;
        var originalFrame = (CaptureFrame)Field("_frame")!;
        Task? oldMatching = null;
        Guid markId = default;
        object Mark() => marks.Cast<object>().Single(mark => Property<Guid>(mark, "Id") == markId);
        Rect MarkBounds() => Property<Rect>(Mark(), "Bounds");
        Rect CropBounds() => (Rect)itemType.GetField("Bounds")!.GetValue(item)!;
        byte[] Template() => Property<byte[]>(Mark(), "Template");
        Point TemplateOffset() => new(Property<double>(Mark(), "TemplateOffsetX"), Property<double>(Mark(), "TemplateOffsetY"));
        long Version() => (long)itemType.GetField("RegionMarkVersion")!.GetValue(item)!;
        Point Center() { var rect = MarkBounds(); return new(rect.X + rect.Width / 2, rect.Y + rect.Height / 2); }
        Point World() { var mark = MarkBounds(); var crop = CropBounds(); return new(mark.X + crop.X, mark.Y + crop.Y); }
        void Tool(string name) => Invoke("SetDrawTool", Enum.Parse(typeof(CaptureOverlayWindow).GetNestedType("DrawTool", BindingFlags.NonPublic)!, name));
        void Commit() { Invoke("CommitSelectedDrawingMove"); canvas.ReleaseMouseCapture(); }
        void Undo() => Invoke("DrawUndo", overlay, new RoutedEventArgs());
        void Redo() => Invoke("DrawRedo", overlay, new RoutedEventArgs());
        void SelectMark()
        {
            Tool("Select");
            require((bool)Invoke("BeginDrawingGesture", item, Center(), canvas, 1, canvas)! &&
                ReferenceEquals(Field("_selectedDrawingRegionMark"), Mark()), "Select explicitly targets the existing emphasis region");
        }
        void MoveBy(double x, double y)
        {
            SelectMark();
            var point = Center();
            Invoke("MoveSelectedDrawingObject", item, new Point(point.X + x, point.Y + y), canvas);
            Commit();
        }
        byte[] Pixels()
        {
            var image = (BitmapSource)Invoke("RenderManualOverlay", item, (int)CropBounds().Width, (int)CropBounds().Height)!;
            var pixels = new byte[image.PixelWidth * image.PixelHeight * 4]; image.CopyPixels(pixels, image.PixelWidth * 4, 0); return pixels;
        }
        void Reframe(Rect bounds)
        {
            var before = Invoke("CaptureOverlaySnapshot")!;
            Invoke("SetSelectionBoundsPreservingManualContent", item, bounds);
            Invoke("InvalidateImageDerivedLayers", item); Invoke("UpdateSelection", item);
            Invoke("RecordGeometryOperationIfChanged", before, "emphasis replay reframe");
        }

        try
        {
            var width = originalFrame.Image.PixelWidth; var height = originalFrame.Image.PixelHeight;
            var pixels = new byte[width * height * 4];
            for (var y = 0; y < height; y++) for (var x = 0; x < width; x++)
            {
                var at = (y * width + x) * 4; var value = (byte)((x * 17 + y * 31 + x * y) % 251);
                pixels[at] = pixels[at + 1] = pixels[at + 2] = value; pixels[at + 3] = 255;
            }
            var pattern = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgra32, null, pixels, width * 4); pattern.Freeze();
            SetField("_frame", new CaptureFrame(originalFrame.OriginX, originalFrame.OriginY, pattern));
            Invoke("DrawClear", overlay, new RoutedEventArgs());
            Invoke("SetDrawColor", Colors.Red);
            Invoke("AddRegionMark", item, new Rect(45, 40, 90, 70));
            var lower = marks[0]!;
            Invoke("AddRegionMark", item, new Rect(80, 60, 130, 80));
            markId = Property<Guid>(marks[1]!, "Id");
            var initialBounds = MarkBounds(); var initialTemplate = Template().ToArray();
            require(initialTemplate.Length > 0 && initialTemplate.Distinct().Count() > 8,
                "emphasis template comes from patterned synthetic source pixels");
            Tool("Select");
            require((bool)Invoke("BeginDrawingGesture", item, new Point(100, 80), canvas, 1, canvas)! &&
                ReferenceEquals(Field("_selectedDrawingRegionMark"), Mark()),
                "overlapping emphasis selects the last visible region at the shared point");
            Commit();
            var overlyingStroke = new Stroke(new StylusPointCollection { new StylusPoint(85, 80), new StylusPoint(125, 80) },
                new DrawingAttributes { Width = 6, Height = 6 });
            canvas.Strokes.Add(overlyingStroke);
            canvas.RaiseEvent(new InkCanvasStrokeCollectedEventArgs(overlyingStroke) { RoutedEvent = InkCanvas.StrokeCollectedEvent });
            Tool("Select"); Invoke("BeginDrawingGesture", item, new Point(100, 80), canvas, 1, canvas); Commit();
            require(ReferenceEquals(Field("_selectedDrawingStroke"), overlyingStroke) && Field("_selectedDrawingRegionMark") is null,
                "overlapping ink takes precedence over the emphasis layer underneath");
            Invoke("DeleteSelectedDrawingObject");
            Tool("Text"); Invoke("BeginDrawingGesture", item, new Point(95, 70), canvas, 1, canvas);
            var overlyingEditor = canvas.Children.OfType<TextBox>().Single(); overlyingEditor.Text = "LAYER QA";
            var textId = (Guid)overlyingEditor.Tag;
            Tool("Select"); Invoke("BeginDrawingGesture", item, new Point(100, 80), canvas, 1, canvas); Commit();
            require(Equals(Field("_selectedDrawingElementId"), textId) && Field("_selectedDrawingRegionMark") is null,
                "overlapping text takes precedence over the emphasis layer underneath");
            Invoke("DeleteSelectedDrawingObject");
            SelectMark(); Commit();
            require(Field("_selectedDrawingStroke") is null && Field("_selectedDrawingElementId") is null,
                "selecting an emphasis region does not select unrelated ink or text");
            var versionBeforeMove = Version(); var beforeMoveCount = order.Count;
            MoveBy(40, 20);
            var movedBounds = MarkBounds(); var movedTemplate = Template().ToArray();
            require(movedBounds == new Rect(initialBounds.X + 40, initialBounds.Y + 20, initialBounds.Width, initialBounds.Height) &&
                order.Count == beforeMoveCount + 1, "moving emphasis changes its bounds and records exactly one action");
            require(Version() > versionBeforeMove && !movedTemplate.SequenceEqual(initialTemplate),
                "explicit emphasis move invalidates old matching and samples its new source pixels");
            // Deliberately mutate only the fixture's live template. Redo must
            // use its owned committed snapshot, not this subsequently changed array.
            Template()[0] ^= 0xff;
            Undo(); require(MarkBounds() == initialBounds && Template().SequenceEqual(initialTemplate),
                "emphasis move undo restores its old geometry and independent template bytes");
            Redo(); require(MarkBounds() == movedBounds && Template().SequenceEqual(movedTemplate),
                "emphasis move redo restores committed template bytes despite later live mutation");

            SelectMark(); Commit();
            var handles = ((IEnumerable)Field("_drawingObjectHandles")!).Cast<Point>().ToArray();
            require(handles.Length == 4, "selected emphasis exposes four resize corners");
            var corner = handles.OrderByDescending(point => point.X + point.Y).First();
            var beforeResizeCount = order.Count;
            require((bool)Invoke("TryBeginDrawingResize", item, corner, canvas)!, "emphasis uses the existing resize gesture entry point");
            Invoke("ResizeSelectedDrawingObjectWithConstraint", item, new Point(corner.X + 30, corner.Y + 25), canvas, false); Commit();
            var resizedBounds = MarkBounds(); var resizedTemplate = Template().ToArray();
            require(resizedBounds == new Rect(movedBounds.X, movedBounds.Y, movedBounds.Width + 30, movedBounds.Height + 25) &&
                order.Count == beforeResizeCount + 1, "emphasis resize changes both dimensions in one history action");
            require(!resizedTemplate.SequenceEqual(movedTemplate), "emphasis resize resamples a template for its new content extent");
            Undo(); require(MarkBounds() == movedBounds && Template().SequenceEqual(movedTemplate), "resize undo restores the preceding moved template");
            Redo(); require(MarkBounds() == resizedBounds && Template().SequenceEqual(resizedTemplate), "resize redo restores its own template and dimensions");

            SelectMark(); Commit(); var beforeColor = order.Count;
            Invoke("SetDrawColor", Colors.Blue);
            require(Property<Color>(Mark(), "Color") == Colors.Blue && Property<Color>(lower, "Color") == Colors.Red &&
                MarkBounds() == resizedBounds && order.Count == beforeColor + 1, "selected emphasis recolor affects only that object and adds one action");
            Undo(); require(Property<Color>(Mark(), "Color") == Colors.Red && MarkBounds() == resizedBounds,
                "emphasis color undo preserves its independently edited geometry");
            Redo(); require(Property<Color>(Mark(), "Color") == Colors.Blue && Template().SequenceEqual(resizedTemplate),
                "emphasis color redo preserves matching template pixels");
            SelectMark(); Commit(); var beforeDelete = order.Count;
            require((bool)Invoke("DeleteSelectedDrawingObject")! && marks.Count == 1 && order.Count == beforeDelete + 1,
                "Delete removes the selected emphasis region as one action");
            Undo(); require(marks.Count == 2 && ReferenceEquals(marks[0], lower) && Property<Guid>(marks[1]!, "Id") == markId,
                "Delete undo restores emphasis in its original overlap order");
            Redo(); require(marks.Count == 1, "Delete redo removes only the same emphasis region"); Undo();

            Invoke("ExitDrawingMode");
            var originalCrop = CropBounds(); var world = World(); var rendered = Pixels();
            Reframe(new Rect(originalCrop.X + 100, originalCrop.Y + 60, originalCrop.Width, originalCrop.Height));
            require(World() == world && Template().SequenceEqual(resizedTemplate), "reframing anchors edited emphasis to the original desktop pixels without resampling it");
            Reframe(originalCrop);
            require(Pixels().SequenceEqual(rendered), "reframing away and back restores the edited emphasis pixels exactly");
            Invoke("UndoOverlayOperation"); require(World() == world, "outer geometry undo keeps the emphasis desktop anchor");
            Invoke("RedoOverlayOperation"); require(Pixels().SequenceEqual(rendered), "outer geometry redo restores the exact edited emphasis appearance");
            Undo(); require(Property<Color>(Mark(), "Color") == Colors.Red && MarkBounds() == resizedBounds,
                "emphasis color history remains usable after outer geometry undo and redo");
            Redo(); require(Property<Color>(Mark(), "Color") == Colors.Blue, "emphasis color redo remains usable after reframing snapshots");
            Redo(); require(marks.Count == 1, "hidden Delete redo is preserved by reframing snapshots");
            Undo(); require(marks.Count == 2 && Template().SequenceEqual(resizedTemplate), "hidden Delete undo restores edited emphasis template after snapshot cloning");

            Invoke("EnterDrawingMode");
            var oldVersion = Version();
            oldMatching = (Task)Invoke("ReanchorRegionMarksAsync", item)!;
            // Do not yield the dispatcher until both edits commit. Even a
            // worker that finishes quickly cannot apply before the new version.
            MoveBy(20, 10); MoveBy(-20, -10);
            var finalBounds = MarkBounds(); var finalTemplate = Template().ToArray();
            require(Version() > oldVersion && !(bool)Invoke("IsRegionMarkMatchVersionCurrent", item, oldVersion)!,
                "editing away and back rejects an older matching version even when geometry is equal again");
            await oldMatching.WaitAsync(TimeSpan.FromSeconds(10)); oldMatching = null;
            require(MarkBounds() == finalBounds && Property<bool>(Mark(), "OnScreen") && Template().SequenceEqual(finalTemplate),
                "the real superseded matching task cannot overwrite subsequent manual geometry or template state");

            void FreshMark(Rect bounds)
            {
                Invoke("DrawClear", overlay, new RoutedEventArgs());
                Invoke("AddRegionMark", item, bounds); markId = Property<Guid>(marks[0]!, "Id");
            }
            void RequireClippedTemplate(string name)
            {
                var visible = Rect.Intersect(MarkBounds(), new Rect(0, 0, CropBounds().Width, CropBounds().Height));
                var source = (BitmapSource)Invoke("RenderSelectionImage", item, false, false, false)!;
                var scaleX = source.PixelWidth / CropBounds().Width; var scaleY = source.PixelHeight / CropBounds().Height;
                var left = Math.Floor(visible.Left * scaleX); var top = Math.Floor(visible.Top * scaleY);
                var right = Math.Ceiling(visible.Right * scaleX); var bottom = Math.Ceiling(visible.Bottom * scaleY);
                var expectedOffset = new Point(left / scaleX - MarkBounds().X, top / scaleY - MarkBounds().Y);
                var stepX = Property<int>(Mark(), "TemplateStepX"); var stepY = Property<int>(Mark(), "TemplateStepY");
                require(TemplateOffset() == expectedOffset &&
                    Property<int>(Mark(), "TemplateWidth") == (int)Math.Ceiling((right - left) / stepX) &&
                    Property<int>(Mark(), "TemplateHeight") == (int)Math.Ceiling((bottom - top) / stepY),
                    name + " samples the visible intersection and retains its offset from the full object bounds");
            }
            void ResizeAt(int index, Vector delta, bool constrain)
            {
                SelectMark(); Commit();
                var centers = ((IEnumerable)Field("_drawingObjectHandles")!).Cast<Point>().ToArray();
                require((bool)Invoke("TryBeginDrawingResize", item, centers[index], canvas)!, "clipped emphasis resize begins from its visible corner");
                Invoke("ResizeSelectedDrawingObjectWithConstraint", item, centers[index] + delta, canvas, constrain); Commit();
            }
            async Task MatchWithoutDrift(string name)
            {
                var expected = MarkBounds();
                // A small brightness difference forces the matcher to apply its
                // found bounds (SAD >= 2), rather than the unchanged-image path.
                var template = Template(); for (var i = 0; i < template.Length; i++) template[i] += 3;
                await ((Task)Invoke("ReanchorRegionMarksAsync", item)!).WaitAsync(TimeSpan.FromSeconds(10));
                require(Property<bool>(Mark(), "OnScreen") && (MarkBounds().TopLeft - expected.TopLeft).Length < 0.000001 &&
                    MarkBounds().Size == expected.Size,
                    name + " actual async matching subtracts the sample offset without shifting full bounds");
            }

            FreshMark(new Rect(-20, 40, 100, 60));
            RequireClippedTemplate("negative X emphasis");
            var beforeClippedMove = MarkBounds(); var beforeClippedPixels = Template().ToArray(); var beforeClippedOffset = TemplateOffset();
            MoveBy(0, 5);
            require(MarkBounds() == new Rect(-20, 45, 100, 60), "vertical movement of clipped emphasis never jumps its unedited negative X axis");
            RequireClippedTemplate("vertically moved negative X emphasis");
            var afterClippedPixels = Template().ToArray(); var afterClippedOffset = TemplateOffset();
            Undo(); require(MarkBounds() == beforeClippedMove && Template().SequenceEqual(beforeClippedPixels) && TemplateOffset() == beforeClippedOffset,
                "clipped move undo restores geometry, template bytes and sample offsets together");
            Redo(); require(MarkBounds() == new Rect(-20, 45, 100, 60) && Template().SequenceEqual(afterClippedPixels) && TemplateOffset() == afterClippedOffset,
                "clipped move redo restores its independent template and offsets");
            await MatchWithoutDrift("negative X emphasis");
            var beforeClippedResize = MarkBounds(); var beforeResizePixels = Template().ToArray(); var beforeResizeOffset = TemplateOffset();
            ResizeAt(2, new Vector(10, 10), false);
            require(MarkBounds() == new Rect(-20, 45, 110, 70), "ordinary resize preserves the clipped opposite corner");
            RequireClippedTemplate("resized negative X emphasis");
            var afterResizePixels = Template().ToArray(); var afterResizeOffset = TemplateOffset();
            Undo(); require(MarkBounds() == beforeClippedResize && Template().SequenceEqual(beforeResizePixels) && TemplateOffset() == beforeResizeOffset,
                "clipped resize undo restores template and offsets");
            Redo(); require(Template().SequenceEqual(afterResizePixels) && TemplateOffset() == afterResizeOffset,
                "clipped resize redo restores template and offsets");
            ResizeAt(2, new Vector(15, 5), true);
            require(MarkBounds().X == -20 && MarkBounds().Y == 45 && Math.Abs(MarkBounds().Width - MarkBounds().Height) < 0.01,
                "Shift resize of clipped emphasis keeps the opposite corner and produces a square");
            RequireClippedTemplate("square clipped emphasis");
            await MatchWithoutDrift("square clipped emphasis");

            var clippedWorld = World(); var clippedOffset = TemplateOffset(); var clippedTemplate = Template().ToArray();
            Invoke("ExitDrawingMode"); var clippedCrop = CropBounds();
            Reframe(new Rect(clippedCrop.X + 10, clippedCrop.Y + 10, clippedCrop.Width, clippedCrop.Height));
            require(World() == clippedWorld && TemplateOffset() == clippedOffset && Template().SequenceEqual(clippedTemplate),
                "reframing rebases clipped bounds while retaining their owned template offset and pixels");
            Invoke("UndoOverlayOperation"); Invoke("RedoOverlayOperation"); Invoke("UndoOverlayOperation");
            require(World() == clippedWorld && TemplateOffset() == clippedOffset && Template().SequenceEqual(clippedTemplate),
                "outer snapshot undo and redo preserve clipped template metadata");
            Invoke("EnterDrawingMode"); Invoke("DrawClear", overlay, new RoutedEventArgs());
            require(marks.Count == 0, "clear removes clipped emphasis from the live layer");
            var clearedSnapshot = Invoke("CaptureOverlaySnapshot")!; Invoke("ApplyOverlaySnapshot", clearedSnapshot);
            Undo(); require(marks.Count == 1 && TemplateOffset() == clippedOffset && Template().SequenceEqual(clippedTemplate),
                "clear undo after full snapshot cloning preserves hidden emphasis pixels and offsets");
            Redo(); require(marks.Count == 0, "clear redo after snapshot cloning targets the same restored emphasis"); Undo();

            FreshMark(new Rect(-20, 40, CropBounds().Width + 40, 60));
            MoveBy(5, 0);
            require(MarkBounds().X == -15 && MarkBounds().Y == 40 && MarkBounds().Width == CropBounds().Width + 40,
                "emphasis wider than the canvas accepts a small drag without forced normalization");
            RequireClippedTemplate("oversized emphasis");

            FreshMark(new Rect(CropBounds().Width - 60, CropBounds().Height - 40, 100, 80));
            MoveBy(-5, -5); RequireClippedTemplate("right and bottom clipped emphasis");
            var farCorner = MarkBounds().BottomRight;
            ResizeAt(0, new Vector(5, 5), false);
            require(MarkBounds().BottomRight == farCorner, "right-bottom clipped resize keeps its original off-canvas opposite corner");
            RequireClippedTemplate("right and bottom clipped resized emphasis");
            await MatchWithoutDrift("right and bottom clipped emphasis");

            BitmapSource MatchingFrame(Func<int, int, byte> sample)
            {
                var framePixels = new byte[width * height * 4];
                for (var y = 0; y < height; y++) for (var x = 0; x < width; x++)
                {
                    var at = (y * width + x) * 4; var value = sample(x, y);
                    framePixels[at] = framePixels[at + 1] = framePixels[at + 2] = value; framePixels[at + 3] = 255;
                }
                var image = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgra32, null, framePixels, width * 4);
                image.Freeze(); return image;
            }
            void SetMatchingFrame(BitmapSource image) => SetField("_frame", new CaptureFrame(originalFrame.OriginX, originalFrame.OriginY, image));
            byte FollowPattern(int x, int y)
            {
                // Nonperiodic blocks retain enough local similarity for the
                // coarse search; fine search must locate the exact pixel shift.
                var column = x / 8; var row = y / 8;
                return (byte)(20 + ((column * 73 + row * 151 + column * row * 37) % 160) + x % 8 + 3 * (y % 8));
            }
            var followSource = MatchingFrame(FollowPattern);
            var followShifted = MatchingFrame((x, y) => x < 10 || y < 7 ? (byte)0 : FollowPattern(x - 10, y - 7));
            foreach (var start in new[] { new Rect(120, 80, 100, 60), new Rect(-20, 40, 100, 60) })
            {
                SetMatchingFrame(followSource); FreshMark(start);
                var source = (BitmapSource)Invoke("RenderSelectionImage", item, false, false, false)!;
                var expected = new Rect(start.X + 10 * CropBounds().Width / source.PixelWidth,
                    start.Y + 7 * CropBounds().Height / source.PixelHeight, start.Width, start.Height);
                var originalOffset = TemplateOffset(); var originalTemplate = Template().ToArray();
                SetMatchingFrame(followShifted);
                await ((Task)Invoke("ReanchorRegionMarksAsync", item)!).WaitAsync(TimeSpan.FromSeconds(10));
                require(Property<bool>(Mark(), "OnScreen") && (MarkBounds().TopLeft - expected.TopLeft).Length < 0.000001 &&
                    MarkBounds().Size == expected.Size && TemplateOffset() == originalOffset && Template().SequenceEqual(originalTemplate),
                    $"nonuniform emphasis at X={start.X} follows an exact +10,+7 source-pixel translation including its sample offset " +
                    $"(expected={expected}; actual={MarkBounds()}; visible={Property<bool>(Mark(), "OnScreen")}; " +
                    $"offset={TemplateOffset()}; originalOffset={originalOffset}; sameTemplate={Template().SequenceEqual(originalTemplate)})");
            }
            SetMatchingFrame(MatchingFrame((_, _) => 90)); FreshMark(new Rect(-20, 40, 100, 60));
            var uniformBounds = MarkBounds();
            SetMatchingFrame(MatchingFrame((_, _) => 90));
            await ((Task)Invoke("ReanchorRegionMarksAsync", item)!).WaitAsync(TimeSpan.FromSeconds(10));
            require(Property<bool>(Mark(), "OnScreen") && MarkBounds() == uniformBounds,
                "uniform emphasis template remains anchored instead of jumping to the first tied search candidate");
            // A repeated but nonuniform texture is a separate ambiguity from a
            // flat template: the original sampled location is an exact tie.
            var repeated = MatchingFrame((x, y) => (byte)((x / 3 + y / 3) % 2 == 0 ? 40 : 210));
            SetMatchingFrame(repeated); FreshMark(new Rect(-20, 40, 100, 60));
            var repeatedBounds = MarkBounds();
            SetMatchingFrame(repeated);
            await ((Task)Invoke("ReanchorRegionMarksAsync", item)!).WaitAsync(TimeSpan.FromSeconds(10));
            require(Property<bool>(Mark(), "OnScreen") && (MarkBounds().TopLeft - repeatedBounds.TopLeft).Length < 0.000001,
                "equal-score nonuniform matches prefer the existing sample origin without coordinate drift");
        }
        finally
        {
            Invoke("InvalidateRegionMarkMatching", item);
            SetField("_frame", originalFrame);
            if (oldMatching is not null) await oldMatching.WaitAsync(TimeSpan.FromSeconds(10));
            Invoke("ClearDrawingObjectSelection"); canvas.ReleaseMouseCapture();
            Invoke("ApplyOverlaySnapshot", snapshot);
        }
    }
}
