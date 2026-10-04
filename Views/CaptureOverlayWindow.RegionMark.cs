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

/// <summary>重点高亮：圈选方框，只对背景上色并保护文字细节（DrawTool.Mark）。
/// 创建时抓取方框内像素的降采样灰度模板；画面刷新（RefreshDesktopFrame…）后
/// 在原位置邻域做模板匹配（SAD）重新定位——像 PDF 标记一样随内容滚动/翻页
/// 不错位；内容滚出当前画面时隐藏保留，翻回时自动恢复。</summary>
public partial class CaptureOverlayWindow
{
    private (SelectionItem Item, Border Preview)? _drawingMarkPreview;

    private void BeginMarkDrawingPreview(SelectionItem item)
    {
        CancelMarkDrawingPreview();
        if (!CanBeginBackgroundHighlight(item)) return;
        try
        {
            var preview = new Border
            {
                CornerRadius = new CornerRadius(3),
                BorderThickness = new Thickness(1),
                IsHitTestVisible = false
            };
            ApplyRegionMarkBrushes(preview, GetBackgroundHighlightSource(item), BackgroundHighlightSourceBounds(item), _drawColor);
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
        var sourceBounds = BackgroundHighlightSourceBounds(item);
        sourceBounds.Offset(-bounds.X, -bounds.Y);
        ApplyRegionMarkBrushes(preview.Preview, GetBackgroundHighlightSource(item), sourceBounds, _drawColor);
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

    /// <summary>创建重点高亮：记录选区坐标 + 颜色，并抓取方框内像素灰度模板。</summary>
    private void AddRegionMark(SelectionItem item, Rect bounds)
    {
        var mark = new RegionMark { Bounds = bounds, Color = _drawColor };
        try { CaptureRegionMarkTemplate(RenderSelectionImage(item, false, false, false), item, mark); }
        catch (Exception error) { new PrivacyLogger().Error("RegionMarkTemplate", error); }
        InvalidateRegionMarkMatching(item);
        item.RegionMarks.Add(mark);
        item.DrawingOrder.Add(new RegionMarkDrawingAction(mark,false,item.RegionMarks.Count-1));item.DrawingRedo.Clear();
        RenderRegionMarks(item);
        MarkDrawingChanged(item);
        PromptStatus.Text = LocalizationService.T(
            $"已添加 {item.RegionMarks.Count} 处重点高亮 · 随内容滚动自动对齐，选择工具可编辑",
            $"{item.RegionMarks.Count} highlight(s) · follow content when scrolling; edit with the selection tool");
    }

    /// <summary>把全部重点高亮渲染到标记层（OnScreen=false 的保留但不显示）。</summary>
    private void RenderRegionMarks(SelectionItem item)
    {
        var layer = item.RegionMarkLayer;
        if (layer is null) return;
        layer.Children.Clear();
        foreach (var mark in item.RegionMarks)
        {
            if (!mark.OnScreen) continue;
            layer.Children.Add(CreateRegionMarkVisual(item, mark));
        }
        if (_drawingMode && ReferenceEquals(item, Active) && SelectedDrawingRegionMark(item) is not null)
            ShowDrawingObjectSelection(item);
    }

    private Border CreateRegionMarkVisual(SelectionItem item, RegionMark mark)
    {
        var sourceBounds = BackgroundHighlightSourceBounds(item);
        sourceBounds.Offset(-mark.Bounds.Left, -mark.Bounds.Top);
        var visual = new Border
        {
            Width = Math.Max(1, mark.Bounds.Width),
            Height = Math.Max(1, mark.Bounds.Height),
            CornerRadius = new CornerRadius(3),
            BorderThickness = new Thickness(1),
            IsHitTestVisible = false,
            Tag = mark.Id
        };
        ApplyRegionMarkBrushes(visual, GetBackgroundHighlightSource(item), sourceBounds, mark.Color);
        Canvas.SetLeft(visual, mark.Bounds.Left);
        Canvas.SetTop(visual, mark.Bounds.Top);
        return visual;
    }

    private static void ApplyRegionMarkBrushes(Border visual, BackgroundHighlightSource source, Rect sourceBounds, Color color)
    {
        visual.Background = source.CreateTintBrush(color, 84, sourceBounds);
        visual.BorderBrush = source.CreateTintBrush(color, 190, sourceBounds);
        visual.OpacityMask = source.CreateOpacityBrush(sourceBounds);
    }

    /// <summary>导出复用现场的文字保护遮罩和圆角边线，避免另一路覆盖文字。</summary>
    private void AddRegionMarksToOverlay(InkCanvas content, SelectionItem item)
    {
        foreach (var mark in item.RegionMarks)
        {
            if (!mark.OnScreen) continue;
            var rectangle = CreateRegionMarkVisual(item, mark);
            InkCanvas.SetLeft(rectangle, mark.Bounds.Left);
            InkCanvas.SetTop(rectangle, mark.Bounds.Top);
            content.Children.Add(rectangle);
        }
    }

    /// <summary>抓取方框内像素灰度模板：降采样到最长边约 120 像素（匹配快且抗噪）。
    /// 模板采样步长记录像素网格密度；偏移记录被裁切后的可见模板相对完整标记的位置。</summary>
    private static void CaptureRegionMarkTemplate(BitmapSource source, SelectionItem item, RegionMark mark)
    {
        var scaleX = source.PixelWidth / Math.Max(1, item.Bounds.Width);
        var scaleY = source.PixelHeight / Math.Max(1, item.Bounds.Height);
        var left = (int)Math.Clamp(Math.Floor(mark.Bounds.Left * scaleX), 0, source.PixelWidth);
        var top = (int)Math.Clamp(Math.Floor(mark.Bounds.Top * scaleY), 0, source.PixelHeight);
        var right = (int)Math.Clamp(Math.Ceiling(mark.Bounds.Right * scaleX), 0, source.PixelWidth);
        var bottom = (int)Math.Clamp(Math.Ceiling(mark.Bounds.Bottom * scaleY), 0, source.PixelHeight);
        var width = right - left;
        var height = bottom - top;
        mark.Template = null;
        mark.TemplateWidth = mark.TemplateHeight = 0;
        mark.TemplateStepX = mark.TemplateStepY = 1;
        mark.TemplateOffsetX = mark.TemplateOffsetY = 0;
        if (width < 2 || height < 2) return;
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
        mark.TemplateOffsetX = left / scaleX - mark.Bounds.Left;
        mark.TemplateOffsetY = top / scaleY - mark.Bounds.Top;
    }

    /// <summary>画面刷新后重新对齐重点高亮：整帧转灰度后逐标记在邻域内 SAD 匹配。</summary>
    private readonly HashSet<SelectionItem> _regionMarkReanchorInFlight = new();
    private readonly HashSet<SelectionItem> _regionMarkReanchorPending = new();

    private void InvalidateRegionMarkMatching(SelectionItem item)
    {
        // Even an edit followed by Undo can recreate the same bounds and object
        // identity. A monotonic version also rejects that stale async result.
        item.RegionMarkVersion++;
        _regionMarkReanchorPending.Remove(item);
        TryCancel(item.RegionMarkReanchorRequest);
    }

    private bool IsRegionMarkMatchVersionCurrent(SelectionItem item, long version) =>
        !_closed && _selections.Contains(item) && item.RegionMarkVersion == version;

    private async Task ReanchorRegionMarksAsync(SelectionItem item)
    {
        if(!Dispatcher.CheckAccess())
        {
            await Dispatcher.InvokeAsync(()=>ReanchorRegionMarksAsync(item)).Task.Unwrap();
            return;
        }
        if (_closed || !_selections.Contains(item) || item.VideoPath is not null || item.RegionMarks.Count == 0) return;
        if (ReferenceEquals(item, Active) && _drawingMoveOriginalRegionMark is not null)
        {
            _regionMarkReanchorPending.Add(item);
            return;
        }
        if (!_regionMarkReanchorInFlight.Add(item)){_regionMarkReanchorPending.Add(item);return;}
        CancellationTokenSource? request = null;
        try
        {
            // Snapshot all mutable UI state before leaving the dispatcher. A restored
            // history clone may have the same ID, but is not the same mark instance.
            request=CancellationTokenSource.CreateLinkedTokenSource(_screenEntityLifetime.Token);
            item.RegionMarkReanchorRequest=request;
            var token=request.Token;
            var version=item.RegionMarkVersion;
            var frame=_frame;var bounds=item.Bounds;var replacement=item.CapturedImageOverride;
            var marks=item.RegionMarks.ToArray();
            var visibility=marks.Select(mark=>mark.OnScreen).ToArray();
            var image=RenderSelectionImage(item,false,false,false);
            if(image.PixelWidth<4||image.PixelHeight<4)return;
            var snapshot=marks.Select(mark=>new RegionMarkMatchInput(mark.Id,mark.Bounds,mark.Template?.ToArray(),mark.TemplateWidth,mark.TemplateHeight,mark.TemplateStepX,mark.TemplateStepY)
                {OffsetX=mark.TemplateOffsetX,OffsetY=mark.TemplateOffsetY}).ToArray();
            var scaleX=image.PixelWidth/Math.Max(1,bounds.Width);var scaleY=image.PixelHeight/Math.Max(1,bounds.Height);
            var updates=await Task.Run(()=>ReanchorMarksCore(image,scaleX,scaleY,snapshot,token),token).ConfigureAwait(false);
            await Dispatcher.InvokeAsync(()=>
            {
                if(token.IsCancellationRequested||!IsRegionMarkMatchVersionCurrent(item,version)||!ReferenceEquals(frame,_frame)||item.Bounds!=bounds||!ReferenceEquals(replacement,item.CapturedImageOverride))return;
                if(item.RegionMarks.Count!=marks.Length){if(item.RegionMarks.Count>0)_regionMarkReanchorPending.Add(item);return;}
                for(var i=0;i<marks.Length;i++)
                    if(!ReferenceEquals(item.RegionMarks[i],marks[i])||marks[i].Bounds!=snapshot[i].Bounds||marks[i].OnScreen!=visibility[i]){_regionMarkReanchorPending.Add(item);return;}
                foreach(var update in updates)
                {
                    var mark=marks.FirstOrDefault(candidate=>candidate.Id==update.Id);
                    if(mark is null)continue;
                    mark.OnScreen=update.OnScreen;if(update.Bounds is {} next)mark.Bounds=next;
                }
                RenderRegionMarks(item);
                var moved=updates.Count(update=>update.Bounds is not null);var hidden=updates.Count(update=>!update.OnScreen);
                PromptStatus.Text=hidden>0
                    ?LocalizationService.T($"重点高亮已随画面重新对齐；{hidden} 处不在当前画面，翻回时自动恢复。",$"Highlights realigned; {hidden} not on screen and will reappear when you scroll back.")
                    :LocalizationService.T($"重点高亮已随画面重新对齐（{moved} 处）。",$"Highlights realigned with the refreshed view ({moved}).");
            }).Task.ConfigureAwait(false);
        }
        catch(OperationCanceledException) when(_closed||request?.IsCancellationRequested==true){}
        catch(Exception error){new PrivacyLogger().Error("RegionMarkReanchor",error);}
        finally
        {
            // Never mutate these UI-owned sets from a Task.Run continuation.
            if(!Dispatcher.HasShutdownStarted)
            {
                try
                {
                    await Dispatcher.InvokeAsync(()=>
                    {
                        _regionMarkReanchorInFlight.Remove(item);
                        if(ReferenceEquals(item.RegionMarkReanchorRequest,request))item.RegionMarkReanchorRequest=null;
                        if(_regionMarkReanchorPending.Remove(item)&&!_closed)_=ReanchorRegionMarksAsync(item);
                    }).Task.ConfigureAwait(false);
                }
                catch(TaskCanceledException) when(_closed||Dispatcher.HasShutdownStarted){}
            }
            request?.Dispose();
        }
    }

    private sealed record RegionMarkMatchInput(Guid Id, Rect Bounds, byte[]? Template, int Width, int Height, int StepX, int StepY)
    {
        public double OffsetX { get; init; }
        public double OffsetY { get; init; }
    }
    private sealed record RegionMarkMatchResult(Guid Id, bool OnScreen, Rect? Bounds);

    /// <summary>整帧灰度化 + 逐标记邻域 SAD 搜索。垂直 ±400px、水平 ±80px（先粗后细）；
    /// 平均差 &lt; 22 判为命中；模板本身近乎纯色时不凭同色区域推断位移。</summary>
    private static List<RegionMarkMatchResult> ReanchorMarksCore(BitmapSource image, double scaleX, double scaleY, RegionMarkMatchInput[] marks,CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
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
            token.ThrowIfCancellationRequested();
            var row = y * stride;
            for (var x = 0; x < width; x++)
            {
                var offset = row + x * 4;
                gray[y * width + x] = (byte)((pixels[offset] * 114 + pixels[offset + 1] * 587 + pixels[offset + 2] * 299) / 1000);
            }
        }
        foreach (var mark in marks)
        {
            token.ThrowIfCancellationRequested();
            if (mark.Template is null || mark.Width < 2 || mark.Height < 2)
            {
                results.Add(new RegionMarkMatchResult(mark.Id, true, null));
                continue;
            }
            var expectedX = (int)Math.Round((mark.Bounds.X + mark.OffsetX) * scaleX);
            var expectedY = (int)Math.Round((mark.Bounds.Y + mark.OffsetY) * scaleY);
            var match = FindTemplate(gray, width, height, mark.Template, mark.Width, mark.Height, mark.StepX, mark.StepY, expectedX, expectedY, token);
            if (match is not { } found)
            {
                results.Add(new RegionMarkMatchResult(mark.Id, false, null));
                continue;
            }
            var (bestX, bestY, _) = found;
            if (IsFlatRegionMarkTemplate(mark.Template))
            {
                // SAD measures agreement with the candidate, not texture. An
                // exact match of a textured template can still have moved.
                results.Add(new RegionMarkMatchResult(mark.Id, true, null));
                continue;
            }
            var bounds = new Rect(bestX / scaleX - mark.OffsetX, bestY / scaleY - mark.OffsetY, mark.Bounds.Width, mark.Bounds.Height);
            results.Add(new RegionMarkMatchResult(mark.Id, true, bounds));
        }
        return results;
    }

    private static bool IsFlatRegionMarkTemplate(byte[] template)
    {
        var minimum = byte.MaxValue;
        var maximum = byte.MinValue;
        foreach (var value in template)
        {
            minimum = Math.Min(minimum, value);
            maximum = Math.Max(maximum, value);
            if (maximum - minimum >= 2) return false;
        }
        return true;
    }

    private static (int X, int Y, double Mean)? FindTemplate(byte[] gray, int width, int height, byte[] template, int tw, int th, int stepX, int stepY, int expectedX, int expectedY,CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        // 采样点过多时抽稀（每 2 个取 1），上限约 4000 个比较点。
        var pointStrideX = tw * th > 4000 ? 2 : 1;
        var pointStrideY = tw * th > 4000 ? 2 : 1;
        double DistanceSquared(int x, int y)
        {
            var dx = (double)x - expectedX;
            var dy = (double)y - expectedY;
            return dx * dx + dy * dy;
        }
        // 局部搜索：返回最优位置与平均差。
        (int, int, double) Search(int startX, int endX, int startY, int endY, int step)
        {
            var bestX = expectedX;
            var bestY = expectedY;
            var best = double.MaxValue;
            for (var oy = startY; oy <= endY; oy += step)
            {
                token.ThrowIfCancellationRequested();
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
                    if (mean < best || mean == best && DistanceSquared(ox, oy) < DistanceSquared(bestX, bestY))
                    { best = mean; bestX = ox; bestY = oy; }
                }
            }
            return (bestX, bestY, best);
        }
        // 粗搜（步长 3）后细搜（±3 步长 1）。
        var (coarseX, coarseY, coarseMean) = Search(expectedX - 78, expectedX + 78, expectedY - 396, expectedY + 396, 3);
        var (fineX, fineY, fineMean) = Search(coarseX - 3, coarseX + 3, coarseY - 3, coarseY + 3, 1);
        if (Math.Min(coarseMean, fineMean) > 22) return null;
        return fineMean < coarseMean || fineMean == coarseMean && DistanceSquared(fineX, fineY) <= DistanceSquared(coarseX, coarseY)
            ? (fineX, fineY, fineMean) : (coarseX, coarseY, coarseMean);
    }
}
