// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Reflection;
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
using Color = System.Windows.Media.Color;
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
                VerifySeekBounds(bar, Check);
                bar.SetState(TimeSpan.FromSeconds(126), TimeSpan.FromSeconds(12.5), TimeSpan.FromSeconds(94.75), TimeSpan.FromSeconds(48.25), false, true);
                var renderRoot = new Canvas { Background = new SolidColorBrush(Color.FromRgb(239, 243, 249)) };
                VisualTreeHelper.SetRootDpi(renderRoot, new DpiScale(2, 2));
                renderRoot.Children.Add(bar); Canvas.SetLeft(bar, 32); Canvas.SetTop(bar, 32);
                bar.Measure(new Size(width, double.PositiveInfinity));
                renderRoot.Width = width + 64; renderRoot.Height = bar.DesiredSize.Height + 64;
                renderRoot.Measure(new Size(renderRoot.Width, renderRoot.Height));
                renderRoot.Arrange(new Rect(0, 0, renderRoot.Width, renderRoot.Height)); renderRoot.UpdateLayout();
                VerifyHandleHitTargets(bar, Check);
                var height = (int)Math.Ceiling(bar.ActualHeight);
                Check(height > 20 && height < 180 && Math.Abs(bar.ActualWidth - width) < .1, $"{width}: finite compact layout");
                var descendants = Descendants(bar).ToArray();
                var buttons = new[] { "_play", "_setStart", "_setEnd", "_reset" }.Select(name => Field<Button>(bar, name)).ToArray();
                Check(buttons.All(button => descendants.Contains(button)), $"{width}: all four original actions available");
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
                var rendered = new RenderTargetBitmap((int)Math.Ceiling(renderRoot.Width * 2), (int)Math.Ceiling(renderRoot.Height * 2), 192, 192, PixelFormats.Pbgra32);
                rendered.Render(renderRoot);
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
            offscreenOnly = true, desktopInput = false, productionSettings = false, clipboard = false,
            interactionMethod = "Actual control state and shared interaction methods invoked in process; no native mouse capture or keyboard focus is simulated."
        }, new JsonSerializerOptions { WriteIndented = true }), new UTF8Encoding(false));
        app.Shutdown();
    }
    private static T Field<T>(VideoTrimBar bar, string name) => (T)typeof(VideoTrimBar).GetField(name, BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(bar)!;

    private static void VerifyHandleHitTargets(VideoTrimBar bar, Action<bool, string> check)
    {
        var track = Field<Canvas>(bar, "_track");
        var first = Field<FrameworkElement>(bar, "_startThumb");
        var last = Field<FrameworkElement>(bar, "_endThumb");
        var position = Field<FrameworkElement>(bar, "_positionThumb");
        var positionLine = Field<FrameworkElement>(bar, "_positionLine");
        check(!positionLine.IsHitTestVisible, "playhead guide cannot intercept range handles");
        foreach (var startSeconds in new[] { 0d, 12.5, 125.9 })
        {
            var start = TimeSpan.FromSeconds(startSeconds); var end = start + TimeSpan.FromMilliseconds(100);
            bar.SetState(TimeSpan.FromSeconds(126), start, end, start + TimeSpan.FromMilliseconds(50), false, true);
            bar.UpdateLayout();
            Rect Bounds(FrameworkElement element) => element.TransformToAncestor(track).TransformBounds(new Rect(element.RenderSize));
            var firstBounds = Bounds(first); var lastBounds = Bounds(last); var positionBounds = Bounds(position);
            check(positionBounds.Bottom <= firstBounds.Top && positionBounds.Bottom <= lastBounds.Top,
                "upper playhead does not overlap endpoint hit rows at " + startSeconds);
            var firstExclusiveWidth = Math.Max(0, Math.Min(firstBounds.Right, lastBounds.Left) - firstBounds.Left);
            var lastExclusiveWidth = Math.Max(0, lastBounds.Right - Math.Max(lastBounds.Left, firstBounds.Right));
            check(firstExclusiveWidth >= 10 && lastExclusiveWidth >= 10 && firstBounds.Left >= 0 && lastBounds.Right <= track.ActualWidth,
                "100ms endpoint handles each retain at least 10 DIP of exclusive hit width inside track at " + startSeconds);
            foreach (var (name, thumb) in new[] { ("start", first), ("end", last), ("position", position) })
            {
                var point = thumb.TranslatePoint(new Point(thumb.ActualWidth / 2, thumb.ActualHeight / 2), bar);
                DependencyObject? hit = null;
                VisualTreeHelper.HitTest(bar,
                    visual => visual is UIElement element && (element.Visibility != Visibility.Visible || !element.IsHitTestVisible)
                        ? HitTestFilterBehavior.ContinueSkipSelfAndChildren : HitTestFilterBehavior.Continue,
                    result => { hit = result.VisualHit; return HitTestResultBehavior.Stop; },
                    new PointHitTestParameters(point));
                while (hit is not null && !ReferenceEquals(hit, thumb)) hit = VisualTreeHelper.GetParent(hit);
                check(ReferenceEquals(hit, thumb), $"100ms {name} handle is independently hit-testable at {startSeconds}, width {bar.Width}");
            }
        }
        bar.SetState(TimeSpan.FromSeconds(126), TimeSpan.FromSeconds(12.5), TimeSpan.FromSeconds(94.75), TimeSpan.FromSeconds(48.25), false, true);
        bar.UpdateLayout();
    }

    private static void VerifySeekBounds(VideoTrimBar bar, Action<bool, string> check)
    {
        var type = typeof(VideoTrimBar);
        var interaction = type.GetNestedType("Interaction", BindingFlags.NonPublic)!;
        object? Invoke(string name, params object?[] values) => type.GetMethod(name, BindingFlags.Instance | BindingFlags.NonPublic)!.Invoke(bar, values);
        bool Begin(string mode) => (bool)Invoke("BeginInteraction", Enum.Parse(interaction, mode), null)!;
        var duration = TimeSpan.FromSeconds(126); var start = TimeSpan.FromSeconds(12.5); var end = TimeSpan.FromSeconds(94.75);
        var initial = TimeSpan.FromSeconds(48.25);
        var seeks = new List<(TimeSpan Position, bool Final)>();
        var ranges = new List<(TimeSpan Start, TimeSpan End, bool Final)>();
        var completions = new List<bool>();
        void Seek(TimeSpan position, bool final) => seeks.Add((position, final));
        void Range(TimeSpan first, TimeSpan last, bool final) => ranges.Add((first, last, final));
        void Complete(bool cancelled) => completions.Add(cancelled);
        bar.SeekRequested += Seek; bar.RangeChanged += Range; bar.InteractionCompleted += Complete;
        try
        {
            bar.SetState(duration, start, end, TimeSpan.Zero, false, true);
            check(Field<TimeSpan>(bar, "_position") == start, "idle playhead before retained start clamps to start");
            bar.SetState(duration, start, end, duration, false, true);
            check(Field<TimeSpan>(bar, "_position") == end, "idle playhead after retained end clamps to end");
            foreach (var (requested, expected) in new[]
            {
                (TimeSpan.FromSeconds(-1), start), (TimeSpan.Zero, start), (start - TimeSpan.FromTicks(1), start),
                (start, start), (initial, initial), (end, end), (end + TimeSpan.FromTicks(1), end),
                (duration, end), (duration + TimeSpan.FromSeconds(1), end)
            })
            {
                bar.SetState(duration, start, end, initial, false, true);
                seeks.Clear(); ranges.Clear(); completions.Clear();
                check(Begin("Seek"), "seek interaction starts for " + requested);
                Invoke("UpdateInteraction", requested);
                check(seeks.Count == 1 && seeks[0] == (expected, false), "intermediate seek is clipped to selected range: " + requested);
                Invoke("CompleteInteraction", false);
                check(seeks.Count == 2 && seeks[1] == (expected, true) && seeks.All(value => value.Position >= start && value.Position <= end),
                    "final seek and every intermediate event remain within retained bounds: " + requested);
                check(ranges.Count == 0 && Field<TimeSpan>(bar, "_start") == start && Field<TimeSpan>(bar, "_end") == end,
                    "seeking never edits clip boundaries: " + requested);
                check(!bar.IsInteracting && completions.SequenceEqual(new[] { false }), "seek completes exactly once: " + requested);
            }
            bar.SetState(duration, start, end, initial, false, true);
            seeks.Clear(); ranges.Clear(); completions.Clear();
            check(Begin("Seek"), "cancelled seek starts");
            Invoke("UpdateInteraction", duration);
            bar.CancelInteraction();
            check(Field<TimeSpan>(bar, "_position") == initial && Field<TimeSpan>(bar, "_start") == start && Field<TimeSpan>(bar, "_end") == end,
                "cancel restores original playhead and range");
            check(seeks.SequenceEqual(new[] { (end, false) }) && ranges.Count == 0 && completions.SequenceEqual(new[] { true }) && !bar.IsInteracting,
                "cancel emits no final seek or range mutation");
            check(Begin("Start"), "start boundary can still be edited");
            Invoke("UpdateInteraction", TimeSpan.FromSeconds(5));
            check(Field<TimeSpan>(bar, "_start") == TimeSpan.FromSeconds(5), "seek restriction does not prevent extending the start boundary");
            bar.CancelInteraction();
            check(Begin("End"), "end boundary can still be edited");
            Invoke("UpdateInteraction", TimeSpan.FromSeconds(110));
            check(Field<TimeSpan>(bar, "_end") == TimeSpan.FromSeconds(110), "seek restriction does not prevent extending the end boundary");
            bar.CancelInteraction();
        }
        finally { bar.CancelInteraction(); bar.SeekRequested -= Seek; bar.RangeChanged -= Range; bar.InteractionCompleted -= Complete; }
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
