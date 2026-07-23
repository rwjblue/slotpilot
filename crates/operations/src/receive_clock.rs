//! Production clock sampling and receive-only FT8 alignment gating.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use slotpilot_audio::{Ft8ReceiveSlot, Ft8ReceiveWindow, IncompleteFt8Slot, ReceiveTimelineEvent};
use thiserror::Error;

use crate::{ClockSample, MonotonicInstant, UtcInstant};

/// Default production receive-clock sampling cadence.
pub const DEFAULT_CLOCK_SAMPLE_CADENCE_MILLIS: u64 = 1_000;
/// Default age after which a mapping is stale.
pub const DEFAULT_CLOCK_FRESHNESS_MILLIS: u64 = 2_500;
/// Default suspend/resume-like monotonic gap limit.
pub const DEFAULT_CLOCK_SAMPLE_GAP_MILLIS: u64 = 5_000;
/// Default UTC/monotonic divergence tolerance.
pub const DEFAULT_CLOCK_JUMP_TOLERANCE_MILLIS: u64 = 100;
/// Default delay permitted between sampling and observation.
pub const DEFAULT_CLOCK_SAMPLING_DELAY_MILLIS: u64 = 250;
/// Default consecutive good samples required to recover.
pub const DEFAULT_CLOCK_RECOVERY_SAMPLES: u8 = 3;

/// Non-zero process generation owning one monotonic clock origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClockProcessGeneration(u64);

impl ClockProcessGeneration {
    /// Constructs a checked process generation.
    pub const fn new(value: u64) -> Result<Self, ReceiveClockError> {
        if value == 0 {
            return Err(ReceiveClockError::InvalidGeneration);
        }
        Ok(Self(value))
    }

    /// Returns the opaque generation value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One paired UTC/monotonic observation scoped to a process generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationClockSample {
    /// Process-local clock generation.
    pub generation: ClockProcessGeneration,
    /// Simultaneously sampled integer-millisecond mapping.
    pub sample: ClockSample,
}

/// Production paired system-clock adapter.
///
/// A monotonic reading is taken on each side of the UTC read and their
/// midpoint is retained. No sleep or scheduling authority exists here.
#[derive(Debug, Clone)]
pub struct SystemClock {
    generation: ClockProcessGeneration,
    monotonic_origin: Instant,
}

impl SystemClock {
    /// Starts a new process-local monotonic origin.
    #[must_use]
    pub fn new(generation: ClockProcessGeneration) -> Self {
        Self {
            generation,
            monotonic_origin: Instant::now(),
        }
    }

    /// Returns one checked paired production sample.
    pub fn sample(&self) -> Result<GenerationClockSample, ReceiveClockError> {
        let before = duration_millis(self.monotonic_origin.elapsed())?;
        let utc = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ReceiveClockError::SystemClockBeforeEpoch)?;
        let after = duration_millis(self.monotonic_origin.elapsed())?;
        let midpoint = before
            .checked_add(after.saturating_sub(before) / 2)
            .ok_or(ReceiveClockError::ArithmeticOverflow)?;
        let utc =
            i64::try_from(utc.as_millis()).map_err(|_| ReceiveClockError::ArithmeticOverflow)?;
        Ok(GenerationClockSample {
            generation: self.generation,
            sample: ClockSample {
                utc: UtcInstant::from_unix_millis(utc)
                    .map_err(|_| ReceiveClockError::SystemClockBeforeEpoch)?,
                monotonic: MonotonicInstant::from_millis(midpoint),
            },
        })
    }
}

/// Fallible source for generation-scoped paired clock samples.
pub trait ReceiveClockSource {
    /// Samples UTC and monotonic time together.
    fn sample(&self) -> Result<GenerationClockSample, ReceiveClockError>;
}

impl ReceiveClockSource for SystemClock {
    fn sample(&self) -> Result<GenerationClockSample, ReceiveClockError> {
        Self::sample(self)
    }
}

