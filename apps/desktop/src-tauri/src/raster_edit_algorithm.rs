// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
//! Port of Mewu 0.7.3 SeamlessEraseService/MaskedInpaintingService.
//! Local screenshot backgrounds, not photographic generative reconstruction.
use image::{Rgba, RgbaImage};
use mewu_core::DrawingPoint;
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, VecDeque},
};

const MAX_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_BRUSH_WORK: u64 = 128 * 1024 * 1024;
type Check<'a> = &'a dyn Fn() -> Result<(), String>;
type Plane = [[f64; 3]; 4];
#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy)]
struct Sample {
    x: f64,
    y: f64,
    pixel: Rgba<u8>,
}
fn byte(value: f64) -> u8 {
    value.round().clamp(0., 255.) as u8
}
fn normalize(value: f64, size: u32) -> f64 {
    (value - (f64::from(size) - 1.) / 2.) / (f64::from(size) / 2.).max(1.)
}
fn missing() -> String {
    "无法补齐背景，请缩小范围并留出周围背景".into()
}
fn valid_area(image: &RgbaImage, area: Area) -> Result<(), String> {
    if area.width == 0
        || area.height == 0
        || area
            .x
            .checked_add(area.width)
            .is_none_or(|v| v > image.width())
        || area
            .y
            .checked_add(area.height)
            .is_none_or(|v| v > image.height())
        || u64::from(area.width) * u64::from(area.height) > MAX_PIXELS
    {
        return Err("修补范围过大或无效".into());
    }
    Ok(())
}
fn evaluate(plane: &Plane, x: f64, y: f64, channel: usize) -> f64 {
    plane[channel][0] + plane[channel][1] * x + plane[channel][2] * y
}
fn residual(sample: &Sample, plane: &Plane) -> f64 {
    (0..4)
        .map(|c| (f64::from(sample.pixel[c]) - evaluate(plane, sample.x, sample.y, c)).abs())
        .fold(0., f64::max)
}
fn fit(samples: &[Sample], check: Check) -> Result<Plane, String> {
    if samples.is_empty() {
        return Err(missing());
    }
    let mut included = vec![true; samples.len()];
    let mut plane = [[0.; 3]; 4];
    for pass in 0..4 {
        check()?;
        let mut mean = [0.; 2];
        let mut used = 0;
        for (s, yes) in samples.iter().zip(&included) {
            if *yes {
                mean[0] += s.x;
                mean[1] += s.y;
                used += 1;
            }
        }
        for axis in &mut mean {
            *axis /= f64::from(used.max(1));
        }
        let mut matrix: [[f64; 7]; 3] = [[0.; 7]; 3];
        for (i, row) in matrix.iter_mut().enumerate() {
            row[i] = 1e-8;
        }
        for (s, yes) in samples.iter().zip(&included) {
            if !yes {
                continue;
            }
            let x = s.x - mean[0];
            let y = s.y - mean[1];
            let basis = [1., x, y];
            for row in 0..3 {
                for c in 0..3 {
                    matrix[row][c] += basis[row] * basis[c];
                }
                for c in 0..4 {
                    matrix[row][3 + c] += basis[row] * f64::from(s.pixel[c]);
                }
            }
        }
        for column in 0..3 {
            let mut pivot = column;
            for row in column + 1..3 {
                if matrix[row][column].abs() > matrix[pivot][column].abs() {
                    pivot = row;
                }
            }
            matrix.swap(column, pivot);
            let divisor = matrix[column][column];
            if !divisor.is_finite() || divisor.abs() < 1e-14 {
                return Err(missing());
            }
            for c in column..7 {
                matrix[column][c] /= divisor;
            }
            for row in 0..3 {
                if row == column {
                    continue;
                }
                let scale = matrix[row][column];
                for c in column..7 {
                    matrix[row][c] -= scale * matrix[column][c];
                }
            }
        }
        for c in 0..4 {
            plane[c] = [
                matrix[0][c + 3] - matrix[1][c + 3] * mean[0] - matrix[2][c + 3] * mean[1],
                matrix[1][c + 3],
                matrix[2][c + 3],
            ];
        }
        if pass < 3 {
            let residuals: Vec<_> = samples.iter().map(|s| residual(s, &plane)).collect();
            let mut sorted = residuals.clone();
            sorted.sort_by(f64::total_cmp);
            let threshold = (sorted[sorted.len() / 2] * 2.5).max(3.);
            for (yes, value) in included.iter_mut().zip(residuals) {
                *yes = value <= threshold;
            }
        }
    }
    Ok(plane)
}
fn supported(samples: &[Sample], plane: &Plane, fraction: f64) -> bool {
    let mut count = 0;
    let mut squared = 0.;
    for s in samples {
        let error = residual(s, plane);
        if !error.is_finite() {
            return false;
        }
        if error <= 6. {
            count += 1;
            squared += error * error;
        }
    }
    f64::from(count) >= samples.len() as f64 * fraction && squared <= f64::from(count) * 9.
}
fn neighbours(index: usize, width: u32, height: u32) -> [Option<usize>; 4] {
    let x = index % width as usize;
    let y = index / width as usize;
    [
        (x > 0).then(|| index - 1),
        (x + 1 < width as usize).then(|| index + 1),
        (y > 0).then(|| index - width as usize),
        (y + 1 < height as usize).then(|| index + width as usize),
    ]
}
/// The independent outer ring must confirm the plane everywhere. A small line
/// continuing through that ring vetoes the shortcut instead of losing a vote.
fn smooth_context(pixels: &RgbaImage, state: &[u8], check: Check) -> Result<Option<Plane>, String> {
    let (width, height) = pixels.dimensions();
    let mut distance = vec![u8::MAX; state.len()];
    let mut queue = VecDeque::new();
    for (index, &s) in state.iter().enumerate() {
        if s == 1 {
            let x = index % width as usize;
            let y = index / width as usize;
            if x == 0 || y == 0 || x + 1 == width as usize || y + 1 == height as usize {
                return Ok(None);
            }
            distance[index] = 0;
            queue.push_back(index);
        }
    }
    let mut visited = 0;
    while let Some(index) = queue.pop_front() {
        if visited % 1024 == 0 {
            check()?;
        }
        visited += 1;
        if distance[index] >= 10 {
            continue;
        }
        for next in neighbours(index, width, height).into_iter().flatten() {
            if distance[next] > distance[index] + 1 {
                distance[next] = distance[index] + 1;
                queue.push_back(next);
            }
        }
    }
    let inner: Vec<_> = distance
        .iter()
        .enumerate()
        .filter_map(|(index, &d)| (4..=6).contains(&d).then_some(index))
        .collect();
    if inner.is_empty() {
        return Ok(None);
    }
    let step = inner.len().div_ceil(4096).max(1);
    if inner.iter().any(|&i| {
        pixels.get_pixel((i % width as usize) as u32, (i / width as usize) as u32)[3] != 255
    }) {
        return Ok(None);
    }
    let sample = |i: usize| Sample {
        x: normalize((i % width as usize) as f64, width),
        y: normalize((i / width as usize) as f64, height),
        pixel: *pixels.get_pixel((i % width as usize) as u32, (i / width as usize) as u32),
    };
    let samples: Vec<_> = inner.iter().step_by(step).map(|&i| sample(i)).collect();
    let plane = fit(&samples, check)?;
    if !supported(&samples, &plane, 0.95) {
        return Ok(None);
    }
    let mut quadrants = [0u32; 4];
    let mut total = 0.;
    let mut count = 0;
    for (i, &d) in distance.iter().enumerate() {
        if i % 4096 == 0 {
            check()?;
        }
        if !(8..=10).contains(&d) {
            continue;
        }
        let observation = sample(i);
        if observation.pixel[3] != 255 {
            return Ok(None);
        }
        let error = residual(&observation, &plane);
        if !error.is_finite() || error > 6. {
            return Ok(None);
        }
        total += error * error;
        count += 1;
        quadrants[usize::from(observation.x >= 0.) + 2 * usize::from(observation.y >= 0.)] += 1;
    }
    if count == 0 || total > f64::from(count) * 9. || quadrants.iter().any(|&n| n < 8) {
        return Ok(None);
    }
    check()?;
    Ok(Some(plane))
}
#[derive(Clone, Copy, PartialEq)]
struct Front {
    distance: f32,
    index: usize,
}
impl Eq for Front {}
impl PartialOrd for Front {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Front {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .distance
            .total_cmp(&self.distance)
            .then_with(|| other.index.cmp(&self.index))
    }
}
fn offer(
    index: usize,
    state: &[u8],
    distance: &mut [f32],
    width: u32,
    height: u32,
    queue: &mut BinaryHeap<Front>,
) {
    if state[index] != 1 {
        return;
    }
    let around = neighbours(index, width, height);
    let known = |i: Option<usize>| {
        i.filter(|&i| state[i] != 1)
            .map(|i| distance[i])
            .unwrap_or(f32::INFINITY)
    };
    let horizontal = known(around[0]).min(known(around[1]));
    let vertical = known(around[2]).min(known(around[3]));
    let lower = horizontal.min(vertical);
    if !lower.is_finite() {
        return;
    }
    let difference = (horizontal - vertical).abs();
    let next = if !difference.is_finite() || difference >= 1. {
        lower + 1.
    } else {
        (horizontal + vertical + (2. - difference * difference).sqrt()) * 0.5
    };
    if next < distance[index] {
        distance[index] = next;
        queue.push(Front {
            distance: next,
            index,
        });
    }
}
struct Profile {
    coordinate: f64,
    pixels: Vec<Rgba<u8>>,
}
fn pair_error(a: Option<&Profile>, b: Option<&Profile>) -> f64 {
    let (Some(a), Some(b)) = (a, b) else {
        return f64::INFINITY;
    };
    a.pixels
        .iter()
        .zip(&b.pixels)
        .map(|(a, b)| (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0) as f64)
        .sum::<f64>()
        / a.pixels.len() as f64
}
fn read_profile(profile: &Profile, position: f64, c: usize) -> f64 {
    let position = position.clamp(0., (profile.pixels.len() - 1) as f64);
    let before = position as usize;
    let after = (before + 1).min(profile.pixels.len() - 1);
    let t = position - before as f64;
    f64::from(profile.pixels[before][c]) * (1. - t) + f64::from(profile.pixels[after][c]) * t
}
fn interpolate(a: &Profile, b: &Profile, along: f64, across: f64, c: usize) -> f64 {
    let t = ((across - a.coordinate) / (b.coordinate - a.coordinate)).clamp(0., 1.);
    read_profile(a, along, c) * (1. - t) + read_profile(b, along, c) * t
}

