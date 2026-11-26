//! When the agent tries to open a device again (DP-USB-02).
//!
//! The rescan looks for new devices every [`RetryPolicy::initial`] (2 s).
//! A device that is listed but cannot be opened, or that opens and then
//! fails within [`RetryPolicy::stable_after`], is not tried again on every
//! rescan: each failure in a row doubles the wait, up to
//! [`RetryPolicy::max`], with a jitter of ±20% so several devices (or
//! agents) do not retry in step. A session that stays healthy for
//! `stable_after` ends the streak.
//!
//! The flap guard: a device whose sessions keep ending soon after they
//! opened ([`RetryPolicy::flap_threshold`] times within
//! [`RetryPolicy::flap_window`]) is *flapping*. Besides the longer waits,
//! a flapping device gets its brightness and key images only once its
//! session has lasted [`RetryPolicy::settle`]; a session that dies sooner
//! sent nothing, so a marginal USB link (a monitor hub that keeps
//! resetting) is not hit with a full redraw on every cycle.
//!
//! The policy only decides when to try; it never resets, re-binds or
//! power-cycles a device, and a reconnect restores presentation state
//! only (page, layout, key images, brightness), never actions.
//!
//! Pure: times come from the caller (the agent's monotonic clock, which
//! tests replace), so the schedule is tested without waiting.

use std::collections::{BTreeMap, VecDeque};
use std::hash::{BuildHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use deckpilot_device::DeviceId;

/// Timing of retries and the flap guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Wait after the first failure (before jitter).
    pub initial: Duration,
    /// Longest wait (also after jitter).
    pub max: Duration,
    /// A session open this long ends a failure streak.
    pub stable_after: Duration,
    /// Short sessions within this window count towards flapping; a
    /// device with no failure for this long starts afresh.
    pub flap_window: Duration,
    /// This many short sessions within `flap_window`: flapping.
    pub flap_threshold: usize,
    /// How long a flapping device's session must last before its keys
    /// and brightness are restored.
    pub settle: Duration,
}

impl RetryPolicy {
    /// The policy for a rescan every `interval`, in multiples of it: for
    /// the agent's 2 s, retries after 2, 4, 8, 16, 32 and then every 60 s;
    /// stable after 30 s; flapping after 3 short sessions in 5 minutes;
    /// a flapping device restored after 10 s. Tests with a faster rescan
    /// get the same schedule, scaled.
    pub fn for_rescan_interval(interval: Duration) -> Self {
        Self {
            initial: interval,
            max: interval * 30,
            stable_after: interval * 15,
            flap_window: interval * 150,
            flap_threshold: 3,
            settle: interval * 5,
        }
    }

    /// The wait after `failures` failures in a row (at least 1), before
    /// jitter.
    pub fn base_delay(&self, failures: u32) -> Duration {
        let doublings = failures.saturating_sub(1).min(31);
        self.initial
            .checked_mul(1 << doublings)
            .map_or(self.max, |delay| delay.min(self.max))
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::for_rescan_interval(Duration::from_secs(2))
    }
}

/// What ended a session, as the retry state sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    /// It had lasted [`RetryPolicy::stable_after`]: the next open may
    /// come at once.
    Stable,
    /// It ended sooner: the next open waits `delay`. `flapping_started`:
    /// this end made the device flapping (it was not before).
    Short {
        delay: Duration,
        flapping_started: bool,
    },
}

#[derive(Debug, Default)]
struct Entry {
    /// Failures in a row: failed opens and short sessions.
    failures: u32,
    /// No open before this.
    not_before: Option<Instant>,
    last_failure: Option<Instant>,
    /// When the current session opened.
    opened_at: Option<Instant>,
    /// Ends of recent short sessions, oldest first.
    short_sessions: VecDeque<Instant>,
    /// The next open ignores `not_before` (a manual Refresh).
    bypass: bool,
}

/// The retry state of every device, by identity (model and serial, so a
/// device that comes back under another path keeps its state).
#[derive(Debug)]
pub struct DeviceRetry {
    policy: RetryPolicy,
    /// Varies the jitter between agent runs.
    seed: u64,
    entries: BTreeMap<DeviceId, Entry>,
}

impl DeviceRetry {
    pub fn new(policy: RetryPolicy) -> Self {
        Self::with_seed(
            policy,
            std::collections::hash_map::RandomState::new().hash_one(0u8),
        )
    }

