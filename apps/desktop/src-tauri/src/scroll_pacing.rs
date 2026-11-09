// SPDX-License-Identifier: MPL-2.0
//! Bounded pacing based on measured work. Never discard stitch input frames.
use std::time::Duration;

#[derive(Default)]
pub(crate) struct ScrollPacing { work_seconds: Option<f64> }

impl ScrollPacing {
    pub(crate) fn delay(&mut self, elapsed: Duration) -> Duration {
        let current = elapsed.as_secs_f64();
        let average = self.work_seconds.map_or(current, |old| old * 0.75 + current * 0.25);
        self.work_seconds = Some(average);
        // 80 ms is the existing minimum interval, now including processing.
        // Keep headroom under load instead of adding a fixed sleep to every frame.
        let period = (average.max(current) * 1.25).clamp(0.080, 0.500);
        Duration::from_secs_f64((period - current).max(0.).min(0.120))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn includes_capture_stitch_and_preview_cost_in_frame_interval() {
        let mut pacing = ScrollPacing::default();
        let delay = pacing.delay(Duration::from_millis(30));
        assert!((delay.as_secs_f64() - 0.050).abs() < 0.000001);
        let delay = pacing.delay(Duration::from_millis(200));
        assert!((delay.as_secs_f64() - 0.050).abs() < 0.000001);
    }
    #[test]
    fn recovers_gradually_after_load_and_never_adds_unbounded_wait() {
        let mut pacing = ScrollPacing::default();
        assert_eq!(pacing.delay(Duration::from_secs(5)), Duration::ZERO);
        for _ in 0..100 {
            let delay = pacing.delay(Duration::from_millis(1));
            assert!(delay <= Duration::from_millis(120));
        }
        assert!((pacing.delay(Duration::from_millis(1)).as_secs_f64() - 0.079).abs() < 0.000001);
    }
}
