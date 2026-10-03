// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application = System.Windows.Application;
using Size = System.Windows.Size;
using Point = System.Windows.Point;
using Button = System.Windows.Controls.Button;

/// <summary>Offscreen layout evidence, using real overlay toolbar resources, not desktop interaction.</summary>
internal static class VideoTrimVisualReplay
{
    internal static void Run(string[] args)
    {
        var directory = ReplayOutputDirectory.PrepareWorkingDirectory("video-trim-visual");
        var english = args.Contains("--english");
        LocalizationService.Initialize(english ? "en-US" : "zh-CN", null);
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(directory, "logs"));
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        app.Resources.MergedDictionaries.Add(new ResourceDictionary { Source = new Uri("/MewuAI;component/Themes/LightTheme.xaml", UriKind.Relative) });
        using var host = new AppHost(app, null, "MewuAI-TrimVisual-" + Guid.NewGuid().ToString("N"));
        host.Settings.EnableVoiceInput = false; host.Settings.AutomaticallyStartListening = false; host.Settings.SaveConversationHistory = false;
        var pixels = new byte[800 * 600 * 4];
        var bitmap = BitmapSource.Create(800, 600, 96, 96, PixelFormats.Bgra32, null, pixels, 800 * 4); bitmap.Freeze();
        var overlay = new CaptureOverlayWindow(host, null, new CaptureFrame(0, 0, bitmap));
        var results = new List<object>(); var checks = new List<string>();
        void Check(bool value, string name) { if (!value) throw new InvalidOperationException(name); checks.Add(name); }
        try
        {
            foreach (var width in new[] { 300, 420, 720 })
            {
                var bar = new VideoTrimBar { Width = width };
                bar.Resources.MergedDictionaries.Add(overlay.Resources);
                bar.RaiseEvent(new RoutedEventArgs(FrameworkElement.LoadedEvent, bar));
                bar.SetState(TimeSpan.FromSeconds(126), TimeSpan.FromSeconds(12.5), TimeSpan.FromSeconds(94.75), TimeSpan.FromSeconds(48.25), false, true);
                bar.Measure(new Size(width, double.PositiveInfinity));
                bar.Arrange(new Rect(0, 0, width, bar.DesiredSize.Height)); bar.UpdateLayout();
                var height = (int)Math.Ceiling(bar.ActualHeight);
                Check(height > 20 && height < 180 && Math.Abs(bar.ActualWidth - width) < .1, $"{width}: finite compact layout");
                var descendants = Descendants(bar).ToArray();
                var buttons = descendants.OfType<Button>().ToArray();
                Check(buttons.Length == 4, $"{width}: all four actions available");
                foreach (var button in buttons)
                {
                    var origin = button.TranslatePoint(new Point(), bar);
                    Check(origin.X >= -.5 && origin.Y >= -.5 && origin.X + button.ActualWidth <= width + .5 && origin.Y + button.ActualHeight <= height + .5,
                        $"{width}: action remains inside surface: {AutomationProperties.GetName(button)}");
                    Check(!string.IsNullOrWhiteSpace(AutomationProperties.GetName(button)), $"{width}: action has accessible name");
                    Check(ReferenceEquals(button.Style, overlay.Resources["ToolbarIconButton"]), $"{width}: action uses real overlay toolbar style");
                }
                foreach (var block in descendants.OfType<TextBlock>().Where(block => block.Visibility == Visibility.Visible && !string.IsNullOrEmpty(block.Text)))
                {
                    var origin = block.TranslatePoint(new Point(), bar);
                    Check(origin.X >= -.5 && origin.Y >= -.5 && origin.X + block.ActualWidth <= width + .5 && origin.Y + block.ActualHeight <= height + .5,
                        $"{width}: time label remains inside surface: {block.Text}");
                }
                var text = descendants.OfType<TextBlock>().Select(block => new { block.Text, block.ActualWidth, block.ActualHeight }).ToArray();
                var visual = new DrawingVisual();
                using (var drawing = visual.RenderOpen()) drawing.DrawRectangle(new VisualBrush(bar), null, new Rect(0, 0, width, height));
                var rendered = new RenderTargetBitmap(width * 2, height * 2, 192, 192, PixelFormats.Pbgra32); rendered.Render(visual);
                var file = Path.Combine(directory, $"trim-{(english ? "en" : "zh")}-{width}.png");
                var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(rendered)); using (var output = File.Create(file)) encoder.Save(output);
                results.Add(new { width, height, file, text, buttons = buttons.Select(button => new { name = AutomationProperties.GetName(button), button.ActualWidth, button.ActualHeight }) });
            }
            Check(host.IsIsolatedReplay && host.GetConversationChannels().Count == 0, "isolated host contains no provider channels");
            Check(new WindowInteropHelper(overlay).Handle == IntPtr.Zero && !overlay.IsLoaded && !overlay.IsVisible, "no native window or Loaded activation");
        }
        finally { overlay.Close(); }
        using var product = File.OpenRead(typeof(VideoTrimBar).Assembly.Location);
        File.WriteAllText(Path.Combine(directory, $"render-{(english ? "en" : "zh")}.json"), JsonSerializer.Serialize(new
        {
            passed = true, language = english ? "en-US" : "zh-CN", productSha256 = Convert.ToHexString(SHA256.HashData(product)), checks, results,
            offscreenOnly = true, desktopInput = false, productionSettings = false, clipboard = false
        }, new JsonSerializerOptions { WriteIndented = true }), new UTF8Encoding(false));
        app.Shutdown();
    }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject parent)
    {
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
        {
            var child = VisualTreeHelper.GetChild(parent, i); yield return child;
            foreach (var descendant in Descendants(child)) yield return descendant;
        }
    }
}
