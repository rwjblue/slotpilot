//! Pure timestamped receive timeline and deterministic rational resampler.

use thiserror::Error;

use crate::{
    CaptureBatch, CaptureDiscontinuityKind, CapturePosition, CaptureTimeEvidence,
    FT8_RECEIVE_SAMPLE_RATE_HZ, FT8_RECEIVE_SLOT_MILLIS, FT8_RECEIVE_WINDOW_SAMPLES,
    Ft8ReceiveWindow, InputConfiguration, ProcessGeneration, StreamGeneration,
};

/// Maximum accepted batch-arrival delay before its samples are rejected.
pub const MAX_RECEIVE_BATCH_LATENESS_MILLIS: u64 = 2_000;
/// Maximum accepted deviation from the source-frame monotonic timeline.
pub const MAX_RECEIVE_JITTER_MILLIS: i64 = 50;
/// Maximum accepted UTC/monotonic offset change.
pub const MAX_RECEIVE_CLOCK_REMAP_MILLIS: i64 = 20;
/// Maximum accepted absolute source-rate error.
pub const MAX_RECEIVE_DRIFT_PARTS_PER_MILLION: i32 = 5_000;

const OUTPUT_TICKS_PER_MILLISECOND: i128 = FT8_RECEIVE_SAMPLE_RATE_HZ as i128 / 1_000;
const OUTPUT_TICKS_PER_SLOT: i128 = FT8_RECEIVE_WINDOW_SAMPLES as i128;

/// Exact identity of one UTC-aligned FT8 receive slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ft8ReceiveSlot(i64);

impl Ft8ReceiveSlot {
    /// Constructs a checked FT8 receive slot identity.
    pub const fn new(start_utc_unix_millis: i64) -> Result<Self, ReceiveTimelineError> {
        if start_utc_unix_millis < 0 || start_utc_unix_millis % FT8_RECEIVE_SLOT_MILLIS != 0 {
            return Err(ReceiveTimelineError::InvalidSlot);
        }
        Ok(Self(start_utc_unix_millis))
    }

    /// Returns the exact UTC slot start in Unix milliseconds.
    #[must_use]
    pub const fn start_utc_unix_millis(self) -> i64 {
        self.0
    }
}

impl Ft8ReceiveWindow {
    /// Returns the exact typed slot identity carried by this window.
    #[must_use]
    pub const fn slot(&self) -> Ft8ReceiveSlot {
        Ft8ReceiveSlot(self.slot_start_utc_millis)
    }
}

/// Reason a partial slot was intentionally not published as a decoder window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncompleteSlotReason {
    /// Capture began after the slot boundary.
    CaptureStartedLate,
    /// A timeline invalidation discarded accumulated samples.
    TimelineInvalidated,
}

/// Observable non-window outcome from timeline processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IncompleteFt8Slot {
    /// Exact slot that was withheld.
    pub slot: Ft8ReceiveSlot,
    /// Evidence-preserving discard reason.
    pub reason: IncompleteSlotReason,
    /// Number of canonical samples accumulated before discard.
    pub accumulated_samples: u32,
}

/// One pure timeline-processing outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiveTimelineEvent {
    /// A complete canonical decoder input.
    Window(Ft8ReceiveWindow),
    /// An incomplete slot explicitly withheld from decode.
    Incomplete(IncompleteFt8Slot),
}

/// Latest bounded health accounting for one timeline generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReceiveTimelineHealth {
    /// Greatest absolute mapping jitter observed.
    pub max_jitter_millis: u32,
    /// Latest signed estimated source-rate error.
    pub drift_parts_per_million: i32,
    /// Number of incomplete slots withheld.
    pub incomplete_slot_count: u64,
    /// Number of rejected late batches.
    pub late_batch_count: u64,
}