/// Only outside strips are donors; extracted text can never tint its replacement.
pub fn rectangle_patch(image: &RgbaImage, area: Area, check: Check) -> Result<RgbaImage, String> {
    valid_area(image, area)?;
    check()?;
    let mut strips: [Vec<(u32, Vec<Rgba<u8>>)>; 4] = std::array::from_fn(|_| vec![]);
    let mut samples = vec![];
    for distance in [1, 3, 5] {
        for side in 0..4 {
            check()?;
            let (horizontal, coordinate) = match side {
                0 if area.y >= distance => (true, area.y - distance),
                1 if area.y + area.height - 1 + distance < image.height() => {
                    (true, area.y + area.height - 1 + distance)
                }
                2 if area.x >= distance => (false, area.x - distance),
                3 if area.x + area.width - 1 + distance < image.width() => {
                    (false, area.x + area.width - 1 + distance)
                }
                _ => continue,
            };
            let length = if horizontal { area.width } else { area.height };
            let pixels: Vec<_> = (0..length)
                .map(|along| {
                    if horizontal {
                        *image.get_pixel(area.x + along, coordinate)
                    } else {
                        *image.get_pixel(coordinate, area.y + along)
                    }
                })
                .collect();
            let entries = length.min(128);
            for entry in 0..entries {
                let along = if entries == 1 {
                    0
                } else {
                    (f64::from(entry) * f64::from(length - 1) / f64::from(entries - 1)).round()
                        as u32
                };
                let (px, py) = if horizontal {
                    (f64::from(along), f64::from(coordinate) - f64::from(area.y))
                } else {
                    (f64::from(coordinate) - f64::from(area.x), f64::from(along))
                };
                samples.push(Sample {
                    x: normalize(px, area.width),
                    y: normalize(py, area.height),
                    pixel: pixels[along as usize],
                });
            }
            strips[side].push((distance, pixels));
        }
    }
    let plane = fit(&samples, check)?;
    let use_plane = supported(&samples, &plane, 0.9);
    let profiles: [Option<Profile>; 4] = std::array::from_fn(|side| {
        let strips = &strips[side];
        if strips.is_empty() {
            return None;
        }
        let distance = strips.iter().map(|s| f64::from(s.0)).sum::<f64>() / strips.len() as f64;
        let coordinate = if side == 0 || side == 2 {
            -distance
        } else {
            f64::from(if side == 1 { area.height } else { area.width }) - 1. + distance
        };
        let pixels = (0..strips[0].1.len())
            .map(|index| {
                let mut rgba = [0; 4];
                for (c, out) in rgba.iter_mut().enumerate() {
                    let mut v: Vec<_> = strips.iter().map(|s| s.1[index][c]).collect();
                    v.sort_unstable();
                    *out = if v.len() == 2 {
                        ((u16::from(v[0]) + u16::from(v[1]) + 1) / 2) as u8
                    } else {
                        v[v.len() / 2]
                    };
                }
                Rgba(rgba)
            })
            .collect();
        Some(Profile { coordinate, pixels })
    });
    let vertical = pair_error(profiles[0].as_ref(), profiles[1].as_ref());
    let horizontal = pair_error(profiles[2].as_ref(), profiles[3].as_ref());
    let use_vertical =
        vertical.is_finite() && (!horizontal.is_finite() || vertical <= horizontal * 0.75);
    let use_horizontal = !use_vertical
        && horizontal.is_finite()
        && (!vertical.is_finite() || horizontal <= vertical * 0.75);
    let mut output = RgbaImage::new(area.width, area.height);
    for y in 0..area.height {
        if y % 32 == 0 {
            check()?;
        }
        for x in 0..area.width {
            let mut pixel = [0; 4];
            for (c, out) in pixel.iter_mut().enumerate() {
                let value = if use_plane {
                    evaluate(
                        &plane,
                        normalize(f64::from(x), area.width),
                        normalize(f64::from(y), area.height),
                        c,
                    )
                } else if use_vertical {
                    interpolate(
                        profiles[0].as_ref().unwrap(),
                        profiles[1].as_ref().unwrap(),
                        f64::from(x),
                        f64::from(y),
                        c,
                    )
                } else if use_horizontal {
                    interpolate(
                        profiles[2].as_ref().unwrap(),
                        profiles[3].as_ref().unwrap(),
                        f64::from(y),
                        f64::from(x),
                        c,
                    )
                } else {
                    let mut total = 0.;
                    let mut weights = 0.;
                    for (side, p) in profiles.iter().enumerate() {
                        if let Some(p) = p {
                            let weight = 1.
                                / ((if side < 2 { f64::from(y) } else { f64::from(x) })
                                    - p.coordinate)
                                    .abs()
                                    .max(1.);
                            total += read_profile(
                                p,
                                if side < 2 { f64::from(x) } else { f64::from(y) },
                                c,
                            ) * weight;
                            weights += weight;
                        }
                    }
                    total / weights
                };
                *out = byte(value);
            }
            output.put_pixel(x, y, Rgba(pixel));
        }
    }
    check()?;
    Ok(output)
}

