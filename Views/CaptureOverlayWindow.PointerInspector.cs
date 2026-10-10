// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private BitmapSource? _pointerSampleImage;
    private int _pointerSampleX = -1, _pointerSampleY = -1;
    private Color? _pointerSampleColor;
    private readonly SolidColorBrush _pointerSampleBrush = new();

    private void UpdatePointerInspector(Point point)
    {
        if (_closed || _recordingMode || _recordingCountdownActive || _drawingMode || _longCaptureMode ||
            _promptDragging || _promptDockAnimating || PointerOverVideoTrim(point) ||
            (!_promptBarHidden && PromptBarHost.Visibility == Visibility.Visible && PointerOverPromptBar(point)) ||
            PointerInToolbarInteractionZone(point))
        {
            PointerInspector.Visibility = Visibility.Collapsed;
            return;
        }
        if (Root.ActualWidth <= 0 || Root.ActualHeight <= 0 || !double.IsFinite(point.X) || !double.IsFinite(point.Y) ||
            point.X < 0 || point.Y < 0 || point.X >= Root.ActualWidth || point.Y >= Root.ActualHeight)
        {
            PointerInspector.Visibility = Visibility.Collapsed;
            return;
        }
        // Use the same physical-point mapping as snapping and capture input.
        // Rounding also removes DIP round-trip error just below an integer.
        var (screenX, screenY) = ScreenCoordinateService.ToScreenPixelPoint(point, Root.ActualWidth, Root.ActualHeight,
            _frame.Image.PixelWidth, _frame.Image.PixelHeight, _frame.OriginX, _frame.OriginY);
        var pixelX = screenX - _frame.OriginX; var pixelY = screenY - _frame.OriginY;
        if (!ReferenceEquals(_pointerSampleImage, _frame.Image) || _pointerSampleX != pixelX || _pointerSampleY != pixelY)
        {
            if (!ScreenPixelSampler.TrySample(_frame.Image, pixelX, pixelY, out var color) ||
                !PointerMagnifier.SetSample(_frame.Image, pixelX, pixelY))
            {
                PointerInspector.Visibility = Visibility.Collapsed;
                return;
            }
            _pointerSampleImage = _frame.Image; _pointerSampleX = pixelX; _pointerSampleY = pixelY;
            if (_pointerSampleColor != color)
            {
                _pointerSampleColor = color; _pointerSampleBrush.Color = color;
                PointerColorSwatch.Fill = _pointerSampleBrush;
                PointerColorText.Text = FormatPointerColor(color);
            }
        }
        // Origins can change when the clean desktop frame is refreshed, even
        // when its bitmap and addressed local pixel are reused.
        PointerCoordinateText.Text = string.Create(CultureInfo.InvariantCulture, $"X {screenX} Y {screenY}");
        PointerInspector.Visibility = Visibility.Visible;
        // XAML fixes the card width and each row's height (18 + 90 + 18,
        // plus 18 when dimensions are shown). Text changes during dragging
        // invalidate Measure on every pixel; forcing it synchronously here
        // stalls the input path even though the outer size cannot change.
        var inspectorSize = new Size(PointerInspector.Width, SizeText.Visibility == Visibility.Visible ? 144 : 126);
        var monitor = System.Windows.Forms.Screen.FromPoint(new System.Drawing.Point(screenX, screenY)).Bounds;
        var bounds = ScreenCoordinateService.ToLocalDipRect(new ScreenRect(monitor.X, monitor.Y, monitor.Width, monitor.Height),
            _frame.OriginX, _frame.OriginY, Root.ActualWidth, Root.ActualHeight, _frame.Image.PixelWidth, _frame.Image.PixelHeight);
        var surface = new Rect(0, 0, Root.ActualWidth, Root.ActualHeight);
        bounds.Intersect(surface);
        if (bounds.IsEmpty || !bounds.Contains(point)) bounds = surface;
        var location = PlacePointerInspector(point, inspectorSize, bounds);
        Canvas.SetLeft(PointerInspector, location.X); Canvas.SetTop(PointerInspector, location.Y);
    }

    private static string FormatPointerColor(Color color) =>
        string.Create(CultureInfo.InvariantCulture, $"#{color.R:X2}{color.G:X2}{color.B:X2}");

    internal static Point PlacePointerInspector(Point point, Size size, Rect bounds)
    {
        const double margin = 4, gap = 16;
        var left = point.X + gap; var top = point.Y + gap;
        if (left + size.Width > bounds.Right - margin) left = point.X - size.Width - gap;
        if (top + size.Height > bounds.Bottom - margin) top = point.Y - size.Height - gap;
        return new Point(Math.Clamp(left, bounds.Left + margin, Math.Max(bounds.Left + margin, bounds.Right - size.Width - margin)),
            Math.Clamp(top, bounds.Top + margin, Math.Max(bounds.Top + margin, bounds.Bottom - size.Height - margin)));
    }
}