fn duration_millis(duration: std::time::Duration) -> Result<u64, ReceiveClockError> {
    u64::try_from(duration.as_millis()).map_err(|_| ReceiveClockError::ArithmeticOverflow)
}

/// Checked receive-clock monitoring policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiveClockConfig {
    /// Desired monotonic sampling cadence.
    pub sample_cadence_millis: u64,
    /// Maximum age of the most recently accepted mapping.
    pub freshness_millis: u64,
    /// Maximum monotonic gap between samples.
    pub max_sample_gap_millis: u64,
    /// Maximum UTC-minus-monotonic divergence between samples.
    pub jump_tolerance_millis: u64,
    /// Maximum delay between a sample and monitor observation.
    pub max_sampling_delay_millis: u64,
    /// Consecutive consistent samples required for recovery.
    pub recovery_samples: u8,
}

impl ReceiveClockConfig {
    /// Constructs a bounded receive-only clock policy.
    pub const fn new(
        sample_cadence_millis: u64,
        freshness_millis: u64,
        max_sample_gap_millis: u64,
        jump_tolerance_millis: u64,
        max_sampling_delay_millis: u64,
        recovery_samples: u8,
    ) -> Result<Self, ReceiveClockError> {
        if sample_cadence_millis < 10
            || sample_cadence_millis > 10_000
            || freshness_millis < sample_cadence_millis
            || freshness_millis > 60_000
            || max_sample_gap_millis < sample_cadence_millis
            || max_sample_gap_millis > 300_000
            || jump_tolerance_millis > 5_000
            || max_sampling_delay_millis > sample_cadence_millis
            || recovery_samples < 2
            || recovery_samples > 10
        {
            return Err(ReceiveClockError::InvalidConfiguration);
        }
        Ok(Self {
            sample_cadence_millis,
            freshness_millis,
            max_sample_gap_millis,
            jump_tolerance_millis,
            max_sampling_delay_millis,
            recovery_samples,
        })
    }
}

impl Default for ReceiveClockConfig {
    fn default() -> Self {
        Self {
            sample_cadence_millis: DEFAULT_CLOCK_SAMPLE_CADENCE_MILLIS,
            freshness_millis: DEFAULT_CLOCK_FRESHNESS_MILLIS,
            max_sample_gap_millis: DEFAULT_CLOCK_SAMPLE_GAP_MILLIS,
            jump_tolerance_millis: DEFAULT_CLOCK_JUMP_TOLERANCE_MILLIS,
            max_sampling_delay_millis: DEFAULT_CLOCK_SAMPLING_DELAY_MILLIS,
            recovery_samples: DEFAULT_CLOCK_RECOVERY_SAMPLES,
        }
    }
}

/// Typed reason a receive clock mapping cannot align live audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveClockFault {
    /// A sample came from another process generation.
    ProcessGenerationChanged,
    /// UTC or monotonic evidence moved backwards.
    TimelineRegressed,
    /// UTC progress diverged from monotonic progress.
    UtcJump {
        /// Signed UTC-minus-monotonic divergence.
        divergence_millis: i64,
    },
    /// No accepted sample arrived within the freshness bound.
    StaleMapping {
        /// Age of the last accepted sample.
        age_millis: u64,
    },
    /// Sampling work was observed too long after its timestamp.
    SamplingDelayed {
        /// Observed delay.
        delay_millis: u64,
    },
    /// A suspend/resume-like gap exceeded the bounded cadence.
    SampleGap {
        /// Monotonic gap between samples.
        gap_millis: u64,
    },
    /// A complete window's capture mapping disagreed with clock mapping.
    WindowMisaligned {
        /// Signed window-minus-clock monotonic difference.
        divergence_millis: i64,
    },
    /// Checked arithmetic could not represent the mapping.
    ArithmeticOverflow,
}