/// Typed failure from pure receive timeline processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ReceiveTimelineError {
    /// A slot identity was negative or not aligned to 15 seconds.
    #[error("FT8 receive slot identity is invalid")]
    InvalidSlot,
    /// The daemon process generation changed.
    #[error("capture process generation changed")]
    ProcessGenerationChanged,
    /// The input stream generation changed.
    #[error("capture stream generation changed")]
    StreamGenerationChanged,
    /// The exact source configuration changed.
    #[error("capture configuration changed")]
    ConfigurationChanged,
    /// The batch begins before the preceding batch.
    #[error("capture batch arrived out of order")]
    OutOfOrder,
    /// Source frames overlap an already consumed range.
    #[error("capture batch overlaps {frames} source frames")]
    Overlap {
        /// Number of overlapping source frames.
        frames: u64,
    },
    /// Source frames are missing.
    #[error("capture timeline is missing {frames} source frames")]
    Gap {
        /// Number of missing source frames.
        frames: u64,
    },
    /// The batch carries an explicit invalidating discontinuity.
    #[error("capture discontinuity invalidated the active slot: {0:?}")]
    Discontinuity(CaptureDiscontinuityKind),
    /// Monotonic evidence regressed.
    #[error("capture monotonic evidence regressed")]
    MonotonicRegression,
    /// Mapping jitter exceeded the bounded tolerance.
    #[error("capture mapping jitter exceeded tolerance: {millis} ms")]
    ExcessiveJitter {
        /// Signed mapping deviation.
        millis: i64,
    },
    /// UTC moved relative to the monotonic timeline.
    #[error("capture UTC/monotonic mapping changed: {millis} ms")]
    ClockRemapped {
        /// Signed UTC/monotonic offset change.
        millis: i64,
    },
    /// Source-rate error exceeded the bounded tolerance.
    #[error("capture source-rate drift exceeded tolerance: {parts_per_million} ppm")]
    ExcessiveDrift {
        /// Signed estimated source-rate error.
        parts_per_million: i32,
    },
    /// The batch arrived too late to retain.
    #[error("capture batch arrived {millis} ms late")]
    LateData {
        /// Observed nonnegative lateness.
        millis: u64,
    },
    /// Checked timeline arithmetic overflowed.
    #[error("capture timeline arithmetic overflowed")]
    ArithmeticOverflow,
}

/// Pure worker-side FT8 receive timeline.
///
/// The processor owns at most one 180,000-sample canonical window plus one
/// previous source sample. It performs no device I/O, sleeping, decode,
/// persistence, client publication, output, rig, PTT, or transmit work.
pub struct Ft8ReceiveTimeline {
    process_generation: ProcessGeneration,
    stream_generation: StreamGeneration,
    configuration: InputConfiguration,
    expected_position: Option<CapturePosition>,
    last_batch_position: Option<CapturePosition>,
    last_monotonic_millis: Option<u64>,
    anchor: Option<TimelineAnchor>,
    resampler: Option<RationalResampler>,
    health: ReceiveTimelineHealth,
}

impl Ft8ReceiveTimeline {
    /// Constructs an empty timeline for one exact capture generation.
    #[must_use]
    pub const fn new(
        process_generation: ProcessGeneration,
        stream_generation: StreamGeneration,
        configuration: InputConfiguration,
    ) -> Self {
        Self {
            process_generation,
            stream_generation,
            configuration,
            expected_position: None,
            last_batch_position: None,
            last_monotonic_millis: None,
            anchor: None,
            resampler: None,
            health: ReceiveTimelineHealth {
                max_jitter_millis: 0,
                drift_parts_per_million: 0,
                incomplete_slot_count: 0,
                late_batch_count: 0,
            },
        }
    }

    /// Returns latest bounded mapping and discard health.
    #[must_use]
    pub const fn health(&self) -> ReceiveTimelineHealth {
        self.health
    }

    /// Consumes one owned capture batch at a supplied monotonic observation.
    ///
    /// The supplied observation makes late-data behavior deterministic under a
    /// virtual clock. No wall-clock read or sleep occurs in this processor.
    pub fn push(
        &mut self,
        batch: &CaptureBatch,
        observed_at_monotonic_millis: u64,
    ) -> Result<Vec<ReceiveTimelineEvent>, ReceiveTimelineError> {
        self.validate_identity(batch)?;
        let lateness =
            observed_at_monotonic_millis.saturating_sub(batch.first_frame.monotonic_millis);
        if lateness > MAX_RECEIVE_BATCH_LATENESS_MILLIS {
            self.health.late_batch_count = self.health.late_batch_count.saturating_add(1);
            self.invalidate_active(IncompleteSlotReason::TimelineInvalidated);
            self.advance_expected(batch)?;
            return Err(ReceiveTimelineError::LateData { millis: lateness });
        }

        if let Err(error) = self.validate_position(batch) {
            self.invalidate_active(IncompleteSlotReason::TimelineInvalidated);
            if matches!(error, ReceiveTimelineError::Gap { .. }) {
                self.advance_expected(batch)?;
            }
            return Err(error);
        }
        if let Some(discontinuity) = batch.discontinuity {
            let initial_restart = self.expected_position.is_none()
                && discontinuity.kind == CaptureDiscontinuityKind::StreamRestart;
            if !initial_restart {
                self.invalidate_active(IncompleteSlotReason::TimelineInvalidated);
                self.advance_expected(batch)?;
                return Err(ReceiveTimelineError::Discontinuity(discontinuity.kind));
            }
        }

        if let Some(anchor) = self.anchor {
            if let Err(error) = self.validate_mapping(anchor, batch) {
                self.invalidate_active(IncompleteSlotReason::TimelineInvalidated);
                self.advance_expected(batch)?;
                return Err(error);
            }
        } else {
            self.anchor = Some(TimelineAnchor::from_batch(batch));
        }

        if self.resampler.is_none() {
            self.resampler = Some(RationalResampler::new(
                batch.first_frame,
                self.configuration.sample_rate_hz(),
                self.process_generation,
                self.stream_generation,
            )?);
        }
        let events = self
            .resampler
            .as_mut()
            .ok_or(ReceiveTimelineError::ArithmeticOverflow)?
            .push(batch)?;
        self.health.incomplete_slot_count = self.health.incomplete_slot_count.saturating_add(
            events
                .iter()
                .filter(|event| matches!(event, ReceiveTimelineEvent::Incomplete(_)))
                .count() as u64,
        );
        self.advance_expected(batch)?;
        Ok(events)
    }

