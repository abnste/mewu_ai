// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Shapes;
using mewu_ai_Assistant.Services;
using Point = System.Windows.Point;

namespace mewu_ai_Assistant.Views;

/// <summary>五彩式方框标记：圈选整个方框半透明上色（DrawTool.Mark）。
/// 创建时抓取方框内像素的降采样灰度模板；画面刷新（RefreshDesktopFrame…）后
/// 在原位置邻域做模板匹配（SAD）重新定位——像 PDF 标记一样随内容滚动/翻页
/// 不错位；内容滚出当前画面时隐藏保留，翻回时自动恢复。</summary>
public partial class CaptureOverlayWindow
{
    private (SelectionItem Item, Border Preview)? _drawingMarkPreview;

    private void BeginMarkDrawingPreview(SelectionItem item)
    {
        CancelMarkDrawingPreview();
        try
        {
            var preview = new Border
            {
                CornerRadius = new CornerRadius(3),
                Background = new SolidColorBrush(Color.FromArgb(84, _drawColor.R, _drawColor.G, _drawColor.B)),
                BorderBrush = new SolidColorBrush(Color.FromArgb(190, _drawColor.R, _drawColor.G, _drawColor.B)),
                BorderThickness = new Thickness(1),
                IsHitTestVisible = false
            };
            InkCanvas.SetLeft(preview, 0);
            InkCanvas.SetTop(preview, 0);
            _drawingMarkPreview = (item, preview);
            item.Markup.Children.Add(preview);
        }
        catch (Exception error)
        {
            CancelMarkDrawingPreview();
            new PrivacyLogger().Error("RegionMarkPreview", error);
        }
    }

    private void UpdateMarkDrawingPreview(SelectionItem item, Point point)
    {
        if (_drawingMarkPreview is not { } preview || !ReferenceEquals(preview.Item, item)) return;
        var bounds = MarkDrawingBounds(item, _drawStart, point);
        preview.Preview.Width = Math.Max(0, bounds.Width);
        preview.Preview.Height = Math.Max(0, bounds.Height);
        InkCanvas.SetLeft(preview.Preview, bounds.X);
        InkCanvas.SetTop(preview.Preview, bounds.Y);
    }

    private void CommitMarkDrawingPreview(SelectionItem item, Point point)
    {
        if (_drawingMarkPreview is not { } preview || !ReferenceEquals(preview.Item, item))
        {
            CancelMarkDrawingPreview();
            return;
        }
        try
        {
            var bounds = MarkDrawingBounds(item, _drawStart, point);
            if (bounds.Width >= 4 && bounds.Height >= 4) AddRegionMark(item, bounds);
        }
        finally { CancelMarkDrawingPreview(); }
    }

    private void CancelMarkDrawingPreview()
    {
        if (_drawingMarkPreview is not { } preview) return;
        _drawingMarkPreview = null;
        preview.Item.Markup.Children.Remove(preview.Preview);
    }

    private static Rect MarkDrawingBounds(SelectionItem item, Point start, Point end)
    {
        var width = Math.Max(0, item.Bounds.Width);
        var height = Math.Max(0, item.Bounds.Height);
        return new Rect(new Point(Math.Clamp(start.X, 0, width), Math.Clamp(start.Y, 0, height)),
            new Point(Math.Clamp(end.X, 0, width), Math.Clamp(end.Y, 0, height)));
    }

    /// <summary>创建方框标记：记录选区坐标 + 颜色，并抓取方框内像素灰度模板。</summary>
    private void AddRegionMark(SelectionItem item, Rect bounds)
    {
        var mark = new RegionMark { Bounds = bounds, Color = _drawColor };
        try { CaptureRegionMarkTemplate(RenderSelectionImage(item, false, false, false), item, mark); }
        catch (Exception error) { new PrivacyLogger().Error("RegionMarkTemplate", error); }
        item.RegionMarks.Add(mark);
        RenderRegionMarks(item);
        MarkDrawingChanged(item);
        PromptStatus.Text = LocalizationService.T(
            $"已上色 {item.RegionMarks.Count} 个方框 · 标记随内容滚动/翻页自动对齐，橡皮可擦除",
            $"{item.RegionMarks.Count} colored region(s) · marks follow the content when scrolling; erase with the eraser");
    }