/// Latched receive-clock state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveClockState {
    /// The latest mapping may align receive windows.
    Healthy,
    /// Mapping is latched unhealthy until recovery completes.
    Unhealthy {
        /// Original/current typed failure.
        fault: ReceiveClockFault,
        /// Consecutive consistent recovery observations.
        recovery_progress: u8,
        /// Required recovery observations.
        recovery_required: u8,
    },
}

/// Internal diagnostic snapshot for later API mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiveClockSnapshot {
    /// Process-local generation.
    pub generation: ClockProcessGeneration,
    /// Current latched state.
    pub state: ReceiveClockState,
    /// Most recently accepted healthy mapping.
    pub last_accepted: ClockSample,
    /// Monotonic time when the snapshot was produced.
    pub observed_at: MonotonicInstant,
    /// Age of the last accepted mapping.
    pub mapping_age_millis: u64,
    /// Next desired monotonic sample instant.
    pub next_sample_due: MonotonicInstant,
}

/// Typed monitor transition emitted on every observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveClockTransition {
    /// Mapping remained healthy.
    Healthy(ReceiveClockSnapshot),
    /// A healthy mapping became latched unhealthy.
    BecameUnhealthy(ReceiveClockSnapshot),
    /// A consistent recovery sequence is incomplete.
    Recovering(ReceiveClockSnapshot),
    /// The documented recovery sequence completed.
    Recovered(ReceiveClockSnapshot),
    /// State remained latched unhealthy without progress.
    Unhealthy(ReceiveClockSnapshot),
}

/// One nonblocking production sampling-driver result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveClockPoll {
    /// The monotonic cadence has not elapsed.
    NotDue {
        /// Next desired monotonic sample instant.
        next_sample_due: MonotonicInstant,
    },
    /// A due sample was delivered to the monitor.
    Observed(ReceiveClockTransition),
}

/// Clock evidence attached to one accepted canonical window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiveWindowAlignment {
    /// Exact FT8 slot.
    pub slot: Ft8ReceiveSlot,
    /// Slot start expressed on the current monotonic timeline.
    pub slot_start_monotonic: MonotonicInstant,
    /// Age of the accepted clock mapping.
    pub mapping_age_millis: u64,
}

/// Receive timeline outcome after clock-health gating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClockGatedTimelineEvent {
    /// Complete window whose slot mapping is currently healthy.
    WindowReady {
        /// Canonical receive window.
        window: Ft8ReceiveWindow,
        /// Independent clock-alignment evidence.
        alignment: ReceiveWindowAlignment,
    },
    /// Complete window withheld because time was uncertain.
    WindowRejected {
        /// Exact slot withheld.
        slot: Ft8ReceiveSlot,
        /// Latched alignment failure.
        fault: ReceiveClockFault,
    },
    /// Existing incomplete timeline evidence passes through unchanged.
    Incomplete(IncompleteFt8Slot),
}

/// Receive-only continuous clock-health monitor and window gate.
pub struct ReceiveClockMonitor {
    generation: ClockProcessGeneration,
    config: ReceiveClockConfig,
    state: ReceiveClockState,
    last_accepted: ClockSample,
    recovery_last: Option<ClockSample>,
}

/// Nonblocking cadence driver for a production or fake clock source.
///
/// The daemon may call `poll` from its own event loop. The driver never sleeps
/// or spawns a thread and samples only when the monotonic cadence is due.
pub struct ReceiveClockDriver<C> {
    source: C,
    monitor: ReceiveClockMonitor,
}

impl<C: ReceiveClockSource> ReceiveClockDriver<C> {
    /// Composes one source with its same-generation monitor.
    #[must_use]
    pub const fn new(source: C, monitor: ReceiveClockMonitor) -> Self {
        Self { source, monitor }
    }