    fn validate_identity(&self, batch: &CaptureBatch) -> Result<(), ReceiveTimelineError> {
        if batch.process_generation != self.process_generation {
            return Err(ReceiveTimelineError::ProcessGenerationChanged);
        }
        if batch.stream_generation != self.stream_generation {
            return Err(ReceiveTimelineError::StreamGenerationChanged);
        }
        if batch.configuration != self.configuration {
            return Err(ReceiveTimelineError::ConfigurationChanged);
        }
        Ok(())
    }

    fn validate_position(&self, batch: &CaptureBatch) -> Result<(), ReceiveTimelineError> {
        let Some(expected) = self.expected_position else {
            return Ok(());
        };
        let actual = batch.first_frame.position.frames();
        let expected = expected.frames();
        if actual == expected {
            return Ok(());
        }
        if actual < expected {
            if self
                .last_batch_position
                .is_some_and(|previous| actual < previous.frames())
            {
                return Err(ReceiveTimelineError::OutOfOrder);
            }
            return Err(ReceiveTimelineError::Overlap {
                frames: expected - actual,
            });
        }
        Err(ReceiveTimelineError::Gap {
            frames: actual - expected,
        })
    }

    fn validate_mapping(
        &mut self,
        anchor: TimelineAnchor,
        batch: &CaptureBatch,
    ) -> Result<(), ReceiveTimelineError> {
        if self
            .last_monotonic_millis
            .is_some_and(|previous| batch.first_frame.monotonic_millis < previous)
        {
            return Err(ReceiveTimelineError::MonotonicRegression);
        }
        if batch.first_frame.monotonic_millis < anchor.monotonic_millis {
            return Err(ReceiveTimelineError::MonotonicRegression);
        }
        let frame_delta = batch
            .first_frame
            .position
            .frames()
            .checked_sub(anchor.position.frames())
            .ok_or(ReceiveTimelineError::OutOfOrder)?;
        let expected_millis = frame_delta
            .checked_mul(1_000)
            .ok_or(ReceiveTimelineError::ArithmeticOverflow)?
            / u64::from(self.configuration.sample_rate_hz());
        let observed_millis = batch.first_frame.monotonic_millis - anchor.monotonic_millis;
        let jitter = i128::from(observed_millis) - i128::from(expected_millis);
        let jitter = i64::try_from(jitter).map_err(|_| ReceiveTimelineError::ArithmeticOverflow)?;
        self.health.max_jitter_millis = self
            .health
            .max_jitter_millis
            .max(u32::try_from(jitter.unsigned_abs()).unwrap_or(u32::MAX));
        if jitter.unsigned_abs() > MAX_RECEIVE_JITTER_MILLIS as u64 {
            return Err(ReceiveTimelineError::ExcessiveJitter { millis: jitter });
        }

        let anchor_offset =
            i128::from(anchor.utc_unix_millis) - i128::from(anchor.monotonic_millis);
        let current_offset = i128::from(batch.first_frame.utc_unix_millis)
            - i128::from(batch.first_frame.monotonic_millis);
        let remap = i64::try_from(current_offset - anchor_offset)
            .map_err(|_| ReceiveTimelineError::ArithmeticOverflow)?;
        if remap.unsigned_abs() > MAX_RECEIVE_CLOCK_REMAP_MILLIS as u64 {
            return Err(ReceiveTimelineError::ClockRemapped { millis: remap });
        }

        if expected_millis >= 1_000 {
            let error_millis = i128::from(observed_millis) - i128::from(expected_millis);
            let ppm = error_millis
                .checked_mul(1_000_000)
                .and_then(|value| value.checked_div(i128::from(expected_millis)))
                .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
            let ppm = i32::try_from(ppm).map_err(|_| ReceiveTimelineError::ArithmeticOverflow)?;
            self.health.drift_parts_per_million = ppm;
            if ppm.unsigned_abs() > MAX_RECEIVE_DRIFT_PARTS_PER_MILLION as u32 {
                return Err(ReceiveTimelineError::ExcessiveDrift {
                    parts_per_million: ppm,
                });
            }
        }
        Ok(())
    }