    /// With a fixed jitter seed (tests).
    pub fn with_seed(policy: RetryPolicy, seed: u64) -> Self {
        Self {
            policy,
            seed,
            entries: BTreeMap::new(),
        }
    }

    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }

    /// Whether `device` may be opened at `now`. Clears a streak whose last
    /// failure is older than [`RetryPolicy::flap_window`].
    pub fn may_open(&mut self, device: &DeviceId, now: Instant) -> bool {
        let Some(entry) = self.entries.get_mut(device) else {
            return true;
        };
        if entry.opened_at.is_none()
            && entry
                .last_failure
                .is_some_and(|at| now.saturating_duration_since(at) >= self.policy.flap_window)
        {
            self.entries.remove(device);
            return true;
        }
        if entry.bypass {
            return true;
        }
        entry.not_before.is_none_or(|at| now >= at)
    }

    /// When `device` may next be opened, while it waits after a failure.
    #[cfg(test)]
    pub fn next_attempt(&self, device: &DeviceId) -> Option<Instant> {
        self.entries
            .get(device)
            .filter(|entry| entry.opened_at.is_none() && !entry.bypass)
            .and_then(|entry| entry.not_before)
    }

    /// An open of `device` failed at `now`. Returns the wait before the
    /// next one.
    pub fn open_failed(&mut self, device: &DeviceId, now: Instant) -> Duration {
        let delay = {
            let entry = self.entries.entry(device.clone()).or_default();
            entry.failures = entry.failures.saturating_add(1);
            entry.opened_at = None;
            entry.bypass = false;
            entry.last_failure = Some(now);
            entry.failures
        };
        let delay = self.jittered(device, delay);
        if let Some(entry) = self.entries.get_mut(device) {
            entry.not_before = Some(now + delay);
        }
        delay
    }

    /// `device` opened at `now`. Returns whether it is flapping: its keys
    /// and brightness then wait for [`RetryPolicy::settle`].
    pub fn opened(&mut self, device: &DeviceId, now: Instant) -> bool {
        let policy = self.policy;
        let Some(entry) = self.entries.get_mut(device) else {
            self.entries.insert(
                device.clone(),
                Entry {
                    opened_at: Some(now),
                    ..Entry::default()
                },
            );
            return false;
        };
        entry.opened_at = Some(now);
        entry.bypass = false;
        entry.not_before = None;
        prune(&mut entry.short_sessions, now, policy.flap_window);
        entry.short_sessions.len() >= policy.flap_threshold
    }

    /// Whether `device`'s session, open since [`DeviceRetry::opened`], has
    /// lasted [`RetryPolicy::stable_after`] at `now`. The first time it
    /// has, the streak ends; returns `true` then if there was one (worth a
    /// log line).
    pub fn check_stable(&mut self, device: &DeviceId, now: Instant) -> bool {
        let Some(entry) = self.entries.get(device) else {
            return false;
        };
        let Some(opened_at) = entry.opened_at else {
            return false;
        };
        if now.saturating_duration_since(opened_at) < self.policy.stable_after {
            return false;
        }
        let had_streak = entry.failures > 0 || !entry.short_sessions.is_empty();
        self.entries.remove(device);
        had_streak
    }

    /// `device`'s session ended at `now`, for any reason (an I/O error, or
    /// the device is no longer listed).
    pub fn closed(&mut self, device: &DeviceId, now: Instant) -> SessionEnd {
        let policy = self.policy;
        let Some(entry) = self.entries.get_mut(device) else {
            // No entry: the session was already stable.
            return SessionEnd::Stable;
        };
        let Some(opened_at) = entry.opened_at.take() else {
            return SessionEnd::Stable;
        };
        if now.saturating_duration_since(opened_at) >= policy.stable_after {
            self.entries.remove(device);
            return SessionEnd::Stable;
        }
        prune(&mut entry.short_sessions, now, policy.flap_window);
        let was_flapping = entry.short_sessions.len() >= policy.flap_threshold;
        entry.short_sessions.push_back(now);
        let flapping_started = !was_flapping && entry.short_sessions.len() >= policy.flap_threshold;
        entry.failures = entry.failures.saturating_add(1);
        entry.last_failure = Some(now);
        let failures = entry.failures;
        let delay = self.jittered(device, failures);
        if let Some(entry) = self.entries.get_mut(device) {
            entry.not_before = Some(now + delay);
        }
        SessionEnd::Short {
            delay,
            flapping_started,
        }
    }

    /// The next open of every waiting device comes at once (a manual
    /// Refresh Devices). The streaks stay: a failure after it waits
    /// longer still.
    pub fn bypass_all(&mut self) {
        for entry in self.entries.values_mut() {
            if entry.opened_at.is_none() {
                entry.bypass = true;
            }
        }
    }

    /// Whether `device` is flapping at `now`.
    #[cfg(test)]
    pub fn is_flapping(&self, device: &DeviceId, now: Instant) -> bool {
        self.entries.get(device).is_some_and(|entry| {
            entry
                .short_sessions
                .iter()
                .filter(|at| now.saturating_duration_since(**at) < self.policy.flap_window)
                .count()
                >= self.policy.flap_threshold
        })
    }

    /// The wait after `failures` failures of `device`: the base delay
    /// ±20%, never above [`RetryPolicy::max`].
    fn jittered(&self, device: &DeviceId, failures: u32) -> Duration {
        let base = self.policy.base_delay(failures);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.seed.hash(&mut hasher);
        device.hash(&mut hasher);
        failures.hash(&mut hasher);
        // A factor in [0.8, 1.2].
        #[expect(
            clippy::cast_precision_loss,
            reason = "a jitter fraction needs no precision"
        )]
        let unit = (hasher.finish() % 10_001) as f64 / 10_000.0;
        base.mul_f64(0.8 + 0.4 * unit).min(self.policy.max)
    }
}