    /// Polls once without blocking or sleeping.
    pub fn poll(&mut self) -> Result<ReceiveClockPoll, ReceiveClockError> {
        let observation = self.source.sample()?;
        let due = self.monitor.next_sample_due()?;
        if observation.sample.monotonic < due {
            return Ok(ReceiveClockPoll::NotDue {
                next_sample_due: due,
            });
        }
        Ok(ReceiveClockPoll::Observed(
            self.monitor
                .observe(observation, observation.sample.monotonic),
        ))
    }

    /// Returns mutable access to the owned receive gate and snapshots.
    #[must_use]
    pub const fn monitor_mut(&mut self) -> &mut ReceiveClockMonitor {
        &mut self.monitor
    }
}

impl ReceiveClockMonitor {
    /// Starts healthy from one same-generation initial mapping.
    pub fn new(
        initial: GenerationClockSample,
        config: ReceiveClockConfig,
    ) -> Result<Self, ReceiveClockError> {
        ReceiveClockConfig::new(
            config.sample_cadence_millis,
            config.freshness_millis,
            config.max_sample_gap_millis,
            config.jump_tolerance_millis,
            config.max_sampling_delay_millis,
            config.recovery_samples,
        )?;
        Ok(Self {
            generation: initial.generation,
            config,
            state: ReceiveClockState::Healthy,
            last_accepted: initial.sample,
            recovery_last: None,
        })
    }

    /// Returns the monotonic instant at which the next sample is desired.
    pub fn next_sample_due(&self) -> Result<MonotonicInstant, ReceiveClockError> {
        self.last_accepted
            .monotonic
            .millis()
            .checked_add(self.config.sample_cadence_millis)
            .map(MonotonicInstant::from_millis)
            .ok_or(ReceiveClockError::ArithmeticOverflow)
    }

    /// Observes one sample without sleeping and updates latch/recovery state.
    pub fn observe(
        &mut self,
        observation: GenerationClockSample,
        observed_at: MonotonicInstant,
    ) -> ReceiveClockTransition {
        if observation.generation != self.generation {
            return self.latch(
                ReceiveClockFault::ProcessGenerationChanged,
                observed_at,
                None,
            );
        }
        let Some(delay) = observed_at
            .millis()
            .checked_sub(observation.sample.monotonic.millis())
        else {
            return self.latch(ReceiveClockFault::TimelineRegressed, observed_at, None);
        };
        if delay > self.config.max_sampling_delay_millis {
            return self.latch(
                ReceiveClockFault::SamplingDelayed {
                    delay_millis: delay,
                },
                observed_at,
                None,
            );
        }

        match self.state {
            ReceiveClockState::Healthy => {
                if let Some(fault) = self.compare(self.last_accepted, observation.sample) {
                    return self.latch(fault, observed_at, None);
                }
                self.last_accepted = observation.sample;
                ReceiveClockTransition::Healthy(self.snapshot(observed_at))
            }
            ReceiveClockState::Unhealthy {
                fault,
                recovery_progress,
                recovery_required,
            } => {
                if fault == ReceiveClockFault::ProcessGenerationChanged {
                    return ReceiveClockTransition::Unhealthy(self.snapshot(observed_at));
                }
                if let Some(previous) = self.recovery_last
                    && let Some(new_fault) = self.compare(previous, observation.sample)
                {
                    return self.latch(new_fault, observed_at, Some(observation.sample));
                }
                let progress = recovery_progress.saturating_add(1);
                self.recovery_last = Some(observation.sample);
                if progress >= recovery_required {
                    self.last_accepted = observation.sample;
                    self.recovery_last = None;
                    self.state = ReceiveClockState::Healthy;
                    ReceiveClockTransition::Recovered(self.snapshot(observed_at))
                } else {
                    self.state = ReceiveClockState::Unhealthy {
                        fault,
                        recovery_progress: progress,
                        recovery_required,
                    };
                    ReceiveClockTransition::Recovering(self.snapshot(observed_at))
                }
            }
        }
    }

