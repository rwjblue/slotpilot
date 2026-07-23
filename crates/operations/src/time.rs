//! UTC/monotonic mapping and deterministic virtual time.

use std::sync::{Arc, Mutex};

use slotpilot_domain::{OperatingMode, UtcSlot};
use thiserror::Error;

/// UTC milliseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct UtcInstant(i64);

impl UtcInstant {
    /// Constructs a non-negative UTC instant.
    pub fn from_unix_millis(value: i64) -> Result<Self, SlotTimeError> {
        if value < 0 {
            return Err(SlotTimeError::BeforeUnixEpoch);
        }
        Ok(Self(value))
    }

    /// Returns milliseconds since the Unix epoch.
    #[must_use]
    pub const fn unix_millis(self) -> i64 {
        self.0
    }
}

/// Monotonic milliseconds from a process-local origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MonotonicInstant(u64);

impl MonotonicInstant {
    /// Constructs a process-local monotonic instant.
    #[must_use]
    pub const fn from_millis(value: u64) -> Self {
        Self(value)
    }

    /// Returns milliseconds from the process-local origin.
    #[must_use]
    pub const fn millis(self) -> u64 {
        self.0
    }
}

/// A future scheduling value expressed only on the monotonic timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MonotonicDeadline(MonotonicInstant);

impl MonotonicDeadline {
    /// Returns the monotonic instant at which the deadline occurs.
    #[must_use]
    pub const fn instant(self) -> MonotonicInstant {
        self.0
    }

    /// Returns whether this deadline has expired at the supplied sample.
    #[must_use]
    pub const fn is_expired_at(self, now: MonotonicInstant) -> bool {
        now.0 >= self.0.0
    }
}

/// A simultaneous observation of UTC and monotonic time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSample {
    /// Observed UTC time.
    pub utc: UtcInstant,
    /// Observed process-local monotonic time.
    pub monotonic: MonotonicInstant,
}

impl ClockSample {
    /// Maps a future UTC instant to a monotonic deadline.
    pub fn deadline_for(self, target: UtcInstant) -> Result<MonotonicDeadline, SlotTimeError> {
        let delta = target
            .0
            .checked_sub(self.utc.0)
            .ok_or(SlotTimeError::ArithmeticOverflow)?;
        if delta < 0 {
            return Err(SlotTimeError::MissedDeadline);
        }
        let delta = u64::try_from(delta).map_err(|_| SlotTimeError::ArithmeticOverflow)?;
        let value = self
            .monotonic
            .0
            .checked_add(delta)
            .ok_or(SlotTimeError::ArithmeticOverflow)?;
        Ok(MonotonicDeadline(MonotonicInstant(value)))
    }

    /// Returns the first protocol slot strictly after this sample.
    pub fn next_slot(
        self,
        mode: OperatingMode,
    ) -> Result<(UtcSlot, MonotonicDeadline), SlotTimeError> {
        let duration = mode.slot_millis();
        let start = self
            .utc
            .0
            .checked_div(duration)
            .and_then(|index| index.checked_add(1))
            .and_then(|index| index.checked_mul(duration))
            .ok_or(SlotTimeError::ArithmeticOverflow)?;
        let utc = UtcInstant::from_unix_millis(start)?;
        let slot = UtcSlot::new(mode, start).map_err(|_| SlotTimeError::MisalignedSlot)?;
        Ok((slot, self.deadline_for(utc)?))
    }

    /// Returns the aligned protocol slot containing this sample.
    pub fn current_slot(self, mode: OperatingMode) -> Result<UtcSlot, SlotTimeError> {
        let start = self.utc.0 - (self.utc.0 % mode.slot_millis());
        UtcSlot::new(mode, start).map_err(|_| SlotTimeError::MisalignedSlot)
    }
}

/// Source of paired UTC and monotonic observations.
pub trait Clock {
    /// Samples both timelines as one mapping point.
    fn sample(&self) -> ClockSample;
}

