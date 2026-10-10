// SPDX-License-Identifier: MPL-2.0
//! Pure shared screenshot/video composition of trusted straight RGBA rasters.
use image::{Rgba, RgbaImage};
use resvg::tiny_skia;

pub fn blend_rgba(target: &mut Rgba<u8>, source: &Rgba<u8>) {
    let source_alpha = u32::from(source[3]);
    if source_alpha == 0 {
        return;
    }
    let target_alpha = u32::from(target[3]);
    let remaining = 255 - source_alpha;
    let combined = source_alpha * 255 + target_alpha * remaining;
    for channel in 0..3 {
        let numerator = u32::from(source[channel]) * source_alpha * 255
            + u32::from(target[channel]) * target_alpha * remaining;
        target[channel] = ((numerator + combined / 2) / combined) as u8;
    }
    target[3] = ((combined + 127) / 255) as u8;
}
pub fn blend_overlay(bitmap: &mut RgbaImage, overlay: &tiny_skia::Pixmap) {
    for (target, source) in bitmap.pixels_mut().zip(overlay.pixels()) {
        if source.alpha() == 0 {
            continue;
        }
        let source = source.demultiply();
        blend_rgba(
            target,
            &Rgba([source.red(), source.green(), source.blue(), source.alpha()]),
        );
    }
}
/// Coordinates are already translated into the target's source/crop pixels.
/// The same premultiplied bilinear transform and rounded source-over is used
/// for a static screenshot crop and a decoded video frame.
pub fn paint_raster(
    bitmap: &mut RgbaImage,
    raster: &RgbaImage,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let sx = width / f64::from(raster.width());
    let sy = height / f64::from(raster.height());
    if raster.width() == 0
        || raster.height() == 0
        || sx <= 0.
        || sy <= 0.
        || [sx, sy, x, y]
            .iter()
            .any(|v| !v.is_finite() || v.abs() > f64::from(f32::MAX))
    {
        return Err("图层坐标无效".into());
    }
    let mut raw = raster.as_raw().clone();
    for pixel in raw.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    let source = tiny_skia::Pixmap::from_vec(
        raw,
        tiny_skia::IntSize::from_wh(raster.width(), raster.height()).ok_or("图层过大")?,
    )
    .ok_or("图层图片无效")?;
    let mut overlay =
        tiny_skia::Pixmap::new(bitmap.width(), bitmap.height()).ok_or("图片过大，无法合成标注")?;
    overlay.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &tiny_skia::PixmapPaint {
            quality: tiny_skia::FilterQuality::Bilinear,
            ..Default::default()
        },
        tiny_skia::Transform::from_row(sx as f32, 0., 0., sy as f32, x as f32, y as f32),
        None,
    );
    blend_overlay(bitmap, &overlay);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn straight_alpha_and_layer_order_preserve_opaque_source() {
        let mut image = RgbaImage::from_pixel(4, 4, Rgba([20, 40, 60, 255]));
        let raster = RgbaImage::from_pixel(2, 2, Rgba([220, 20, 20, 128]));
        paint_raster(&mut image, &raster, 1., 1., 2., 2.).unwrap();
        assert_eq!(image.get_pixel(1, 1).0, [120, 30, 40, 255]);
        assert_eq!(image.get_pixel(0, 0).0, [20, 40, 60, 255]);
        paint_raster(
            &mut image,
            &RgbaImage::from_pixel(2, 2, Rgba([250, 10, 10, 0])),
            1.,
            1.,
            2.,
            2.,
        )
        .unwrap();
        assert_eq!(image.get_pixel(1, 1).0, [120, 30, 40, 255]);
    }
}