    fn advance_expected(&mut self, batch: &CaptureBatch) -> Result<(), ReceiveTimelineError> {
        self.last_batch_position = Some(batch.first_frame.position);
        self.last_monotonic_millis = Some(batch.first_frame.monotonic_millis);
        self.expected_position = Some(
            batch
                .end_position()
                .map_err(|_| ReceiveTimelineError::ArithmeticOverflow)?,
        );
        Ok(())
    }

    fn invalidate_active(&mut self, _reason: IncompleteSlotReason) {
        if self
            .resampler
            .as_mut()
            .and_then(RationalResampler::discard_active)
            .is_some()
        {
            self.health.incomplete_slot_count = self.health.incomplete_slot_count.saturating_add(1);
        }
        self.anchor = None;
        self.resampler = None;
    }
}

#[derive(Debug, Clone, Copy)]
struct TimelineAnchor {
    position: CapturePosition,
    utc_unix_millis: i64,
    monotonic_millis: u64,
}

impl TimelineAnchor {
    const fn from_batch(batch: &CaptureBatch) -> Self {
        Self {
            position: batch.first_frame.position,
            utc_unix_millis: batch.first_frame.utc_unix_millis,
            monotonic_millis: batch.first_frame.monotonic_millis,
        }
    }
}

struct RationalResampler {
    anchor: CaptureTimeEvidence,
    source_rate_hz: u32,
    process_generation: ProcessGeneration,
    stream_generation: StreamGeneration,
    next_output_tick: i128,
    previous: Option<(u64, i16)>,
    active: Option<WindowAccumulator>,
}

impl RationalResampler {
    fn new(
        anchor: CaptureTimeEvidence,
        source_rate_hz: u32,
        process_generation: ProcessGeneration,
        stream_generation: StreamGeneration,
    ) -> Result<Self, ReceiveTimelineError> {
        let next_output_tick = i128::from(anchor.utc_unix_millis)
            .checked_mul(OUTPUT_TICKS_PER_MILLISECOND)
            .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
        Ok(Self {
            anchor,
            source_rate_hz,
            process_generation,
            stream_generation,
            next_output_tick,
            previous: None,
            active: None,
        })
    }

    fn push(
        &mut self,
        batch: &CaptureBatch,
    ) -> Result<Vec<ReceiveTimelineEvent>, ReceiveTimelineError> {
        let mut events = Vec::with_capacity(2);
        let first_position = batch.first_frame.position.frames();
        for (index, sample) in batch.samples().iter().copied().enumerate() {
            let position = first_position
                .checked_add(
                    u64::try_from(index).map_err(|_| ReceiveTimelineError::ArithmeticOverflow)?,
                )
                .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
            self.push_sample(position, sample, &mut events)?;
        }
        Ok(events)
    }