/// A cloneable clock advanced explicitly by tests and deterministic simulations.
#[derive(Debug, Clone)]
pub struct VirtualClock {
    state: Arc<Mutex<ClockSample>>,
}

impl VirtualClock {
    /// Creates a virtual clock at one sampled mapping.
    #[must_use]
    pub fn new(sample: ClockSample) -> Self {
        Self {
            state: Arc::new(Mutex::new(sample)),
        }
    }

    /// Advances UTC and monotonic time together without sleeping.
    pub fn advance(&self, millis: u64) -> Result<(), SlotTimeError> {
        let mut state = self.lock();
        let utc_delta = i64::try_from(millis).map_err(|_| SlotTimeError::ArithmeticOverflow)?;
        state.utc.0 = state
            .utc
            .0
            .checked_add(utc_delta)
            .ok_or(SlotTimeError::ArithmeticOverflow)?;
        state.monotonic.0 = state
            .monotonic
            .0
            .checked_add(millis)
            .ok_or(SlotTimeError::ArithmeticOverflow)?;
        Ok(())
    }

    /// Moves only wall-clock UTC to inject a jump while monotonic time continues.
    pub fn jump_utc(&self, delta_millis: i64) -> Result<(), SlotTimeError> {
        let mut state = self.lock();
        let updated = state
            .utc
            .0
            .checked_add(delta_millis)
            .ok_or(SlotTimeError::ArithmeticOverflow)?;
        if updated < 0 {
            return Err(SlotTimeError::BeforeUnixEpoch);
        }
        state.utc.0 = updated;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ClockSample> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Clock for VirtualClock {
    fn sample(&self) -> ClockSample {
        *self.lock()
    }
}

/// Typed clock-health state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockHealth {
    /// UTC and monotonic progress remain within tolerance.
    Healthy,
    /// The sampled mapping is unsafe for synchronized scheduling.
    Unhealthy(ClockFault),
}

/// Observable reason a clock mapping became unhealthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockFault {
    /// UTC progress diverged from monotonic progress beyond tolerance.
    WallClockJump {
        /// Signed UTC-minus-monotonic divergence in milliseconds.
        divergence_millis: i64,
    },
    /// A sampled timeline moved backwards.
    TimelineRegressed,
}

/// Stateful comparison of sampled UTC/monotonic mappings.
#[derive(Debug, Clone, Copy)]
pub struct ClockMonitor {
    last: ClockSample,
    tolerance_millis: u64,
    health: ClockHealth,
}

impl ClockMonitor {
    /// Starts a healthy monitor from an initial mapping and jump tolerance.
    #[must_use]
    pub const fn new(initial: ClockSample, tolerance_millis: u64) -> Self {
        Self {
            last: initial,
            tolerance_millis,
            health: ClockHealth::Healthy,
        }
    }

    /// Observes a new sample and latches unhealthy state after inconsistency.
    pub fn observe(&mut self, sample: ClockSample) -> ClockHealth {
        if self.health != ClockHealth::Healthy {
            return self.health;
        }
        let Some(utc_delta) = sample.utc.0.checked_sub(self.last.utc.0) else {
            self.health = ClockHealth::Unhealthy(ClockFault::TimelineRegressed);
            return self.health;
        };
        let Some(monotonic_delta) = sample.monotonic.0.checked_sub(self.last.monotonic.0) else {
            self.health = ClockHealth::Unhealthy(ClockFault::TimelineRegressed);
            return self.health;
        };
        if utc_delta < 0 {
            self.health = ClockHealth::Unhealthy(ClockFault::TimelineRegressed);
            return self.health;
        }
        let monotonic_delta = i64::try_from(monotonic_delta).unwrap_or(i64::MAX);
        let divergence = utc_delta.saturating_sub(monotonic_delta);
        if divergence.unsigned_abs() > self.tolerance_millis {
            self.health = ClockHealth::Unhealthy(ClockFault::WallClockJump {
                divergence_millis: divergence,
            });
        } else {
            self.last = sample;
        }
        self.health
    }

