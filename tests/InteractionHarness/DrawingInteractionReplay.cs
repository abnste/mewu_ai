// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.IO;
using System.Reflection;
using System.Security.Cryptography;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application = System.Windows.Application;
using Brushes = System.Windows.Media.Brushes;
using Button = System.Windows.Controls.Button;
using Color = System.Windows.Media.Color;
using ComboBox = System.Windows.Controls.ComboBox;
using Point = System.Windows.Point;
using Panel = System.Windows.Controls.Panel;
using Size = System.Windows.Size;
using TextBox = System.Windows.Controls.TextBox;

/// <summary>
/// Exercises the real drawing controls and gesture dispatcher on a synthetic
/// frame. This is a reflected WPF replay, not a claim of real pointer input.
/// </summary>
internal static class DrawingInteractionReplay
{
    private const BindingFlags Private = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public | BindingFlags.DeclaredOnly;
    private static string OutputDirectory => Path.Combine(Environment.CurrentDirectory, ".codex-build", "drawing-interaction");

    internal static void ConfigureOutput()
    {
        ReplayOutputDirectory.PrepareWorkingDirectory("drawing-interaction");
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(OutputDirectory, "logs"));
    }

    internal static void Run(Application app)
    {
        using var host = new AppHost(app, null, "MewuAI-IsolatedDrawing-" + Guid.NewGuid().ToString("N"));
        host.Settings.EnableVoiceInput = false;
        host.Settings.AutomaticallyStartListening = false;
        host.Settings.TeachingMode = false;
        var bytes = new byte[2560 * 1440 * 4];
        for (var i = 0; i < bytes.Length; i += 4)
        {
            bytes[i] = 245; bytes[i + 1] = 245; bytes[i + 2] = 245; bytes[i + 3] = 255;
        }
        var bitmap = BitmapSource.Create(2560, 1440, 96, 96, PixelFormats.Bgra32, null, bytes, 2560 * 4);
        bitmap.Freeze();
        Array.Clear(bytes);
        var frame = new CaptureFrame(0, 0, bitmap);
        var overlay = new CaptureOverlayWindow(host, null, frame)
        {
            Title = "Mewu isolated drawing interaction replay",
            ShowActivated = false,
            IsHitTestVisible = false
        };
        app.ShutdownMode = ShutdownMode.OnExplicitShutdown;
        var checks = new List<string>();
        var layouts = new List<object>();
        string? failure = null;
        var activeAfterReplay = false;
        var finished = false;
        var watchdog = new DispatcherTimer { Interval = TimeSpan.FromSeconds(60) };
        watchdog.Tick += (_, _) => { failure = "Drawing interaction replay exceeded 60 seconds."; Finish(); };

        void Require(bool condition, string description)
        {
            if (!condition) throw new InvalidOperationException(description);
            checks.Add(description);
        }
        object? Invoke(string name, params object?[] args) =>
            typeof(CaptureOverlayWindow).GetMethod(name, Private)!.Invoke(overlay, args);
        object? Field(string name) => typeof(CaptureOverlayWindow).GetField(name, Private)!.GetValue(overlay);
        T Control<T>(string name) where T : FrameworkElement =>
            overlay.FindName(name) as T ?? throw new InvalidOperationException("Missing drawing control: " + name);
        void Tool(string name)
        {
            var enumType = typeof(CaptureOverlayWindow).GetNestedType("DrawTool", BindingFlags.NonPublic)!;
            Invoke("SetDrawTool", Enum.Parse(enumType, name));
        }
        void Finish()
        {
            if (finished) return;
            finished = true;
            watchdog.Stop();
            try
            {
                Directory.CreateDirectory(OutputDirectory);
                File.WriteAllText(Path.Combine(OutputDirectory, "result.json"), JsonSerializer.Serialize(new
                {
                    passed = failure is null, checks, layouts, failure,
                    productSha256 = Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(typeof(CaptureOverlayWindow).Assembly.Location))),
                    harnessSha256 = Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(typeof(DrawingInteractionReplay).Assembly.Location))),
                    syntheticFrameOnly = true, reflectedProductGestures = true,
                    globalInputInjected = false, realPointerCaptureTested = false,
                    realTextFocusTested = false, nativeInkCollectionTested = false,
                    localKeyboardFocusTested = true, routedChineseCompositionTested = true,
                    physicalImeTypingTested = false, inputMethodGlobalStateChanged = false,
                    productionSettingsLoaded = false, productionHostStarted = false,
                    activeAfterReplay,
                    activationBoundary = "Real product handlers can focus their WPF canvas during the reflected replay. ShowActivated=false is an initial preference, not an assertion that the window never activates.",
                    clipboardAccessed = false, desktopPixelsCaptured = false,
                    hitTestBoundary = "The replay window rejects external pointer input. Text editor assertions inspect local hit-test configuration, not effective hit testing through that disabled ancestor."
                }, new JsonSerializerOptions { WriteIndented = true }), new System.Text.UTF8Encoding(false));
            }
            catch (Exception error) { failure ??= error.ToString(); }
            finally
            {
                try { overlay.Close(); }
                finally { app.Shutdown(failure is null ? 0 : 1); }
            }
        }

        overlay.Loaded += (_, _) => app.Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(async () =>
        {
            try
            {
                Require(ReferenceEquals(Field("_frame"), frame), "overlay uses only the supplied synthetic frame");
                Require(Field("_rightPassThrough") is null, "isolated replay does not install the global pass-through hook");
                Require(typeof(AppHost).GetField("_settingsService", Private)!.GetValue(host) is null,
                    "isolated host has not loaded the saved settings service");

                var root = Control<Canvas>("Root");
                Require(root.ActualWidth >= 1050 && root.ActualHeight >= 700, "desktop has room for the normal two-row layout scenario");
                var item = Invoke("CreateSelection", false)!;
                var itemType = item.GetType();
                var bounds = new Rect(100, 180, Math.Min(900, root.ActualWidth - 180), Math.Min(480, root.ActualHeight - 250));
                itemType.GetField("Bounds")!.SetValue(item, bounds);
                ((IList)Field("_selections")!).Add(item);
                Invoke("Select", 0);
                Invoke("EnterDrawingMode");
                overlay.UpdateLayout();

                var canvas = Property<InkCanvas>(item, "Markup");
                var elements = Property<IList>(item, "DrawingElements");
                var order = Property<IList>(item, "DrawingOrder");
                var marks = (IList)itemType.GetField("RegionMarks")!.GetValue(item)!;
                bool Gesture(Point point, int clicks = 1, DependencyObject? source = null) =>
                    (bool)Invoke("BeginDrawingGesture", item, point, canvas, clicks, source ?? canvas)!;
                bool NoSelection() => Field("_selectedDrawingStroke") is null && Field("_selectedDrawingElementId") is null && Field("_selectedDrawingRegionMark") is null;
                bool NoMove() => Field("_drawingMoveOriginalStroke") is null && Field("_drawingMoveOriginalElement") is null && Field("_drawingMoveOriginalRegionMark") is null;
                void Undo() => Invoke("DrawUndo", overlay, new RoutedEventArgs());
                void Redo() => Invoke("DrawRedo", overlay, new RoutedEventArgs());
                TextBox Editor(Guid id) => canvas.Children.OfType<TextBox>().Single(editor => Equals(editor.Tag, id));
                object Element(Guid id) => elements.Cast<object>().Single(element => Property<Guid>(element, "Id") == id);
                string Words(Guid id) => Property<string>(Element(id), "Text");
                Color TextColor(Guid id) => Property<Color>(Element(id), "Color");
                bool LocallyInteractive(TextBox editor) =>
                    editor.ReadLocalValue(UIElement.IsHitTestVisibleProperty) is true && editor.Focusable;
                void NoTextInput(string name) => Require(canvas.Children.OfType<TextBox>().All(editor =>
                    editor.ReadLocalValue(UIElement.IsHitTestVisibleProperty) is false && !editor.Focusable), name);

                // Populate actual product editors and history, without a desktop pointer.
                Invoke("SetDrawColor", Colors.Red);
                var stroke = new Stroke(new StylusPointCollection
                {
                    new StylusPoint(40, 70), new StylusPoint(240, 70)
                }, new DrawingAttributes { Color = Colors.Red, Width = 6, Height = 6, FitToCurve = false });
                canvas.Strokes.Add(stroke);
                canvas.RaiseEvent(new InkCanvasStrokeCollectedEventArgs(stroke) { RoutedEvent = InkCanvas.StrokeCollectedEvent });

                Tool("Text");
                Require(Gesture(new Point(80, 150)), "text creation gesture is consumed by the text tool");
                var firstEditor = canvas.Children.OfType<TextBox>().Single();
                firstEditor.Text = "FIRST QA";
                var firstId = (Guid)firstEditor.Tag;
                Require(LocallyInteractive(firstEditor), "new text editor is the sole locally interactive editor");

                Require(Gesture(new Point(430, 280)), "second text creation gesture is consumed");
                var secondEditor = canvas.Children.OfType<TextBox>().Single(editor => !Equals(editor.Tag, firstId));
                secondEditor.Text = "SECOND QA";
                var secondId = (Guid)secondEditor.Tag;
                Require(elements.Count == 2 && !LocallyInteractive(Editor(firstId)) && LocallyInteractive(Editor(secondId)),
                    "starting a second text editor makes the previous editor inert");

                Tool("Text");
                NoTextInput("choosing Text without a target leaves all old editors inert");
                Require(Gesture(new Point(100, 162)), "Text can explicitly reopen an existing editor");
                Require(elements.Count == 2 && LocallyInteractive(Editor(firstId)) && !LocallyInteractive(Editor(secondId)),
                    "only the explicitly reopened text receives text input");

                Invoke("AddRegionMark", item, new Rect(530, 45, 130, 80));
                Require(marks.Count == 1, "synthetic colored region is available as an overlapping creation target");
                var mark = marks[0];
                var points = new[]
                {
                    (Name: "old ink", Point: new Point(100, 70), Source: (DependencyObject)canvas),
                    (Name: "old text", Point: new Point(100, 162), Source: (DependencyObject)Editor(firstId)),
                    (Name: "old color region", Point: new Point(550, 70), Source: (DependencyObject)canvas)
                };
                foreach (var point in points)
                {
                    Invoke("DrawPen", overlay, new RoutedEventArgs());
                    var count = order.Count;
                    Require(!Gesture(point.Point, 1, point.Source), "pen hands " + point.Name + " to native InkCanvas");
                    Require(NoSelection() && NoMove() && order.Count == count, "pen does not select or move " + point.Name);
                    NoTextInput("pen leaves old editors inert over " + point.Name);
                }
                Require(!Gesture(new Point(100, 162), 2, Editor(firstId)) && Field("_drawTool")!.ToString() == "Freehand" &&
                    NoSelection() && NoMove(), "double-click with pen does not reopen old text");

                foreach (var tool in new[] { "Line", "Rectangle", "Ellipse", "Arrow", "Mosaic", "SeamlessErase", "Heal", "Mark" })
                    foreach (var point in points)
                    {
                        Tool(tool);
                        Require(Gesture(point.Point, 1, point.Source), tool + " consumes a creation start over " + point.Name);
                        var startsAtPointer = tool == "Heal"
                            ? Field("_drawingHealPreview") is { } healing && Property<Stroke>(healing, "Stroke").StylusPoints[0].ToPoint() == point.Point
                            : Equals(Field("_drawStart"), point.Point);
                        Require(startsAtPointer && NoSelection() && NoMove(),
                            tool + " starts at the new point without moving " + point.Name);
                        NoTextInput(tool + " keeps old text inert");
                    }
                Invoke("DrawPen", overlay, new RoutedEventArgs());
                Require(canvas.Strokes.Count == 1 && elements.Count == 2 && marks.Count == 1 && ReferenceEquals(marks[0], mark),
                    "creation-start routing preserves every existing object");

                var widths = Control<ComboBox>("DrawingStrokeWidth");
                var historyBeforeDefaults = order.Count;
                Invoke("SetDrawColor", Colors.Blue);
                widths.SelectedItem = 8d;
                Require(canvas.DefaultDrawingAttributes.Color == Colors.Blue && canvas.DefaultDrawingAttributes.Width == 8,
                    "pen color and width change the next stroke defaults");
                Require(stroke.DrawingAttributes.Color == Colors.Red && stroke.DrawingAttributes.Width == 6 &&
                    TextColor(firstId) == Colors.Red && TextColor(secondId) == Colors.Red && order.Count == historyBeforeDefaults,
                    "creation properties do not restyle old ink or text or add edit history");

                Tool("Select");
                Require(Gesture(new Point(100, 70)) && ReferenceEquals(Field("_selectedDrawingStroke"), stroke),
                    "explicit Select can select existing ink");
                Invoke("CommitSelectedDrawingMove");
                NoTextInput("selecting ink does not enable an old text editor");
                var beforeStyle = order.Count;
                Invoke("SetDrawColor", Colors.Blue);
                Require(stroke.DrawingAttributes.Color == Colors.Blue && order.Count == beforeStyle + 1,
                    "explicit selected-ink recolor creates one history action");
                widths.SelectedItem = 12d;
                Require(stroke.DrawingAttributes.Width == 12 && stroke.DrawingAttributes.Height == 12 && order.Count == beforeStyle + 2,
                    "explicit selected-ink width creates one additional history action");
                Undo();
                Require(stroke.DrawingAttributes.Width == 6 && stroke.DrawingAttributes.Color == Colors.Blue,
                    "one undo restores width while retaining the earlier color change");
                Undo();
                Require(stroke.DrawingAttributes.Color == Colors.Red && stroke.DrawingAttributes.Width == 6,
                    "second undo restores the selected ink original color");
                Redo();
                Require(stroke.DrawingAttributes.Color == Colors.Blue && stroke.DrawingAttributes.Width == 6,
                    "redo reapplies only the color action");
                Redo();
                Require(stroke.DrawingAttributes.Color == Colors.Blue && stroke.DrawingAttributes.Width == 12,
                    "redo reapplies the width action");
                Require(Words(firstId) == "FIRST QA" && Words(secondId) == "SECOND QA" &&
                    TextColor(firstId) == Colors.Red && TextColor(secondId) == Colors.Red,
                    "editing selected ink leaves unrelated text content and style unchanged");

                Tool("Select");
                Require(Gesture(new Point(100, 162), 2) && Field("_drawTool")!.ToString() == "Text" &&
                    Equals(Field("_selectedDrawingElementId"), firstId), "Select double-click explicitly enters text editing");
                Require(LocallyInteractive(Editor(firstId)) && !LocallyInteractive(Editor(secondId)) && elements.Count == 2,
                    "Select double-click reuses exactly one existing editor");
                Editor(firstId).Text = "FIRST QA EDIT";
                Invoke("SetDrawColor", Colors.Green);
                Require(TextColor(firstId) == Colors.Green && TextColor(secondId) == Colors.Red,
                    "active text recolor affects only its explicit target");
                Undo();
                Require(TextColor(firstId) == Colors.Red && Words(firstId) == "FIRST QA EDIT",
                    "text style undo preserves edited wording");
                Redo();
                Require(TextColor(firstId) == Colors.Green && Words(firstId) == "FIRST QA EDIT",
                    "text style redo preserves wording");

                Tool("Select");
                Require(Gesture(new Point(100, 162)) && Equals(Field("_selectedDrawingElementId"), firstId),
                    "explicit Select can select the text to resize");
                Invoke("CommitSelectedDrawingMove");
                var fontSizeBeforeResize = Property<double>(Element(firstId), "FontSize");
                var textBounds = (Rect)Invoke("DrawingElementBounds", item, Element(firstId))!;
                var handles = ((IEnumerable)Field("_drawingObjectHandles")!).Cast<Point>().ToArray();
                var corner = handles.OrderByDescending(point => point.X + point.Y).First();
                Require((bool)Invoke("TryBeginDrawingResize", item, corner, canvas)!, "selected text exposes a real resize handle");
                Invoke("ResizeSelectedDrawingObjectWithConstraint", item,
                    new Point(corner.X + 36, corner.Y + textBounds.Height * .5), canvas, false);
                Invoke("CommitSelectedDrawingMove");
                var resizedFontSize = Property<double>(Element(firstId), "FontSize");
                Require(resizedFontSize > fontSizeBeforeResize &&
                    Control<ComboBox>("DrawingFontSize").SelectedItem is double selectedFontSize &&
                    Math.Abs(selectedFontSize - resizedFontSize) < .01,
                    "committing text resize synchronizes the property control to the resulting font size");
                var fontControl = Control<ComboBox>("DrawingFontFamily");
                var previousFamily = Property<string>(Element(firstId), "FontFamily");
                var nextFamily = fontControl.Items.Cast<object>().First(choice => Property<string>(choice, "Source") != previousFamily);
                fontControl.SelectedItem = nextFamily;
                Require(Property<string>(Element(firstId), "FontFamily") == Property<string>(nextFamily, "Source") &&
                    Math.Abs(Property<double>(Element(firstId), "FontSize") - resizedFontSize) < .01,
                    "changing the selected resized text font family preserves its resized font size");
                Undo();
                Require(Property<string>(Element(firstId), "FontFamily") == previousFamily &&
                    Math.Abs(Property<double>(Element(firstId), "FontSize") - resizedFontSize) < .01,
                    "font family undo preserves the preceding text resize");
                Redo();
                Require(Property<string>(Element(firstId), "FontFamily") == Property<string>(nextFamily, "Source") &&
                    Math.Abs(Property<double>(Element(firstId), "FontSize") - resizedFontSize) < .01,
                    "font family redo also preserves the resized font size");
                Invoke("DrawPen", overlay, new RoutedEventArgs());
                Invoke("SetDrawColor", Colors.Purple);
                Require(TextColor(firstId) == Colors.Green && TextColor(secondId) == Colors.Red &&
                    stroke.DrawingAttributes.Color == Colors.Blue, "returning to pen detaches all earlier property targets");

                // Repeated mode changes should not mutate content or history.
                var stableHistory = order.Count;
                foreach (var cycle in Enumerable.Range(0, 6))
                    foreach (var tool in new[] { "Freehand", "Text", "Line", "Rectangle", "Ellipse", "Arrow", "Mark", "Mosaic", "SeamlessErase", "Heal", "Number", "Eraser", "Select" })
                    {
                        Tool(tool);
                        Require(NoSelection() && NoMove(), "tool switch resets transient selection " + cycle + ":" + tool);
                        NoTextInput("tool switch does not reactivate old text " + cycle + ":" + tool);
                    }
                Require(canvas.Strokes.Count == 1 && elements.Count == 2 && marks.Count == 1 && order.Count == stableHistory &&
                    Words(firstId) == "FIRST QA EDIT" && Words(secondId) == "SECOND QA" &&
                    stroke.DrawingAttributes.Width == 12 && stroke.DrawingAttributes.Color == Colors.Blue,
                    "repeated tool switches preserve content and history");

                async Task Layout(string name, string tool, bool selectStroke = false, bool longestFont = false)
                {
                    Tool(tool);
                    if (selectStroke)
                    {
                        Gesture(new Point(100, 70));
                        Invoke("CommitSelectedDrawingMove");
                    }
                    if (longestFont)
                    {
                        var font = Control<ComboBox>("DrawingFontFamily");
                        font.SelectedItem = font.Items.Cast<object>().OrderByDescending(choice =>
                            choice.GetType().GetProperty("DisplayName")?.GetValue(choice)?.ToString()?.Length ?? 0).First();
                    }
                    await app.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
                    overlay.UpdateLayout();
                    var toolbar = Control<FrameworkElement>("DrawingToolbar");
                    var main = Control<FrameworkElement>("DrawingToolbarSurface");
                    var items = Control<Panel>("DrawingToolbarItems");
                    var properties = Control<FrameworkElement>("DrawingPropertiesRow");
                    var toolbarRect = Bounds(toolbar, root);
                    var mainRect = Bounds(main, root);
                    var propertyRect = Bounds(properties, root);
                    Require(mainRect.Height > 0 && propertyRect.Height > 0 && propertyRect.Top >= mainRect.Bottom - 0.5,
                        name + " main and property rows have separate non-overlapping Y ranges");
                    Require(Math.Abs(mainRect.Left - toolbarRect.Left) <= 0.5,
                        name + " main row aligns to the toolbar left edge");
                    Require(Math.Abs(propertyRect.Left - toolbarRect.Left) <= 0.5,
                        name + " property row aligns to the toolbar left edge");
                    Require(toolbarRect.Left >= -0.5 && toolbarRect.Top >= -0.5 &&
                        toolbarRect.Right <= root.ActualWidth + 0.5 && toolbarRect.Bottom <= root.ActualHeight + 0.5,
                        name + " whole toolbar stays within the actual DIP viewport");
                    var buttons = VisualDescendants<Button>(items).Where(button => button.IsVisible).ToArray();
                    var buttonRects = buttons.Select(button => Bounds(button, root)).ToArray();
                    Require(buttonRects.Length == 14 && buttonRects.All(rect => rect.Width > 0 && rect.Height > 0),
                        name + " fourteen drawing tool buttons have actual visible bounds, including image-tool group");
                    Require(buttons.Select(button => button.Name).SequenceEqual(new[] { "DrawingSelectButton", "DrawingPenButton", "DrawingEraserButton", "DrawingHighlightButton",
                        "DrawingLineButton", "DrawingArrowButton", "DrawingRectangleButton", "DrawingEllipseButton", "DrawingTextButton", "DrawingNumberButton",
                        "DrawingMosaicButton", "DrawingSeamlessEraseButton", "DrawingHealButton", "DrawingMarkButton" }),
                        name + " eraser immediately follows pen and lift/heal remain distinct tools");
                    var imageTools = Control<Panel>("DrawingImageTools");
                    Require(VisualDescendants<Button>(imageTools).Select(button => button.Name).SequenceEqual(new[] { "DrawingMosaicButton", "DrawingSeamlessEraseButton", "DrawingHealButton", "DrawingMarkButton" }),
                        name + " mosaic, lift, healing and emphasis form one image-tool group");
                    Require(buttonRects.Max(rect => rect.Top) - buttonRects.Min(rect => rect.Top) <= 1,
                        name + " primary drawing buttons remain on one row at the current display size");
                    for (var a = 0; a < buttonRects.Length; a++)
                        for (var b = a + 1; b < buttonRects.Length; b++)
                            Require(!PositiveIntersection(buttonRects[a], buttonRects[b]), name + " primary controls do not overlap " + a + ":" + b);
                    var contextual = new[] { "DrawingColorControls", "DrawingStrokeControls", "DrawingTextControls", "DrawingSelectionHint", "DrawingToolHint" }
                        .Select(control => Control<FrameworkElement>(control)).Where(control => control.Visibility == Visibility.Visible).ToArray();
                    var contextRects = contextual.Select(control => Bounds(control, root)).ToArray();
                    for (var a = 0; a < contextRects.Length; a++)
                    {
                        Require(contextRects[a].Left >= propertyRect.Left - 0.5 && contextRects[a].Right <= propertyRect.Right + 0.5,
                            name + " contextual group remains inside the property row: " + contextual[a].Name);
                        for (var b = a + 1; b < contextRects.Length; b++)
                            Require(!PositiveIntersection(contextRects[a], contextRects[b]),
                                name + " contextual groups do not overlap " + contextual[a].Name + ":" + contextual[b].Name);
                    }
                    var actions = VisualDescendants<Button>(Control<Panel>("DrawingActionsGroup")).Where(button => button.IsVisible).ToArray();
                    Require(actions.Select(button => button.Name).SequenceEqual(new[] { "DrawingUndoButton", "DrawingRedoButton", "DrawingClearButton", "DrawingDoneButton" }),
                        name + " undo, redo, clear and done occupy the second-row action group");
                    var actionRects = actions.Select(button => Bounds(button, root)).ToArray();
                    Require(actionRects.All(rect => rect.Width > 0 && rect.Height > 0 && rect.Left >= propertyRect.Left - .5 && rect.Right <= propertyRect.Right + .5 && rect.Top >= propertyRect.Top - .5 && rect.Bottom <= propertyRect.Bottom + .5),
                        name + " all action buttons remain inside the actual second row");
                    Require(actionRects.Max(rect => rect.Top) - actionRects.Min(rect => rect.Top) <= 1,
                        name + " second-row actions stay on one horizontal row");
                    for (var a = 0; a < actionRects.Length; a++)
                    {
                        for (var b = a + 1; b < actionRects.Length; b++)
                            Require(!PositiveIntersection(actionRects[a], actionRects[b]), name + " actions do not overlap " + a + ":" + b);
                        foreach (var contextRect in contextRects)
                            Require(!PositiveIntersection(actionRects[a], contextRect), name + " actions do not overlap contextual properties " + a);
                    }
                    if (longestFont)
                    {
                        foreach (var control in new[] { "DrawingFontFamily", "DrawingFontSize", "TextHighlightButton" })
                        {
                            var rect = Bounds(Control<FrameworkElement>(control), root);
                            Require(rect.Left >= propertyRect.Left - 0.5 && rect.Right <= propertyRect.Right + 0.5 &&
                                rect.Top >= propertyRect.Top - 0.5 && rect.Bottom <= propertyRect.Bottom + 0.5,
                                "longest installed font choice keeps " + control + " within the text property row");
                        }
                    }
                    var dpi = VisualTreeHelper.GetDpi(overlay);
                    layouts.Add(new
                    {
                        name, tool, dpiX = dpi.DpiScaleX, dpiY = dpi.DpiScaleY,
                        viewport = new { width = root.ActualWidth, height = root.ActualHeight },
                        toolbar = RectData(toolbarRect), main = RectData(mainRect), properties = RectData(propertyRect),
                        primaryButtons = buttons.Select((button, index) => new { button.Name, bounds = RectData(buttonRects[index]) }),
                        actionButtons = actions.Select((button, index) => new { button.Name, bounds = RectData(actionRects[index]) }),
                        controls = contextual.Select((control, index) => new { control.Name, bounds = RectData(contextRects[index]) }),
                        screenshot = name + ".png"
                    });
                    SaveVisual(toolbar, Path.Combine(OutputDirectory, name + ".png"), dpi);
                }

                await Layout("toolbar-pen", "Freehand");
                await Layout("toolbar-select-stroke", "Select", selectStroke: true);
                await Layout("toolbar-text-longest-font", "Text", longestFont: true);
                await Layout("toolbar-eraser", "Eraser");
                await Layout("toolbar-mosaic", "Mosaic");
                await Layout("toolbar-seamless-erase", "SeamlessErase");
                await Layout("toolbar-heal", "Heal");
                await Layout("toolbar-number", "Number");
                await Layout("toolbar-select-empty", "Select");
                await DrawingTextInputReplay.Verify(overlay, item, Require);
                DrawingEraserReplay.Verify(overlay, item, Require);
                await DrawingRegionMarkReplay.Verify(overlay, item, Require);
                DrawingSeamlessEraseReplay.Verify(overlay, item, Require, OutputDirectory);
                await DrawingHealReplay.Verify(overlay, item, Require, OutputDirectory);
                DrawingBackgroundHighlightReplay.Verify(overlay, item, Require, OutputDirectory);
                DrawingNumbersReplay.Verify(overlay, item, Require);
                activeAfterReplay = overlay.IsActive;
                Require(typeof(AppHost).GetField("_settingsService", Private)!.GetValue(host) is null,
                    "saved settings service remains unloaded after interaction and layout replay");
            }
            catch (Exception error) { failure = error.ToString(); }
            finally { Finish(); }
        }));
        watchdog.Start();
        overlay.Show();
        app.Run();
    }

    private static T Property<T>(object owner, string name) =>
        (T)(owner.GetType().GetProperty(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic)?.GetValue(owner)
            ?? throw new InvalidOperationException("Missing property " + name));

    private static Rect Bounds(FrameworkElement element, Visual ancestor) =>
        element.TransformToAncestor(ancestor).TransformBounds(new Rect(new Point(), element.RenderSize));

    private static IEnumerable<T> VisualDescendants<T>(DependencyObject root) where T : DependencyObject
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(root); index++)
        {
            var child = VisualTreeHelper.GetChild(root, index);
            if (child is T typed) yield return typed;
            foreach (var descendant in VisualDescendants<T>(child)) yield return descendant;
        }
    }

    private static object RectData(Rect rect) => new { x = rect.X, y = rect.Y, width = rect.Width, height = rect.Height };

    private static bool PositiveIntersection(Rect a, Rect b) =>
        Math.Min(a.Right, b.Right) - Math.Max(a.Left, b.Left) > 0.5 &&
        Math.Min(a.Bottom, b.Bottom) - Math.Max(a.Top, b.Top) > 0.5;

    private static void SaveVisual(FrameworkElement visual, string path, DpiScale dpi)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        const double padding = 16;
        var width = visual.ActualWidth + padding * 2; var height = visual.ActualHeight + padding * 2;
        var image = new RenderTargetBitmap(
            Math.Max(1, (int)Math.Ceiling(width * dpi.DpiScaleX)),
            Math.Max(1, (int)Math.Ceiling(height * dpi.DpiScaleY)),
            96 * dpi.DpiScaleX, 96 * dpi.DpiScaleY, PixelFormats.Pbgra32);
        var parent = (Panel)VisualTreeHelper.GetParent(visual); var index = parent.Children.IndexOf(visual);
        var left = Canvas.GetLeft(visual); var top = Canvas.GetTop(visual);
        var renderHost = new Canvas { Width = width, Height = height };
        parent.Children.Remove(visual);
        try
        {
            renderHost.Children.Add(visual); Canvas.SetLeft(visual, padding); Canvas.SetTop(visual, padding);
            renderHost.Measure(new Size(width, height)); renderHost.Arrange(new Rect(0, 0, width, height)); renderHost.UpdateLayout();
            image.Render(renderHost);
        }
        finally
        {
            renderHost.Children.Remove(visual); parent.Children.Insert(index, visual);
            Canvas.SetLeft(visual, left); Canvas.SetTop(visual, top); parent.UpdateLayout();
        }
        var pixels = new byte[image.PixelWidth * image.PixelHeight * 4]; image.CopyPixels(pixels, image.PixelWidth * 4, 0);
        if (!Enumerable.Range(0, pixels.Length / 4).Any(index => pixels[index * 4 + 3] > 0)) throw new InvalidOperationException("Toolbar rendering produced no visible pixels.");
        var encoder = new PngBitmapEncoder();
        encoder.Frames.Add(BitmapFrame.Create(image));
        using var stream = File.Create(path);
        encoder.Save(stream);
    }
}
