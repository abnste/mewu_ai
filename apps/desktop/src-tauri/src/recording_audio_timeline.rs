// SPDX-License-Identifier: MPL-2.0
//! Device-independent recording clock and bounded, timestamped PCM mixer.
//! QPC positions from IAudioCaptureClient are already in 100 ns units.

use std::collections::VecDeque;

pub const TICKS_PER_SECOND: u64 = 10_000_000;
pub const SAMPLE_RATE: u32 = 48_000;
pub const MIX_FRAMES: usize = 480;
pub const JITTER_TICKS: u64 = 1_000_000; // 100 ms of capture/MFT scheduling slack.
pub const MAX_PACKET_FRAMES: usize = SAMPLE_RATE as usize / 2;
const MAX_SPANS: usize = 128;
const MAX_QUEUE_FRAMES: usize = SAMPLE_RATE as usize * 2;
const MAX_CORRECTION_FRAMES: u64 = SAMPLE_RATE as u64 / 50; // 20 ms; never conceal a long gap.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimingError {
    Invalid,
    Overrun,
}

#[derive(Clone, Debug)]
struct Span {
    start: u64,
    end: Option<u64>,
    active: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketPart {
    pub first: usize,
    pub end: usize,
    pub active_ticks: u64,
    pub segment: u64,
}

#[derive(Debug)]
pub struct Timeline {
    spans: VecDeque<Span>,
    elapsed: u64,
    last_qpc: u64,
    paused: bool,
}

impl Timeline {
    pub fn new(epoch: u64, paused: bool) -> Self {
        let mut spans = VecDeque::new();
        if !paused {
            spans.push_back(Span {
                start: epoch,
                end: None,
                active: 0,
            });
        }
        Self {
            spans,
            elapsed: 0,
            last_qpc: epoch,
            paused,
        }
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn elapsed(&self, now: u64) -> Result<u64, TimingError> {
        if now < self.last_qpc {
            return Err(TimingError::Invalid);
        }
        Ok(if self.paused {
            self.elapsed
        } else {
            let span = self.spans.back().ok_or(TimingError::Invalid)?;
            span.active
                .checked_add(now - span.start)
                .ok_or(TimingError::Invalid)?
        })
    }

    pub fn set_paused(&mut self, paused: bool, now: u64) -> Result<(), TimingError> {
        let elapsed = self.elapsed(now)?;
        if paused == self.paused {
            self.last_qpc = now;
            return Ok(());
        }
        if paused {
            self.spans.back_mut().ok_or(TimingError::Invalid)?.end = Some(now);
        } else {
            if self.spans.len() >= MAX_SPANS {
                return Err(TimingError::Overrun);
            }
            self.spans.push_back(Span {
                start: now,
                end: None,
                active: elapsed,
            });
        }
        self.elapsed = elapsed;
        self.last_qpc = now;
        self.paused = paused;
        Ok(())
    }

    /// Discard only intervals older than every accepted packet's bounded queue.
    /// Retain the last span so an open interval is never lost.
    pub fn prune_before(&mut self, qpc: u64) {
        while self.spans.len() > 1
            && self
                .spans
                .front()
                .and_then(|s| s.end)
                .is_some_and(|end| end <= qpc)
        {
            self.spans.pop_front();
        }
    }

    pub fn parts(
        &self,
        qpc: u64,
        frames: usize,
        rate: u32,
    ) -> Result<Vec<PacketPart>, TimingError> {
        if rate == 0 || rate > 192_000 || frames > rate as usize / 2 {
            return Err(TimingError::Invalid);
        }
        let end = qpc
            .checked_add(frames_to_ticks(frames as u64, rate))
            .ok_or(TimingError::Invalid)?;
        let mut result = Vec::new();
        for span in &self.spans {
            let stop = span.end.unwrap_or(u64::MAX);
            if end <= span.start || qpc >= stop {
                continue;
            }
            let first =
                ceil_frames(span.start.saturating_sub(qpc), rate).min(frames as u64) as usize;
            let last = if stop == u64::MAX {
                frames
            } else {
                ceil_frames(stop.saturating_sub(qpc), rate).min(frames as u64) as usize
            };
            if first < last {
                let sample_qpc = qpc
                    .checked_add(frames_to_ticks(first as u64, rate))
                    .ok_or(TimingError::Invalid)?;
                result.push(PacketPart {
                    first,
                    end: last,
                    active_ticks: span
                        .active
                        .checked_add(sample_qpc.saturating_sub(span.start))
                        .ok_or(TimingError::Invalid)?,
                    segment: span.start,
                });
            }
        }
        Ok(result)
    }
}

pub fn frames_to_ticks(frames: u64, rate: u32) -> u64 {
    ((u128::from(frames) * u128::from(TICKS_PER_SECOND)) / u128::from(rate.max(1)))
        .min(u64::MAX as u128) as u64
}
pub fn ticks_to_frames(ticks: u64) -> u64 {
    ((u128::from(ticks) * u128::from(SAMPLE_RATE)) / u128::from(TICKS_PER_SECOND))
        .min(u64::MAX as u128) as u64
}
fn ceil_frames(ticks: u64, rate: u32) -> u64 {
    (u128::from(ticks) * u128::from(rate))
        .div_ceil(u128::from(TICKS_PER_SECOND))
        .min(u64::MAX as u128) as u64
}

#[derive(Debug)]
struct Block {
    start: u64,
    samples: Vec<[f32; 2]>,
    consumed: usize,
}

#[derive(Debug, Default)]
struct Track {
    blocks: VecDeque<Block>,
    queued: usize,
    last_end: Option<u64>,
}

#[derive(Debug)]
pub struct Mixer {
    tracks: Vec<Track>,
    position: u64,
}

impl Mixer {
    pub fn new(sources: usize) -> Result<Self, TimingError> {
        if !(1..=2).contains(&sources) {
            return Err(TimingError::Invalid);
        }
        Ok(Self {
            tracks: (0..sources).map(|_| Track::default()).collect(),
            position: 0,
        })
    }
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Each block is placed from an absolute clock anchor. Small clock drift is
    /// absorbed by explicit overlap trimming/zero gaps, never cumulative retime.
    pub fn push(
        &mut self,
        source: usize,
        start: u64,
        samples: Vec<[f32; 2]>,
        discontinuity: bool,
    ) -> Result<(), TimingError> {
        let track = self.tracks.get_mut(source).ok_or(TimingError::Invalid)?;
        if samples.is_empty() {
            return Ok(());
        }
        if samples.len() > MAX_PACKET_FRAMES || samples.iter().flatten().any(|v| !v.is_finite()) {
            return Err(TimingError::Invalid);
        }
        let end = start
            .checked_add(samples.len() as u64)
            .ok_or(TimingError::Invalid)?;
        if start > self.position.saturating_add(MAX_QUEUE_FRAMES as u64) {
            return Err(TimingError::Overrun);
        }
        if let Some(previous) = track.last_end {
            if !discontinuity && start.abs_diff(previous) > MAX_CORRECTION_FRAMES {
                return Err(TimingError::Invalid);
            }
        }
        // Packets wholly behind the already encoded deadline are not silently
        // discarded: that would present missing real audio as successful capture.
        if end.saturating_add(MAX_CORRECTION_FRAMES) < self.position {
            return Err(TimingError::Overrun);
        }
        let accepted_start = start
            .max(track.last_end.unwrap_or(start))
            .max(self.position);
        let skip = accepted_start
            .saturating_sub(start)
            .min(samples.len() as u64) as usize;
        let count = samples.len() - skip;
        if track
            .queued
            .checked_add(count)
            .is_none_or(|n| n > MAX_QUEUE_FRAMES)
        {
            return Err(TimingError::Overrun);
        }
        track.last_end = Some(track.last_end.unwrap_or(0).max(end));
        if count > 0 {
            track.queued += count;
            track.blocks.push_back(Block {
                start,
                samples,
                consumed: skip,
            });
        }
        Ok(())
    }