    fn push_sample(
        &mut self,
        position: u64,
        sample: i16,
        events: &mut Vec<ReceiveTimelineEvent>,
    ) -> Result<(), ReceiveTimelineError> {
        let position_delta = position
            .checked_sub(self.anchor.position.frames())
            .ok_or(ReceiveTimelineError::OutOfOrder)?;
        if self.previous.is_none() {
            self.previous = Some((position_delta, sample));
        }
        let current_source_tick = i128::from(position_delta)
            .checked_mul(i128::from(FT8_RECEIVE_SAMPLE_RATE_HZ))
            .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
        loop {
            let target_from_anchor = self
                .next_output_tick
                .checked_sub(
                    i128::from(self.anchor.utc_unix_millis)
                        .checked_mul(OUTPUT_TICKS_PER_MILLISECOND)
                        .ok_or(ReceiveTimelineError::ArithmeticOverflow)?,
                )
                .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
            let target_source_tick = target_from_anchor
                .checked_mul(i128::from(self.source_rate_hz))
                .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
            if target_source_tick > current_source_tick {
                break;
            }
            let (previous_position, previous_sample) = self
                .previous
                .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
            let previous_source_tick = i128::from(previous_position)
                .checked_mul(i128::from(FT8_RECEIVE_SAMPLE_RATE_HZ))
                .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
            if target_source_tick < previous_source_tick {
                return Err(ReceiveTimelineError::OutOfOrder);
            }
            let interpolated = if position_delta == previous_position {
                previous_sample
            } else {
                interpolate(
                    previous_sample,
                    sample,
                    target_source_tick - previous_source_tick,
                    current_source_tick - previous_source_tick,
                )?
            };
            self.push_output(interpolated, events)?;
            self.next_output_tick = self
                .next_output_tick
                .checked_add(1)
                .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
        }
        self.previous = Some((position_delta, sample));
        Ok(())
    }

    fn push_output(
        &mut self,
        sample: i16,
        events: &mut Vec<ReceiveTimelineEvent>,
    ) -> Result<(), ReceiveTimelineError> {
        let slot_tick =
            self.next_output_tick.div_euclid(OUTPUT_TICKS_PER_SLOT) * OUTPUT_TICKS_PER_SLOT;
        if self
            .active
            .as_ref()
            .is_none_or(|active| active.slot_tick != slot_tick)
        {
            if let Some(previous) = self.active.take()
                && let Some(event) = previous.incomplete_event()?
            {
                events.push(event);
            }
            self.active = Some(WindowAccumulator::new(
                slot_tick,
                self.next_output_tick,
                self.mapping_for_tick(slot_tick)?,
            )?);
        }
        let active = self
            .active
            .as_mut()
            .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
        active.samples.push(sample);
        if active.samples.len() == FT8_RECEIVE_WINDOW_SAMPLES {
            let complete = self
                .active
                .take()
                .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
            events.push(ReceiveTimelineEvent::Window(
                Ft8ReceiveWindow::new(
                    self.process_generation,
                    self.stream_generation,
                    complete.slot.start_utc_unix_millis(),
                    complete.mapping,
                    complete.samples,
                )
                .map_err(|_| ReceiveTimelineError::ArithmeticOverflow)?,
            ));
        }
        Ok(())
    }

    fn mapping_for_tick(
        &self,
        slot_tick: i128,
    ) -> Result<CaptureTimeEvidence, ReceiveTimelineError> {
        let anchor_tick = i128::from(self.anchor.utc_unix_millis)
            .checked_mul(OUTPUT_TICKS_PER_MILLISECOND)
            .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
        let tick_delta = slot_tick
            .checked_sub(anchor_tick)
            .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
        let source_delta = tick_delta
            .checked_mul(i128::from(self.source_rate_hz))
            .and_then(|value| value.checked_div(i128::from(FT8_RECEIVE_SAMPLE_RATE_HZ)))
            .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
        let source_delta =
            i64::try_from(source_delta).map_err(|_| ReceiveTimelineError::ArithmeticOverflow)?;
        let source_position = if source_delta >= 0 {
            self.anchor
                .position
                .frames()
                .checked_add(source_delta.unsigned_abs())
        } else {
            self.anchor
                .position
                .frames()
                .checked_sub(source_delta.unsigned_abs())
        };
        let Some(source_position) = source_position else {
            // A partial first slot can begin before the first retained source
            // frame. Its mapping is diagnostic only because the slot is
            // withheld; retain the actual anchor rather than inventing a
            // negative capture position.
            return Ok(self.anchor);
        };
        let slot_millis = i64::try_from(
            slot_tick
                .checked_div(OUTPUT_TICKS_PER_MILLISECOND)
                .ok_or(ReceiveTimelineError::ArithmeticOverflow)?,
        )
        .map_err(|_| ReceiveTimelineError::ArithmeticOverflow)?;
        let monotonic_delta = slot_millis - self.anchor.utc_unix_millis;
        let monotonic_millis = if monotonic_delta >= 0 {
            self.anchor
                .monotonic_millis
                .checked_add(monotonic_delta.unsigned_abs())
        } else {
            self.anchor
                .monotonic_millis
                .checked_sub(monotonic_delta.unsigned_abs())
        }
        .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
        CaptureTimeEvidence::new(
            CapturePosition::from_frames(source_position),
            slot_millis,
            monotonic_millis,
        )
        .map_err(|_| ReceiveTimelineError::ArithmeticOverflow)
    }

