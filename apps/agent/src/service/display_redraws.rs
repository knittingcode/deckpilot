//! Redraws for variable changes, coalesced per device (`docs/key-text.md`,
//! "Labels"; design §5).
//!
//! A variable that a shown title or label depends on may change many times
//! a second (a plugin publishing a value, a work session's timer). Such a
//! change does not redraw at once: the first one schedules the device's
//! redraw [`DISPLAY_REDRAW_DELAY`] later, further changes until then ride
//! along. A new change is scheduled only once the pending redraw is done,
//! so a device is redrawn for variables at most once per
//! [`DISPLAY_REDRAW_DELAY`] (20 a second), and a change shows within about
//! 50 ms. Input, actions, profile, layout and page changes
//! still redraw at once (`Agent::render` directly); such a redraw also
//! takes care of a pending variable redraw.
//!
//! The service task never waits for a redraw: the run loop sleeps until
//! [`DisplayRedraws::next_due`] next to its other timers, so a key press
//! that arrives during a burst of changes is handled first.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use deckpilot_device::DeviceId;

/// How long after the first change a device is redrawn; also the shortest
/// time between two redraws of a device for variables (at most 20 a
/// second).
pub const DISPLAY_REDRAW_DELAY: Duration = Duration::from_millis(50);

/// Pending variable redraws, by device. Times are passed in, so tests
/// choose the clock.
#[derive(Debug, Default)]
pub struct DisplayRedraws {
    pending: HashMap<DeviceId, Instant>,
}

impl DisplayRedraws {
    /// A variable `device` shows changed at `now`. Returns whether this
    /// scheduled a redraw (`false`: one was pending; this change rides
    /// along).
    pub fn schedule(&mut self, device: &DeviceId, now: Instant) -> bool {
        // A pending redraw takes the change along, so a device is redrawn
        // for variables at most once per DISPLAY_REDRAW_DELAY.
        if self.pending.contains_key(device) {
            return false;
        }
        self.pending
            .insert(device.clone(), now + DISPLAY_REDRAW_DELAY);
        true
    }

    /// When the next pending redraw is due.
    pub fn next_due(&self) -> Option<Instant> {
        self.pending.values().min().copied()
    }

    /// The devices whose redraw is due at `now` (no longer pending), in a
    /// stable order.
    pub fn take_due(&mut self, now: Instant) -> Vec<DeviceId> {
        let mut due: Vec<DeviceId> = self
            .pending
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(device, _)| device.clone())
            .collect();
        due.sort();
        for device in &due {
            self.pending.remove(device);
        }
        due
    }

    /// `device` was redrawn (for any reason): a pending variable redraw is
    /// done with.
    pub fn rendered(&mut self, device: &DeviceId) {
        self.pending.remove(device);
    }

    pub fn forget(&mut self, device: &DeviceId) {
        self.pending.remove(device);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_device::{DeviceModelId, models};

    fn device(serial: &str) -> DeviceId {
        DeviceId {
            model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
            serial: serial.into(),
        }
    }

    /// Runs `changes` (offsets from the start) through the redraws, the
    /// way the run loop does, and returns when each redraw happened.
    fn simulate(changes: &[Duration], until: Duration) -> Vec<Duration> {
        let start = Instant::now();
        let deck = device("A");
        let mut redraws = DisplayRedraws::default();
        let mut drawn = Vec::new();
        let mut changes = changes.iter().peekable();
        let mut now = Duration::ZERO;
        while now <= until {
            while changes.peek().is_some_and(|at| **at <= now) {
                changes.next();
                redraws.schedule(&deck, start + now);
            }
            for due in redraws.take_due(start + now) {
                redraws.rendered(&due);
                drawn.push(now);
            }
            now += Duration::from_millis(1);
        }
        drawn
    }

    #[test]
    fn a_burst_of_changes_is_one_redraw() {
        // Ten changes within 50 ms: one redraw, 50 ms after the first.
        let burst: Vec<Duration> = (0..10).map(|i| Duration::from_millis(i * 5)).collect();
        assert_eq!(
            simulate(&burst, Duration::from_secs(1)),
            [Duration::from_millis(50)]
        );
    }

    #[test]
    fn steady_changes_redraw_at_most_twenty_times_a_second() {
        // A change every millisecond for a second.
        let steady: Vec<Duration> = (0..1000).map(Duration::from_millis).collect();
        let drawn = simulate(&steady, Duration::from_millis(999));
        assert!(drawn.len() <= 20, "{}", drawn.len());
        assert!(drawn.len() >= 19, "{}", drawn.len());
        // The last change shows within about 100 ms.
        let all = simulate(&steady, Duration::from_secs(2));
        assert!(*all.last().unwrap() <= Duration::from_millis(1100));
    }

    #[test]
    fn devices_are_independent_and_an_immediate_redraw_takes_over() {
        let now = Instant::now();
        let (a, b) = (device("A"), device("B"));
        let mut redraws = DisplayRedraws::default();
        assert!(redraws.schedule(&a, now));
        assert!(!redraws.schedule(&a, now + Duration::from_millis(1)));
        assert!(redraws.schedule(&b, now + Duration::from_millis(20)));
        assert_eq!(redraws.next_due(), Some(now + DISPLAY_REDRAW_DELAY));
        // A press redraws A at once: nothing left to do for it.
        redraws.rendered(&a);
        assert_eq!(redraws.take_due(now + Duration::from_millis(60)), []);
        assert_eq!(
            redraws.take_due(now + Duration::from_millis(70)),
            std::slice::from_ref(&b)
        );
        // A change right after a redraw waits for the interval.
        assert!(redraws.schedule(&a, now + Duration::from_millis(11)));
        assert_eq!(redraws.next_due(), Some(now + Duration::from_millis(61)));
        redraws.forget(&a);
        assert_eq!(redraws.next_due(), None);
    }
}
