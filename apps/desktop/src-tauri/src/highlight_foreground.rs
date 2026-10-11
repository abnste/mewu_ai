// SPDX-License-Identifier: MPL-2.0
//! Frozen foreground separation, ported from Mewu's BackgroundHighlightService.
//! Opening/closing recover background polarity per channel; coverage preserves
//! antialiased foreground when a marker tints only its background contribution.
//! https://docs.opencv.org/4.x/d9/d61/tutorial_py_morphological_ops.html
//! https://www.w3.org/TR/compositing-1/#simplealphacompositing
#[cfg(test)]
use image::Rgba;
use image::{DynamicImage, RgbaImage};

const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const RADIUS: usize = 6;
const TILE: u32 = 512;

pub struct Foreground {
    // Premultiplied foreground; alpha=255 also excludes translucent source.
    pub pixels: RgbaImage,
}
impl Foreground {
    pub fn tint(&self, x: u32, y: u32, color: [u8; 3]) -> Option<[u8; 3]> {
        let p = self.pixels.get_pixel(x, y).0;
        (p[3] < 255).then(|| {
            std::array::from_fn(|i| {
                (u16::from(p[i]) + ((255 - u16::from(p[3])) * u16::from(color[i]) + 127) / 255)
                    .min(255) as u8
            })
        })
    }
    pub fn straight(mut self) -> RgbaImage {
        for p in self.pixels.pixels_mut() {
            let a = u16::from(p[3]);
            for component in &mut p.0[..3] {
                *component = if a == 0 {
                    0
                } else {
                    ((u16::from(*component) * 255 + a / 2) / a).min(255) as u8
                };
            }
        }
        self.pixels
    }
}