    /// <summary>把全部方框标记渲染到标记层（OnScreen=false 的保留但不显示）。</summary>
    private void RenderRegionMarks(SelectionItem item)
    {
        var layer = item.RegionMarkLayer;
        if (layer is null) return;
        layer.Children.Clear();
        foreach (var mark in item.RegionMarks)
        {
            if (!mark.OnScreen) continue;
            layer.Children.Add(CreateRegionMarkVisual(mark));
        }
    }

    private static Border CreateRegionMarkVisual(RegionMark mark)
    {
        var visual = new Border
        {
            Width = Math.Max(1, mark.Bounds.Width),
            Height = Math.Max(1, mark.Bounds.Height),
            CornerRadius = new CornerRadius(3),
            Background = new SolidColorBrush(Color.FromArgb(84, mark.Color.R, mark.Color.G, mark.Color.B)),
            BorderBrush = new SolidColorBrush(Color.FromArgb(190, mark.Color.R, mark.Color.G, mark.Color.B)),
            BorderThickness = new Thickness(1),
            IsHitTestVisible = false,
            Tag = mark.Id
        };
        Canvas.SetLeft(visual, mark.Bounds.Left);
        Canvas.SetTop(visual, mark.Bounds.Top);
        return visual;
    }

    /// <summary>导出/复制/发送截图时把方框标记画进手工标注层（半透明圆角矩形）。</summary>
    private static void AddRegionMarksToOverlay(InkCanvas content, SelectionItem item)
    {
        foreach (var mark in item.RegionMarks)
        {
            if (!mark.OnScreen) continue;
            var rectangle = new Rectangle
            {
                Width = Math.Max(1, mark.Bounds.Width),
                Height = Math.Max(1, mark.Bounds.Height),
                RadiusX = 3,
                RadiusY = 3,
                Fill = new SolidColorBrush(Color.FromArgb(84, mark.Color.R, mark.Color.G, mark.Color.B)),
                Stroke = new SolidColorBrush(Color.FromArgb(190, mark.Color.R, mark.Color.G, mark.Color.B)),
                StrokeThickness = 1,
                IsHitTestVisible = false
            };
            InkCanvas.SetLeft(rectangle, mark.Bounds.Left);
            InkCanvas.SetTop(rectangle, mark.Bounds.Top);
            content.Children.Add(rectangle);
        }
    }

    /// <summary>抓取方框内像素灰度模板：降采样到最长边约 120 像素（匹配快且抗噪）。
    /// 模板采样步长（TemplateStepX/Y）记录像素网格密度，匹配时按同一网格取样。</summary>
    private static void CaptureRegionMarkTemplate(BitmapSource source, SelectionItem item, RegionMark mark)
    {
        var scaleX = source.PixelWidth / Math.Max(1, item.Bounds.Width);
        var scaleY = source.PixelHeight / Math.Max(1, item.Bounds.Height);
        var left = (int)Math.Clamp(Math.Floor(mark.Bounds.Left * scaleX), 0, Math.Max(0, source.PixelWidth - 4));
        var top = (int)Math.Clamp(Math.Floor(mark.Bounds.Top * scaleY), 0, Math.Max(0, source.PixelHeight - 4));
        var width = Math.Clamp((int)Math.Round(mark.Bounds.Width * scaleX), 4, source.PixelWidth - left);
        var height = Math.Clamp((int)Math.Round(mark.Bounds.Height * scaleY), 4, source.PixelHeight - top);
        var stepX = Math.Max(1, (int)Math.Ceiling(width / 120.0));
        var stepY = Math.Max(1, (int)Math.Ceiling(height / 120.0));
        var columns = (width + stepX - 1) / stepX;
        var rows = (height + stepY - 1) / stepY;
        var stride = width * 4;
        var pixels = new byte[checked(stride * height)];
        var formatted = source.Format == PixelFormats.Bgra32 ? source : new FormatConvertedBitmap(source, PixelFormats.Bgra32, null, 0);
        formatted.CopyPixels(new Int32Rect(left, top, width, height), pixels, stride, 0);
        var gray = new byte[columns * rows];
        for (var row = 0; row < rows; row++)
        {
            var sampleY = row * stepY;
            for (var column = 0; column < columns; column++)
            {
                var offset = sampleY * stride + column * stepX * 4;
                gray[row * columns + column] = (byte)((pixels[offset] * 114 + pixels[offset + 1] * 587 + pixels[offset + 2] * 299) / 1000);
            }
        }
        mark.Template = gray;
        mark.TemplateWidth = columns;
        mark.TemplateHeight = rows;
        mark.TemplateStepX = stepX;
        mark.TemplateStepY = stepY;
    }