    fn discard_active(&mut self) -> Option<IncompleteFt8Slot> {
        let active = self.active.take()?;
        if active.samples.is_empty() {
            return None;
        }
        Some(IncompleteFt8Slot {
            slot: active.slot,
            reason: IncompleteSlotReason::TimelineInvalidated,
            accumulated_samples: u32::try_from(active.samples.len()).unwrap_or(u32::MAX),
        })
    }
}

struct WindowAccumulator {
    slot_tick: i128,
    slot: Ft8ReceiveSlot,
    first_tick: i128,
    mapping: CaptureTimeEvidence,
    samples: Vec<i16>,
}

impl WindowAccumulator {
    fn new(
        slot_tick: i128,
        first_tick: i128,
        mapping: CaptureTimeEvidence,
    ) -> Result<Self, ReceiveTimelineError> {
        let slot_millis = i64::try_from(
            slot_tick
                .checked_div(OUTPUT_TICKS_PER_MILLISECOND)
                .ok_or(ReceiveTimelineError::ArithmeticOverflow)?,
        )
        .map_err(|_| ReceiveTimelineError::ArithmeticOverflow)?;
        Ok(Self {
            slot_tick,
            slot: Ft8ReceiveSlot::new(slot_millis)?,
            first_tick,
            mapping,
            samples: Vec::with_capacity(FT8_RECEIVE_WINDOW_SAMPLES),
        })
    }

    fn incomplete_event(self) -> Result<Option<ReceiveTimelineEvent>, ReceiveTimelineError> {
        if self.samples.is_empty() {
            return Ok(None);
        }
        Ok(Some(ReceiveTimelineEvent::Incomplete(IncompleteFt8Slot {
            slot: self.slot,
            reason: if self.first_tick == self.slot_tick {
                IncompleteSlotReason::TimelineInvalidated
            } else {
                IncompleteSlotReason::CaptureStartedLate
            },
            accumulated_samples: u32::try_from(self.samples.len())
                .map_err(|_| ReceiveTimelineError::ArithmeticOverflow)?,
        })))
    }
}