/// Source-over matting preserves thin anti-aliased text, without a background halo.
pub fn extract_content(
    source: &RgbaImage,
    area: Area,
    background: &RgbaImage,
    check: Check,
) -> Result<RgbaImage, String> {
    valid_area(source, area)?;
    if background.dimensions() != (area.width, area.height) {
        return Err("提取图层尺寸无效".into());
    }
    let length = (area.width * area.height) as usize;
    let observed = source
        .view(area.x, area.y, area.width, area.height)
        .to_image();
    let mut contrast = vec![0u8; length];
    for (i, (pixel, clean)) in observed.pixels().zip(background.pixels()).enumerate() {
        if pixel[3] != 0 {
            contrast[i] = (0..3)
                .map(|c| pixel[c].abs_diff(clean[c]))
                .max()
                .unwrap_or(0);
        }
    }
    let mut output = RgbaImage::new(area.width, area.height);
    for y in 0..area.height {
        if y % 32 == 0 {
            check()?;
        }
        for x in 0..area.width {
            let index = (y * area.width + x) as usize;
            let original = observed.get_pixel(x, y);
            let clean = background.get_pixel(x, y);
            if original[3] == 0 || (contrast[index] <= 3 && original[3].abs_diff(clean[3]) <= 3) {
                continue;
            }
            let mut pixel = [0; 4];
            if clean[3] < 255 && original[3] > clean[3] {
                let coverage = f64::from(original[3] - clean[3]) / f64::from(255 - clean[3]);
                let alpha = byte(coverage * 255.).max(1);
                let coverage = f64::from(alpha) / 255.;
                for c in 0..3 {
                    pixel[c] = byte(
                        (f64::from(original[c]) * f64::from(original[3]) / 255.
                            - (1. - coverage) * f64::from(clean[c]) * f64::from(clean[3]) / 255.)
                            / coverage,
                    );
                }
                pixel[3] = alpha;
            } else {
                let strength = contrast[index];
                if strength <= 3 {
                    continue;
                }
                let delta: Vec<_> = (0..3)
                    .map(|c| f64::from(original[c]) - f64::from(clean[c]))
                    .collect();
                let mut estimate = 1.;
                let near = (y.saturating_sub(2)..=(y + 2).min(area.height - 1)).any(|ny| {
                    (x.saturating_sub(2)..=(x + 2).min(area.width - 1))
                        .any(|nx| contrast[(ny * area.width + nx) as usize] <= 3)
                });
                if strength < 255 && near {
                    for ny in y.saturating_sub(3)..=(y + 3).min(area.height - 1) {
                        for nx in x.saturating_sub(3)..=(x + 3).min(area.width - 1) {
                            let neighbour = observed.get_pixel(nx, ny);
                            if contrast[(ny * area.width + nx) as usize] <= strength
                                || neighbour[3] < original[3]
                            {
                                continue;
                            }
                            let foreground: Vec<_> = (0..3)
                                .map(|c| f64::from(neighbour[c]) - f64::from(clean[c]))
                                .collect();
                            let norm = foreground.iter().map(|v| v * v).sum::<f64>();
                            if norm == 0. {
                                continue;
                            }
                            let alpha = delta
                                .iter()
                                .zip(&foreground)
                                .map(|(d, f)| d * f)
                                .sum::<f64>()
                                / norm;
                            if alpha <= 0. || alpha >= estimate {
                                continue;
                            }
                            let error = delta
                                .iter()
                                .zip(&foreground)
                                .map(|(d, f)| (d - alpha * f).abs())
                                .fold(0., f64::max);
                            let steps = x.abs_diff(nx).max(y.abs_diff(ny));
                            let connected = (1..steps).all(|step| {
                                let px = (f64::from(x)
                                    + (f64::from(nx) - f64::from(x)) * f64::from(step)
                                        / f64::from(steps))
                                .round() as u32;
                                let py = (f64::from(y)
                                    + (f64::from(ny) - f64::from(y)) * f64::from(step)
                                        / f64::from(steps))
                                .round() as u32;
                                contrast[(py * area.width + px) as usize] > 3
                            });
                            if error <= 2f64.max(f64::from(strength) * 0.025) && connected {
                                estimate = alpha;
                            }
                        }
                    }
                }
                let mut minimum: f64 = 0.;
                for c in 0..3 {
                    let difference = delta[c];
                    if difference > 0. {
                        minimum = minimum.max(difference / f64::from(255 - clean[c]));
                    } else if difference < 0. {
                        minimum = minimum.max(-difference / f64::from(clean[c]));
                    }
                }
                let alpha = byte((minimum * 255.).ceil().max((estimate * 255.).round())).max(1);
                let coverage = f64::from(alpha) / 255.;
                for c in 0..3 {
                    pixel[c] = byte(f64::from(clean[c]) + delta[c] / coverage);
                }
                pixel[3] = byte(coverage * f64::from(original[3])).max(1);
            }
            output.put_pixel(x, y, Rgba(pixel));
        }
    }
    check()?;
    Ok(output)
}