    /// Returns a snapshot, latching staleness when necessary.
    pub fn snapshot(&mut self, observed_at: MonotonicInstant) -> ReceiveClockSnapshot {
        let age = observed_at
            .millis()
            .saturating_sub(self.last_accepted.monotonic.millis());
        if age > self.config.freshness_millis && self.state == ReceiveClockState::Healthy {
            self.state = ReceiveClockState::Unhealthy {
                fault: ReceiveClockFault::StaleMapping { age_millis: age },
                recovery_progress: 0,
                recovery_required: self.config.recovery_samples,
            };
            self.recovery_last = None;
        }
        self.snapshot_unchecked(observed_at)
    }

    /// Gates one timeline event; unhealthy time can never return `WindowReady`.
    pub fn gate(
        &mut self,
        event: ReceiveTimelineEvent,
        observed_at: MonotonicInstant,
    ) -> ClockGatedTimelineEvent {
        match event {
            ReceiveTimelineEvent::Incomplete(incomplete) => {
                ClockGatedTimelineEvent::Incomplete(incomplete)
            }
            ReceiveTimelineEvent::Window(window) => {
                let slot = window.slot();
                let snapshot = self.snapshot(observed_at);
                if let ReceiveClockState::Unhealthy { fault, .. } = snapshot.state {
                    return ClockGatedTimelineEvent::WindowRejected { slot, fault };
                }
                match self.align_window(&window, slot, snapshot.mapping_age_millis) {
                    Ok(alignment) => ClockGatedTimelineEvent::WindowReady { window, alignment },
                    Err(fault) => {
                        self.state = ReceiveClockState::Unhealthy {
                            fault,
                            recovery_progress: 0,
                            recovery_required: self.config.recovery_samples,
                        };
                        self.recovery_last = None;
                        ClockGatedTimelineEvent::WindowRejected { slot, fault }
                    }
                }
            }
        }
    }

    fn align_window(
        &self,
        window: &Ft8ReceiveWindow,
        slot: Ft8ReceiveSlot,
        mapping_age_millis: u64,
    ) -> Result<ReceiveWindowAlignment, ReceiveClockFault> {
        let slot_start = slot.start_utc_unix_millis();
        let utc_delta = i128::from(slot_start) - i128::from(self.last_accepted.utc.unix_millis());
        let predicted = i128::from(self.last_accepted.monotonic.millis()) + utc_delta;
        let predicted =
            u64::try_from(predicted).map_err(|_| ReceiveClockFault::ArithmeticOverflow)?;
        let divergence = i128::from(window.mapping.monotonic_millis) - i128::from(predicted);
        let divergence =
            i64::try_from(divergence).map_err(|_| ReceiveClockFault::ArithmeticOverflow)?;
        if divergence.unsigned_abs() > self.config.jump_tolerance_millis {
            return Err(ReceiveClockFault::WindowMisaligned {
                divergence_millis: divergence,
            });
        }
        Ok(ReceiveWindowAlignment {
            slot,
            slot_start_monotonic: MonotonicInstant::from_millis(predicted),
            mapping_age_millis,
        })
    }

    fn compare(&self, previous: ClockSample, current: ClockSample) -> Option<ReceiveClockFault> {
        let Some(utc_delta) = current
            .utc
            .unix_millis()
            .checked_sub(previous.utc.unix_millis())
        else {
            return Some(ReceiveClockFault::ArithmeticOverflow);
        };
        let monotonic_delta = current
            .monotonic
            .millis()
            .checked_sub(previous.monotonic.millis());
        let Some(monotonic_delta) = monotonic_delta else {
            return Some(ReceiveClockFault::TimelineRegressed);
        };
        if utc_delta < 0 {
            let divergence = i128::from(utc_delta) - i128::from(monotonic_delta);
            return Some(ReceiveClockFault::UtcJump {
                divergence_millis: i64::try_from(divergence).unwrap_or(i64::MIN),
            });
        }
        if monotonic_delta > self.config.max_sample_gap_millis {
            return Some(ReceiveClockFault::SampleGap {
                gap_millis: monotonic_delta,
            });
        }
        let divergence = i128::from(utc_delta) - i128::from(monotonic_delta);
        let divergence = i64::try_from(divergence).unwrap_or_else(|_| {
            if divergence.is_negative() {
                i64::MIN
            } else {
                i64::MAX
            }
        });
        (divergence.unsigned_abs() > self.config.jump_tolerance_millis).then_some(
            ReceiveClockFault::UtcJump {
                divergence_millis: divergence,
            },
        )
    }

