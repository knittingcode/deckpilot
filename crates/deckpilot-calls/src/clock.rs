//! The engine's monotonic clock: milliseconds since an arbitrary origin.
//!
//! Not wall time and never persisted: the agent maps its own monotonic
//! clock to it, and tests advance it by hand. Every timeout of the Calls
//! engine is computed on it, so a test can step a 60 s timeout without
//! waiting.

use std::fmt;
use std::time::Duration;

/// A point on the engine's monotonic clock, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Instant(u64);

impl Instant {
    /// The clock's origin.
    pub const ZERO: Self = Self(0);

    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    pub const fn from_secs(secs: u64) -> Self {
        Self(secs.saturating_mul(1000))
    }

    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// `self + duration`, saturating at the end of the clock.
    #[must_use]
    pub fn after(self, duration: Duration) -> Self {
        let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        Self(self.0.saturating_add(millis))
    }

    /// The time from `earlier` to `self`, zero when `earlier` is later.
    pub fn since(self, earlier: Self) -> Duration {
        Duration::from_millis(self.0.saturating_sub(earlier.0))
    }
}

impl fmt::Display for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t+{}ms", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_saturates() {
        let t = Instant::from_secs(2);
        assert_eq!(t.as_millis(), 2000);
        assert_eq!(t.after(Duration::from_millis(500)).as_millis(), 2500);
        assert_eq!(t.since(Instant::from_secs(1)), Duration::from_secs(1));
        assert_eq!(Instant::ZERO.since(t), Duration::ZERO);
        assert_eq!(
            Instant::from_millis(u64::MAX).after(Duration::from_secs(1)),
            Instant::from_millis(u64::MAX)
        );
    }
}