/// Antialiased circular capsules; every nonzero coverage is excluded as a donor.
pub fn brush_mask(
    size: (u32, u32),
    points: &[DrawingPoint],
    width: f64,
    check: Check,
) -> Result<(Area, Vec<u8>), String> {
    if points.is_empty()
        || points.len() > 4096
        || !width.is_finite()
        || !(1. ..=256.).contains(&width)
        || points.iter().any(|p| {
            !p.x.is_finite()
                || !p.y.is_finite()
                || p.x < 0.
                || p.y < 0.
                || p.x > f64::from(size.0)
                || p.y > f64::from(size.1)
        })
    {
        return Err("涂抹笔划无效".into());
    }
    let radius = width / 2.;
    let min_x = points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let max_x = points.iter().map(|p| p.x).fold(0., f64::max);
    let min_y = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let max_y = points.iter().map(|p| p.y).fold(0., f64::max);
    let x = (min_x - radius - 0.5).floor().max(0.) as u32;
    let y = (min_y - radius - 0.5).floor().max(0.) as u32;
    let right = (max_x + radius + 0.5).ceil().min(f64::from(size.0)) as u32;
    let bottom = (max_y + radius + 0.5).ceil().min(f64::from(size.1)) as u32;
    let area = Area {
        x,
        y,
        width: right - x,
        height: bottom - y,
    };
    if area.width == 0
        || area.height == 0
        || u64::from(area.width) * u64::from(area.height) > MAX_PIXELS
    {
        return Err("涂抹范围过大".into());
    }
    let mut mask = vec![0; (area.width * area.height) as usize];
    let mut work = 0u64;
    for index in 0..points.len().saturating_sub(1).max(1) {
        check()?;
        let a = points[index];
        let b = points.get(index + 1).copied().unwrap_or(a);
        let left = (a.x.min(b.x) - radius - 0.5).floor().max(f64::from(x)) as u32;
        let top = (a.y.min(b.y) - radius - 0.5).floor().max(f64::from(y)) as u32;
        let right = (a.x.max(b.x) + radius + 0.5).ceil().min(f64::from(right)) as u32;
        let bottom = (a.y.max(b.y) + radius + 0.5).ceil().min(f64::from(bottom)) as u32;
        work = work
            .checked_add(u64::from(right - left) * u64::from(bottom - top))
            .ok_or("涂抹笔划过大")?;
        if work > MAX_BRUSH_WORK {
            return Err("涂抹笔划过长，请分笔修补".into());
        }
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let length = dx * dx + dy * dy;
        for py in top..bottom {
            if py % 32 == 0 {
                check()?;
            }
            for px in left..right {
                let xx = f64::from(px) + 0.5;
                let yy = f64::from(py) + 0.5;
                let t = if length == 0. {
                    0.
                } else {
                    ((xx - a.x) * dx + (yy - a.y) * dy) / length
                }
                .clamp(0., 1.);
                let distance = (xx - a.x - t * dx).hypot(yy - a.y - t * dy);
                let coverage = byte((radius + 0.5 - distance).clamp(0., 1.) * 255.);
                let at = ((py - y) * area.width + px - x) as usize;
                mask[at] = mask[at].max(coverage);
            }
        }
    }
    check()?;
    Ok((area, mask))
}

