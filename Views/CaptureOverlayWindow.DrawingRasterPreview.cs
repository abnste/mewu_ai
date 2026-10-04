// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows.Controls;
using System.Windows.Media;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private sealed record RasterObjectDrawingPreview(SelectionItem Item, Guid ElementId, Image Visual);
    private RasterObjectDrawingPreview? _rasterObjectDrawingPreview;

    // The sampled raster and its cache key must describe exactly the same layer.
    // An actively dragged image is drawn separately above both kinds of highlight.
    private MosaicDrawingElement[] BackgroundHighlightRasterElements(SelectionItem item) =>
        item.DrawingElements.OfType<MosaicDrawingElement>()
            .Where(element => !IsRasterObjectDrawingPreview(item, element.Id)).ToArray();

    private bool IsRasterObjectDrawingPreview(SelectionItem item, Guid id) =>
        _rasterObjectDrawingPreview is { } preview && ReferenceEquals(preview.Item, item) && preview.ElementId == id;

    private void BeginRasterObjectDrawingPreview(SelectionItem item, DrawingElementSpec element)
    {
        if (element is not MosaicDrawingElement || IsRasterObjectDrawingPreview(item, element.Id)) return;
        EndRasterObjectDrawingPreview();
        if (FindDrawingElementVisual(item, element.Id) is not Image visual || !item.RasterLayer.Children.Contains(visual)) return;
        _rasterObjectDrawingPreview = new RasterObjectDrawingPreview(item, element.Id, visual);
        item.RasterLayer.Children.Remove(visual);
        item.RasterPreviewLayer.Children.Add(visual);
        // Once per gesture, not per pointer update: highlights now protect only
        // the fixed raster below them. The immutable moving pixels stay on top.
        RefreshBackgroundHighlightSources(item);
    }

    private void EndRasterObjectDrawingPreview(SelectionItem? item = null, bool refresh = true)
    {
        if (_rasterObjectDrawingPreview is not { } preview || (item is not null && !ReferenceEquals(preview.Item, item))) return;
        _rasterObjectDrawingPreview = null;
        preview.Item.RasterPreviewLayer.Children.Remove(preview.Visual);
        var modelIndex = preview.Item.DrawingElements.FindIndex(element => element.Id == preview.ElementId);
        if (modelIndex >= 0)
        {
            var rasterIndex = preview.Item.DrawingElements.Take(modelIndex).Count(element => element is MosaicDrawingElement);
            preview.Item.RasterLayer.Children.Insert(Math.Min(rasterIndex, preview.Item.RasterLayer.Children.Count), preview.Visual);
        }
        if (refresh && !_closed) RefreshBackgroundHighlightSources(preview.Item);
    }

    private static Image CloneRasterPreviewVisual(Image source)
    {
        var image = new Image
        {
            Source = source.Source, Width = source.Width, Height = source.Height,
            Stretch = source.Stretch, Clip = source.Clip?.CloneCurrentValue(), IsHitTestVisible = false
        };
        RenderOptions.SetBitmapScalingMode(image, RenderOptions.GetBitmapScalingMode(source));
        InkCanvas.SetLeft(image, InkCanvas.GetLeft(source));
        InkCanvas.SetTop(image, InkCanvas.GetTop(source));
        return image;
    }
}