    /// Returns the current latched health state.
    #[must_use]
    pub const fn health(self) -> ClockHealth {
        self.health
    }
}

/// Failure mapping protocol time to monotonic deadlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SlotTimeError {
    /// UTC time predates the supported epoch.
    #[error("UTC instant must not precede the Unix epoch")]
    BeforeUnixEpoch,
    /// A requested UTC deadline is already in the past.
    #[error("requested slot deadline has already been missed")]
    MissedDeadline,
    /// Integer time arithmetic exceeded the bounded representation.
    #[error("time arithmetic overflow")]
    ArithmeticOverflow,
    /// A protocol slot was not aligned to its mode.
    #[error("protocol slot is not aligned")]
    MisalignedSlot,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(utc: i64, monotonic: u64) -> ClockSample {
        ClockSample {
            utc: UtcInstant::from_unix_millis(utc).unwrap(),
            monotonic: MonotonicInstant::from_millis(monotonic),
        }
    }

    #[test]
    fn exact_ft8_and_wspr_boundaries_map_to_monotonic_deadlines() {
        let exact = sample(120_000, 5_000);
        let (ft8, ft8_deadline) = exact.next_slot(OperatingMode::Ft8).unwrap();
        assert_eq!(ft8.start_unix_millis(), 135_000);
        assert_eq!(ft8_deadline.instant().millis(), 20_000);
        let (wspr, wspr_deadline) = exact.next_slot(OperatingMode::Wspr).unwrap();
        assert_eq!(wspr.start_unix_millis(), 240_000);
        assert_eq!(wspr_deadline.instant().millis(), 125_000);
    }

    #[test]
    fn late_calls_use_next_boundary_and_past_slots_are_missed() {
        let late = sample(15_001, 500);
        let (slot, deadline) = late.next_slot(OperatingMode::Ft8).unwrap();
        assert_eq!(slot.start_unix_millis(), 30_000);
        assert_eq!(deadline.instant().millis(), 15_499);
        assert_eq!(
            late.deadline_for(UtcInstant::from_unix_millis(15_000).unwrap()),
            Err(SlotTimeError::MissedDeadline)
        );
    }

    #[test]
    fn utc_midnight_is_an_exact_boundary() {
        let before_midnight = sample(86_399_999, 10);
        let (slot, deadline) = before_midnight.next_slot(OperatingMode::Ft8).unwrap();
        assert_eq!(slot.start_unix_millis(), 86_400_000);
        assert_eq!(deadline.instant().millis(), 11);
    }

    #[test]
    fn wall_clock_jump_latches_typed_unhealthy_state() {
        let clock = VirtualClock::new(sample(100_000, 1_000));
        let mut monitor = ClockMonitor::new(clock.sample(), 100);
        clock.advance(1_000).unwrap();
        assert_eq!(monitor.observe(clock.sample()), ClockHealth::Healthy);
        clock.jump_utc(2_000).unwrap();
        assert_eq!(
            monitor.observe(clock.sample()),
            ClockHealth::Unhealthy(ClockFault::WallClockJump {
                divergence_millis: 2_000
            })
        );
        clock.advance(1_000).unwrap();
        assert!(matches!(
            monitor.observe(clock.sample()),
            ClockHealth::Unhealthy(_)
        ));
    }

    #[test]
    fn virtual_time_expires_authority_deadline_without_sleeping() {
        let clock = VirtualClock::new(sample(1_000, 50));
        let expiry = clock
            .sample()
            .deadline_for(UtcInstant::from_unix_millis(2_000).unwrap())
            .unwrap();
        assert!(!expiry.is_expired_at(clock.sample().monotonic));
        clock.advance(1_000).unwrap();
        assert!(expiry.is_expired_at(clock.sample().monotonic));
    }
}