fn prune(short_sessions: &mut VecDeque<Instant>, now: Instant, window: Duration) {
    while short_sessions
        .front()
        .is_some_and(|at| now.saturating_duration_since(*at) >= window)
    {
        short_sessions.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_device::DeviceModelId;

    fn device(serial: &str) -> DeviceId {
        DeviceId {
            model: DeviceModelId::new("elgato.stream-deck-mk2"),
            serial: serial.into(),
        }
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn within_jitter(delay: Duration, base: Duration, max: Duration) -> bool {
        delay >= base.mul_f64(0.8) && delay <= base.mul_f64(1.2).min(max)
    }

    #[test]
    fn the_default_schedule_doubles_from_2_to_60_seconds() {
        let policy = RetryPolicy::default();
        let bases: Vec<u64> = (1..=8).map(|n| policy.base_delay(n).as_secs()).collect();
        assert_eq!(bases, vec![2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(policy.base_delay(u32::MAX), secs(60));
        assert_eq!(policy.stable_after, secs(30));
        assert_eq!(policy.settle, secs(10));
    }

    #[test]
    fn failed_opens_back_off_with_bounded_jitter() {
        let policy = RetryPolicy::default();
        let mut retry = DeviceRetry::with_seed(policy, 7);
        let a = device("A");
        let start = Instant::now();
        let mut now = start;
        assert!(retry.may_open(&a, now));
        for failures in 1..=10 {
            let delay = retry.open_failed(&a, now);
            assert!(
                within_jitter(delay, policy.base_delay(failures), policy.max),
                "failure {failures}: {delay:?}"
            );
            assert!(delay <= policy.max);
            assert!(!retry.may_open(&a, now));
            assert!(!retry.may_open(&a, now + delay.saturating_sub(Duration::from_millis(1))));
            assert_eq!(retry.next_attempt(&a), Some(now + delay));
            now += delay;
            assert!(retry.may_open(&a, now));
        }
        // Ten failures take at least the sum of the lower bounds: bounded
        // opens over time.
        assert!(now.duration_since(start) >= secs(2 + 4 + 8 + 16 + 32 + 60 * 5).mul_f64(0.8));
    }

    #[test]
    fn jitter_differs_between_devices() {
        let policy = RetryPolicy::default();
        let mut retry = DeviceRetry::with_seed(policy, 1);
        let now = Instant::now();
        let delays: std::collections::BTreeSet<Duration> = (0..8)
            .map(|n| retry.open_failed(&device(&format!("S{n}")), now))
            .collect();
        assert!(delays.len() > 1, "devices retry out of step");
    }

    #[test]
    fn a_stable_session_ends_the_streak() {
        let policy = RetryPolicy::default();
        let mut retry = DeviceRetry::with_seed(policy, 3);
        let a = device("A");
        let mut now = Instant::now();
        for _ in 0..4 {
            now += retry.open_failed(&a, now);
        }
        assert!(!retry.opened(&a, now));
        assert!(!retry.check_stable(&a, now + secs(29)));
        assert!(
            retry.check_stable(&a, now + secs(30)),
            "the streak is told once"
        );
        assert!(!retry.check_stable(&a, now + secs(31)));
        assert_eq!(retry.closed(&a, now + secs(40)), SessionEnd::Stable);
        assert!(retry.may_open(&a, now + secs(40)));
        // A failure now starts from 2 s again.
        let delay = retry.open_failed(&a, now + secs(40));
        assert!(within_jitter(delay, secs(2), policy.max), "{delay:?}");
    }

    #[test]
    fn a_long_session_that_ends_is_stable_even_unchecked() {
        let mut retry = DeviceRetry::with_seed(RetryPolicy::default(), 3);
        let a = device("A");
        let now = Instant::now();
        retry.open_failed(&a, now);
        retry.opened(&a, now + secs(3));
        assert_eq!(retry.closed(&a, now + secs(60)), SessionEnd::Stable);
        assert!(retry.may_open(&a, now + secs(60)));
    }

    #[test]
    fn short_sessions_back_off_and_then_flap() {
        let policy = RetryPolicy::default();
        let mut retry = DeviceRetry::with_seed(policy, 5);
        let a = device("A");
        let mut now = Instant::now();
        let mut flapping_told = 0;
        for cycle in 1..=6u32 {
            let flapping = retry.opened(&a, now);
            assert_eq!(flapping, cycle > 3, "cycle {cycle}");
            now += secs(1);
            match retry.closed(&a, now) {
                SessionEnd::Short {
                    delay,
                    flapping_started,
                } => {
                    assert!(within_jitter(delay, policy.base_delay(cycle), policy.max));
                    if flapping_started {
                        flapping_told += 1;
                        assert_eq!(cycle, 3);
                    }
                    assert!(!retry.may_open(&a, now));
                    now += delay;
                }
                SessionEnd::Stable => panic!("cycle {cycle} was short"),
            }
        }
        assert_eq!(flapping_told, 1, "flapping is told once");
        assert!(retry.is_flapping(&a, now));
        // A session that lasts ends it.
        retry.opened(&a, now);
        assert!(retry.check_stable(&a, now + policy.stable_after));
        assert!(!retry.is_flapping(&a, now + policy.stable_after));
    }

    #[test]
    fn old_short_sessions_stop_counting() {
        let policy = RetryPolicy::default();
        let mut retry = DeviceRetry::with_seed(policy, 5);
        let a = device("A");
        let mut now = Instant::now();
        for _ in 0..2 {
            retry.opened(&a, now);
            now += secs(1);
            let SessionEnd::Short { delay, .. } = retry.closed(&a, now) else {
                panic!("short");
            };
            now += delay;
        }
        // Long after: a streak this old is forgotten.
        now += policy.flap_window;
        assert!(retry.may_open(&a, now));
        assert!(!retry.opened(&a, now));
        now += secs(1);
        let SessionEnd::Short {
            delay,
            flapping_started,
        } = retry.closed(&a, now)
        else {
            panic!("short");
        };
        assert!(!flapping_started);
        assert!(within_jitter(delay, secs(2), policy.max), "{delay:?}");
    }

    #[test]
    fn a_manual_refresh_bypasses_the_wait_once() {
        let policy = RetryPolicy::default();
        let mut retry = DeviceRetry::with_seed(policy, 9);
        let a = device("A");
        let now = Instant::now();
        for _ in 0..5 {
            retry.open_failed(&a, now);
        }
        assert!(!retry.may_open(&a, now));
        retry.bypass_all();
        assert!(retry.may_open(&a, now));
        assert_eq!(retry.next_attempt(&a), None);
        // It failed again: the streak continued (6th failure: 60 s).
        let delay = retry.open_failed(&a, now);
        assert!(within_jitter(delay, secs(60), policy.max), "{delay:?}");
        assert!(!retry.may_open(&a, now));
    }

    #[test]
    fn devices_have_their_own_state() {
        let mut retry = DeviceRetry::with_seed(RetryPolicy::default(), 2);
        let (a, b) = (device("A"), device("B"));
        let now = Instant::now();
        retry.open_failed(&a, now);
        assert!(!retry.may_open(&a, now));
        assert!(retry.may_open(&b, now));
        assert!(!retry.opened(&b, now));
        assert_eq!(retry.closed(&b, now + secs(31)), SessionEnd::Stable);
        assert!(!retry.may_open(&a, now));
    }

    #[test]
    fn the_scaled_policy_keeps_the_ratios() {
        let policy = RetryPolicy::for_rescan_interval(Duration::from_millis(50));
        assert_eq!(policy.max, Duration::from_millis(1500));
        assert_eq!(policy.stable_after, Duration::from_millis(750));
        assert_eq!(policy.settle, Duration::from_millis(250));
    }
}