    /// <summary>画面刷新后重新对齐方框标记：整帧转灰度后逐标记在邻域内 SAD 匹配。</summary>
    private readonly HashSet<SelectionItem> _regionMarkReanchorInFlight = new();

    private async Task ReanchorRegionMarksAsync(SelectionItem item)
    {
        if (_closed || !_selections.Contains(item) || item.VideoPath is not null) return;
        if (item.RegionMarks.Count == 0) return;
        if (!_regionMarkReanchorInFlight.Add(item)) return;
        try
        {
            BitmapSource? image = null;
            await Dispatcher.InvokeAsync(() =>
            {
                if (_closed || !_selections.Contains(item)) return;
                image = RenderSelectionImage(item, false, false, false);
            }).Task;
            if (image is null || image.PixelWidth < 4 || image.PixelHeight < 4) return;
            var snapshot = item.RegionMarks.Select(mark => new RegionMarkMatchInput(mark.Id, mark.Bounds, mark.Template, mark.TemplateWidth, mark.TemplateHeight, mark.TemplateStepX, mark.TemplateStepY)).ToArray();
            var scaleX = image.PixelWidth / Math.Max(1, item.Bounds.Width);
            var scaleY = image.PixelHeight / Math.Max(1, item.Bounds.Height);
            var updates = await Task.Run(() => ReanchorMarksCore(image, scaleX, scaleY, snapshot)).ConfigureAwait(false);
            await Dispatcher.InvokeAsync(() =>
            {
                if (_closed || !_selections.Contains(item)) return;
                foreach (var update in updates)
                {
                    var mark = item.RegionMarks.FirstOrDefault(candidate => candidate.Id == update.Id);
                    if (mark is null) continue;
                    mark.OnScreen = update.OnScreen;
                    if (update.Bounds is { } bounds) mark.Bounds = bounds;
                }
                RenderRegionMarks(item);
                var moved = updates.Count(update => update.Bounds is not null);
                var hidden = updates.Count(update => !update.OnScreen);
                PromptStatus.Text = hidden > 0
                    ? LocalizationService.T($"方框标记已随画面重新对齐；{hidden} 个不在当前画面，翻回时自动恢复。", $"Region marks realigned; {hidden} not on screen and will reappear when you scroll back.")
                    : LocalizationService.T($"方框标记已随画面重新对齐（{moved} 处）。", $"Region marks realigned with the refreshed view ({moved}).");
            }).Task;
        }
        catch (Exception error) { new PrivacyLogger().Error("RegionMarkReanchor", error); }
        finally { _regionMarkReanchorInFlight.Remove(item); }
    }

    private sealed record RegionMarkMatchInput(Guid Id, Rect Bounds, byte[]? Template, int Width, int Height, int StepX, int StepY);
    private sealed record RegionMarkMatchResult(Guid Id, bool OnScreen, Rect? Bounds);