pub fn foreground(
    source: &DynamicImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<Foreground, String> {
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || x.checked_add(width).is_none_or(|v| v > source.width())
        || y.checked_add(height).is_none_or(|v| v > source.height())
    {
        return Err("荧光笔图片尺寸无效".into());
    }
    let mut pixels = RgbaImage::new(width, height);
    // Only tiles use full working arrays. A 32M-pixel long capture must not
    // allocate eight full-size morphology buffers. A 2r+1 halo covers both
    // opening/closing passes, the wider mean and the final one-pixel dilation.
    let halo = (RADIUS * 2 + 1) as u32;
    for top in (0..height).step_by(TILE as usize) {
        for left in (0..width).step_by(TILE as usize) {
            let w = TILE.min(width - left);
            let h = TILE.min(height - top);
            let sx = (x + left).saturating_sub(halo);
            let sy = (y + top).saturating_sub(halo);
            let right = (x + left + w).saturating_add(halo).min(source.width());
            let bottom = (y + top + h).saturating_add(halo).min(source.height());
            let input = source
                .crop_imm(sx, sy, right - sx, bottom - sy)
                .into_rgba8();
            let tile = separate(&input);
            for ty in 0..h {
                for tx in 0..w {
                    pixels.put_pixel(
                        left + tx,
                        top + ty,
                        *tile.get_pixel(x + left + tx - sx, y + top + ty - sy),
                    );
                }
            }
        }
    }
    Ok(Foreground { pixels })
}

fn separate(source: &RgbaImage) -> RgbaImage {
    let (width, height) = (source.width() as usize, source.height() as usize);
    let count = width * height;
    let mut channel = vec![0u8; count];
    let mut first = vec![0u8; count];
    let mut second = vec![0u8; count];
    let mut scratch = vec![0u8; count];
    let mut protection = vec![0u8; count];
    let mut work = vec![0u8; count * 4];
    let mut foreground = vec![0u8; count * 4];
    for component in 0..3 {
        for pixel in 0..count {
            channel[pixel] = source.as_raw()[pixel * 4 + component];
        }
        box_mean(&channel, &mut scratch, &mut work, width, height, RADIUS * 2);
        morphology(
            &channel,
            &mut first,
            &mut scratch,
            width,
            height,
            RADIUS,
            false,
        );
        for pixel in 0..count {
            work[pixel * 4 + 1] = first[pixel];
        }
        morphology(
            &first,
            &mut second,
            &mut scratch,
            width,
            height,
            RADIUS,
            true,
        );
        for pixel in 0..count {
            work[pixel * 4 + 3] = second[pixel];
        }
        morphology(
            &channel,
            &mut first,
            &mut scratch,
            width,
            height,
            RADIUS,
            true,
        );
        for pixel in 0..count {
            work[pixel * 4 + 2] = first[pixel];
        }
        morphology(
            &first,
            &mut second,
            &mut scratch,
            width,
            height,
            RADIUS,
            false,
        );
        for pixel in 0..count {
            let base = pixel * 4;
            let mean = work[base];
            let minimum = work[base + 1];
            let maximum = work[base + 2];
            let background = if mean.abs_diff(minimum) < mean.abs_diff(maximum) {
                work[base + 3]
            } else {
                second[pixel]
            };
            protection[pixel] = protection[pixel].max(channel[pixel].abs_diff(background));
            foreground[base + component] = background;
        }
    }
    morphology(
        &protection,
        &mut first,
        &mut scratch,
        width,
        height,
        1,
        true,
    );
    for pixel in 0..count {
        let contrast = u32::from(first[pixel]);
        let residual = u32::from(protection[pixel]);
        let base = pixel * 4;
        let mut coverage = if contrast < 2 {
            0
        } else if contrast - residual <= 1 {
            255
        } else {
            (residual * 255 + contrast / 2) / contrast
        };
        for component in 0..3 {
            let value = u32::from(source.as_raw()[base + component]);
            let background = u32::from(foreground[base + component]);
            if value < background {
                coverage = coverage.max(((background - value) * 255 + background - 1) / background);
            } else if value > background {
                coverage = coverage
                    .max(((value - background) * 255 + 254 - background) / (255 - background));
            }
        }
        for component in 0..3 {
            let value = i32::from(source.as_raw()[base + component]);
            let background = i32::from(foreground[base + component]);
            foreground[base + component] =
                ((value * 255 - (255 - coverage as i32) * background + 127) / 255)
                    .clamp(0, coverage as i32) as u8;
        }
        foreground[base + 3] = if source.as_raw()[base + 3] == 255 {
            coverage as u8
        } else {
            255
        };
    }
    RgbaImage::from_raw(width as u32, height as u32, foreground)
        .expect("bounded foreground dimensions")
}

fn box_mean(
    source: &[u8],
    scratch: &mut [u8],
    output: &mut [u8],
    width: usize,
    height: usize,
    radius: usize,
) {
    for y in 0..height {
        let row = y * width;
        let mut sum: usize = source[row..=row + radius.min(width - 1)]
            .iter()
            .map(|v| usize::from(*v))
            .sum();
        for x in 0..width {
            let samples = (x + radius).min(width - 1) - x.saturating_sub(radius) + 1;
            scratch[row + x] = ((sum + samples / 2) / samples) as u8;
            if x >= radius {
                sum -= usize::from(source[row + x - radius]);
            }
            if x + radius + 1 < width {
                sum += usize::from(source[row + x + radius + 1]);
            }
        }
    }
    for x in 0..width {
        let mut sum: usize = (0..=radius.min(height - 1))
            .map(|y| usize::from(scratch[y * width + x]))
            .sum();
        for y in 0..height {
            let samples = (y + radius).min(height - 1) - y.saturating_sub(radius) + 1;
            output[(y * width + x) * 4] = ((sum + samples / 2) / samples) as u8;
            if y >= radius {
                sum -= usize::from(scratch[(y - radius) * width + x]);
            }
            if y + radius + 1 < height {
                sum += usize::from(scratch[(y + radius + 1) * width + x]);
            }
        }
    }
}

fn morphology(
    source: &[u8],
    target: &mut [u8],
    scratch: &mut [u8],
    width: usize,
    height: usize,
    radius: usize,
    maximum: bool,
) {
    let mut deque = vec![0usize; width.max(height)];
    for y in 0..height {
        filter_line(
            source,
            scratch,
            y * width,
            1,
            width,
            radius,
            maximum,
            &mut deque,
        );
    }
    for x in 0..width {
        filter_line(
            scratch, target, x, width, height, radius, maximum, &mut deque,
        );
    }
}
fn filter_line(
    source: &[u8],
    target: &mut [u8],
    start: usize,
    step: usize,
    length: usize,
    radius: usize,
    maximum: bool,
    deque: &mut [usize],
) {
    let (mut head, mut tail, mut next) = (0, 0, 0);
    for position in 0..length {
        let end = (position + radius).min(length - 1);
        while next <= end {
            let value = source[start + next * step];
            while tail > head
                && if maximum {
                    source[start + deque[tail - 1] * step] <= value
                } else {
                    source[start + deque[tail - 1] * step] >= value
                }
            {
                tail -= 1;
            }
            deque[tail] = next;
            tail += 1;
            next += 1;
        }
        while head < tail && deque[head] < position.saturating_sub(radius) {
            head += 1;
        }
        target[start + position * step] = source[start + deque[head] * step];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreground_preserves_solid_dark_light_and_colored_details() {
        for (background, ink) in [
            ([255, 255, 255], [0, 0, 0]),
            ([0, 0, 0], [255, 255, 255]),
            ([240, 240, 240], [255, 0, 120]),
            ([20, 20, 20], [15, 110, 180]),
        ] {
            let mut image = RgbaImage::from_pixel(
                80,
                40,
                Rgba([background[0], background[1], background[2], 255]),
            );
            for y in 9..31 {
                for x in 25..28 {
                    image.put_pixel(x, y, Rgba([ink[0], ink[1], ink[2], 255]));
                }
            }
            let f = foreground(&DynamicImage::ImageRgba8(image.clone()), 0, 0, 80, 40).unwrap();
            assert_eq!(f.tint(26, 20, [255, 230, 0]), None);
            assert_eq!(f.tint(60, 20, [255, 230, 0]), Some([255, 230, 0]));
        }
    }
    #[test]
    fn antialias_recovery_tints_background_without_a_pale_or_colored_halo() {
        let mut source = RgbaImage::from_pixel(80, 40, Rgba([255, 255, 255, 255]));
        for y in 10..30 {
            source.put_pixel(25, y, Rgba([0, 0, 0, 255]));
            source.put_pixel(26, y, Rgba([128, 128, 128, 255]));
        }
        let f = foreground(&DynamicImage::ImageRgba8(source), 0, 0, 80, 40).unwrap();
        let color = f.tint(26, 20, [255, 230, 0]).unwrap();
        assert!(color[0].abs_diff(128) <= 1);
        assert!(color[1].abs_diff(115) <= 1);
        assert!(color[2] <= 1);
        assert_eq!(f.tint(25, 20, [255, 230, 0]), None);
    }
    #[test]
    fn tiled_halos_match_unpartitioned_oracle_and_crop_at_all_edges() {
        let source = RgbaImage::from_fn(1057, 31, |x, y| {
            let v = if x % 41 < 3 && y > 2 && y < 29 {
                30
            } else {
                230
            };
            Rgba([v, v / 2, v, 255])
        });
        let oracle = separate(&source);
        let image = DynamicImage::ImageRgba8(source);
        assert_eq!(foreground(&image, 0, 0, 1057, 31).unwrap().pixels, oracle);
        assert_eq!(
            foreground(&image, 503, 7, 542, 19).unwrap().pixels,
            DynamicImage::ImageRgba8(oracle)
                .crop_imm(503, 7, 542, 19)
                .into_rgba8()
        );
    }
    #[test]
    fn nonopaque_sources_are_not_changed_and_invalid_bounds_never_allocate() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(3, 3, Rgba([100, 40, 90, 128])));
        assert_eq!(
            foreground(&image, 0, 0, 3, 3)
                .unwrap()
                .tint(1, 1, [255, 230, 0]),
            None
        );
        for (x, y, w, h) in [
            (0, 0, 0, 1),
            (3, 0, 1, 1),
            (0, 0, u32::MAX, 1),
            (0, u32::MAX, 1, 1),
        ] {
            assert!(foreground(&image, x, y, w, h).is_err());
        }
    }
}