/// Smooth backgrounds use the original robust plane; other regions advance a
/// deterministic front with positive, alpha-aware donor weights (radius three).
pub fn masked_patch(
    source: &RgbaImage,
    area: Area,
    mask: &[u8],
    check: Check,
) -> Result<RgbaImage, String> {
    valid_area(source, area)?;
    if mask.len() != (area.width * area.height) as usize {
        return Err("涂抹遮罩无效".into());
    }
    check()?;
    let left = area.x.saturating_sub(12);
    let top = area.y.saturating_sub(12);
    let right = (area.x + area.width + 12).min(source.width());
    let bottom = (area.y + area.height + 12).min(source.height());
    let width = right - left;
    let height = bottom - top;
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("涂抹范围过大".into());
    }
    let mut pixels = source.view(left, top, width, height).to_image();
    let mut state = vec![0u8; (width * height) as usize];
    let mut missing_count = 0;
    for y in 0..area.height {
        if y % 32 == 0 {
            check()?;
        }
        for x in 0..area.width {
            if mask[(y * area.width + x) as usize] == 0 {
                continue;
            }
            let px = area.x + x - left;
            let py = area.y + y - top;
            state[(py * width + px) as usize] = 1;
            pixels.put_pixel(px, py, Rgba([0; 4]));
            missing_count += 1;
        }
    }
    if missing_count == 0 {
        return Ok(RgbaImage::new(area.width, area.height));
    }
    let adjacent = |index: usize| {
        let x = index % width as usize;
        let y = index / width as usize;
        [
            (x > 0).then(|| index - 1),
            (x + 1 < width as usize).then(|| index + 1),
            (y > 0).then(|| index - width as usize),
            (y + 1 < height as usize).then(|| index + width as usize),
        ]
    };
    let boundaries: Vec<_> = state
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            (*s == 0 && adjacent(i).into_iter().flatten().any(|j| state[j] == 1)).then_some(i)
        })
        .collect();
    if boundaries.is_empty() {
        return Err(missing());
    }
    let step = boundaries.len().div_ceil(4096).max(1);
    let samples: Vec<_> = boundaries
        .iter()
        .step_by(step)
        .map(|&i| Sample {
            x: normalize((i % width as usize) as f64, width),
            y: normalize((i / width as usize) as f64, height),
            pixel: *pixels.get_pixel((i % width as usize) as u32, (i / width as usize) as u32),
        })
        .collect();
    let candidate = fit(&samples, check)?;
    let plane = if supported(&samples, &candidate, 0.95)
        && boundaries.iter().all(|&i| {
            residual(
                &Sample {
                    x: normalize((i % width as usize) as f64, width),
                    y: normalize((i / width as usize) as f64, height),
                    pixel: *pixels
                        .get_pixel((i % width as usize) as u32, (i / width as usize) as u32),
                },
                &candidate,
            ) <= 6.
        }) {
        Some(candidate)
    } else {
        smooth_context(&pixels, &state, check)?
    };
    if plane.is_none() {
        let mut distances: Vec<_> = state
            .iter()
            .map(|&s| if s == 1 { f32::INFINITY } else { 0. })
            .collect();
        let mut queue = BinaryHeap::new();
        for index in 0..state.len() {
            if index % 4096 == 0 {
                check()?;
            }
            offer(index, &state, &mut distances, width, height, &mut queue);
        }
        let mut completed = 0;
        while let Some(Front { distance, index }) = queue.pop() {
            if completed % 256 == 0 {
                check()?;
            }
            if state[index] == 2 || distances[index] != distance {
                continue;
            }
            let x = (index % width as usize) as i32;
            let y = (index / width as usize) as i32;
            let gradient = |ax: i32, ay: i32, bx: i32, by: i32| {
                let at = |px: i32, py: i32| {
                    if px >= 0 && py >= 0 && px < width as i32 && py < height as i32 {
                        let value = distances[py as usize * width as usize + px as usize];
                        value.is_finite().then_some(value)
                    } else {
                        None
                    }
                };
                match (at(ax, ay), at(bx, by)) {
                    (Some(a), Some(b)) => f64::from(b - a) * 0.5,
                    (Some(a), None) => f64::from(distance - a),
                    (None, Some(b)) => f64::from(b - distance),
                    _ => 0.,
                }
            };
            let normal_x = gradient(x - 1, y, x + 1, y);
            let normal_y = gradient(x, y - 1, x, y + 1);
            let mut channels = [0.; 4];
            let mut total = 0.;
            for dy in -3i32..=3 {
                for dx in -3i32..=3 {
                    let squared = dx * dx + dy * dy;
                    if squared == 0 || squared > 9 {
                        continue;
                    }
                    let px = x + dx;
                    let py = y + dy;
                    if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                        continue;
                    }
                    let j = py as usize * width as usize + px as usize;
                    if state[j] == 1 {
                        continue;
                    }
                    let direction =
                        0.05 + (f64::from(dx) * normal_x + f64::from(dy) * normal_y).abs();
                    let weight = direction
                        / (f64::from(squared).powf(1.5)
                            * (1. + f64::from((distance - distances[j]).abs())))
                        * if state[j] == 2 { 0.75 } else { 1. };
                    let p = pixels.get_pixel(px as u32, py as u32);
                    let alpha = weight * f64::from(p[3]);
                    for c in 0..3 {
                        channels[c] += alpha * f64::from(p[c]);
                    }
                    channels[3] += alpha;
                    total += weight;
                }
            }
            if total <= 0. {
                return Err(missing());
            }
            let mut pixel = [0; 4];
            for c in 0..3 {
                pixel[c] = if channels[3] > 0. {
                    byte(channels[c] / channels[3])
                } else {
                    0
                };
            }
            pixel[3] = byte(channels[3] / total);
            pixels.put_pixel(x as u32, y as u32, Rgba(pixel));
            state[index] = 2;
            completed += 1;
            for j in adjacent(index).into_iter().flatten() {
                offer(j, &state, &mut distances, width, height, &mut queue);
            }
        }
        if completed != missing_count {
            return Err(missing());
        }
    }
    let mut output = RgbaImage::new(area.width, area.height);
    for y in 0..area.height {
        if y % 32 == 0 {
            check()?;
        }
        for x in 0..area.width {
            let coverage = mask[(y * area.width + x) as usize];
            if coverage == 0 {
                continue;
            }
            let px = area.x + x - left;
            let py = area.y + y - top;
            let mut pixel = [0; 4];
            for c in 0..4 {
                let value = if let Some(plane) = &plane {
                    byte(evaluate(
                        plane,
                        normalize(f64::from(px), width),
                        normalize(f64::from(py), height),
                        c,
                    ))
                } else {
                    pixels.get_pixel(px, py)[c]
                };
                pixel[c] = if c == 3 {
                    ((u16::from(value) * u16::from(coverage) + 127) / 255) as u8
                } else {
                    value
                };
            }
            if pixel[3] == 0 {
                pixel = [0; 4];
            }
            output.put_pixel(x, y, Rgba(pixel));
        }
    }
    check()?;
    Ok(output)
}