    /// Missing endpoint packets are silence once the caller's jitter deadline
    /// expires. The caller, not a device event, advances the common output clock.
    pub fn next(&mut self, until_frame: u64) -> Option<Vec<i16>> {
        if self.position >= until_frame {
            return None;
        }
        let count = (until_frame - self.position).min(MIX_FRAMES as u64) as usize;
        let gain = 1.0 / self.tracks.len() as f32;
        let mut output = vec![0i16; count * 2];
        for offset in 0..count {
            let position = self.position + offset as u64;
            let mut mixed = [0f32; 2];
            for track in &mut self.tracks {
                loop {
                    let Some(block) = track.blocks.front_mut() else {
                        break;
                    };
                    let current = block.start + block.consumed as u64;
                    if current > position {
                        break;
                    }
                    if current < position {
                        let skip = (position - current)
                            .min((block.samples.len() - block.consumed) as u64)
                            as usize;
                        block.consumed += skip;
                        track.queued -= skip;
                    }
                    if block.consumed == block.samples.len() {
                        track.blocks.pop_front();
                        continue;
                    }
                    let sample = block.samples[block.consumed];
                    mixed[0] += sample[0] * gain;
                    mixed[1] += sample[1] * gain;
                    block.consumed += 1;
                    track.queued -= 1;
                    if block.consumed == block.samples.len() {
                        track.blocks.pop_front();
                    }
                    break;
                }
            }
            for channel in 0..2 {
                output[offset * 2 + channel] =
                    (mixed[channel].clamp(-1.0, 1.0) * 32767.0).round() as i16;
            }
        }
        self.position += count as u64;
        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packet_crossing_pauses_is_split_and_never_replays_paused_audio() {
        let mut timeline = Timeline::new(100_000_000, false);
        timeline.set_paused(true, 100_100_000).unwrap();
        timeline.set_paused(true, 100_120_000).unwrap();
        timeline.set_paused(false, 100_200_000).unwrap();
        let parts = timeline.parts(100_000_000, 1440, SAMPLE_RATE).unwrap();
        assert_eq!(
            parts,
            vec![
                PacketPart {
                    first: 0,
                    end: 480,
                    active_ticks: 0,
                    segment: 100_000_000
                },
                PacketPart {
                    first: 960,
                    end: 1440,
                    active_ticks: 100_000,
                    segment: 100_200_000
                }
            ]
        );
        assert_eq!(timeline.elapsed(100_300_000).unwrap(), 200_000);
    }
    #[test]
    fn bounded_history_prunes_without_reopening_old_pauses() {
        let mut t = Timeline::new(0, false);
        for n in 1..10_000u64 {
            t.set_paused(true, n * 1000).unwrap();
            t.set_paused(false, n * 1000 + 100).unwrap();
            t.prune_before(n * 1000);
            assert!(t.spans.len() <= 2);
        }
        assert!(t.set_paused(true, 1).is_err());
    }
    #[test]
    fn stereo_mixing_headroom_silence_and_nonfinite_bounds() {
        let mut mixer = Mixer::new(2).unwrap();
        mixer.push(0, 0, vec![[1.0, 0.2]; 480], true).unwrap();
        mixer.push(1, 0, vec![[0.0, 0.8]; 480], true).unwrap();
        assert_eq!(&mixer.next(480).unwrap()[..2], &[16384, 16384]);
        assert!(mixer.next(960).unwrap().iter().all(|v| *v == 0));
        assert!(mixer.push(0, 960, vec![[f32::NAN, 0.0]], true).is_err());
        assert!(mixer.push(0, 200_000, vec![[0.0, 0.0]; 10], true).is_err());
    }
    #[test]
    fn absolute_anchors_bound_drift_over_ten_minutes() {
        for ppm in [-500i64, 500] {
            let mut mixer = Mixer::new(1).unwrap();
            let mut last = 0;
            for packet in 0..60_000u64 {
                let start = ((packet as i128 * 480 * (1_000_000 + ppm) as i128) / 1_000_000) as u64;
                mixer
                    .push(0, start, vec![[0.1; 2]; 480], packet == 0)
                    .unwrap();
                let until = start + 480;
                while mixer.next(until).is_some() {}
                last = until;
            }
            assert_eq!(mixer.position(), last);
            assert!(mixer.tracks[0].queued < 480);
        }
    }
    #[test]
    fn packets_arriving_after_deadline_fail_instead_of_silently_losing_audio() {
        let mut mixer = Mixer::new(1).unwrap();
        while mixer.next(4800).is_some() {}
        assert_eq!(
            mixer.push(0, 0, vec![[0.2; 2]; 480], true),
            Err(TimingError::Overrun)
        );
    }
}