    /// <summary>整帧灰度化 + 逐标记邻域 SAD 搜索。垂直 ±400px、水平 ±80px（先粗后细）；
    /// 平均差 &lt; 22 判为命中；纯色平区（&lt; 2）视为内容未变保持原位。</summary>
    private static List<RegionMarkMatchResult> ReanchorMarksCore(BitmapSource image, double scaleX, double scaleY, RegionMarkMatchInput[] marks)
    {
        var results = new List<RegionMarkMatchResult>(marks.Length);
        if (marks.Length == 0) return results;
        var width = image.PixelWidth;
        var height = image.PixelHeight;
        var stride = checked(width * 4);
        var pixels = new byte[checked(stride * height)];
        var formatted = image.Format == PixelFormats.Bgra32 ? image : new FormatConvertedBitmap(image, PixelFormats.Bgra32, null, 0);
        formatted.CopyPixels(new Int32Rect(0, 0, width, height), pixels, stride, 0);
        var gray = new byte[width * height];
        for (var y = 0; y < height; y++)
        {
            var row = y * stride;
            for (var x = 0; x < width; x++)
            {
                var offset = row + x * 4;
                gray[y * width + x] = (byte)((pixels[offset] * 114 + pixels[offset + 1] * 587 + pixels[offset + 2] * 299) / 1000);
            }
        }
        foreach (var mark in marks)
        {
            if (mark.Template is null || mark.Width < 2 || mark.Height < 2)
            {
                results.Add(new RegionMarkMatchResult(mark.Id, true, null));
                continue;
            }
            var expectedX = (int)Math.Round(mark.Bounds.X * scaleX);
            var expectedY = (int)Math.Round(mark.Bounds.Y * scaleY);
            var match = FindTemplate(gray, width, height, mark.Template, mark.Width, mark.Height, mark.StepX, mark.StepY, expectedX, expectedY);
            if (match is not { } found)
            {
                results.Add(new RegionMarkMatchResult(mark.Id, false, null));
                continue;
            }
            var (bestX, bestY, mean) = found;
            if (mean < 2)
            {
                // 纯色平区：内容可能未变也可能整体同色，保持原位最稳。
                results.Add(new RegionMarkMatchResult(mark.Id, true, null));
                continue;
            }
            var bounds = new Rect(bestX / scaleX, bestY / scaleY, mark.Bounds.Width, mark.Bounds.Height);
            results.Add(new RegionMarkMatchResult(mark.Id, true, bounds));
        }
        return results;
    }

    private static (int X, int Y, double Mean)? FindTemplate(byte[] gray, int width, int height, byte[] template, int tw, int th, int stepX, int stepY, int expectedX, int expectedY)
    {
        // 采样点过多时抽稀（每 2 个取 1），上限约 4000 个比较点。
        var pointStrideX = tw * th > 4000 ? 2 : 1;
        var pointStrideY = tw * th > 4000 ? 2 : 1;
        // 局部搜索：返回最优位置与平均差。
        (int, int, double) Search(int startX, int endX, int startY, int endY, int step)
        {
            var bestX = expectedX;
            var bestY = expectedY;
            var best = double.MaxValue;
            for (var oy = startY; oy <= endY; oy += step)
            {
                for (var ox = startX; ox <= endX; ox += step)
                {
                    if (ox < 0 || oy < 0 || ox + (tw - 1) * stepX >= width || oy + (th - 1) * stepY >= height) continue;
                    long sum = 0;
                    var count = 0;
                    for (var ty = 0; ty < th; ty += pointStrideY)
                    {
                        var rowBase = (oy + ty * stepY) * width + ox;
                        var templateRow = ty * tw;
                        for (var tx = 0; tx < tw; tx += pointStrideX)
                        {
                            sum += Math.Abs(gray[rowBase + tx * stepX] - template[templateRow + tx]);
                            count++;
                        }
                    }
                    var mean = count == 0 ? double.MaxValue : (double)sum / count;
                    if (mean < best) { best = mean; bestX = ox; bestY = oy; }
                }
            }
            return (bestX, bestY, best);
        }
        // 粗搜（步长 3）后细搜（±3 步长 1）。
        var (coarseX, coarseY, coarseMean) = Search(expectedX - 78, expectedX + 78, expectedY - 396, expectedY + 396, 3);
        var (fineX, fineY, fineMean) = Search(coarseX - 3, coarseX + 3, coarseY - 3, coarseY + 3, 1);
        if (Math.Min(coarseMean, fineMean) > 22) return null;
        return fineMean <= coarseMean ? (fineX, fineY, fineMean) : (coarseX, coarseY, coarseMean);
    }
}
