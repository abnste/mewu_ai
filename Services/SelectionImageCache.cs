// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

/// <summary>Selection styling changes do not require another WIC crop.</summary>
internal sealed class SelectionImageCache
{
    private BitmapSource? _source;
    private BitmapSource? _crop;
    private Int32Rect _bounds;
    private readonly ImageBrush _preview=new(){ViewboxUnits=BrushMappingMode.RelativeToBoundingBox,Stretch=Stretch.Fill};

    internal ImageBrush GetPreview(BitmapSource source,Int32Rect bounds)
    {
        _preview.ImageSource=source;
        _preview.Viewbox=new Rect(bounds.X/(double)source.PixelWidth,bounds.Y/(double)source.PixelHeight,bounds.Width/(double)source.PixelWidth,bounds.Height/(double)source.PixelHeight);
        return _preview;
    }

    internal void ClearPreview()=>_preview.ImageSource=null;

    internal BitmapSource Get(BitmapSource source, Int32Rect bounds, BitmapSource? replacement = null)
    {
        if (replacement is not null) return replacement;
        if (_crop is not null && ReferenceEquals(_source, source) && _bounds == bounds) return _crop;
        var crop = ScreenCaptureService.Crop(source, bounds);
        _source = source;
        _bounds = bounds;
        return _crop = crop;
    }

    internal void Clear() { _source = null; _crop = null; ClearPreview(); }
}