use image::GenericImageView;

#[cfg(test)]
mod tests {
    use super::*;
    fn ready() -> Result<(), String> {
        Ok(())
    }
    fn gradient() -> RgbaImage {
        RgbaImage::from_fn(96, 80, |x, y| {
            Rgba([(90 + x) as u8, (110 + y) as u8, 180, 255])
        })
    }
    #[test]
    fn independent_outer_ring_repairs_a_thin_glyph_fringe() {
        let mut image = RgbaImage::from_pixel(80, 80, Rgba([240, 240, 240, 255]));
        for y in 30..50 {
            for x in 30..50 {
                image.put_pixel(x, y, Rgba([10, 30, 50, 255]));
            }
        }
        // The immediate donor band still contains antialias fringe.
        for y in 29..51 {
            for x in 29..51 {
                if x == 29 || x == 50 || y == 29 || y == 50 {
                    image.put_pixel(
                        x,
                        y,
                        if (x + y) % 2 == 0 {
                            Rgba([180, 190, 200, 255])
                        } else {
                            Rgba([220, 225, 230, 255])
                        },
                    );
                }
            }
        }
        let area = Area {
            x: 30,
            y: 30,
            width: 20,
            height: 20,
        };
        let patch = masked_patch(&image, area, &[255; 400], &ready).unwrap();
        assert_eq!(patch.get_pixel(10, 10), &Rgba([240, 240, 240, 255]));
    }
    #[test]
    fn independent_ring_vetoes_a_real_line_continuing_outside_the_stroke() {
        let mut pixels = RgbaImage::from_pixel(48, 48, Rgba([240, 240, 240, 255]));
        let mut state = vec![0; 48 * 48];
        for y in 20..28 {
            for x in 20..28 {
                state[y * 48 + x] = 1;
                pixels.put_pixel(x as u32, y as u32, Rgba([0; 4]));
            }
        }
        for y in 0..48 {
            if !(20..28).contains(&y) {
                pixels.put_pixel(24, y, Rgba([10, 20, 30, 255]));
            }
        }
        assert!(smooth_context(&pixels, &state, &ready).unwrap().is_none());
    }
    #[test]
    fn rectangle_reconstructs_gradient_and_never_samples_removed_ink() {
        let clean = gradient();
        let area = Area {
            x: 20,
            y: 20,
            width: 36,
            height: 24,
        };
        let mut image = clean.clone();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                image.put_pixel(x, y, Rgba([255, 0, 255, 255]));
            }
        }
        let patch = rectangle_patch(&image, area, &ready).unwrap();
        for y in 0..area.height {
            for x in 0..area.width {
                let expected = clean.get_pixel(x + area.x, y + area.y);
                for c in 0..4 {
                    assert!(patch.get_pixel(x, y)[c].abs_diff(expected[c]) <= 1);
                }
            }
        }
        assert_eq!(image.get_pixel(30, 30), &Rgba([255, 0, 255, 255]));
    }
    #[test]
    fn rectangle_preserves_hard_panel_boundary() {
        let image = RgbaImage::from_fn(80, 64, |x, _| {
            if x < 40 {
                Rgba([30, 40, 50, 255])
            } else {
                Rgba([220, 230, 240, 255])
            }
        });
        let area = Area {
            x: 20,
            y: 20,
            width: 40,
            height: 24,
        };
        let patch = rectangle_patch(&image, area, &ready).unwrap();
        assert_eq!(patch.get_pixel(5, 10), image.get_pixel(25, 30));
        assert_eq!(patch.get_pixel(30, 10), image.get_pixel(50, 30));
    }
    #[test]
    fn full_source_rectangle_without_surroundings_is_rejected() {
        let image = gradient();
        assert!(rectangle_patch(
            &image,
            Area {
                x: 0,
                y: 0,
                width: 96,
                height: 80
            },
            &ready
        )
        .is_err());
    }
    #[test]
    fn extracted_background_transparent_and_content_movable() {
        let mut source = RgbaImage::from_pixel(48, 40, Rgba([240, 240, 240, 255]));
        for y in 15..25 {
            for x in 20..28 {
                source.put_pixel(x, y, Rgba([20, 30, 60, 255]));
            }
        }
        let area = Area {
            x: 10,
            y: 10,
            width: 28,
            height: 22,
        };
        let patch = rectangle_patch(&source, area, &ready).unwrap();
        let extracted = extract_content(&source, area, &patch, &ready).unwrap();
        assert_eq!(extracted.get_pixel(0, 0)[3], 0);
        assert_eq!(extracted.get_pixel(14, 10), &Rgba([20, 30, 60, 255]));
        assert_eq!(patch.get_pixel(14, 10), &Rgba([240, 240, 240, 255]));
        let mut target = RgbaImage::from_pixel(28, 22, Rgba([60, 100, 200, 255]));
        crate::raster_compositor::paint_raster(&mut target, &extracted, 0., 0., 28., 22.).unwrap();
        assert_eq!(target.get_pixel(0, 0), &Rgba([60, 100, 200, 255]));
        assert_eq!(target.get_pixel(14, 10), &Rgba([20, 30, 60, 255]));
    }
    #[test]
    fn transparent_background_retains_black_foreground() {
        let mut source = RgbaImage::new(32, 32);
        source.put_pixel(16, 16, Rgba([0, 0, 0, 160]));
        let area = Area {
            x: 10,
            y: 10,
            width: 12,
            height: 12,
        };
        let patch = rectangle_patch(&source, area, &ready).unwrap();
        let extracted = extract_content(&source, area, &patch, &ready).unwrap();
        assert_eq!(extracted.get_pixel(6, 6), &Rgba([0, 0, 0, 160]));
    }
    #[test]
    fn antialiased_extraction_unmixes_old_background_halo() {
        let mut source = RgbaImage::from_pixel(32, 32, Rgba([240, 240, 240, 255]));
        source.put_pixel(15, 16, Rgba([120, 120, 120, 255]));
        source.put_pixel(16, 16, Rgba([0, 0, 0, 255]));
        let area = Area {
            x: 10,
            y: 10,
            width: 12,
            height: 12,
        };
        let patch = rectangle_patch(&source, area, &ready).unwrap();
        let extracted = extract_content(&source, area, &patch, &ready).unwrap();
        let edge = extracted.get_pixel(5, 6);
        assert!(edge[3] >= 126 && edge[3] <= 129);
        assert!(edge[0] <= 2);
        let mut dark = Rgba([30, 30, 30, 255]);
        crate::raster_compositor::blend_rgba(&mut dark, edge);
        assert!(dark[0] >= 14 && dark[0] <= 16);
    }
    #[test]
    fn brush_capsule_has_nonzero_soft_edges_and_no_outside_fill() {
        let (area, mask) = brush_mask(
            (96, 80),
            &[
                DrawingPoint { x: 24., y: 24. },
                DrawingPoint { x: 50., y: 36. },
            ],
            9.,
            &ready,
        )
        .unwrap();
        assert!(mask.iter().any(|&m| m > 0 && m < 255));
        assert!(mask.contains(&255));
        let image = gradient();
        let patch = masked_patch(&image, area, &mask, &ready).unwrap();
        for (p, &m) in patch.pixels().zip(&mask) {
            if m == 0 {
                assert_eq!(p, &Rgba([0; 4]));
            }
        }
    }
    #[test]
    fn painted_source_pixels_never_become_repair_donors() {
        let clean = gradient();
        let (area, mask) = brush_mask(
            clean.dimensions(),
            &[
                DrawingPoint { x: 30., y: 32. },
                DrawingPoint { x: 50., y: 38. },
            ],
            12.,
            &ready,
        )
        .unwrap();
        let mut a = clean.clone();
        let mut b = clean.clone();
        for y in 0..area.height {
            for x in 0..area.width {
                if mask[(y * area.width + x) as usize] > 0 {
                    a.put_pixel(area.x + x, area.y + y, Rgba([255, 0, 255, 255]));
                    b.put_pixel(area.x + x, area.y + y, Rgba([0, 255, 0, 30]));
                }
            }
        }
        assert_eq!(
            masked_patch(&a, area, &mask, &ready).unwrap(),
            masked_patch(&b, area, &mask, &ready).unwrap()
        );
    }
    #[test]
    fn fully_painted_source_is_rejected_without_donors() {
        let image = RgbaImage::from_pixel(8, 8, Rgba([200; 4]));
        assert!(masked_patch(
            &image,
            Area {
                x: 0,
                y: 0,
                width: 8,
                height: 8
            },
            &[255; 64],
            &ready
        )
        .is_err());
    }
    #[test]
    fn nonplanar_repair_uses_bounded_colors_without_amplification() {
        let image = RgbaImage::from_fn(48, 48, |x, y| {
            if (x / 4 + y / 4) % 2 == 0 {
                Rgba([20, 40, 60, 255])
            } else {
                Rgba([80, 140, 200, 255])
            }
        });
        let area = Area {
            x: 16,
            y: 16,
            width: 16,
            height: 16,
        };
        let patch = masked_patch(&image, area, &[255; 256], &ready).unwrap();
        for p in patch.pixels() {
            assert!((20..=80).contains(&p[0]));
            assert!((40..=140).contains(&p[1]));
            assert!((60..=200).contains(&p[2]));
            assert_eq!(p[3], 255);
        }
    }
    #[test]
    fn brush_and_rectangle_cancel_during_work_without_source_mutation() {
        let image = gradient();
        let before = image.clone();
        let calls = std::cell::Cell::new(0);
        let check = || {
            let n = calls.get() + 1;
            calls.set(n);
            if n > 3 {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        };
        assert!(rectangle_patch(
            &image,
            Area {
                x: 12,
                y: 12,
                width: 32,
                height: 32
            },
            &check
        )
        .is_err());
        assert_eq!(image, before);
        assert!(
            brush_mask((96, 80), &[DrawingPoint { x: 30., y: 30. }], 32., &|| Err(
                "cancelled".into()
            ))
            .is_err()
        );
    }
    #[test]
    fn invalid_and_huge_strokes_reject_before_allocation() {
        assert!(brush_mask(
            (100, 100),
            &[DrawingPoint { x: f64::NAN, y: 2. }],
            32.,
            &ready
        )
        .is_err());
        assert!(brush_mask((100, 100), &[DrawingPoint { x: 2., y: 2. }], 257., &ready).is_err());
        assert!(brush_mask(
            (6000, 6000),
            &[
                DrawingPoint { x: 0., y: 0. },
                DrawingPoint { x: 6000., y: 6000. }
            ],
            32.,
            &ready
        )
        .is_err());
    }
}
