// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application = System.Windows.Application;
using Color = System.Windows.Media.Color;
using Point = System.Windows.Point;
using Size = System.Windows.Size;

/// <summary>Actual inspector and sample rendering with synthetic frames, no HWND or input injection.</summary>
internal static class PointerMagnifierReplay
{
    private const BindingFlags Private = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public;

    internal static void Run(string[] args)
    {
        var directory = ReplayOutputDirectory.PrepareWorkingDirectory("pointer-magnifier");
        var english = args.Contains("--english"); var language = english ? "en" : "zh";
        LocalizationService.Initialize(english ? "en-US" : "zh-CN", null);
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(directory, "logs"));
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        app.Resources.MergedDictionaries.Add(new ResourceDictionary { Source = new Uri("/MewuAI;component/Themes/LightTheme.xaml", UriKind.Relative) });
        using var host = new AppHost(app, null, "MewuAI-PointerMagnifier-" + Guid.NewGuid().ToString("N"));
        host.Settings.EnableVoiceInput = false; host.Settings.AutomaticallyStartListening = false; host.Settings.SaveConversationHistory = false;
        var source = CreateSource(800, 600, 0);
        var original = Pixels(source);
        var overlay = new CaptureOverlayWindow(host, null, new CaptureFrame(-1920, -1080, source));
        var checks = new List<string>(); var layouts = new List<object>(); var renders = new List<string>(); string? failure = null;
        void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); checks.Add(message); }
        void Set(string name, object value) => typeof(CaptureOverlayWindow).GetField(name, Private)!.SetValue(overlay, value);
        object? Invoke(string name, params object[] values) => typeof(CaptureOverlayWindow).GetMethod(name, Private)!.Invoke(overlay, values);
        try
        {
            var root = (Canvas)overlay.FindName("Root");
            var inspector = (Border)overlay.FindName("PointerInspector");
            var magnifier = (FrameworkElement)overlay.FindName("PointerMagnifier");
            var colorText = (TextBlock)overlay.FindName("PointerColorText");
            var coordinateText = (TextBlock)overlay.FindName("PointerCoordinateText");
            var swatch = (System.Windows.Shapes.Shape)overlay.FindName("PointerColorSwatch");
            Set("_promptBarHidden", true);
            ((FrameworkElement)overlay.FindName("PromptBarHost")).Visibility = Visibility.Collapsed;
            ((FrameworkElement)overlay.FindName("Toolbar")).Visibility = Visibility.Collapsed;
            Check(magnifier.GetType().FullName == "mewu_ai_Assistant.Views.CaptureMagnifier", "actual magnifier is the product custom element");
            Check(!inspector.IsHitTestVisible && !magnifier.IsHitTestVisible && !magnifier.Focusable,
                "inspector cannot intercept pointer or keyboard focus");
            var formatColor = typeof(CaptureOverlayWindow).GetMethod("FormatPointerColor", BindingFlags.Static | BindingFlags.NonPublic)!;
            foreach (var sample in new[] { (Color: Color.FromRgb(0, 0, 0), Text: "#000000"),
                (Color: Color.FromRgb(1, 2, 15), Text: "#01020F"), (Color: Color.FromRgb(255, 255, 255), Text: "#FFFFFF") })
                Check((string)formatColor.Invoke(null, [sample.Color])! == sample.Text,
                    "shared display/copy formatter uses uppercase six-digit HEX " + sample.Text);
            foreach (var dpi in new[] { 96, 168, 192 })
            {
                var scale = dpi / 96d;
                root.Width = source.PixelWidth / scale; root.Height = source.PixelHeight / scale;
                root.Measure(new Size(root.Width, root.Height)); root.Arrange(new Rect(0, 0, root.Width, root.Height)); root.UpdateLayout();
                (string Name, int X, int Y)[] positions = [("center", 317, 241), ("top-left", 0, 0), ("top-right", 799, 0), ("bottom-left", 0, 599), ("bottom-right", 799, 599)];
                foreach (var position in positions)
                {
                    var point = new Point((position.X + .25) * root.ActualWidth / 800, (position.Y + .25) * root.ActualHeight / 600);
                    Invoke("UpdatePointerInspector", point); root.UpdateLayout();
                    inspector.Arrange(new Rect(new Point(Canvas.GetLeft(inspector), Canvas.GetTop(inspector)), inspector.DesiredSize));
                    var prefix = position.Name + "@" + dpi;
                    Check(inspector.Visibility == Visibility.Visible, prefix + " inspector shown");
                    var expected = SourceColor(position.X, position.Y, 0);
                    Check(colorText.Text == $"#{expected.R:X2}{expected.G:X2}{expected.B:X2}", prefix + " HEX readout uses the exact center color");
                    Check(coordinateText.Text == $"X {-1920 + position.X} Y {-1080 + position.Y}", prefix + " negative virtual-screen coordinates retain physical pixels");
                    Check(((SolidColorBrush)swatch.Fill).Color == expected, prefix + " swatch matches center pixel");
                    var sample = Sample(magnifier);
                    Check(sample.PixelWidth == 25 && sample.PixelHeight == 25, prefix + " fixed 25 by 25 physical sample");
                    var bytes = Pixels(sample);
                    Check(Pixel(bytes, 25, 12, 12) == expected, prefix + " cursor pixel stays at sample center even at source edges");
                    foreach (var samplePoint in new[] { (X: 4, Y: 4), (X: 20, Y: 4), (X: 4, Y: 20), (X: 20, Y: 20) })
                    {
                        var x = position.X + samplePoint.X - 12;
                        var y = position.Y + samplePoint.Y - 12;
                        var wanted = x >= 0 && x < 800 && y >= 0 && y < 600 ? SourceColor(x, y, 0) : Color.FromArgb(0, 0, 0, 0);
                        Check(Pixel(bytes, 25, samplePoint.X, samplePoint.Y) == wanted, prefix + " exterior sample remains transparent without shifting target " + samplePoint);
                    }
                    var left = Canvas.GetLeft(inspector); var top = Canvas.GetTop(inspector);
                    Check(left >= 3.9 && top >= 3.9 && left + inspector.ActualWidth <= root.ActualWidth - 3.9 && top + inspector.ActualHeight <= root.ActualHeight - 3.9,
                        prefix + " complete card stays inside visible overlay");
                    Check(magnifier.Width == 90 && magnifier.Height == 90 && Math.Abs(magnifier.ActualWidth - 90) <= 1 && Math.Abs(magnifier.ActualHeight - 90) <= 1,
                        prefix + " magnifier is a square 90 DIP viewport with physical-pixel layout rounding: " + magnifier.RenderSize);
                    Check(inspector.Width == 90 && Math.Abs(inspector.ActualHeight - 126) <= 1,
                        prefix + " compact card retains both readout rows below the magnifier");
                    Invoke("UpdatePointerInspector", new Point((position.X + .1) * root.ActualWidth / 800, (position.Y + .1) * root.ActualHeight / 600));
                    Check(ReferenceEquals(sample, Sample(magnifier)) && bytes.SequenceEqual(Pixels(Sample(magnifier))), prefix + " motion within same physical pixel reuses sample unchanged");
                    if (position.Name == "center")
                    {
                        Invoke("UpdatePointerInspector", new Point(position.X * root.ActualWidth / 800 - 1e-10, position.Y * root.ActualHeight / 600 - 1e-10));
                        Check(Pixel(Pixels(Sample(magnifier)), 25, 12, 12) == expected, prefix + " DIP round-trip epsilon does not select preceding pixel");
                    }
                    layouts.Add(new { dpi, position.Name, position.X, position.Y, left, top, inspector.ActualWidth, inspector.ActualHeight });
                    if (position.Name is "center" or "top-left")
                    {
                        var path = Path.Combine(directory, "magnifier-" + language + "-" + position.Name + "-" + dpi + ".png");
                        RenderCard(inspector, magnifier, sample, dpi, path, Check); renders.Add(path);
                    }
                }
            }
            var centerPoint = new Point(158.625, 120.625);
            Invoke("UpdatePointerInspector", centerPoint);
            var prior = Pixels(Sample(magnifier));
            var replacement = CreateSource(800, 600, 73);
            Set("_frame", new CaptureFrame(-1920, -1080, replacement));
            Invoke("UpdatePointerInspector", centerPoint);
            Check(!prior.SequenceEqual(Pixels(Sample(magnifier))) && Pixel(Pixels(Sample(magnifier)), 25, 12, 12) == SourceColor(317, 241, 73),
                "new frame refreshes sample at the same physical coordinate");
            Set("_frame", new CaptureFrame(-2560, -1440, replacement));
            Invoke("UpdatePointerInspector", centerPoint);
            Check(coordinateText.Text == "X -2243 Y -1199", "same bitmap with new virtual origin refreshes coordinate readout");
            foreach (var origin in new[] { (X: -32768, Y: -32768), (X: 65536, Y: -16384) })
            {
                Set("_frame", new CaptureFrame(origin.X, origin.Y, replacement));
                Invoke("UpdatePointerInspector", centerPoint); root.UpdateLayout();
                inspector.Arrange(new Rect(new Point(Canvas.GetLeft(inspector), Canvas.GetTop(inspector)), inspector.DesiredSize));
                var expectedText = $"X {origin.X + 317} Y {origin.Y + 241}";
                Check(coordinateText.Text == expectedText, "wide virtual-screen coordinates retain every digit: " + expectedText);
                var text = new FormattedText(coordinateText.Text, System.Globalization.CultureInfo.InvariantCulture,
                    coordinateText.FlowDirection, new Typeface(coordinateText.FontFamily, coordinateText.FontStyle,
                        coordinateText.FontWeight, coordinateText.FontStretch), coordinateText.FontSize, coordinateText.Foreground,
                    null, TextOptions.GetTextFormattingMode(coordinateText),
                    VisualTreeHelper.GetDpi(coordinateText).PixelsPerDip);
                var textBounds = coordinateText.TransformToAncestor(inspector).TransformBounds(new Rect(0, 0, text.WidthIncludingTrailingWhitespace, text.Height));
                var arrangedTextBounds = coordinateText.TransformToAncestor(inspector).TransformBounds(new Rect(coordinateText.RenderSize));
                var coordinateHost = coordinateText.Parent as FrameworkElement;
                layouts.Add(new
                {
                    scenario = "long-coordinate", expectedText, coordinateText.FontSize, fontFamily = coordinateText.FontFamily.Source,
                    dpi = VisualTreeHelper.GetDpi(coordinateText).PixelsPerDip, formattingMode = TextOptions.GetTextFormattingMode(coordinateText).ToString(),
                    language = coordinateText.Language.IetfLanguageTag, measuredWidth = text.WidthIncludingTrailingWhitespace, measuredHeight = text.Height,
                    textBounds = new { textBounds.X, textBounds.Y, textBounds.Width, textBounds.Height },
                    arrangedTextBounds = new { arrangedTextBounds.X, arrangedTextBounds.Y, arrangedTextBounds.Width, arrangedTextBounds.Height },
                    textRenderSize = coordinateText.RenderSize, textDesiredSize = coordinateText.DesiredSize, inspectorSize = inspector.RenderSize,
                    coordinateHostType = coordinateHost?.GetType().Name, coordinateHostSize = coordinateHost?.RenderSize,
                    transform = coordinateText.TransformToAncestor(inspector).ToString(),
                    measureValid = coordinateText.IsMeasureValid, arrangeValid = coordinateText.IsArrangeValid
                });
                var coordinateRender = Path.Combine(directory, $"magnifier-{language}-coordinates-{origin.X}-{origin.Y}-168.png");
                RenderCard(inspector, magnifier, Sample(magnifier), 168, coordinateRender, Check); renders.Add(coordinateRender);
                Check(textBounds.Left >= 3 && textBounds.Right <= inspector.ActualWidth - 3 && textBounds.Top >= 90 && textBounds.Bottom <= inspector.ActualHeight,
                    "complete long coordinate glyphs fit inside the compact second row: " + expectedText + "; measured=" + textBounds +
                    "; arranged=" + arrangedTextBounds + "; card=" + inspector.RenderSize);
            }
            foreach (var bounds in new[] { new Rect(-1920, -1080, 1920, 1080), new Rect(100, 200, 800, 600) })
            foreach (var point in new[] { bounds.TopLeft, bounds.TopRight, bounds.BottomLeft, bounds.BottomRight })
            {
                var location = CaptureOverlayWindow.PlacePointerInspector(point, inspector.DesiredSize, bounds);
                Check(location.X >= bounds.Left + 4 && location.Y >= bounds.Top + 4 &&
                    location.X + inspector.DesiredSize.Width <= bounds.Right - 4 && location.Y + inspector.DesiredSize.Height <= bounds.Bottom - 4,
                    "placement respects offset monitor edges " + bounds + " at " + point);
            }
            var tiny = new CaptureMagnifier(); var onePixel = CreateSource(1, 1, 73);
            Check(tiny.SetSample(onePixel, 0, 0), "one-pixel source accepts bounded sample");
            var tinyBytes = Pixels(tiny.Sample);
            Check(Pixel(tinyBytes, 25, 12, 12) == SourceColor(0, 0, 73) && Enumerable.Range(0, 625).Count(index => tinyBytes[index * 4 + 3] != 0) == 1,
                "one-pixel source keeps its only opaque pixel at center");
            Set("_frame", new CaptureFrame(-1920, -1080, CreateTextSource()));
            Invoke("UpdatePointerInspector", centerPoint); root.UpdateLayout();
            var textRender = Path.Combine(directory, "magnifier-" + language + "-text-edge-168.png");
            RenderCard(inspector, magnifier, Sample(magnifier), 168, textRender, Check); renders.Add(textRender);
            foreach (var field in new[] { "_recordingMode", "_recordingCountdownActive", "_drawingMode", "_longCaptureMode", "_promptDragging", "_promptDockAnimating" })
            {
                inspector.Visibility = Visibility.Visible; Set(field, true);
                try { Invoke("UpdatePointerInteraction", centerPoint); Check(inspector.Visibility == Visibility.Collapsed, field + " suppresses inspector through real pointer route"); }
                finally { Set(field, false); }
            }
            foreach (var outside in new[] { new Point(-.01, 10), new Point(10, -.01), new Point(root.Width, 10), new Point(10, root.Height) })
            { Invoke("UpdatePointerInspector", outside); Check(inspector.Visibility == Visibility.Collapsed, "outside overlay hides inspector " + outside); }
            Check(original.SequenceEqual(Pixels(source)), "sampling and rendering never modify source frame");
            Check(new WindowInteropHelper(overlay).Handle == IntPtr.Zero && !overlay.IsLoaded && !overlay.IsVisible,
                "replay never shows a window or creates a native HWND");
            Check(host.IsIsolatedReplay && host.GetConversationChannels().Count == 0, "isolated host has no production account or AI connection");
        }
        catch (Exception error) { failure = error.ToString(); }
        finally { overlay.Close(); app.Shutdown(); }
        using var assembly = File.OpenRead(typeof(CaptureOverlayWindow).Assembly.Location);
        File.WriteAllText(Path.Combine(directory, "result.json"), JsonSerializer.Serialize(new
        {
            passed = failure is null, language, productSha256 = Convert.ToHexString(SHA256.HashData(assembly)), checks, layouts, renders, failure,
            actualDesktopInput = false, nativeWindowCreated = false, source = "synthetic opaque BGRA coordinate pattern"
        }, new JsonSerializerOptions { WriteIndented = true }), new UTF8Encoding(false));
        Environment.ExitCode = failure is null ? 0 : 1;
    }

    private static BitmapSource Sample(FrameworkElement magnifier) => ((CaptureMagnifier)magnifier).Sample;

    private static Color SourceColor(int x, int y, int variant) => Color.FromRgb((byte)((x * 7 + y * 3 + variant) % 256), (byte)((x * 2 + y * 11 + variant) % 256), (byte)((x * 5 + y * 13 + variant) % 256));

    private static BitmapSource CreateSource(int width, int height, int variant)
    {
        var bytes = new byte[width * height * 4];
        for (var y = 0; y < height; ++y)
        for (var x = 0; x < width; ++x)
        {
            var color = SourceColor(x, y, variant); var offset = (y * width + x) * 4;
            bytes[offset] = color.B; bytes[offset + 1] = color.G; bytes[offset + 2] = color.R; bytes[offset + 3] = 255;
        }
        var bitmap = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgra32, null, bytes, width * 4); bitmap.Freeze(); return bitmap;
    }

    private static BitmapSource CreateTextSource()
    {
        var visual = new DrawingVisual();
        using (var drawing = visual.RenderOpen())
        {
            drawing.DrawRectangle(System.Windows.Media.Brushes.White, null, new Rect(0, 0, 800, 600));
            var text = new FormattedText("放大镜 Magnifier", System.Globalization.CultureInfo.InvariantCulture,
                System.Windows.FlowDirection.LeftToRight, new Typeface("Segoe UI"), 20, System.Windows.Media.Brushes.Black, 1);
            drawing.DrawText(text, new Point(300, 222));
            drawing.DrawRectangle(new SolidColorBrush(Color.FromRgb(94, 110, 235)), null, new Rect(302, 250, 190, 2));
        }
        var bitmap = new RenderTargetBitmap(800, 600, 96, 96, PixelFormats.Pbgra32); bitmap.Render(visual);
        // A desktop capture is opaque. WPF antialiasing can leave A=254 even
        // over a painted background; flatten its premultiplied pixels onto white.
        var bytes = Pixels(bitmap);
        for (var index = 0; index < bytes.Length; index += 4)
        {
            var remaining = 255 - bytes[index + 3];
            for (var channel = 0; channel < 3; channel++) bytes[index + channel] = (byte)Math.Min(255, bytes[index + channel] + remaining);
            bytes[index + 3] = 255;
        }
        var opaque = BitmapSource.Create(800, 600, 96, 96, PixelFormats.Bgra32, null, bytes, 800 * 4); opaque.Freeze(); return opaque;
    }

    private static byte[] Pixels(BitmapSource bitmap)
    {
        if (bitmap.Format != PixelFormats.Bgra32 && bitmap.Format != PixelFormats.Pbgra32)
            bitmap = new FormatConvertedBitmap(bitmap, PixelFormats.Bgra32, null, 0);
        var bytes = new byte[bitmap.PixelWidth * bitmap.PixelHeight * 4]; bitmap.CopyPixels(bytes, bitmap.PixelWidth * 4, 0); return bytes;
    }

    private static Color Pixel(byte[] bytes, int width, int x, int y)
    { var offset = (y * width + x) * 4; return Color.FromArgb(bytes[offset + 3], bytes[offset + 2], bytes[offset + 1], bytes[offset]); }

    private static void RenderCard(Border card, FrameworkElement magnifier, BitmapSource sample, int dpi, string path, Action<bool, string> check)
    {
        var parent = (Canvas)VisualTreeHelper.GetParent(card); var index = parent.Children.IndexOf(card);
        var left = Canvas.GetLeft(card); var top = Canvas.GetTop(card);
        var renderRoot = new Canvas();
        parent.Children.Remove(card);
        try
        {
            VisualTreeHelper.SetRootDpi(renderRoot, new DpiScale(dpi / 96d, dpi / 96d));
            card.Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
            renderRoot.Width = card.DesiredSize.Width + 16; renderRoot.Height = card.DesiredSize.Height + 16;
            renderRoot.Children.Add(card); Canvas.SetLeft(card, 8); Canvas.SetTop(card, 8);
            renderRoot.Measure(new Size(renderRoot.Width, renderRoot.Height)); renderRoot.Arrange(new Rect(0, 0, renderRoot.Width, renderRoot.Height)); renderRoot.UpdateLayout();
            var bitmap = new RenderTargetBitmap((int)Math.Ceiling(renderRoot.Width * dpi / 96d), (int)Math.Ceiling(renderRoot.Height * dpi / 96d), dpi, dpi, PixelFormats.Pbgra32);
            bitmap.Render(renderRoot);
            var bytes = Pixels(bitmap); var sampleBytes = Pixels(sample);
            var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(bitmap)); using (var stream = File.Create(path)) encoder.Save(stream);
            foreach (var location in new[] { (X: 4, Y: 4), (X: 12, Y: 12), (X: 20, Y: 20) })
            {
                var dip = magnifier.TransformToAncestor(renderRoot).Transform(new Point((location.X + .5) * magnifier.ActualWidth / 25, (location.Y + .5) * magnifier.ActualHeight / 25));
                var actual = Pixel(bytes, bitmap.PixelWidth, (int)Math.Floor(dip.X * dpi / 96d), (int)Math.Floor(dip.Y * dpi / 96d));
                var wanted = Pixel(sampleBytes, 25, location.X, location.Y);
                if (wanted.A == 0) wanted = Color.FromRgb(238, 243, 249);
                check(actual == wanted, "actual WPF magnification preserves nearest-neighbor source color or empty-edge backdrop at " + dpi + " DPI " + location + ": actual=" + actual + ", expected=" + wanted);
            }
        }
        finally { renderRoot.Children.Remove(card); parent.Children.Insert(index, card); Canvas.SetLeft(card, left); Canvas.SetTop(card, top); parent.UpdateLayout(); }
    }
}
