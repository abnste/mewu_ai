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
using Point = System.Windows.Point;
using Size = System.Windows.Size;

/// <summary>Actual countdown visuals, with no recording, native window or input routing changes.</summary>
internal static class RecordingCountdownVisualReplay
{
    internal static void Run(string[] args)
    {
        var directory = ReplayOutputDirectory.PrepareWorkingDirectory("recording-countdown");
        var english = args.Contains("--english"); var language = english ? "en" : "zh";
        LocalizationService.Initialize(english ? "en-US" : "zh-CN", null);
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(directory, "logs"));
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        app.Resources.MergedDictionaries.Add(new ResourceDictionary { Source = new Uri("/MewuAI;component/Themes/LightTheme.xaml", UriKind.Relative) });
        using var host = new AppHost(app, null, "MewuAI-CountdownVisual-" + Guid.NewGuid().ToString("N"));
        host.Settings.EnableVoiceInput = false; host.Settings.AutomaticallyStartListening = false; host.Settings.SaveConversationHistory = false;
        var image = BitmapSource.Create(800, 600, 96, 96, PixelFormats.Bgra32, null, new byte[800 * 600 * 4], 800 * 4); image.Freeze();
        var overlay = new CaptureOverlayWindow(host, null, new CaptureFrame(0, 0, image));
        var checks = new List<string>(); var layouts = new List<object>(); var renders = new List<string>(); string? failure = null;
        const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic;
        object? Invoke(string name, params object[] values) => typeof(CaptureOverlayWindow).GetMethod(name, flags)!.Invoke(overlay, values);
        void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); checks.Add(message); }
        try
        {
            var root = (Canvas)overlay.FindName("Root"); root.Width = 800; root.Height = 600;
            root.Measure(new Size(800, 600)); root.Arrange(new Rect(0, 0, 800, 600)); root.UpdateLayout();
            var card = (FrameworkElement)overlay.FindName("RecordingCountdown"); card.Visibility = Visibility.Visible;
            var digit = (GlyphCenteredText)overlay.FindName("RecordingCountdownText");
            var label = (TextBlock)overlay.FindName("RecordingCountdownLabel"); var hint = (TextBlock)overlay.FindName("RecordingCountdownHint");
            var scale = (ScaleTransform)overlay.FindName("RecordingCountdownScale");
            var arcs = Enumerable.Range(1, 3).Select(index => (UIElement)overlay.FindName("RecordingCountdownArc" + index)).ToArray();
            var monitor = (Rect)Invoke("MonitorBounds", new Rect(80, 80, 320, 180))!;
            var center = new Point(monitor.Left + monitor.Width / 2, monitor.Top + monitor.Height / 2);
            var selections = new[] { new Rect(center.X - 300, center.Y - 200, 600, 400),
                new Rect(center.X - 15, center.Y - 10, 30, 20), new Rect(monitor.Left, monitor.Top, 30, 20),
                new Rect(monitor.Right - 30, monitor.Bottom - 20, 30, 20) };
            foreach (var selection in selections)
            {
                Invoke("PositionRecordingCountdown", selection);
                var left = Canvas.GetLeft(card); var top = Canvas.GetTop(card);
                Check(left >= monitor.Left + 7.9 && top >= monitor.Top + 7.9 && left + card.Width <= monitor.Right - 7.9 && top + card.Height <= monitor.Bottom - 7.9,
                    "countdown stays within monitor safe margins for selection " + selection);
                layouts.Add(new { selection, left, top, card.Width, card.Height, monitor });
            }
            foreach (var value in new[] { 3, 2, 1 })
            {
                var left = Canvas.GetLeft(card); var top = Canvas.GetTop(card);
                Invoke("ShowRecordingCountdownStep", value);
                Check(digit.Text == value.ToString(System.Globalization.CultureInfo.InvariantCulture), "actual step displays " + value);
                Check(Canvas.GetLeft(card) == left && Canvas.GetTop(card) == top && !card.HasAnimatedProperties,
                    "step " + value + " does not animate or move the card");
                Check(label.Text == (english ? "Recording soon" : "即将录制") && hint.Text == (english ? "Esc to cancel" : "Esc 取消"),
                    "step " + value + " has localized status and cancel hint");
                Check(arcs.Select(arc => arc.Opacity >= .99).SequenceEqual(new[] { value >= 3, value >= 2, value >= 1 }),
                    "step " + value + " updates three progress segments");
                Invoke("ClearRecordingCountdownAnimations");
                Check(!digit.HasAnimatedProperties && !scale.HasAnimatedProperties && digit.Opacity == 1 && scale.ScaleX == 1 && scale.ScaleY == 1,
                    "step " + value + " clears visual clocks and restores stable values");
                card.Measure(new Size(card.Width, card.Height)); card.Arrange(new Rect(0, 0, card.Width, card.Height)); card.UpdateLayout();
                foreach (var text in new[] { label, hint })
                {
                    var origin = text.TranslatePoint(new Point(), card);
                    Check(origin.X >= 0 && origin.Y >= 0 && origin.X + text.ActualWidth <= card.ActualWidth + .1 && origin.Y + text.ActualHeight <= card.ActualHeight + .1,
                        "step " + value + " text remains within card: " + text.Text);
                    var desired = new FormattedText(text.Text, System.Globalization.CultureInfo.CurrentUICulture, text.FlowDirection,
                        new Typeface(text.FontFamily, text.FontStyle, text.FontWeight, text.FontStretch), text.FontSize, text.Foreground, 1);
                    Check(desired.Width <= text.ActualWidth + 1, "step " + value + " text is not horizontally clipped: " + text.Text);
                }
                GlyphCenteredTextReplay.Verify(digit, checks, directory, language + "-countdown-" + value);
                foreach (var dpi in new[] { 96, 168, 192 })
                {
                    var width = card.ActualWidth + 32; var height = card.ActualHeight + 32;
                    var originalLeft = Canvas.GetLeft(card); var originalTop = Canvas.GetTop(card);
                    var originalIndex = root.Children.IndexOf(card);
                    var renderHost = new Canvas { Width = width, Height = height };
                    var rendered = new RenderTargetBitmap((int)Math.Ceiling(width * dpi / 96), (int)Math.Ceiling(height * dpi / 96), dpi, dpi, PixelFormats.Pbgra32);
                    root.Children.Remove(card);
                    try
                    {
                        renderHost.Children.Add(card); Canvas.SetLeft(card, 16); Canvas.SetTop(card, 16);
                        renderHost.Measure(new Size(width, height)); renderHost.Arrange(new Rect(0, 0, width, height)); renderHost.UpdateLayout();
                        rendered.Render(renderHost);
                    }
                    finally
                    {
                        renderHost.Children.Remove(card); root.Children.Insert(originalIndex, card);
                        Canvas.SetLeft(card, originalLeft); Canvas.SetTop(card, originalTop);
                    }
                    var pixels = new byte[rendered.PixelWidth * rendered.PixelHeight * 4];
                    rendered.CopyPixels(pixels, rendered.PixelWidth * 4, 0);
                    Check(Enumerable.Range(0, pixels.Length / 4).Count(index => pixels[index * 4 + 3] > 0) > pixels.Length / 16,
                        "step " + value + " renders nontransparent actual card pixels at " + dpi + " DPI");
                    var path = Path.Combine(directory, $"countdown-{language}-{value}-{dpi}.png"); var encoder = new PngBitmapEncoder();
                    encoder.Frames.Add(BitmapFrame.Create(rendered)); using (var output = File.Create(path)) encoder.Save(output); renders.Add(path);
                }
            }
            GlyphCenteredTextReplay.VerifyAll(overlay, checks, directory, language);
            using var canceled = new CancellationTokenSource(); canceled.Cancel();
            var task = (Task)Invoke("RunRecordingCountdownAsync", canceled.Token)!;
            var wasCanceled = false; try { task.GetAwaiter().GetResult(); } catch (OperationCanceledException) { wasCanceled = true; }
            Check(wasCanceled, "canceled countdown task terminates before another visual step");
            Invoke("ClearRecordingCountdownAnimations");
            Check(new WindowInteropHelper(overlay).Handle == IntPtr.Zero && !overlay.IsLoaded && !overlay.IsVisible,
                "visual replay never creates native window or starts recording/input routing");
            Check(host.IsIsolatedReplay && host.GetConversationChannels().Count == 0, "visual replay uses isolated host without providers");
        }
        catch (Exception error) { failure = error.ToString(); }
        finally { overlay.Close(); app.Shutdown(); }
        using var assembly = File.OpenRead(typeof(CaptureOverlayWindow).Assembly.Location);
        File.WriteAllText(Path.Combine(directory, "countdown-" + language + ".json"), JsonSerializer.Serialize(new
        {
            passed = failure is null, language, productSha256 = Convert.ToHexString(SHA256.HashData(assembly)), checks, layouts, renders, failure,
            systemAllowsAnimation = SystemParameters.ClientAreaAnimation, smallSelectionMayBeSmallerThanCard = true, actualDesktopInput = false, nativeRecordingStarted = false
        }, new JsonSerializerOptions { WriteIndented = true }), new UTF8Encoding(false));
        Environment.ExitCode = failure is null ? 0 : 1;
    }
}