fn interpolate(
    previous: i16,
    current: i16,
    numerator: i128,
    denominator: i128,
) -> Result<i16, ReceiveTimelineError> {
    if denominator <= 0 || numerator < 0 || numerator > denominator {
        return Err(ReceiveTimelineError::ArithmeticOverflow);
    }
    let previous = i128::from(previous);
    let difference = i128::from(current) - previous;
    let adjustment = difference
        .checked_mul(numerator)
        .ok_or(ReceiveTimelineError::ArithmeticOverflow)?;
    let rounded = if adjustment >= 0 {
        adjustment + denominator / 2
    } else {
        adjustment - denominator / 2
    } / denominator;
    i16::try_from(previous + rounded).map_err(|_| ReceiveTimelineError::ArithmeticOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CaptureDiagnostics, CaptureDiscontinuity, InputSampleFormat, ReceiveAudioError};

    fn configuration(rate: u32) -> InputConfiguration {
        InputConfiguration::new(rate, 1, InputSampleFormat::Signed16, 0).unwrap()
    }

    fn batch(
        rate: u32,
        position: u64,
        frames: usize,
        utc_start: i64,
        monotonic_start: u64,
        discontinuity: Option<CaptureDiscontinuity>,
    ) -> CaptureBatch {
        let elapsed = position.saturating_mul(1_000) / u64::from(rate);
        CaptureBatch::new(
            ProcessGeneration::new(1).unwrap(),
            StreamGeneration::new(1).unwrap(),
            configuration(rate),
            CaptureTimeEvidence::new(
                CapturePosition::from_frames(position),
                utc_start + i64::try_from(elapsed).unwrap(),
                monotonic_start + elapsed,
            )
            .unwrap(),
            discontinuity,
            CaptureDiagnostics::new(0, 1).unwrap(),
            (0..frames)
                .map(|index| i16::try_from(index % 2_000).unwrap() - 1_000)
                .collect(),
        )
        .unwrap()
    }

    fn run_duration(rate: u32, utc_start: i64, duration_millis: u64) -> Vec<ReceiveTimelineEvent> {
        let mut timeline = Ft8ReceiveTimeline::new(
            ProcessGeneration::new(1).unwrap(),
            StreamGeneration::new(1).unwrap(),
            configuration(rate),
        );
        // Rational upsampling may need the first source sample after the slot
        // to interpolate the final canonical sample.
        let total = usize::try_from(
            u64::from(rate)
                .saturating_mul(duration_millis)
                .saturating_div(1_000),
        )
        .unwrap()
            + 1;
        let mut position = 0usize;
        let mut events = Vec::new();
        while position < total {
            let frames = (total - position).min(4_096);
            let discontinuity = (position == 0).then_some(CaptureDiscontinuity {
                at: CapturePosition::from_frames(0),
                kind: CaptureDiscontinuityKind::StreamRestart,
                dropped_frames: 0,
            });
            let batch = batch(
                rate,
                position as u64,
                frames,
                utc_start,
                1_000,
                discontinuity,
            );
            events.extend(
                timeline
                    .push(&batch, batch.first_frame.monotonic_millis)
                    .unwrap(),
            );
            position += frames;
        }
        events
    }

    fn run_slot(rate: u32, utc_start: i64) -> Vec<ReceiveTimelineEvent> {
        run_duration(rate, utc_start, 15_000)
    }

    #[test]
    fn exact_boundary_common_rates_produce_one_canonical_window() {
        for rate in [8_000, 12_000, 44_100, 48_000, 96_000] {
            let events = run_slot(rate, 30_000);
            let windows = events
                .iter()
                .filter_map(|event| match event {
                    ReceiveTimelineEvent::Window(window) => Some(window),
                    ReceiveTimelineEvent::Incomplete(_) => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(windows.len(), 1, "rate {rate}");
            assert_eq!(windows[0].slot_start_utc_millis, 30_000);
            assert_eq!(windows[0].samples().len(), FT8_RECEIVE_WINDOW_SAMPLES);
            assert_eq!(windows[0].sample_rate_hz(), 12_000);
        }
    }

    #[test]
    fn post_boundary_capture_is_withheld_not_padded() {
        let events = run_slot(12_000, 30_100);
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, ReceiveTimelineEvent::Window(_)))
        );
    }

    #[test]
    fn pre_boundary_capture_discards_only_the_partial_lead_in() {
        let events = run_duration(12_000, 29_900, 15_100);
        assert!(matches!(
            events.first(),
            Some(ReceiveTimelineEvent::Incomplete(IncompleteFt8Slot {
                slot,
                reason: IncompleteSlotReason::CaptureStartedLate,
                ..
            })) if slot.start_utc_unix_millis() == 15_000
        ));
        let window = events.iter().find_map(|event| match event {
            ReceiveTimelineEvent::Window(window) => Some(window),
            ReceiveTimelineEvent::Incomplete(_) => None,
        });
        assert_eq!(window.unwrap().slot_start_utc_millis, 30_000);
    }

    #[test]
    fn gap_overlap_out_of_order_and_discontinuity_are_typed() {
        let mut timeline = Ft8ReceiveTimeline::new(
            ProcessGeneration::new(1).unwrap(),
            StreamGeneration::new(1).unwrap(),
            configuration(48_000),
        );
        let first = batch(48_000, 0, 100, 30_000, 1_000, None);
        timeline.push(&first, 1_000).unwrap();
        let overlap = batch(48_000, 99, 10, 30_000, 1_000, None);
        assert_eq!(
            timeline.push(&overlap, overlap.first_frame.monotonic_millis),
            Err(ReceiveTimelineError::Overlap { frames: 1 })
        );
        let gap = batch(48_000, 101, 10, 30_000, 1_000, None);
        assert_eq!(
            timeline.push(&gap, gap.first_frame.monotonic_millis),
            Err(ReceiveTimelineError::Gap { frames: 1 })
        );
        let mut out_of_order_timeline = Ft8ReceiveTimeline::new(
            ProcessGeneration::new(1).unwrap(),
            StreamGeneration::new(1).unwrap(),
            configuration(48_000),
        );
        let later = batch(48_000, 100, 10, 30_000, 1_000, None);
        out_of_order_timeline
            .push(&later, later.first_frame.monotonic_millis)
            .unwrap();
        let out_of_order = batch(48_000, 0, 10, 30_000, 1_000, None);
        assert_eq!(
            out_of_order_timeline.push(&out_of_order, out_of_order.first_frame.monotonic_millis),
            Err(ReceiveTimelineError::OutOfOrder)
        );
        let marked = batch(
            48_000,
            111,
            10,
            30_000,
            1_000,
            Some(CaptureDiscontinuity {
                at: CapturePosition::from_frames(111),
                kind: CaptureDiscontinuityKind::Overflow,
                dropped_frames: 1,
            }),
        );
        assert_eq!(
            timeline.push(&marked, marked.first_frame.monotonic_millis),
            Err(ReceiveTimelineError::Discontinuity(
                CaptureDiscontinuityKind::Overflow
            ))
        );
    }

    #[test]
    fn jitter_drift_clock_jump_and_late_data_are_deterministic() {
        let mut timeline = Ft8ReceiveTimeline::new(
            ProcessGeneration::new(1).unwrap(),
            StreamGeneration::new(1).unwrap(),
            configuration(48_000),
        );
        let first = batch(48_000, 0, 100, 30_000, 1_000, None);
        timeline.push(&first, 1_000).unwrap();

        let mut jitter = batch(48_000, 100, 100, 30_000, 1_000, None);
        jitter.first_frame.monotonic_millis += 10;
        jitter.first_frame.utc_unix_millis += 10;
        timeline
            .push(&jitter, jitter.first_frame.monotonic_millis)
            .unwrap();
        assert_eq!(timeline.health().max_jitter_millis, 10);

        let mut remapped = batch(48_000, 200, 100, 30_000, 1_000, None);
        remapped.first_frame.monotonic_millis += 12;
        remapped.first_frame.utc_unix_millis += 62;
        assert!(matches!(
            timeline.push(&remapped, remapped.first_frame.monotonic_millis),
            Err(ReceiveTimelineError::ClockRemapped { .. })
        ));

        let mut late_timeline = Ft8ReceiveTimeline::new(
            ProcessGeneration::new(1).unwrap(),
            StreamGeneration::new(1).unwrap(),
            configuration(48_000),
        );
        assert_eq!(
            late_timeline.push(&first, 3_001),
            Err(ReceiveTimelineError::LateData { millis: 2_001 })
        );
        assert_eq!(late_timeline.health().late_batch_count, 1);

        let mut drift_timeline = Ft8ReceiveTimeline::new(
            ProcessGeneration::new(1).unwrap(),
            StreamGeneration::new(1).unwrap(),
            configuration(48_000),
        );
        drift_timeline.push(&first, 1_000).unwrap();
        let mut position = 100_u64;
        while position < 48_100 {
            let frames = usize::try_from((48_100 - position).min(8_192)).unwrap();
            let ordinary = batch(48_000, position, frames, 30_000, 1_000, None);
            drift_timeline
                .push(&ordinary, ordinary.first_frame.monotonic_millis)
                .unwrap();
            position += u64::try_from(frames).unwrap();
        }
        let mut drifted = batch(48_000, 48_100, 100, 30_000, 1_000, None);
        drifted.first_frame.monotonic_millis += 10;
        drifted.first_frame.utc_unix_millis += 10;
        assert!(matches!(
            drift_timeline.push(&drifted, drifted.first_frame.monotonic_millis),
            Err(ReceiveTimelineError::ExcessiveDrift { .. })
        ));
    }

    #[test]
    fn generation_configuration_and_restart_boundaries_do_not_share_state() {
        let mut timeline = Ft8ReceiveTimeline::new(
            ProcessGeneration::new(1).unwrap(),
            StreamGeneration::new(1).unwrap(),
            configuration(48_000),
        );
        let mut wrong_stream = batch(48_000, 0, 10, 30_000, 1_000, None);
        wrong_stream.stream_generation = StreamGeneration::new(2).unwrap();
        assert_eq!(
            timeline.push(&wrong_stream, 1_000),
            Err(ReceiveTimelineError::StreamGenerationChanged)
        );
        let wrong_config = batch(44_100, 0, 10, 30_000, 1_000, None);
        assert_eq!(
            timeline.push(&wrong_config, 1_000),
            Err(ReceiveTimelineError::ConfigurationChanged)
        );
    }

    #[test]
    fn interpolation_is_checked_and_reproducible() {
        assert_eq!(interpolate(-1_000, 1_000, 1, 2).unwrap(), 0);
        assert_eq!(interpolate(i16::MIN, i16::MAX, 1, 2).unwrap(), 0);
        assert_eq!(
            interpolate(0, 1, 2, 1),
            Err(ReceiveTimelineError::ArithmeticOverflow)
        );
        assert_eq!(
            CaptureBatch::new(
                ProcessGeneration::new(1).unwrap(),
                StreamGeneration::new(1).unwrap(),
                configuration(48_000),
                CaptureTimeEvidence::new(CapturePosition::from_frames(0), 30_000, 1_000).unwrap(),
                None,
                CaptureDiagnostics::new(0, 0).unwrap(),
                Vec::new(),
            ),
            Err(ReceiveAudioError::InvalidBatchShape)
        );
    }
}
