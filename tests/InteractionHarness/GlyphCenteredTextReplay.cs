// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Automation.Peers;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Views;
using Color = System.Windows.Media.Color;
using Panel = System.Windows.Controls.Panel;
using Size = System.Windows.Size;

internal static class GlyphCenteredTextReplay
{
    internal static void Verify(GlyphCenteredText glyph, List<string> checks, string directory, string name)
    {
        var parent = VisualTreeHelper.GetParent(glyph) as FrameworkElement;
        var width = parent?.ActualWidth > 0 ? parent.ActualWidth : 64;
        var height = parent?.ActualHeight > 0 ? parent.ActualHeight : 64;
        var index = parent is Panel panel ? panel.Children.IndexOf(glyph) : -1;
        if (parent is Panel oldPanel) oldPanel.Children.Remove(glyph);
        else if (parent is Decorator decorator) decorator.Child = null;
        var host = new Grid { Width = width, Height = height };
        host.Children.Add(glyph);
        try
        {
            foreach (var dpi in new[] { 96, 168, 192 })
            {
                VisualTreeHelper.SetRootDpi(host, new DpiScale(dpi / 96d, dpi / 96d));
                host.Measure(new Size(width, height)); host.Arrange(new Rect(0, 0, width, height)); host.UpdateLayout();
                var image = new RenderTargetBitmap((int)Math.Ceiling(width * dpi / 96d), (int)Math.Ceiling(height * dpi / 96d), dpi, dpi, PixelFormats.Pbgra32);
                image.Render(host);
                var bytes = new byte[image.PixelWidth * image.PixelHeight * 4]; image.CopyPixels(bytes, image.PixelWidth * 4, 0);
                var xs = new List<int>(); var ys = new List<int>();
                for (var y = 0; y < image.PixelHeight; y++) for (var x = 0; x < image.PixelWidth; x++)
                    if (bytes[(y * image.PixelWidth + x) * 4 + 3] > 16) { xs.Add(x); ys.Add(y); }
                if (string.IsNullOrWhiteSpace(glyph.Text)) Require(xs.Count == 0, name + " whitespace has no pixels " + dpi);
                else
                {
                    Require(xs.Count > 0, name + " has visible ink " + dpi);
                    Require(Math.Abs((xs.Min() + xs.Max() + 1) / 2d - image.PixelWidth / 2d) <= 1 &&
                        Math.Abs((ys.Min() + ys.Max() + 1) / 2d - image.PixelHeight / 2d) <= 1,
                        name + " actual ink centered within 1 physical pixel " + dpi);
                    Require(xs.Min() > 0 && ys.Min() > 0 && xs.Max() < image.PixelWidth - 1 && ys.Max() < image.PixelHeight - 1,
                        name + " ink not clipped " + dpi);
                }
                var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(image));
                using var stream = File.Create(Path.Combine(directory, "glyph-" + name + "-" + dpi + ".png")); encoder.Save(stream);
            }
        }
        finally
        {
            host.Children.Remove(glyph);
            if (parent is Panel restoredPanel) restoredPanel.Children.Insert(index, glyph);
            else if (parent is Decorator restoredDecorator) restoredDecorator.Child = glyph;
            parent?.UpdateLayout();
        }
        void Require(bool value, string description) { if (!value) throw new InvalidOperationException(description); checks.Add(description); }
    }

    internal static void VerifyAll(CaptureOverlayWindow overlay, List<string> checks, string directory, string language)
    {
        var icon = (Border)((System.Windows.Controls.Button)overlay.FindName("DrawingNumberButton")).Content;
        icon.Measure(new Size(20, 20)); icon.Arrange(new Rect(0, 0, 20, 20)); icon.UpdateLayout();
        Verify((GlyphCenteredText)icon.Child, checks, directory, language + "-toolbar-icon");
        var numberType = typeof(CaptureOverlayWindow).GetNestedType("NumberDrawingElement", BindingFlags.NonPublic)!;
        var create = typeof(CaptureOverlayWindow).GetMethod("CreateNumberDrawingVisual", BindingFlags.Static | BindingFlags.NonPublic)!;
        foreach (var number in new[] { 1, 2, 3, 10, 100 })
        {
            var element = Activator.CreateInstance(numberType, [Guid.NewGuid(), 0d, 0d, 40d, number, Color.FromRgb(49, 140, 255)])!;
            var border = (Border)create.Invoke(null, [element])!;
            border.Measure(new Size(40, 40)); border.Arrange(new Rect(0, 0, 40, 40)); border.UpdateLayout();
            Verify((GlyphCenteredText)border.Child, checks, directory, language + "-number-" + number);
        }
        var sample = new GlyphCenteredText { Text = "100", FontSize = 22 };
        Verify(sample, checks, directory, language + "-changed-100");
        sample.Text = "1"; sample.FontSize = 30; sample.FontFamily = new System.Windows.Media.FontFamily("Arial");
        Verify(sample, checks, directory, language + "-changed-1");
        sample.Text = "  "; Verify(sample, checks, directory, language + "-empty");
        sample.Text = "12";
        var peer = UIElementAutomationPeer.CreatePeerForElement(sample)!;
        if (peer.GetName() != "12" || peer.GetAutomationControlType() != AutomationControlType.Text) throw new InvalidOperationException("Glyph text automation fallback");
        AutomationProperties.SetName(sample, "Countdown value");
        if (peer.GetName() != "Countdown value") throw new InvalidOperationException("Glyph explicit automation name");
        checks.Add("glyph automation exposes Text control and respects explicit name");
    }
}