    fn latch(
        &mut self,
        fault: ReceiveClockFault,
        observed_at: MonotonicInstant,
        recovery_last: Option<ClockSample>,
    ) -> ReceiveClockTransition {
        let was_healthy = self.state == ReceiveClockState::Healthy;
        self.state = ReceiveClockState::Unhealthy {
            fault,
            recovery_progress: u8::from(recovery_last.is_some()),
            recovery_required: self.config.recovery_samples,
        };
        self.recovery_last = recovery_last;
        if was_healthy {
            ReceiveClockTransition::BecameUnhealthy(self.snapshot_unchecked(observed_at))
        } else {
            ReceiveClockTransition::Unhealthy(self.snapshot_unchecked(observed_at))
        }
    }

    fn snapshot_unchecked(&self, observed_at: MonotonicInstant) -> ReceiveClockSnapshot {
        ReceiveClockSnapshot {
            generation: self.generation,
            state: self.state,
            last_accepted: self.last_accepted,
            observed_at,
            mapping_age_millis: observed_at
                .millis()
                .saturating_sub(self.last_accepted.monotonic.millis()),
            next_sample_due: self
                .next_sample_due()
                .unwrap_or(MonotonicInstant::from_millis(u64::MAX)),
        }
    }
}

/// Checked production/monitor construction failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ReceiveClockError {
    /// A process generation must be non-zero.
    #[error("clock process generation is invalid")]
    InvalidGeneration,
    /// Monitoring bounds are inconsistent or outside supported limits.
    #[error("receive clock configuration is invalid")]
    InvalidConfiguration,
    /// The production UTC clock predates the Unix epoch.
    #[error("system UTC clock predates the Unix epoch")]
    SystemClockBeforeEpoch,
    /// Checked integer clock arithmetic overflowed.
    #[error("receive clock arithmetic overflowed")]
    ArithmeticOverflow,
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, collections::VecDeque};

    use slotpilot_audio::{
        CapturePosition, CaptureTimeEvidence, FT8_RECEIVE_WINDOW_SAMPLES, ProcessGeneration,
        StreamGeneration,
    };

    use super::*;

    fn generation(value: u64) -> ClockProcessGeneration {
        ClockProcessGeneration::new(value).unwrap()
    }

    fn sample(generation_value: u64, utc: i64, monotonic: u64) -> GenerationClockSample {
        GenerationClockSample {
            generation: generation(generation_value),
            sample: ClockSample {
                utc: UtcInstant::from_unix_millis(utc).unwrap(),
                monotonic: MonotonicInstant::from_millis(monotonic),
            },
        }
    }

    fn monitor() -> ReceiveClockMonitor {
        ReceiveClockMonitor::new(sample(1, 30_000, 1_000), ReceiveClockConfig::default()).unwrap()
    }

    fn window(slot: i64, monotonic: u64) -> Ft8ReceiveWindow {
        Ft8ReceiveWindow::new(
            ProcessGeneration::new(1).unwrap(),
            StreamGeneration::new(1).unwrap(),
            slot,
            CaptureTimeEvidence::new(CapturePosition::from_frames(0), slot, monotonic).unwrap(),
            vec![0; FT8_RECEIVE_WINDOW_SAMPLES],
        )
        .unwrap()
    }

    #[test]
    fn configuration_and_generation_are_bounded() {
        assert_eq!(
            ClockProcessGeneration::new(0),
            Err(ReceiveClockError::InvalidGeneration)
        );
        assert_eq!(
            ReceiveClockConfig::new(1, 2_500, 5_000, 100, 250, 3),
            Err(ReceiveClockError::InvalidConfiguration)
        );
        assert_eq!(
            ReceiveClockConfig::new(1_000, 500, 5_000, 100, 250, 3),
            Err(ReceiveClockError::InvalidConfiguration)
        );
    }

    #[test]
    fn exact_boundary_maps_to_monotonic_and_gates_window() {
        let mut monitor = monitor();
        let event = monitor.gate(
            ReceiveTimelineEvent::Window(window(30_000, 1_000)),
            MonotonicInstant::from_millis(1_100),
        );
        match event {
            ClockGatedTimelineEvent::WindowReady { alignment, .. } => {
                assert_eq!(alignment.slot.start_utc_unix_millis(), 30_000);
                assert_eq!(alignment.slot_start_monotonic.millis(), 1_000);
                assert_eq!(alignment.mapping_age_millis, 100);
            }
            other => panic!("unexpected gate result: {other:?}"),
        }
        assert_eq!(monitor.next_sample_due().unwrap().millis(), 2_000);
    }

    #[test]
    fn forward_and_backward_utc_jumps_latch_unhealthy() {
        for utc in [33_000, 29_000] {
            let mut monitor = monitor();
            let transition =
                monitor.observe(sample(1, utc, 2_000), MonotonicInstant::from_millis(2_000));
            assert!(matches!(
                transition,
                ReceiveClockTransition::BecameUnhealthy(ReceiveClockSnapshot {
                    state: ReceiveClockState::Unhealthy {
                        fault: ReceiveClockFault::UtcJump { .. },
                        ..
                    },
                    ..
                })
            ));
        }
    }

    #[test]
    fn regression_delay_suspend_and_restart_are_distinct() {
        let mut regressed = monitor();
        assert!(matches!(
            regressed.observe(sample(1, 30_100, 900), MonotonicInstant::from_millis(900)),
            ReceiveClockTransition::BecameUnhealthy(ReceiveClockSnapshot {
                state: ReceiveClockState::Unhealthy {
                    fault: ReceiveClockFault::TimelineRegressed,
                    ..
                },
                ..
            })
        ));

        let mut delayed = monitor();
        assert!(matches!(
            delayed.observe(
                sample(1, 31_000, 2_000),
                MonotonicInstant::from_millis(2_251)
            ),
            ReceiveClockTransition::BecameUnhealthy(ReceiveClockSnapshot {
                state: ReceiveClockState::Unhealthy {
                    fault: ReceiveClockFault::SamplingDelayed { delay_millis: 251 },
                    ..
                },
                ..
            })
        ));

        let mut suspended = monitor();
        assert!(matches!(
            suspended.observe(
                sample(1, 36_001, 7_001),
                MonotonicInstant::from_millis(7_001)
            ),
            ReceiveClockTransition::BecameUnhealthy(ReceiveClockSnapshot {
                state: ReceiveClockState::Unhealthy {
                    fault: ReceiveClockFault::SampleGap { gap_millis: 6_001 },
                    ..
                },
                ..
            })
        ));

        let mut restarted = monitor();
        assert!(matches!(
            restarted.observe(
                sample(2, 31_000, 2_000),
                MonotonicInstant::from_millis(2_000)
            ),
            ReceiveClockTransition::BecameUnhealthy(ReceiveClockSnapshot {
                state: ReceiveClockState::Unhealthy {
                    fault: ReceiveClockFault::ProcessGenerationChanged,
                    ..
                },
                ..
            })
        ));
    }

    #[test]
    fn stale_or_misaligned_time_never_yields_a_ready_window() {
        let mut stale = monitor();
        let rejected = stale.gate(
            ReceiveTimelineEvent::Window(window(30_000, 1_000)),
            MonotonicInstant::from_millis(3_501),
        );
        assert!(matches!(
            rejected,
            ClockGatedTimelineEvent::WindowRejected {
                fault: ReceiveClockFault::StaleMapping { age_millis: 2_501 },
                ..
            }
        ));

        let mut misaligned = monitor();
        let rejected = misaligned.gate(
            ReceiveTimelineEvent::Window(window(30_000, 1_101)),
            MonotonicInstant::from_millis(1_100),
        );
        assert!(matches!(
            rejected,
            ClockGatedTimelineEvent::WindowRejected {
                fault: ReceiveClockFault::WindowMisaligned {
                    divergence_millis: 101
                },
                ..
            }
        ));
    }

    #[test]
    fn recovery_requires_three_visible_consistent_samples() {
        let mut monitor = monitor();
        monitor.observe(
            sample(1, 33_000, 2_000),
            MonotonicInstant::from_millis(2_000),
        );
        for (index, (utc, monotonic)) in
            [(40_000, 10_000), (41_000, 11_000)].into_iter().enumerate()
        {
            let transition = monitor.observe(
                sample(1, utc, monotonic),
                MonotonicInstant::from_millis(monotonic),
            );
            assert!(matches!(transition, ReceiveClockTransition::Recovering(_)));
            let rejected = monitor.gate(
                ReceiveTimelineEvent::Window(window(45_000, 15_000)),
                MonotonicInstant::from_millis(monotonic),
            );
            assert!(
                matches!(rejected, ClockGatedTimelineEvent::WindowRejected { .. }),
                "recovery sample {index}"
            );
        }
        let transition = monitor.observe(
            sample(1, 42_000, 12_000),
            MonotonicInstant::from_millis(12_000),
        );
        assert!(matches!(transition, ReceiveClockTransition::Recovered(_)));
    }

    #[test]
    fn system_clock_sample_keeps_explicit_generation_and_units() {
        let clock = SystemClock::new(generation(7));
        let first = clock.sample().unwrap();
        let second = clock.sample().unwrap();
        assert_eq!(first.generation, generation(7));
        assert_eq!(second.generation, generation(7));
        assert!(second.sample.utc.unix_millis() >= first.sample.utc.unix_millis());
        assert!(second.sample.monotonic.millis() >= first.sample.monotonic.millis());
    }

    struct ReplayClock {
        samples: Cell<Option<VecDeque<GenerationClockSample>>>,
    }

    impl ReplayClock {
        fn new(samples: Vec<GenerationClockSample>) -> Self {
            Self {
                samples: Cell::new(Some(samples.into())),
            }
        }
    }

    impl ReceiveClockSource for ReplayClock {
        fn sample(&self) -> Result<GenerationClockSample, ReceiveClockError> {
            let mut samples = self.samples.take().unwrap_or_default();
            let sample = samples
                .pop_front()
                .ok_or(ReceiveClockError::ArithmeticOverflow)?;
            self.samples.set(Some(samples));
            Ok(sample)
        }
    }

    #[test]
    fn nonblocking_driver_samples_only_when_monotonic_cadence_is_due() {
        let source = ReplayClock::new(vec![sample(1, 30_500, 1_500), sample(1, 31_000, 2_000)]);
        let mut driver = ReceiveClockDriver::new(source, monitor());
        assert_eq!(
            driver.poll().unwrap(),
            ReceiveClockPoll::NotDue {
                next_sample_due: MonotonicInstant::from_millis(2_000)
            }
        );
        assert!(matches!(
            driver.poll().unwrap(),
            ReceiveClockPoll::Observed(ReceiveClockTransition::Healthy(_))
        ));
    }
}
