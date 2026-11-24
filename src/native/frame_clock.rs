//! Frame pacing at the DMG refresh rate.

use std::time::{Duration, Instant};

/// 70 224 T-cycles at 4 194 304 Hz.
const FRAME_PERIOD: Duration = Duration::from_nanos(16_742_706);
/// Lag beyond which time is dropped instead of replayed as a burst of frames.
const MAX_LAG: Duration = Duration::from_millis(100);

/// Schedules frames against absolute deadlines, so wake-up jitter does not accumulate.
pub(super) struct FrameClock {
    next_frame: Instant,
}

impl FrameClock {
    pub(super) fn new(now: Instant) -> Self {
        Self { next_frame: now }
    }

    /// Returns whether a frame is due, scheduling the following one if so.
    pub(super) fn frame_due(&mut self, now: Instant) -> bool {
        if now < self.next_frame {
            return false;
        }
        if now.duration_since(self.next_frame) > MAX_LAG {
            self.next_frame = now;
        }
        self.next_frame += FRAME_PERIOD;
        true
    }

    pub(super) fn next_frame(&self) -> Instant {
        self.next_frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_due_fires_once_per_period() {
        let start = Instant::now();
        let mut clock = FrameClock::new(start);
        assert!(clock.frame_due(start));
        assert!(!clock.frame_due(start + FRAME_PERIOD - Duration::from_nanos(1)));
        assert!(clock.frame_due(start + FRAME_PERIOD));
    }

    #[test]
    fn frame_due_late_wakeup_does_not_shift_deadlines() {
        let start = Instant::now();
        let mut clock = FrameClock::new(start);
        assert!(clock.frame_due(start + FRAME_PERIOD / 2));
        assert_eq!(clock.next_frame(), start + FRAME_PERIOD);
    }

    #[test]
    fn frame_due_long_stall_drops_time() {
        let start = Instant::now();
        let resumed = start + MAX_LAG + Duration::from_nanos(1);
        let mut clock = FrameClock::new(start);
        assert!(clock.frame_due(resumed));
        assert_eq!(clock.next_frame(), resumed + FRAME_PERIOD);
    }
}
