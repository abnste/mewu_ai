// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
//! Original-image block averages, shared by preview and final cropped output.

use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use mewu_core::{Drawing, DrawingKind};
use std::collections::HashMap;

const MAX_SOURCE_PIXELS: u64 = 32 * 1024 * 1024;

/// One opaque pixel per background-aligned block. Enlarge with nearest-neighbor
/// to (grid.width * block_size, grid.height * block_size), then clip to source
/// dimensions; stretching directly to source dimensions moves partial blocks.
pub fn mosaic_grid(image: &DynamicImage, block_size: u32) -> Result<RgbaImage, String> {
    Ok(PixelGrid::new(image, block_size, (0, 0, image.width(), image.height()))?.pixels)
}

struct PixelGrid {
    pixels: RgbaImage,
    column: u32,
    row: u32,
    block_size: u32,
}

impl PixelGrid {
    fn new(
        image: &DynamicImage,
        block_size: u32,
        area: (u32, u32, u32, u32),
    ) -> Result<Self, String> {
        if !(6..=40).contains(&block_size) {
            return Err("马赛克块大小需为 6 至 40 的整数".into());
        }
        if image.width() == 0
            || image.height() == 0
            || image.width() > 16384
            || image.height() > 16384
            || u64::from(image.width()) * u64::from(image.height()) > MAX_SOURCE_PIXELS
        {
            return Err("马赛克背景最多支持 3200 万像素，单边不超过 16384 像素".into());
        }
        let (x, y, width, height) = area;
        let right = x
            .checked_add(width)
            .filter(|&v| v <= image.width())
            .ok_or("马赛克范围无效")?;
        let bottom = y
            .checked_add(height)
            .filter(|&v| v <= image.height())
            .ok_or("马赛克范围无效")?;
        if width == 0 || height == 0 {
            return Err("马赛克范围无效".into());
        }
        let column = x / block_size;
        let row = y / block_size;
        let columns = right.div_ceil(block_size) - column;
        let rows = bottom.div_ceil(block_size) - row;
        let pixels = RgbaImage::from_fn(columns, rows, |cx, cy| {
            average_block(
                image,
                (column + cx) * block_size,
                (row + cy) * block_size,
                block_size,
            )
        });
        Ok(Self {
            pixels,
            column,
            row,
            block_size,
        })
    }

    fn at(&self, x: u32, y: u32) -> Rgba<u8> {
        *self.pixels.get_pixel(
            x / self.block_size - self.column,
            y / self.block_size - self.row,
        )
    }
}

fn average_block(image: &DynamicImage, x: u32, y: u32, block_size: u32) -> Rgba<u8> {
    let right = (x + block_size).min(image.width());
    let bottom = (y + block_size).min(image.height());
    let mut channels = [0u32; 3];
    for py in y..bottom {
        for px in x..right {
            let pixel = image.get_pixel(px, py);
            let alpha = u32::from(pixel[3]);
            for channel in 0..3 {
                // Straight alpha over opaque white, rounded to the nearest byte.
                channels[channel] +=
                    (u32::from(pixel[channel]) * alpha + 255 * (255 - alpha) + 127) / 255;
            }
        }
    }
    let count = (right - x) * (bottom - y);
    Rgba([
        (channels[0] / count) as u8,
        (channels[1] / count) as u8,
        (channels[2] / count) as u8,
        255,
    ])
}

/// Paint all raster annotations before the vector layer. Each block is sampled
/// from `source`, never from `output` or a previously committed mosaic. Cache
/// only the grid cells touching the crop, not a full-size image per annotation.
pub(super) fn paint_mosaics(
    source: &DynamicImage,
    drawings: &[Drawing],
    output: &mut RgbaImage,
    x: u32,
    y: u32,
) -> Result<(), String> {
    let area = (x, y, output.width(), output.height());
    let mut grids = HashMap::new();
    for drawing in drawings
        .iter()
        .filter(|drawing| drawing.kind == DrawingKind::Mosaic)
    {
        let a = drawing.points[0];
        let b = drawing.points[1];
        let left = (a.x.min(b.x).floor() as u32).max(x);
        let top = (a.y.min(b.y).floor() as u32).max(y);
        let right = (a.x.max(b.x).ceil() as u32).min(x + output.width());
        let bottom = (a.y.max(b.y).ceil() as u32).min(y + output.height());
        if left >= right || top >= bottom {
            continue;
        }
        let size = drawing.stroke_width as u32;
        if let std::collections::hash_map::Entry::Vacant(entry) = grids.entry(size) {
            entry.insert(PixelGrid::new(source, size, area)?);
        }
        let grid = &grids[&size];
        for py in top..bottom {
            for px in left..right {
                output.put_pixel(px - x, py - y, grid.at(px, py));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_visible_tail_pixels_and_composites_alpha_before_averaging() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_fn(7, 7, |x, y| {
            if x == 6 && y == 6 {
                Rgba([10, 20, 30, 128])
            } else if x == 6 {
                Rgba([20, 40, 60, 255])
            } else if y == 6 {
                Rgba([50, 80, 110, 255])
            } else if x < 3 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 0])
            }
        }));
        let grid = mosaic_grid(&image, 6).unwrap();
        assert_eq!(grid.dimensions(), (2, 2));
        assert_eq!(*grid.get_pixel(0, 0), Rgba([255, 127, 127, 255]));
        assert_eq!(*grid.get_pixel(1, 0), Rgba([20, 40, 60, 255]));
        assert_eq!(*grid.get_pixel(0, 1), Rgba([50, 80, 110, 255]));
        assert_eq!(*grid.get_pixel(1, 1), Rgba([132, 137, 142, 255]));
    }

    #[test]
    fn crop_grid_is_identical_to_full_preview_and_rejects_invalid_parameters() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_fn(73, 51, |x, y| {
            Rgba([
                (x * 3) as u8,
                (y * 5) as u8,
                (x + y) as u8,
                (x * 7 + y) as u8,
            ])
        }));
        for size in [6, 12, 40] {
            let full = mosaic_grid(&image, size).unwrap();
            let crop = PixelGrid::new(&image, size, (17, 13, 55, 38)).unwrap();
            for y in 13..51 {
                for x in 17..72 {
                    assert_eq!(crop.at(x, y), *full.get_pixel(x / size, y / size));
                }
            }
        }
        for size in [0, 5, 41, u32::MAX] {
            assert!(mosaic_grid(&image, size).is_err());
        }
        assert!(mosaic_grid(&DynamicImage::new_rgba8(0, 1), 6).is_err());
        assert!(PixelGrid::new(&image, 6, (72, 0, 2, 1)).is_err());
    }
}
