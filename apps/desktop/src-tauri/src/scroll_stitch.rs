// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
//! Pixel-preserving vertical scrolling capture. Adapted from this project's
//! ScrollingCaptureComposer/Accumulator.cs; no capture, input or file I/O.
//!
//! Matching follows masked translation/template comparison, not panorama
//! homography: https://docs.opencv.org/4.13.0/de/da9/tutorial_template_matching.html
//! RGB/edge descriptors shortlist offsets; original pixels independently verify
//! them. Ambiguous textures, low detail and insufficient overlap do not move the
//! anchor. A lost anchor can reconnect to already captured strips. All output
//! bytes come from accepted frames, without interpolation or feathering.

use image::RgbaImage;
use std::{
    collections::VecDeque,
    fmt,
    mem::size_of,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_FRAME_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_OUTPUT_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_FRAME_EDGE: u32 = 8192;
const MAX_HEIGHT: u32 = 131072;
const MAX_RESIDENT: u64 = 512 * 1024 * 1024;
const CANDIDATES: usize = 24;

#[derive(Debug, Clone, Copy)]
pub struct StitchLimits {
    pub max_frame_pixels: u64,
    pub max_output_pixels: u64,
    pub max_output_height: u32,
    /// Conservative bound on owned pixel/descriptor buffers, strip metadata,
    /// incoming frame and final assembly. Excludes the caller's capture queue
    /// and allocator bookkeeping; the host must bound these separately.
    pub max_resident_bytes: u64,
}

impl Default for StitchLimits {
    fn default() -> Self {
        Self {
            max_frame_pixels: MAX_FRAME_PIXELS,
            max_output_pixels: MAX_OUTPUT_PIXELS,
            max_output_height: 32768,
            max_resident_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StitchStatus {
    Initial,
    Extended,
    Retraced,
    Unchanged,
    LowInformation,
    Ambiguous,
    LostOverlap,
    LimitReached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StitchState {
    pub width: u32,
    pub height: u32,
    /// Accepted frame's origin, in rows relative to the first captured frame.
    pub offset: i64,
    pub added_top: u32,
    pub added_bottom: u32,
    pub status: StitchStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StitchError {
    InvalidLimits,
    InvalidFrame,
    FrameSizeChanged,
    MemoryLimit,
    Allocation,
    Canceled,
}
impl fmt::Display for StitchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidLimits => "长截图容量配置无效",
            Self::InvalidFrame => "长截图画面需至少 16×64 像素，且不超过采集上限",
            Self::FrameSizeChanged => "长截图画面尺寸已改变，请重新开始",
            Self::MemoryLimit => "长截图超过内存容量",
            Self::Allocation => "无法分配长截图图像内存",
            Self::Canceled => "长截图已取消",
        })
    }
}
impl std::error::Error for StitchError {}
type Result<T> = std::result::Result<T, StitchError>;

#[derive(Clone, Copy, Default)]
struct Feature {
    rgb: [u8; 3],
    activity: u8,
    texture: u8,
}
impl Feature {
    fn informative(self) -> bool {
        self.activity >= 5
            || self.texture >= 4
            || self.rgb.iter().max().unwrap() - self.rgb.iter().min().unwrap() >= 20
    }
    fn distance(self, other: Self) -> f64 {
        color_distance(self.rgb, other.rgb)
            + self.activity.abs_diff(other.activity) as f64 * 0.25
            + self.texture.abs_diff(other.texture) as f64 * 0.25
    }
    fn same_color(self, other: Self) -> bool {
        self.rgb
            .into_iter()
            .zip(other.rgb)
            .all(|(a, b)| a.abs_diff(b) <= 1)
    }
}

struct Grid {
    columns: usize,
    height: usize,
    cells: Vec<Feature>,
}
impl Grid {
    fn new(image: &RgbaImage, cancel: &AtomicBool) -> Result<Self> {
        let width = image.width() as usize;
        let height = image.height() as usize;
        let columns = (width / 4).clamp(8, 192).min(width);
        let mut cells = capacity(columns * height)?;
        for y in 0..height {
            check_cancel(cancel)?;
            for column in 0..columns {
                let from = column * width / columns;
                let to = (column + 1) * width / columns;
                let mut rgb = [0u32; 3];
                let (mut low, mut high) = (255u16, 0u16);
                for x in from..to {
                    let pixel = image.get_pixel(x as u32, y as u32).0;
                    let alpha = u32::from(pixel[3]);
                    let values: [u32; 3] = std::array::from_fn(|c| {
                        (u32::from(pixel[c]) * alpha + 255 * (255 - alpha) + 127) / 255
                    });
                    for c in 0..3 {
                        rgb[c] += values[c];
                    }
                    let luma = ((values[0] * 77 + values[1] * 150 + values[2] * 29) >> 8) as u16;
                    low = low.min(luma);
                    high = high.max(luma);
                }
                cells.push(Feature {
                    rgb: rgb.map(|sum| (sum / (to - from) as u32) as u8),
                    activity: (high - low) as u8,
                    texture: 0,
                });
            }
        }
        for y in 0..height {
            for x in 0..columns {
                let index = y * columns + x;
                let value = cells[index];
                let mut activity = value.activity;
                if x > 0 {
                    activity = activity.max(color_distance(value.rgb, cells[index - 1].rgb) as u8);
                }
                if y > 0 {
                    activity =
                        activity.max(color_distance(value.rgb, cells[index - columns].rgb) as u8);
                }
                cells[index].activity = activity;
                if y > 0 && y + 1 < height {
                    // A smooth gradient/solid color has no vertical curvature,
                    // even when its chroma or horizontal detail is high.
                    cells[index].texture = (0..3)
                        .map(|c| {
                            (i16::from(cells[index - columns].rgb[c])
                                + i16::from(cells[index + columns].rgb[c])
                                - 2 * i16::from(value.rgb[c]))
                            .unsigned_abs()
                            .min(255) as u8
                        })
                        .max()
                        .unwrap();
                }
            }
        }
        Ok(Self {
            columns,
            height,
            cells,
        })
    }
    fn row(&self, y: usize) -> &[Feature] {
        &self.cells[y * self.columns..(y + 1) * self.columns]
    }
    fn informative(&self, trim: Trim) -> bool {
        let end = self.height - trim.bottom;
        let span = end.saturating_sub(trim.top).max(1);
        let (mut count, mut rows, mut columns) = (0usize, 0u8, 0u8);
        for y in trim.top.max(1)..end.min(self.height - 1) {
            for x in 1..self.columns - 1 {
                if self.row(y)[x].texture >= 4 {
                    count += 1;
                    rows |= 1 << (((y - trim.top) * 6 / span).min(5));
                    columns |= 1 << ((x * 8 / self.columns).min(7));
                }
            }
        }
        count >= 24 && rows.count_ones() >= 2 && columns.count_ones() >= 2
    }
}

/// Each retained strip owns only its actual rows. When a seam cuts a strip,
/// copy that one boundary fragment before committing; retaining a large Vec's
/// capacity after each one-pixel scroll would otherwise quietly grow memory.
struct Strip {
    pixels: Vec<u8>,
    features: Vec<Feature>,
    rows: usize,
}
impl Strip {
    fn from_frame(frame: &RgbaImage, grid: &Grid, from: usize, to: usize) -> Result<Self> {
        let stride = frame.width() as usize * 4;
        Ok(Self {
            pixels: copy(&frame.as_raw()[from * stride..to * stride])?,
            features: copy(&grid.cells[from * grid.columns..to * grid.columns])?,
            rows: to - from,
        })
    }
    fn part(&self, from: usize, to: usize, stride: usize, columns: usize) -> Result<Self> {
        Ok(Self {
            pixels: copy(&self.pixels[from * stride..to * stride])?,
            features: copy(&self.features[from * columns..to * columns])?,
            rows: to - from,
        })
    }
}

pub struct ScrollStitcher {
    limits: StitchLimits,
    width: u32,
    frame_height: u32,
    top: i64,
    state: StitchState,
    anchor: RgbaImage,
    grid: Grid,
    strips: VecDeque<Strip>,
}

impl ScrollStitcher {
    pub fn new(frame: RgbaImage, limits: StitchLimits) -> Result<Self> {
        validate_limits(limits)?;
        validate_frame(&frame, limits)?;
        let (width, height) = frame.dimensions();
        if !fits(limits, width, height, height, frame.as_raw().capacity()) {
            return Err(StitchError::MemoryLimit);
        }
        let grid = Grid::new(&frame, &AtomicBool::new(false))?;
        let strip = Strip::from_frame(&frame, &grid, 0, height as usize)?;
        let mut strips = VecDeque::new();
        strips
            .try_reserve_exact(1)
            .map_err(|_| StitchError::Allocation)?;
        strips.push_back(strip);
        Ok(Self {
            limits,
            width,
            frame_height: height,
            top: 0,
            state: StitchState {
                width,
                height,
                offset: 0,
                added_top: 0,
                added_bottom: 0,
                status: StitchStatus::Initial,
            },
            anchor: frame,
            grid,
            strips,
        })
    }

    pub fn state(&self) -> StitchState {
        self.state
    }

    pub fn accept(&mut self, frame: RgbaImage) -> Result<StitchState> {
        self.accept_with_cancel(frame, &AtomicBool::new(false))
    }

    pub fn accept_with_cancel(
        &mut self,
        frame: RgbaImage,
        cancel: &AtomicBool,
    ) -> Result<StitchState> {
        check_cancel(cancel)?;
        if frame.dimensions() != (self.width, self.frame_height) {
            return Err(StitchError::FrameSizeChanged);
        }
        let frame_capacity = frame
            .as_raw()
            .capacity()
            .max(self.anchor.as_raw().capacity());
        if !fits(
            self.limits,
            self.width,
            self.frame_height,
            self.state.height,
            frame_capacity,
        ) {
            return Ok(self.status(StitchStatus::LimitReached));
        }
        let current = Grid::new(&frame, cancel)?;
        let trim = stationary_edges(&self.grid, &current);
        if !current.informative(trim) || !self.grid.informative(trim) {
            return Ok(self.status(StitchStatus::LowInformation));
        }
        let minimum = 24usize.max((current.height - trim.top - trim.bottom) / 5);
        let local = {
            let source = Rows::frame(&self.anchor, &self.grid)?;
            if let Some(score) = score(&source, &current, 0, trim, minimum, false, None) {
                if score.error <= 0.75
                    && raw_agrees(&source, &frame, &current, 0, trim, minimum, 1., 0.02)
                {
                    return Ok(self.status(StitchStatus::Unchanged));
                }
            }
            locate(
                &source,
                &frame,
                &current,
                trim,
                minimum,
                Some((&self.grid, &current)),
                cancel,
            )?
        };
        let origin = match local {
            Located::Offset(shift) => self.state.offset + shift,
            Located::Ambiguous => return Ok(self.status(StitchStatus::Ambiguous)),
            Located::Missing => {
                let global = {
                    let source = Rows::strips(
                        &self.strips,
                        self.width,
                        self.grid.columns,
                        self.state.height as usize,
                    )?;
                    locate(&source, &frame, &current, trim, minimum, None, cancel)?
                };
                match global {
                    Located::Offset(offset) => self.top + offset,
                    Located::Ambiguous => return Ok(self.status(StitchStatus::Ambiguous)),
                    Located::Missing => return Ok(self.status(StitchStatus::LostOverlap)),
                }
            }
        };
        check_cancel(cancel)?;
        let old_bottom = self.top + i64::from(self.state.height);
        let new_top = self.top.min(origin);
        let new_bottom = old_bottom.max(origin + i64::from(self.frame_height));
        let height = (new_bottom - new_top) as u32;
        if !fits(
            self.limits,
            self.width,
            self.frame_height,
            height,
            frame_capacity,
        ) {
            return Ok(self.status(StitchStatus::LimitReached));
        }
        let added_top = (self.top - new_top) as u32;
        let added_bottom = (new_bottom - old_bottom) as u32;
        if added_top > 0 || added_bottom > 0 {
            // Choose the seam inside common content, away from both frames'
            // fixed header/footer. This works symmetrically when extending up.
            let overlap_start = (self.top + trim.top as i64).max(origin + trim.top as i64);
            let overlap_end = (old_bottom - trim.bottom as i64)
                .min(origin + i64::from(self.frame_height) - trim.bottom as i64);
            if overlap_end - overlap_start < minimum as i64 {
                return Ok(self.status(StitchStatus::LostOverlap));
            }
            let seam = overlap_start + (overlap_end - overlap_start) / 2;
            let source_seam = (seam - origin) as usize;
            let output_seam = (seam - self.top) as usize;
            let extend_up = added_top > 0;
            let incoming = if extend_up {
                Strip::from_frame(&frame, &current, 0, source_seam)?
            } else {
                Strip::from_frame(&frame, &current, source_seam, self.frame_height as usize)?
            };
            let (cut, boundary) = self.boundary(output_seam, extend_up)?;
            self.strips
                .try_reserve(2)
                .map_err(|_| StitchError::Allocation)?;
            check_cancel(cancel)?;
            // No fallible operation follows: the accepted bytes and anchor
            // change together, even on allocation failure or cancellation.
            if extend_up {
                for _ in 0..cut {
                    self.strips.pop_front();
                }
                if let Some(boundary) = boundary {
                    self.strips.push_front(boundary);
                }
                self.strips.push_front(incoming);
            } else {
                self.strips.truncate(cut);
                if let Some(boundary) = boundary {
                    self.strips.push_back(boundary);
                }
                self.strips.push_back(incoming);
            }
        }
        self.top = new_top;
        let status = if added_top > 0 || added_bottom > 0 {
            StitchStatus::Extended
        } else if origin == self.state.offset {
            StitchStatus::Unchanged
        } else {
            StitchStatus::Retraced
        };
        self.state = StitchState {
            width: self.width,
            height,
            offset: origin,
            added_top,
            added_bottom,
            status,
        };
        self.anchor = frame;
        self.grid = current;
        Ok(self.state)
    }

    fn boundary(&self, seam: usize, suffix: bool) -> Result<(usize, Option<Strip>)> {
        let mut position = 0;
        for (index, strip) in self.strips.iter().enumerate() {
            let end = position + strip.rows;
            if seam == position {
                return Ok((index, None));
            }
            if seam < end {
                let split = seam - position;
                let boundary = if suffix {
                    strip.part(
                        split,
                        strip.rows,
                        self.width as usize * 4,
                        self.grid.columns,
                    )?
                } else {
                    strip.part(0, split, self.width as usize * 4, self.grid.columns)?
                };
                return Ok((if suffix { index + 1 } else { index }, Some(boundary)));
            }
            position = end;
        }
        Ok((self.strips.len(), None))
    }

    fn status(&mut self, status: StitchStatus) -> StitchState {
        self.state.status = status;
        self.state.added_top = 0;
        self.state.added_bottom = 0;
        self.state
    }

    pub fn finish(self) -> Result<RgbaImage> {
        let Self {
            state,
            mut strips,
            anchor,
            grid,
            ..
        } = self;
        drop(anchor);
        drop(grid);
        if strips.len() == 1 {
            let strip = strips.pop_front().unwrap();
            return RgbaImage::from_raw(state.width, state.height, strip.pixels)
                .ok_or(StitchError::InvalidFrame);
        }
        let bytes = state.width as usize * state.height as usize * 4;
        let mut output = capacity(bytes)?;
        for strip in strips {
            output.extend_from_slice(&strip.pixels);
        }
        RgbaImage::from_raw(state.width, state.height, output).ok_or(StitchError::InvalidFrame)
    }
}

#[derive(Clone, Copy, Default)]
struct Trim {
    top: usize,
    bottom: usize,
}
fn stationary_edges(first: &Grid, second: &Grid) -> Trim {
    let unchanged = |y| {
        first
            .row(y)
            .iter()
            .zip(second.row(y))
            .filter(|(a, b)| a.same_color(**b))
            .count()
            * 100
            >= first.columns * 95
    };
    let maximum = first.height / 4;
    let top = (0..maximum).take_while(|&y| unchanged(y)).count();
    let bottom = (0..maximum)
        .take_while(|&y| unchanged(first.height - 1 - y))
        .count();
    Trim { top, bottom }
}

struct Rows<'a> {
    features: Vec<&'a [Feature]>,
    pixels: Vec<&'a [u8]>,
    width: usize,
    columns: usize,
}
impl<'a> Rows<'a> {
    fn frame(image: &'a RgbaImage, grid: &'a Grid) -> Result<Self> {
        let mut features = capacity(grid.height)?;
        let mut pixels = capacity(grid.height)?;
        for y in 0..grid.height {
            features.push(grid.row(y));
            let start = y * image.width() as usize * 4;
            pixels.push(&image.as_raw()[start..start + image.width() as usize * 4]);
        }
        Ok(Self {
            features,
            pixels,
            width: image.width() as usize,
            columns: grid.columns,
        })
    }
    fn strips(
        strips: &'a VecDeque<Strip>,
        width: u32,
        columns: usize,
        height: usize,
    ) -> Result<Self> {
        let mut features = capacity(height)?;
        let mut pixels = capacity(height)?;
        for strip in strips {
            features.extend(strip.features.chunks_exact(columns));
            pixels.extend(strip.pixels.chunks_exact(width as usize * 4));
        }
        Ok(Self {
            features,
            pixels,
            width: width as usize,
            columns,
        })
    }
    fn overlap(
        &self,
        target: &Grid,
        origin: i64,
        trim: Trim,
        minimum: usize,
    ) -> Option<(usize, usize)> {
        let start = (trim.top as i64).max(trim.top as i64 - origin).max(0);
        let end = (target.height as i64 - trim.bottom as i64)
            .min(self.features.len() as i64 - trim.bottom as i64 - origin);
        (end - start >= minimum as i64).then_some((start as usize, end.max(0) as usize))
    }
}

#[derive(Clone, Copy)]
struct Score {
    error: f64,
    good: usize,
    count: usize,
    rows: u8,
    columns: u8,
}
impl Score {
    fn acceptable(self) -> bool {
        self.error <= 10.
            && self.good * 100 >= self.count * 80
            && self.rows.count_ones() >= 2
            && self.columns.count_ones() >= 2
    }
}

fn score(
    source: &Rows<'_>,
    target: &Grid,
    origin: i64,
    trim: Trim,
    minimum: usize,
    coarse: bool,
    stationary: Option<(&Grid, &Grid)>,
) -> Option<Score> {
    let (start, end) = source.overlap(target, origin, trim, minimum)?;
    let step_y = ((end - start) / if coarse { 40 } else { 240 }).max(1);
    let step_x = if coarse {
        (target.columns / 24).max(1)
    } else {
        1
    };
    let mut value = Score {
        error: 0.,
        good: 0,
        count: 0,
        rows: 0,
        columns: 0,
    };
    for y in (start + 1..end - 1).step_by(step_y) {
        let sy = (y as i64 + origin) as usize;
        for x in (1..target.columns - 1).step_by(step_x) {
            let a = source.features[sy][x];
            let b = target.row(y)[x];
            if !a.informative() && !b.informative() {
                continue;
            }
            if origin != 0 {
                if let Some((old, new)) = stationary {
                    if old.row(sy)[x].same_color(new.row(sy)[x])
                        && old.row(y)[x].same_color(new.row(y)[x])
                    {
                        continue;
                    }
                }
            }
            let error = a.distance(b);
            value.error += error.min(48.);
            value.count += 1;
            if error <= 8. {
                value.good += 1;
                value.rows |= 1 << (((y - start) * 6 / (end - start)).min(5));
                value.columns |= 1 << ((x * 8 / target.columns).min(7));
            }
        }
    }
    if value.count < if coarse { 16 } else { 24 } {
        return None;
    }
    value.error /= value.count as f64;
    Some(value)
}

fn raw_agrees(
    source: &Rows<'_>,
    frame: &RgbaImage,
    grid: &Grid,
    origin: i64,
    trim: Trim,
    minimum: usize,
    maximum_error: f64,
    maximum_bad: f64,
) -> bool {
    let Some((start, end)) = source.overlap(grid, origin, trim, minimum) else {
        return false;
    };
    let (mut sum, mut count, mut bad) = (0f64, 0usize, 0usize);
    for y in (start + 1..end - 1).step_by(((end - start) / 120).max(1)) {
        let sy = (y as i64 + origin) as usize;
        for x in (1..source.width - 1).step_by((source.width / 96).max(1)) {
            let column = (x * source.columns / source.width).min(source.columns - 1);
            if !source.features[sy][column].informative() && !grid.row(y)[column].informative() {
                continue;
            }
            let a = &source.pixels[sy][x * 4..x * 4 + 3];
            let b = frame.get_pixel(x as u32, y as u32).0;
            let error = color_distance([a[0], a[1], a[2]], [b[0], b[1], b[2]]);
            sum += error;
            count += 1;
            if error > 32. {
                bad += 1;
            }
        }
    }
    count >= 24 && sum / count as f64 <= maximum_error && bad as f64 / count as f64 <= maximum_bad
}

enum Located {
    Offset(i64),
    Ambiguous,
    Missing,
}
fn locate(
    source: &Rows<'_>,
    frame: &RgbaImage,
    grid: &Grid,
    trim: Trim,
    minimum: usize,
    stationary: Option<(&Grid, &Grid)>,
    cancel: &AtomicBool,
) -> Result<Located> {
    let mut candidates: Vec<(i64, f64)> = capacity(CANDIDATES + 1)?;
    let first = -(grid.height as i64 - minimum as i64);
    let last = source.features.len() as i64 - minimum as i64;
    for origin in first..=last {
        if origin & 31 == 0 {
            check_cancel(cancel)?;
        }
        let Some(score) = score(source, grid, origin, trim, minimum, true, stationary) else {
            continue;
        };
        let position = candidates.partition_point(|(_, error)| *error < score.error);
        if position < CANDIDATES {
            candidates.insert(position, (origin, score.error));
            candidates.truncate(CANDIDATES);
        }
    }
    let mut verified: Vec<(i64, f64)> = capacity(CANDIDATES)?;
    for (origin, _) in candidates {
        check_cancel(cancel)?;
        if let Some(value) = score(source, grid, origin, trim, minimum, false, stationary) {
            if value.acceptable()
                && raw_agrees(source, frame, grid, origin, trim, minimum, 12., 0.12)
            {
                verified.push((origin, value.error));
            }
        }
    }
    verified.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
    let Some(&(origin, error)) = verified.first() else {
        return Ok(Located::Missing);
    };
    if verified
        .get(1)
        .is_some_and(|(_, other)| *other - error <= 1.5f64.max(error * 0.25))
    {
        return Ok(Located::Ambiguous);
    }
    Ok(Located::Offset(origin))
}

fn validate_limits(limits: StitchLimits) -> Result<()> {
    if limits.max_frame_pixels == 0
        || limits.max_frame_pixels > MAX_FRAME_PIXELS
        || limits.max_output_pixels < limits.max_frame_pixels
        || limits.max_output_pixels > MAX_OUTPUT_PIXELS
        || limits.max_output_height < 64
        || limits.max_output_height > MAX_HEIGHT
        || limits.max_resident_bytes == 0
        || limits.max_resident_bytes > MAX_RESIDENT
    {
        return Err(StitchError::InvalidLimits);
    }
    Ok(())
}
fn validate_frame(frame: &RgbaImage, limits: StitchLimits) -> Result<()> {
    let (width, height) = frame.dimensions();
    if width < 16
        || height < 64
        || width > MAX_FRAME_EDGE
        || height > MAX_FRAME_EDGE
        || u64::from(width) * u64::from(height) > limits.max_frame_pixels
    {
        return Err(StitchError::InvalidFrame);
    }
    Ok(())
}
fn fits(
    limits: StitchLimits,
    width: u32,
    frame_height: u32,
    height: u32,
    frame_capacity: usize,
) -> bool {
    let pixels = u64::from(width) * u64::from(height);
    if height > limits.max_output_height || pixels > limits.max_output_pixels {
        return false;
    }
    let columns = (width as u64 / 4).clamp(8, 192).min(u64::from(width));
    let descriptors =
        columns * (u64::from(height) + u64::from(frame_height)) * size_of::<Feature>() as u64;
    let metadata = (u64::from(limits.max_output_height) + 4) * (4 * size_of::<Strip>() as u64 + 32);
    let owned = pixels * 8 + frame_capacity as u64 * 2 + descriptors * 2 + metadata + 65536;
    owned <= limits.max_resident_bytes
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(StitchError::Canceled)
    } else {
        Ok(())
    }
}
fn color_distance(a: [u8; 3], b: [u8; 3]) -> f64 {
    (u16::from(a[0].abs_diff(b[0]))
        + u16::from(a[1].abs_diff(b[1]))
        + u16::from(a[2].abs_diff(b[2]))) as f64
        / 3.
}
fn capacity<T>(count: usize) -> Result<Vec<T>> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| StitchError::Allocation)?;
    Ok(values)
}
fn copy<T: Copy>(source: &[T]) -> Result<Vec<T>> {
    let mut values = capacity(source.len())?;
    values.extend_from_slice(source);
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn pixel(x: u32, y: i64, seed: u32) -> Rgba<u8> {
        let mut value = (y as u32).wrapping_mul(0x9e3779b9) ^ x.wrapping_mul(0x85ebca6b) ^ seed;
        value ^= value >> 16;
        value = value.wrapping_mul(0x7feb352d);
        value ^= value >> 15;
        value = value.wrapping_mul(0x846ca68b);
        value ^= value >> 16;
        Rgba([value as u8, (value >> 8) as u8, (value >> 16) as u8, 255])
    }
    fn frame(top: i64) -> RgbaImage {
        RgbaImage::from_fn(128, 96, |x, y| pixel(x, top + i64::from(y), 42))
    }
    fn expected(top: i64, height: u32) -> RgbaImage {
        RgbaImage::from_fn(128, height, |x, y| pixel(x, top + i64::from(y), 42))
    }
    fn limits() -> StitchLimits {
        StitchLimits {
            max_output_height: 2048,
            ..Default::default()
        }
    }

    #[test]
    fn exact_bidirectional_pixels_and_retracing_do_not_duplicate_rows() {
        let mut stitch = ScrollStitcher::new(frame(40), limits()).unwrap();
        let down = stitch.accept(frame(72)).unwrap();
        assert_eq!(
            (down.status, down.offset, down.height, down.added_bottom),
            (StitchStatus::Extended, 32, 128, 32)
        );
        let retrace = stitch.accept(frame(52)).unwrap();
        assert_eq!(
            (retrace.status, retrace.offset, retrace.height),
            (StitchStatus::Retraced, 12, 128)
        );
        assert_eq!((retrace.added_top, retrace.added_bottom), (0, 0));
        let up = stitch.accept(frame(12)).unwrap();
        assert_eq!(
            (up.status, up.offset, up.height, up.added_top),
            (StitchStatus::Extended, -28, 156, 28)
        );
        stitch.accept(frame(-20)).unwrap();
        assert_eq!(stitch.finish().unwrap(), expected(-20, 188));
    }

    #[test]
    fn hundreds_of_single_pixel_scrolls_preserve_rows_and_release_cut_strip_capacity() {
        let mut stitch = ScrollStitcher::new(frame(0), limits()).unwrap();
        for top in 1..=300 {
            let state = stitch.accept(frame(top)).unwrap();
            assert_eq!(
                (state.status, state.offset, state.height),
                (StitchStatus::Extended, top, 96 + top as u32)
            );
        }
        let stored_bytes: usize = stitch
            .strips
            .iter()
            .map(|strip| strip.pixels.capacity())
            .sum();
        assert_eq!(stored_bytes, 128 * 396 * 4);
        assert_eq!(
            stitch.strips.iter().map(|strip| strip.rows).sum::<usize>(),
            396
        );
        assert_eq!(stitch.finish().unwrap(), expected(0, 396));
    }

    #[test]
    fn far_return_reconnects_to_accumulated_content_and_failed_frames_do_not_replace_anchor() {
        let mut stitch = ScrollStitcher::new(frame(0), limits()).unwrap();
        for top in (48..=336).step_by(48) {
            assert_eq!(
                stitch.accept(frame(top)).unwrap().status,
                StitchStatus::Extended
            );
        }
        let saved = stitch.state();
        let unrelated = RgbaImage::from_fn(128, 96, |x, y| pixel(x, i64::from(y), 97531));
        let failure = stitch.accept(unrelated).unwrap();
        assert_eq!(failure.status, StitchStatus::LostOverlap);
        assert_eq!(
            (failure.height, failure.offset),
            (saved.height, saved.offset)
        );
        let back = stitch.accept(frame(0)).unwrap();
        assert_eq!(
            (back.status, back.offset, back.height),
            (StitchStatus::Retraced, 0, 432)
        );
        let forward = stitch.accept(frame(300)).unwrap();
        assert_eq!(
            (forward.status, forward.offset, forward.height),
            (StitchStatus::Retraced, 300, 432)
        );
        let up = stitch.accept(frame(-40)).unwrap();
        assert_eq!(
            (up.status, up.offset, up.height),
            (StitchStatus::Extended, -40, 472)
        );
        assert_eq!(stitch.finish().unwrap(), expected(-40, 472));
    }

    fn fixed_frame(top: i64) -> RgbaImage {
        RgbaImage::from_fn(128, 128, |x, y| {
            if y < 12 {
                pixel(x, i64::from(y), 9123)
            } else if y >= 118 {
                pixel(x, i64::from(y), 7411)
            } else {
                pixel(x, top + i64::from(y), 42)
            }
        })
    }
    #[test]
    fn fixed_header_and_footer_appear_only_at_the_outer_edges_in_both_directions() {
        let mut stitch = ScrollStitcher::new(fixed_frame(0), limits()).unwrap();
        for top in [20, 40, 15, -20] {
            assert!(matches!(
                stitch.accept(fixed_frame(top)).unwrap().status,
                StitchStatus::Extended | StitchStatus::Retraced
            ));
        }
        let output = stitch.finish().unwrap();
        assert_eq!(output.dimensions(), (128, 188));
        for y in 0..188 {
            for x in 0..128 {
                let expected = if y < 12 {
                    pixel(x, i64::from(y), 9123)
                } else if y >= 178 {
                    pixel(x, i64::from(y - 60), 7411)
                } else {
                    pixel(x, i64::from(y) - 20, 42)
                };
                assert_eq!(*output.get_pixel(x, y), expected, "wrong seam at ({x},{y})");
            }
        }
    }

    #[test]
    fn white_smooth_gradients_and_periodic_texture_never_guess_new_content() {
        for gradient in [false, true] {
            let make = |top: u8| {
                RgbaImage::from_fn(128, 96, |x, y| {
                    if gradient {
                        Rgba([x as u8, y as u8 + top, 0, 255])
                    } else {
                        Rgba([255, 255, 255, 255])
                    }
                })
            };
            let initial = make(0);
            let mut stitch = ScrollStitcher::new(initial.clone(), limits()).unwrap();
            assert_eq!(
                stitch.accept(make(20)).unwrap().status,
                StitchStatus::LowInformation
            );
            assert_eq!(stitch.finish().unwrap(), initial);
        }
        let repeat = |offset: i64| {
            RgbaImage::from_fn(128, 128, |x, y| {
                pixel(x, (i64::from(y) + offset).rem_euclid(16), 42)
            })
        };
        let first = repeat(0);
        let mut stitch = ScrollStitcher::new(first.clone(), limits()).unwrap();
        let state = stitch.accept(repeat(7)).unwrap();
        assert_eq!(state.status, StitchStatus::Ambiguous);
        assert_eq!((state.height, state.offset), (128, 0));
        assert_eq!(stitch.finish().unwrap(), first);
    }

    #[test]
    fn unchanged_or_small_pointer_animation_does_not_append_and_loss_can_recover() {
        let original = frame(0);
        let mut stitch = ScrollStitcher::new(original.clone(), limits()).unwrap();
        assert_eq!(
            stitch.accept(original.clone()).unwrap().status,
            StitchStatus::Unchanged
        );
        let mut pointer = original.clone();
        for y in 43..46 {
            for x in 61..64 {
                pointer.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let state = stitch.accept(pointer).unwrap();
        assert_ne!(state.status, StitchStatus::Extended);
        assert_eq!((state.offset, state.height), (0, 96));
        assert_eq!(
            stitch.accept(frame(1000)).unwrap().status,
            StitchStatus::LostOverlap
        );
        assert_eq!(
            stitch.accept(frame(24)).unwrap().status,
            StitchStatus::Extended
        );
        assert_eq!(stitch.finish().unwrap(), expected(0, 120));
    }

    #[test]
    fn sparse_text_like_document_still_resolves_translation_among_blank_rows() {
        let document = |top: i64| {
            RgbaImage::from_fn(256, 192, |x, y| {
                let gy = top + i64::from(y);
                let row = gy.rem_euclid(28);
                let character = (x / 9) as i64;
                if (16..238).contains(&x)
                    && (6..18).contains(&row)
                    && x % 9 < 6
                    && (character * 17 + gy * 7 + (gy / 28) * 23).rem_euclid(11) < 5
                {
                    Rgba([25, 25, 25, 255])
                } else {
                    Rgba([250, 250, 250, 255])
                }
            })
        };
        let mut stitch = ScrollStitcher::new(document(0), limits()).unwrap();
        let state = stitch.accept(document(53)).unwrap();
        assert_eq!(
            (state.status, state.offset, state.height),
            (StitchStatus::Extended, 53, 245)
        );
        let output = stitch.finish().unwrap();
        let start = document(0);
        let end = document(53);
        for y in 0..245 {
            for x in 0..256 {
                let expected = if y < 192 {
                    start.get_pixel(x, y)
                } else {
                    end.get_pixel(x, y - 53)
                };
                assert_eq!(output.get_pixel(x, y), expected);
            }
        }
    }

    #[test]
    fn size_cancel_and_output_budgets_leave_previously_accepted_pixels_usable() {
        let small = StitchLimits {
            max_frame_pixels: 128 * 96,
            max_output_pixels: 128 * 160,
            max_output_height: 160,
            max_resident_bytes: 4 * 1024 * 1024,
        };
        let mut stitch = ScrollStitcher::new(frame(0), small).unwrap();
        stitch.accept(frame(48)).unwrap();
        let before = stitch.state();
        assert_eq!(
            stitch.accept(RgbaImage::new(129, 96)),
            Err(StitchError::FrameSizeChanged)
        );
        assert_eq!(stitch.state(), before);
        assert_eq!(
            stitch.accept_with_cancel(frame(60), &AtomicBool::new(true)),
            Err(StitchError::Canceled)
        );
        assert_eq!(stitch.state(), before);
        let full = stitch.accept(frame(96)).unwrap();
        assert_eq!(
            (full.status, full.offset, full.height),
            (StitchStatus::LimitReached, 48, 144)
        );
        let back = stitch.accept(frame(16)).unwrap();
        assert_eq!(
            (back.status, back.offset, back.height),
            (StitchStatus::Retraced, 16, 144)
        );
        assert_eq!(stitch.finish().unwrap(), expected(0, 144));
        assert!(matches!(
            ScrollStitcher::new(
                frame(0),
                StitchLimits {
                    max_resident_bytes: 1,
                    ..small
                }
            ),
            Err(StitchError::MemoryLimit)
        ));
        assert!(matches!(
            ScrollStitcher::new(
                frame(0),
                StitchLimits {
                    max_output_pixels: 1,
                    ..small
                }
            ),
            Err(StitchError::InvalidLimits)
        ));
        assert!(matches!(
            ScrollStitcher::new(RgbaImage::new(15, 96), small),
            Err(StitchError::InvalidFrame)
        ));
    }

    /// Explicit CPU-only probe: reports matching separately from synthetic frame
    /// creation, includes a return beyond the last viewport, and verifies every
    /// output pixel. Timings are evidence, not machine-dependent pass criteria.
    #[test]
    #[ignore = "synthetic typical-viewport performance probe; run explicitly"]
    fn typical_viewport_performance_and_far_reconnection() {
        use std::time::Instant;

        for (width, height) in [(1280, 720), (1920, 1080)] {
            let make = |top: i64| {
                RgbaImage::from_fn(width, height, |x, y| pixel(x, top + i64::from(y), 42))
            };
            let initial = make(0);
            let started = Instant::now();
            let mut stitch = ScrollStitcher::new(
                initial,
                StitchLimits {
                    max_output_height: 16384,
                    ..Default::default()
                },
            )
            .unwrap();
            eprintln!(
                "scroll {width}x{height} initialize: {:?}",
                started.elapsed()
            );

            let step = i64::from(height / 3);
            let mut elapsed = Vec::new();
            for top in [step, step * 2, step * 3, step * 4] {
                let next = make(top);
                let started = Instant::now();
                let state = stitch.accept(next).unwrap();
                let duration = started.elapsed();
                elapsed.push(duration);
                assert_eq!((state.status, state.offset), (StitchStatus::Extended, top));
                eprintln!("scroll {width}x{height} extend to {top}: {duration:?}");
            }
            let unchanged = make(step * 4);
            let started = Instant::now();
            assert_eq!(
                stitch.accept(unchanged).unwrap().status,
                StitchStatus::Unchanged
            );
            eprintln!("scroll {width}x{height} unchanged: {:?}", started.elapsed());

            let earlier = make(0);
            let started = Instant::now();
            let state = stitch.accept(earlier).unwrap();
            assert_eq!((state.status, state.offset), (StitchStatus::Retraced, 0));
            eprintln!(
                "scroll {width}x{height} far reconnect: {:?}",
                started.elapsed()
            );
            let bytes: usize = stitch
                .strips
                .iter()
                .map(|strip| strip.pixels.capacity())
                .sum();
            let started = Instant::now();
            let output = stitch.finish().unwrap();
            assert_eq!(output.dimensions(), (width, height + step as u32 * 4));
            eprintln!(
                "scroll {width}x{height} finish: {:?}; retained strip bytes={bytes}; extension max={:?}",
                started.elapsed(),
                elapsed.iter().max().unwrap()
            );
            for (x, y, actual) in output.enumerate_pixels() {
                assert_eq!(
                    *actual,
                    pixel(x, i64::from(y), 42),
                    "wrong output at ({x},{y})"
                );
            }
        }
    }
}
