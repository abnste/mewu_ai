// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application = System.Windows.Application;
using Brushes = System.Windows.Media.Brushes;
using Image = System.Windows.Controls.Image;
using Point = System.Windows.Point;
using Size = System.Windows.Size;
using FlowDirection = System.Windows.FlowDirection;
using RichTextBox = System.Windows.Controls.RichTextBox;
using Panel = System.Windows.Controls.Panel;

internal static class TranslationSingleLineReplay
{
    private const BindingFlags Private = BindingFlags.Instance | BindingFlags.NonPublic;
    private sealed record Case(string Name, OcrLine[] Lines, string[] Translations, string[] Expected);

    internal static void Run()
    {
        var directory = ReplayOutputDirectory.PrepareWorkingDirectory("translation-single-line");
        LocalizationService.Initialize("zh-CN", null);
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(directory, "logs"));
        var checks = new List<string>(); var results = new List<object>(); string? failure = null;
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        app.Resources.MergedDictionaries.Add(new ResourceDictionary { Source = new Uri("/MewuAI;component/Themes/LightTheme.xaml", UriKind.Relative) });
        using var host = new AppHost(app, null, "MewuAI-TranslationSingleLine-" + Guid.NewGuid().ToString("N"));
        host.Settings.EnableVoiceInput = false; host.Settings.AutomaticallyStartListening = false; host.Settings.SaveConversationHistory = false;
        CaptureOverlayWindow? overlay = null;
        void Check(bool passed, string name) { if (!passed) throw new InvalidOperationException(name); checks.Add(name); }
        try
        {
            var veryLong = string.Concat(Enumerable.Repeat("完整长句不应因为下方留白被拆成多行，", 8));
            Case[] cases = [
                new("single-line", [Line("Source line", 24, 35, 420, 32)],
                    ["原本只有一行的文字应保持一行，并在可用宽度内完整显示，不自动拆到下一行。"],
                    ["原本只有一行的文字应保持一行，并在可用宽度内完整显示，不自动拆到下一行。"]),
                new("two-source-lines", [Line("First physical row", 24, 30, 360, 24), Line("Second physical row", 24, 88, 360, 24)],
                    ["第一条\r\n保持单行", "第二条\n也保持单行"], ["第一条 保持单行", "第二条 也保持单行"]),
                new("long-single-line", [Line("A long translated label", 24, 35, 420, 28)], [veryLong], [veryLong]),
                new("two-columns", [Line("Right lower", 380, 100, 250, 24), Line("Left upper", 24, 30, 250, 24),
                    Line("Right upper", 380, 30, 250, 24), Line("Left lower", 24, 100, 250, 24)],
                    ["右栏第二行的完整译文不可串到左栏", "左栏第一行的完整译文不可移到第二行", "右栏第一行保持原始位置", "左栏第二行保持原始位置"],
                    ["右栏第二行的完整译文不可串到左栏", "左栏第一行的完整译文不可移到第二行", "右栏第一行保持原始位置", "左栏第二行保持原始位置"])
            ];
            var background = Source(cases[0].Lines);
            overlay = new CaptureOverlayWindow(host, null, new CaptureFrame(0, 0, background));
            var root = (Canvas)overlay.FindName("Root"); root.Width = 800; root.Height = 600;
            root.Measure(new Size(800, 600)); root.Arrange(new Rect(0, 0, 800, 600)); root.UpdateLayout();
            object? Invoke(string name, params object[] values) => typeof(CaptureOverlayWindow).GetMethod(name, Private)!.Invoke(overlay, values);
            var item = Invoke("CreateSelection", false)!; var type = item.GetType();
            ((IList)typeof(CaptureOverlayWindow).GetField("_selections", Private)!.GetValue(overlay)!).Add(item);
            foreach (var scenario in cases)
            foreach (var scale in new[] { 1d, .5d })
            {
                var prefix = scenario.Name + "@" + scale.ToString(CultureInfo.InvariantCulture);
                var source = Source(scenario.Lines); var width = source.PixelWidth * scale; var height = source.PixelHeight * scale;
                type.GetField("Bounds")!.SetValue(item, new Rect(25, 25, width, height)); Invoke("UpdateSelection", item);
                ((Image)type.GetProperty("Image")!.GetValue(item)!).Source = source;
                Invoke("RenderTextOverlays", item, source, scenario.Lines, scenario.Translations, true);
                var canvas = (Canvas)type.GetProperty("TextOverlays")!.GetValue(item)!;
                canvas.Measure(new Size(width, height)); canvas.Arrange(new Rect(0, 0, width, height)); canvas.UpdateLayout();
                var hosts = canvas.Children.OfType<Grid>().Where(grid => grid.Children.Count == 1 && grid.Children[0] is OutlinedTextVisual).ToArray();
                Check(hosts.Length == scenario.Lines.Length, prefix + " visible row identity count");
                var cells = TranslationOverlayLayoutService.AllocateCells(scenario.Lines.Select(line => new Rect(line.X * scale, line.Y * scale, line.Width * scale, line.Height * scale)).ToArray(), new Size(width, height));
                var geometries = new List<object>();
                for (var index = 0; index < hosts.Length; index++)
                {
                    var visual = (OutlinedTextVisual)hosts[index].Children[0];
                    var rows = (IReadOnlyList<string>)typeof(OutlinedTextVisual).GetField("_lines", Private)!.GetValue(visual)!;
                    var fontSize = (double)typeof(OutlinedTextVisual).GetField("_fontSize", Private)!.GetValue(visual)!;
                    var lineHeight = (double)typeof(OutlinedTextVisual).GetField("_lineHeight", Private)!.GetValue(visual)!;
                    var bounds = new Rect(Canvas.GetLeft(hosts[index]), Canvas.GetTop(hosts[index]), hosts[index].Width, hosts[index].Height);
                    Check(rows.Count == 1, prefix + " row " + index + " stays one physical line");
                    Check(rows[0] == scenario.Expected[index], prefix + " row " + index + " preserves normalized complete translation");
                    Check(Math.Abs(bounds.Left + 3 - scenario.Lines[index].X * scale) < .1 && Math.Abs(bounds.Top + 1 - scenario.Lines[index].Y * scale) < .1,
                        prefix + " row " + index + " preserves source ink origin");
                    var cell = cells[index]; cell.Inflate(.01, .01);
                    Check(cell.Contains(bounds), prefix + " row " + index + " stays inside assigned cell");
                    Check(double.IsFinite(fontSize) && fontSize > 0 && lineHeight + 2 <= bounds.Height + .1,
                        prefix + " row " + index + " fits available vertical space");
                    var measured = (double)typeof(CaptureOverlayWindow).GetMethod("MeasureTranslationText", BindingFlags.Static | BindingFlags.NonPublic)!.Invoke(null, [rows[0], fontSize, 1d])!;
                    Check(measured + 6 <= bounds.Width + .15, prefix + " row " + index + " complete text fits horizontally");
                    for (var previous = 0; previous < index; previous++)
                    {
                        var other = hosts[previous]; var overlap = Rect.Intersect(bounds, new Rect(Canvas.GetLeft(other), Canvas.GetTop(other), other.Width, other.Height));
                        Check(overlap.IsEmpty || overlap.Width * overlap.Height < .001, prefix + " row " + index + " does not overlap " + previous);
                    }
                    geometries.Add(new { index, rows, fontSize, bounds });
                }
                var selectable = (Canvas)type.GetProperty("TextSelection")!.GetValue(item)!;
                var box = selectable.Children.OfType<RichTextBox>().Single();
                var paragraphs = box.Document.Blocks.OfType<Paragraph>().Select(paragraph => new TextRange(paragraph.ContentStart, paragraph.ContentEnd).Text.TrimEnd('\r', '\n')).ToArray();
                var expectedOrder = Enumerable.Range(0, scenario.Lines.Length).OrderBy(i => scenario.Lines[i].Y).ThenBy(i => scenario.Lines[i].X).Select(i => scenario.Expected[i]).ToArray();
                Check(paragraphs.SequenceEqual(expectedOrder), prefix + " selectable rows equal visible single-line translations in reading order");
                var path = Path.Combine(directory, prefix.Replace('@', '-') + ".png");
                SaveActualCanvas(canvas, source, width, height, path);
                var exported = (BitmapSource)typeof(CaptureOverlayWindow).GetMethod("RenderTranslationOverlay", BindingFlags.Static | BindingFlags.NonPublic)!.Invoke(null, [item, source.PixelWidth, source.PixelHeight])!;
                Check(exported.PixelWidth == source.PixelWidth && exported.PixelHeight == source.PixelHeight, prefix + " export retains physical image size");
                var pixels = new byte[exported.PixelWidth * exported.PixelHeight * 4]; exported.CopyPixels(pixels, exported.PixelWidth * 4, 0);
                Check(Enumerable.Range(0, pixels.Length / 4).Any(i => pixels[i * 4 + 3] != 0), prefix + " exported actual visual has nontransparent pixels");
                results.Add(new { scenario = scenario.Name, scale, geometries, selectableRows = paragraphs, path });
            }
            Check(new WindowInteropHelper(overlay).Handle == IntPtr.Zero && !overlay.IsLoaded && !overlay.IsVisible, "replay creates no native window or desktop input");
            Check(host.IsIsolatedReplay && host.GetConversationChannels().Count == 0, "isolated host has no provider channels");
        }
        catch (Exception error) { failure = error.ToString(); }
        finally { overlay?.Close(); app.Shutdown(); }
        File.WriteAllText(Path.Combine(directory, "result.json"), JsonSerializer.Serialize(new
        {
            passed = failure is null, checks, results, failure, productSha256 = Hash(typeof(CaptureOverlayWindow).Assembly.Location), harnessSha256 = Hash(typeof(TranslationSingleLineReplay).Assembly.Location),
            syntheticOnly = true, realProviderInvoked = false, productionSettingsUsed = false, desktopInputInjected = false, clipboardUsed = false
        }, new JsonSerializerOptions { WriteIndented = true }), new UTF8Encoding(false));
        Environment.ExitCode = failure is null ? 0 : 1;
    }

    private static OcrLine Line(string text, double x, double y, double width, double height) => new(text, x, y, width, height, [new OcrWord(text, x, y, width, height)]);
    private static string Hash(string path) { using var stream = File.OpenRead(path); return Convert.ToHexString(SHA256.HashData(stream)); }
    private static BitmapSource Source(IEnumerable<OcrLine> lines)
    {
        var visual = new DrawingVisual();
        using (var drawing = visual.RenderOpen())
        {
            drawing.DrawRectangle(Brushes.White, null, new Rect(0, 0, 720, 240));
            foreach (var line in lines) drawing.DrawText(new FormattedText(line.Text, CultureInfo.InvariantCulture, FlowDirection.LeftToRight, new Typeface("Segoe UI"), line.Height * .78, Brushes.Black, 1), new Point(line.X, line.Y));
        }
        var bitmap = new RenderTargetBitmap(720, 240, 96, 96, PixelFormats.Pbgra32); bitmap.Render(visual); bitmap.Freeze(); return bitmap;
    }
    private static void SaveActualCanvas(Canvas canvas, BitmapSource source, double width, double height, string path)
    {
        var parent = (Panel)VisualTreeHelper.GetParent(canvas); var index = parent.Children.IndexOf(canvas);
        var renderHost = new Canvas { Width = width, Height = height };
        renderHost.Children.Add(new Image { Source = source, Width = width, Height = height, Stretch = Stretch.Fill });
        parent.Children.Remove(canvas);
        try
        {
            renderHost.Children.Add(canvas); renderHost.Measure(new Size(width, height)); renderHost.Arrange(new Rect(0, 0, width, height)); renderHost.UpdateLayout();
            var bitmap = new RenderTargetBitmap((int)Math.Ceiling(width), (int)Math.Ceiling(height), 96, 96, PixelFormats.Pbgra32); bitmap.Render(renderHost);
            var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(bitmap)); using var stream = File.Create(path); encoder.Save(stream);
        }
        finally { renderHost.Children.Remove(canvas); parent.Children.Insert(index, canvas); }
    }
}
