//! Confirmation of actions that lose progress.

use std::time::{Duration, Instant};

/// Repeats closer than this are one gesture, such as dropping two files.
const MIN_GAP: Duration = Duration::from_millis(500);
/// A later repeat no longer answers the warning.
const MAX_GAP: Duration = Duration::from_secs(10);

/// Action that would drop the running game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Close,
    Drop,
}

/// The last warning shown, if it can still be answered.
#[derive(Debug, Default)]
pub(super) struct Discard {
    warned: Option<(Action, Instant)>,
}

impl Discard {
    /// Returns whether `action` repeats a warned one in time; otherwise warns about it.
    pub(super) fn confirm(&mut self, action: Action, now: Instant) -> bool {
        if let Some((warned, at)) = self.warned {
            let gap = now.saturating_duration_since(at);
            if warned == action && gap < MIN_GAP {
                return false;
            }
            if warned == action && gap <= MAX_GAP {
                self.warned = None;
                return true;
            }
        }
        self.warned = Some((action, now));
        false
    }

    /// Forgets the warning once progress is safe.
    pub(super) fn reset(&mut self) {
        self.warned = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_requires_same_action_in_time() {
        for (second, gap_ms, expected) in [
            (Action::Close, 499, false),
            (Action::Close, 500, true),
            (Action::Close, 10_000, true),
            (Action::Close, 10_001, false),
            (Action::Drop, 1_000, false),
        ] {
            let start = Instant::now();
            let mut discard = Discard::default();
            assert!(!discard.confirm(Action::Close, start));
            let now = start + Duration::from_millis(gap_ms);
            assert_eq!(
                discard.confirm(second, now),
                expected,
                "{second:?} {gap_ms}"
            );
        }
    }

    #[test]
    fn confirm_after_reset_warns_again() {
        let start = Instant::now();
        let mut discard = Discard::default();
        discard.confirm(Action::Close, start);
        discard.reset();
        assert!(!discard.confirm(Action::Close, start + Duration::from_secs(1)));
    }

    #[test]
    fn confirm_is_consumed() {
        let start = Instant::now();
        let mut discard = Discard::default();
        discard.confirm(Action::Drop, start);
        assert!(discard.confirm(Action::Drop, start + Duration::from_secs(1)));
        assert!(!discard.confirm(Action::Drop, start + Duration::from_secs(2)));
    }
}
