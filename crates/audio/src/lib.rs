//! SlotPilot-owned receive-only audio and capture-timeline contracts.
//!
//! This crate contains checked values only. It cannot enumerate or open a
//! device, allocate from a real-time callback, decode protocol data, persist
//! records, control a rig, play audio, key PTT, or transmit.

use thiserror::Error;

mod capture;
mod discovery;
mod timeline;

pub use capture::{
    CALLBACK_DELAY_FAULT_MILLIS, DEFAULT_CAPTURE_QUEUE_BATCHES, InputCaptureError,
    MAX_CAPTURE_QUEUE_BATCHES, SystemInputCapture,
};
pub use discovery::SystemInputDiscovery;
pub use timeline::{
    Ft8ReceiveSlot, Ft8ReceiveTimeline, IncompleteFt8Slot, IncompleteSlotReason,
    MAX_RECEIVE_BATCH_LATENESS_MILLIS, MAX_RECEIVE_CLOCK_REMAP_MILLIS,
    MAX_RECEIVE_DRIFT_PARTS_PER_MILLION, MAX_RECEIVE_JITTER_MILLIS, ReceiveTimelineError,
    ReceiveTimelineEvent, ReceiveTimelineHealth,
};

/// Canonical FT8 receive sample rate shared with the offline protocol contract.
pub const FT8_RECEIVE_SAMPLE_RATE_HZ: u32 = 12_000;
/// Canonical number of mono samples in one complete FT8 receive slot.
pub const FT8_RECEIVE_WINDOW_SAMPLES: usize = 180_000;
/// Nominal FT8 slot duration in milliseconds.
pub const FT8_RECEIVE_SLOT_MILLIS: i64 = 15_000;
/// Maximum frames accepted in one callback-to-worker handoff.
pub const MAX_CAPTURE_BATCH_FRAMES: u32 = 8_192;
/// Maximum channel count represented by a receive configuration.
pub const MAX_INPUT_CHANNELS: u16 = 32;
/// Maximum configurations retained for one discovered input device.
pub const MAX_DEVICE_CONFIGURATIONS: usize = 64;

const MAX_IDENTITY_BYTES: usize = 256;
const MAX_DISPLAY_NAME_BYTES: usize = 256;
const MIN_SAMPLE_RATE_HZ: u32 = 8_000;
const MAX_SAMPLE_RATE_HZ: u32 = 384_000;
const MAX_CALLBACK_DELAY_MILLIS: u32 = 60_000;
const MAX_LATENCY_MILLIS: u32 = 60_000;
const MAX_DRIFT_PARTS_PER_MILLION: i32 = 100_000;

/// Operating-system audio family associated with a stable input identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InputPlatform {
    /// Core Audio on macOS.
    MacOsCoreAudio,
    /// WASAPI on Windows.
    WindowsWasapi,
    /// ALSA on Linux.
    LinuxAlsa,
    /// JACK on Linux.
    LinuxJack,
}

/// Stable platform identity for one input device.
///
/// Display metadata has no conversion into this type. Callers must obtain the
/// opaque value from a platform adapter that can provide a durable identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InputDeviceIdentity {
    platform: InputPlatform,
    opaque_id: String,
}

impl InputDeviceIdentity {
    /// Constructs a checked stable platform identity.
    pub fn new(
        platform: InputPlatform,
        opaque_id: impl Into<String>,
    ) -> Result<Self, ReceiveAudioError> {
        let opaque_id = opaque_id.into();
        validate_bounded_text(&opaque_id, MAX_IDENTITY_BYTES)
            .map_err(|()| ReceiveAudioError::InvalidStableIdentity)?;
        Ok(Self {
            platform,
            opaque_id,
        })
    }

    /// Returns the platform family that defined this identity.
    #[must_use]
    pub const fn platform(&self) -> InputPlatform {
        self.platform
    }

    /// Returns the platform-owned opaque identity.
    #[must_use]
    pub fn opaque_id(&self) -> &str {
        &self.opaque_id
    }
}

/// Human-facing input-device metadata that is never a selection identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceDisplay {
    name: String,
    manufacturer: Option<String>,
}

impl InputDeviceDisplay {
    /// Constructs bounded display metadata.
    pub fn new(
        name: impl Into<String>,
        manufacturer: Option<String>,
    ) -> Result<Self, ReceiveAudioError> {
        let name = name.into();
        validate_bounded_text(&name, MAX_DISPLAY_NAME_BYTES)
            .map_err(|()| ReceiveAudioError::InvalidDisplayMetadata)?;
        if let Some(value) = manufacturer.as_deref() {
            validate_bounded_text(value, MAX_DISPLAY_NAME_BYTES)
                .map_err(|()| ReceiveAudioError::InvalidDisplayMetadata)?;
        }
        Ok(Self { name, manufacturer })
    }

    /// Returns the non-identifying display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns optional non-identifying manufacturer text.
    #[must_use]
    pub fn manufacturer(&self) -> Option<&str> {
        self.manufacturer.as_deref()
    }
}

/// Owned sample representation reported by an input configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InputSampleFormat {
    /// Signed 8-bit integer samples.
    Signed8,
    /// Signed 16-bit integer samples.
    Signed16,
    /// Signed 24-bit integer samples in a 32-bit container.
    Signed24,
    /// Signed 32-bit integer samples.
    Signed32,
    /// Signed 64-bit integer samples.
    Signed64,
    /// Unsigned 8-bit integer samples.
    Unsigned8,
    /// Unsigned 16-bit integer samples.
    Unsigned16,
    /// Unsigned 24-bit integer samples in a 32-bit container.
    Unsigned24,
    /// Unsigned 32-bit integer samples.
    Unsigned32,
    /// Unsigned 64-bit integer samples.
    Unsigned64,
    /// IEEE 754 32-bit floating-point samples.
    Float32,
    /// IEEE 754 64-bit floating-point samples.
    Float64,
}

/// Checked range of input configurations exposed by discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InputConfigurationRange {
    min_sample_rate_hz: u32,
    max_sample_rate_hz: u32,
    channels: u16,
    sample_format: InputSampleFormat,
}

impl InputConfigurationRange {
    /// Constructs a checked inclusive sample-rate range.
    pub fn new(
        min_sample_rate_hz: u32,
        max_sample_rate_hz: u32,
        channels: u16,
        sample_format: InputSampleFormat,
    ) -> Result<Self, ReceiveAudioError> {
        if min_sample_rate_hz > max_sample_rate_hz
            || !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&min_sample_rate_hz)
            || !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&max_sample_rate_hz)
        {
            return Err(ReceiveAudioError::InvalidSampleRate);
        }
        if channels == 0 || channels > MAX_INPUT_CHANNELS {
            return Err(ReceiveAudioError::InvalidChannelCount);
        }
        Ok(Self {
            min_sample_rate_hz,
            max_sample_rate_hz,
            channels,
            sample_format,
        })
    }

    /// Returns the inclusive minimum supported sample rate.
    #[must_use]
    pub const fn min_sample_rate_hz(self) -> u32 {
        self.min_sample_rate_hz
    }

    /// Returns the inclusive maximum supported sample rate.
    #[must_use]
    pub const fn max_sample_rate_hz(self) -> u32 {
        self.max_sample_rate_hz
    }

    /// Returns the supported interleaved channel count.
    #[must_use]
    pub const fn channels(self) -> u16 {
        self.channels
    }

    /// Returns the supported source sample representation.
    #[must_use]
    pub const fn sample_format(self) -> InputSampleFormat {
        self.sample_format
    }

    /// Selects one exact sample rate and zero-based channel from this range.
    pub fn select(
        self,
        sample_rate_hz: u32,
        selected_channel: u16,
    ) -> Result<InputConfiguration, ReceiveAudioError> {
        if !(self.min_sample_rate_hz..=self.max_sample_rate_hz).contains(&sample_rate_hz) {
            return Err(ReceiveAudioError::InvalidSampleRate);
        }
        InputConfiguration::new(
            sample_rate_hz,
            self.channels,
            self.sample_format,
            selected_channel,
        )
    }
}

/// Checked receive-only input stream configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputConfiguration {
    sample_rate_hz: u32,
    channels: u16,
    sample_format: InputSampleFormat,
    selected_channel: u16,
}

impl InputConfiguration {
    /// Constructs a checked input configuration and selected zero-based channel.
    pub fn new(
        sample_rate_hz: u32,
        channels: u16,
        sample_format: InputSampleFormat,
        selected_channel: u16,
    ) -> Result<Self, ReceiveAudioError> {
        if !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&sample_rate_hz) {
            return Err(ReceiveAudioError::InvalidSampleRate);
        }
        if channels == 0 || channels > MAX_INPUT_CHANNELS {
            return Err(ReceiveAudioError::InvalidChannelCount);
        }
        if selected_channel >= channels {
            return Err(ReceiveAudioError::InvalidSelectedChannel);
        }
        Ok(Self {
            sample_rate_hz,
            channels,
            sample_format,
            selected_channel,
        })
    }

    /// Returns source frames per second.
    #[must_use]
    pub const fn sample_rate_hz(self) -> u32 {
        self.sample_rate_hz
    }

    /// Returns the interleaved source channel count.
    #[must_use]
    pub const fn channels(self) -> u16 {
        self.channels
    }

    /// Returns the source sample representation.
    #[must_use]
    pub const fn sample_format(self) -> InputSampleFormat {
        self.sample_format
    }

    /// Returns the selected zero-based receive channel.
    #[must_use]
    pub const fn selected_channel(self) -> u16 {
        self.selected_channel
    }
}

/// Discovered receive-only device plus its bounded supported configurations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceDescriptor {
    identity: InputDeviceIdentity,
    display: InputDeviceDisplay,
    configuration_ranges: Vec<InputConfigurationRange>,
}

impl InputDeviceDescriptor {
    /// Constructs a descriptor with at least one bounded configuration.
    pub fn new(
        identity: InputDeviceIdentity,
        display: InputDeviceDisplay,
        configuration_ranges: Vec<InputConfigurationRange>,
    ) -> Result<Self, ReceiveAudioError> {
        if configuration_ranges.is_empty() || configuration_ranges.len() > MAX_DEVICE_CONFIGURATIONS
        {
            return Err(ReceiveAudioError::InvalidConfigurationCount);
        }
        Ok(Self {
            identity,
            display,
            configuration_ranges,
        })
    }

    /// Returns the only value permitted for explicit device selection.
    #[must_use]
    pub const fn identity(&self) -> &InputDeviceIdentity {
        &self.identity
    }

    /// Returns human-facing metadata, never a fallback identity.
    #[must_use]
    pub const fn display(&self) -> &InputDeviceDisplay {
        &self.display
    }

    /// Returns bounded supported receive configurations.
    #[must_use]
    pub fn configuration_ranges(&self) -> &[InputConfigurationRange] {
        &self.configuration_ranges
    }
}

/// Receive-only device discovery boundary.
pub trait InputDeviceDiscovery {
    /// Enumerates all input-capable devices with stable identities.
    fn enumerate(&self) -> Result<Vec<InputDeviceDescriptor>, InputDiscoveryError>;

    /// Looks up one exact stable identity without default or name fallback.
    fn find(
        &self,
        identity: &InputDeviceIdentity,
    ) -> Result<InputDeviceDescriptor, InputDiscoveryError>;
}

/// Typed failure from receive-only device discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum InputDiscoveryError {
    /// The platform audio host is not available.
    #[error("input audio host is unavailable")]
    HostUnavailable,
    /// The operating system denied input-device access.
    #[error("input audio permission denied")]
    PermissionDenied,
    /// A previously visible or selected device disappeared.
    #[error("input device disappeared")]
    DeviceDisappeared,
    /// A device exposes no supported bounded PCM configuration.
    #[error("input device has no supported configuration")]
    UnsupportedConfiguration,
    /// The backend could not provide a stable identity.
    #[error("stable input device identity is unavailable")]
    IdentityUnavailable,
    /// No input-capable devices were found.
    #[error("no input-capable device was found")]
    NoInputDevices,
    /// The backend failed without a stable implementation type escaping.
    #[error("input discovery backend failed")]
    BackendFailure,
}

/// Daemon-process generation for capture state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProcessGeneration(u64);

impl ProcessGeneration {
    /// Constructs a non-zero process generation.
    pub const fn new(value: u64) -> Result<Self, ReceiveAudioError> {
        if value == 0 {
            return Err(ReceiveAudioError::InvalidGeneration);
        }
        Ok(Self(value))
    }

    /// Returns the opaque generation value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Stream generation scoped to one daemon process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StreamGeneration(u64);

impl StreamGeneration {
    /// Constructs a non-zero stream generation.
    pub const fn new(value: u64) -> Result<Self, ReceiveAudioError> {
        if value == 0 {
            return Err(ReceiveAudioError::InvalidGeneration);
        }
        Ok(Self(value))
    }

    /// Returns the opaque generation value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Monotonic position in source frames since a stream generation began.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CapturePosition(u64);

impl CapturePosition {
    /// Constructs a source-frame position.
    #[must_use]
    pub const fn from_frames(value: u64) -> Self {
        Self(value)
    }

    /// Returns source frames since stream start.
    #[must_use]
    pub const fn frames(self) -> u64 {
        self.0
    }

    /// Returns a checked position advanced by a frame count.
    pub fn checked_advance(self, frames: u32) -> Result<Self, ReceiveAudioError> {
        self.0
            .checked_add(u64::from(frames))
            .map(Self)
            .ok_or(ReceiveAudioError::PositionOverflow)
    }
}

/// Simultaneous capture-position, UTC, and monotonic mapping evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureTimeEvidence {
    /// Source-frame position represented by the sample.
    pub position: CapturePosition,
    /// UTC milliseconds since the Unix epoch.
    pub utc_unix_millis: i64,
    /// Monotonic milliseconds from the current process origin.
    pub monotonic_millis: u64,
}

impl CaptureTimeEvidence {
    /// Constructs checked timing evidence.
    pub const fn new(
        position: CapturePosition,
        utc_unix_millis: i64,
        monotonic_millis: u64,
    ) -> Result<Self, ReceiveAudioError> {
        if utc_unix_millis < 0 {
            return Err(ReceiveAudioError::InvalidUtcTimestamp);
        }
        Ok(Self {
            position,
            utc_unix_millis,
            monotonic_millis,
        })
    }
}

/// Explicit reason continuity was lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureDiscontinuityKind {
    /// Bounded queue overflow dropped source frames.
    Overflow,
    /// The backend reported a gap.
    BackendGap,
    /// Capture restarted with a new stream generation.
    StreamRestart,
    /// Clock evidence became uncertain.
    ClockRemapped,
}

/// Explicit capture discontinuity at a source-frame position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureDiscontinuity {
    /// Position at which continuity ceased.
    pub at: CapturePosition,
    /// Specific reason for the discontinuity.
    pub kind: CaptureDiscontinuityKind,
    /// Known dropped frames, or zero when the size is unknown.
    pub dropped_frames: u64,
}

/// Bounded callback diagnostic counters attached to one batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureDiagnostics {
    /// Samples at or beyond the clipping threshold.
    pub clipped_samples: u32,
    /// Observed callback delay.
    pub callback_delay_millis: u32,
}

impl CaptureDiagnostics {
    /// Constructs bounded callback diagnostics.
    pub const fn new(
        clipped_samples: u32,
        callback_delay_millis: u32,
    ) -> Result<Self, ReceiveAudioError> {
        if callback_delay_millis > MAX_CALLBACK_DELAY_MILLIS {
            return Err(ReceiveAudioError::InvalidCallbackDelay);
        }
        Ok(Self {
            clipped_samples,
            callback_delay_millis,
        })
    }
}

/// One bounded, normalized mono signed-16-bit callback-to-worker handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureBatch {
    /// Daemon-process generation.
    pub process_generation: ProcessGeneration,
    /// Input-stream generation.
    pub stream_generation: StreamGeneration,
    /// Exact selected input configuration.
    pub configuration: InputConfiguration,
    /// Timing evidence for the first source frame.
    pub first_frame: CaptureTimeEvidence,
    /// Optional explicit discontinuity preceding this batch.
    pub discontinuity: Option<CaptureDiscontinuity>,
    /// Bounded callback diagnostics.
    pub diagnostics: CaptureDiagnostics,
    samples: Vec<i16>,
}

impl CaptureBatch {
    /// Constructs a bounded interleaved batch.
    ///
    /// Construction and allocation occur outside the real-time callback. A
    /// future callback adapter may only move already-preallocated storage.
    pub fn new(
        process_generation: ProcessGeneration,
        stream_generation: StreamGeneration,
        configuration: InputConfiguration,
        first_frame: CaptureTimeEvidence,
        discontinuity: Option<CaptureDiscontinuity>,
        diagnostics: CaptureDiagnostics,
        samples: Vec<i16>,
    ) -> Result<Self, ReceiveAudioError> {
        if samples.is_empty() {
            return Err(ReceiveAudioError::InvalidBatchShape);
        }
        if samples.len() > MAX_CAPTURE_BATCH_FRAMES as usize {
            return Err(ReceiveAudioError::BatchTooLarge);
        }
        if first_frame.position != discontinuity.map_or(first_frame.position, |value| value.at) {
            return Err(ReceiveAudioError::DiscontinuityPositionMismatch);
        }
        if usize::try_from(diagnostics.clipped_samples).map_or(true, |count| count > samples.len())
        {
            return Err(ReceiveAudioError::InvalidClippedSampleCount);
        }
        Ok(Self {
            process_generation,
            stream_generation,
            configuration,
            first_frame,
            discontinuity,
            diagnostics,
            samples,
        })
    }

    /// Returns normalized samples from the exact selected source channel.
    #[must_use]
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }

    /// Returns source frames in this batch.
    #[must_use]
    pub fn frame_count(&self) -> u32 {
        u32::try_from(self.samples.len()).unwrap_or(MAX_CAPTURE_BATCH_FRAMES)
    }

    /// Returns the position immediately after this batch.
    pub fn end_position(&self) -> Result<CapturePosition, ReceiveAudioError> {
        self.first_frame
            .position
            .checked_advance(self.frame_count())
    }
}

/// Checked canonical mono FT8 receive window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ft8ReceiveWindow {
    /// Daemon-process generation.
    pub process_generation: ProcessGeneration,
    /// Input-stream generation.
    pub stream_generation: StreamGeneration,
    /// UTC start of the aligned 15-second FT8 slot.
    pub slot_start_utc_millis: i64,
    /// Capture mapping evidence used to assemble the window.
    pub mapping: CaptureTimeEvidence,
    samples: Vec<i16>,
}

impl Ft8ReceiveWindow {
    /// Constructs one complete canonical decoder input.
    pub fn new(
        process_generation: ProcessGeneration,
        stream_generation: StreamGeneration,
        slot_start_utc_millis: i64,
        mapping: CaptureTimeEvidence,
        samples: Vec<i16>,
    ) -> Result<Self, ReceiveAudioError> {
        if slot_start_utc_millis < 0 || slot_start_utc_millis % FT8_RECEIVE_SLOT_MILLIS != 0 {
            return Err(ReceiveAudioError::MisalignedFt8Window);
        }
        if samples.len() != FT8_RECEIVE_WINDOW_SAMPLES {
            return Err(ReceiveAudioError::InvalidFt8WindowSize);
        }
        Ok(Self {
            process_generation,
            stream_generation,
            slot_start_utc_millis,
            mapping,
            samples,
        })
    }

    /// Returns canonical mono signed-16-bit 12 kHz samples.
    #[must_use]
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }

    /// Returns the canonical receive sample rate.
    #[must_use]
    pub const fn sample_rate_hz(&self) -> u32 {
        FT8_RECEIVE_SAMPLE_RATE_HZ
    }
}

/// Observable receive-input health.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputHealth {
    /// Estimated input latency.
    pub latency_millis: u32,
    /// Estimated signed source-clock drift.
    pub drift_parts_per_million: i32,
    /// Number of bounded queue overflows in the generation.
    pub overflow_count: u64,
    /// Number of clipped samples in the generation.
    pub clipped_sample_count: u64,
    /// Greatest observed callback delay.
    pub max_callback_delay_millis: u32,
}

impl InputHealth {
    /// Constructs checked health evidence.
    pub const fn new(
        latency_millis: u32,
        drift_parts_per_million: i32,
        overflow_count: u64,
        clipped_sample_count: u64,
        max_callback_delay_millis: u32,
    ) -> Result<Self, ReceiveAudioError> {
        if latency_millis > MAX_LATENCY_MILLIS {
            return Err(ReceiveAudioError::InvalidLatency);
        }
        if drift_parts_per_million < -MAX_DRIFT_PARTS_PER_MILLION
            || drift_parts_per_million > MAX_DRIFT_PARTS_PER_MILLION
        {
            return Err(ReceiveAudioError::InvalidDrift);
        }
        if max_callback_delay_millis > MAX_CALLBACK_DELAY_MILLIS {
            return Err(ReceiveAudioError::InvalidCallbackDelay);
        }
        Ok(Self {
            latency_millis,
            drift_parts_per_million,
            overflow_count,
            clipped_sample_count,
            max_callback_delay_millis,
        })
    }
}

/// Typed receive-only input fault kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFaultKind {
    /// Access to input devices was denied.
    PermissionDenied,
    /// The explicitly configured device disappeared.
    DeviceLost,
    /// The bounded queue dropped source frames.
    Overflow {
        /// Known number of dropped source frames.
        dropped_frames: u64,
    },
    /// Capture continuity became uncertain.
    Discontinuity(CaptureDiscontinuityKind),
    /// Samples crossed the clipping threshold.
    Clipping {
        /// Number of clipped samples.
        sample_count: u32,
    },
    /// Source-clock drift exceeded the configured bound.
    Drift {
        /// Observed signed drift.
        parts_per_million: i32,
    },
    /// Callback execution was delayed beyond its configured bound.
    CallbackDelay {
        /// Observed callback delay.
        millis: u32,
    },
    /// Input backend failed without a stable backend-specific type escaping.
    BackendFailure,
}

/// Timestamped receive-only input fault.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("input fault at monotonic {monotonic_millis} ms: {kind:?}")]
pub struct InputFault {
    /// Daemon-process generation.
    pub process_generation: ProcessGeneration,
    /// Input-stream generation, if a stream had started.
    pub stream_generation: Option<StreamGeneration>,
    /// Process-local monotonic occurrence time.
    pub monotonic_millis: u64,
    /// Specific owned fault kind.
    pub kind: InputFaultKind,
}

/// Stateful validator for capture ordering within one stream generation.
#[derive(Debug, Clone, Copy)]
pub struct CaptureSequence {
    process_generation: ProcessGeneration,
    stream_generation: StreamGeneration,
    next_position: CapturePosition,
    last_monotonic_millis: u64,
}

impl CaptureSequence {
    /// Starts a sequence from a first batch.
    pub fn start(first: &CaptureBatch) -> Result<Self, CaptureSequenceError> {
        Ok(Self {
            process_generation: first.process_generation,
            stream_generation: first.stream_generation,
            next_position: first
                .end_position()
                .map_err(|_| CaptureSequenceError::PositionOverflow)?,
            last_monotonic_millis: first.first_frame.monotonic_millis,
        })
    }

    /// Validates and advances over one contiguous batch.
    pub fn observe(&mut self, batch: &CaptureBatch) -> Result<(), CaptureSequenceError> {
        if batch.process_generation != self.process_generation
            || batch.stream_generation != self.stream_generation
        {
            return Err(CaptureSequenceError::GenerationChanged);
        }
        if batch.first_frame.position < self.next_position {
            return Err(CaptureSequenceError::PositionRegressed);
        }
        if batch.first_frame.position > self.next_position && batch.discontinuity.is_none() {
            return Err(CaptureSequenceError::UnmarkedGap);
        }
        if batch.first_frame.monotonic_millis < self.last_monotonic_millis {
            return Err(CaptureSequenceError::MonotonicRegressed);
        }
        self.next_position = batch
            .end_position()
            .map_err(|_| CaptureSequenceError::PositionOverflow)?;
        self.last_monotonic_millis = batch.first_frame.monotonic_millis;
        Ok(())
    }
}

/// Failure validating capture sequence ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CaptureSequenceError {
    /// A batch belongs to a different process or stream.
    #[error("capture generation changed")]
    GenerationChanged,
    /// A source-frame position overlapped or moved backwards.
    #[error("capture position regressed")]
    PositionRegressed,
    /// A source-frame gap lacked explicit discontinuity evidence.
    #[error("capture sequence contains an unmarked gap")]
    UnmarkedGap,
    /// Monotonic evidence moved backwards.
    #[error("capture monotonic time regressed")]
    MonotonicRegressed,
    /// A position could not be advanced within its representation.
    #[error("capture position overflow")]
    PositionOverflow,
}

/// Failure constructing a receive-audio contract value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ReceiveAudioError {
    /// Stable platform identity was empty, invalid, or oversized.
    #[error("stable input identity must be bounded non-control text")]
    InvalidStableIdentity,
    /// Display metadata was empty, invalid, or oversized.
    #[error("input display metadata must be bounded printable text")]
    InvalidDisplayMetadata,
    /// Sample rate falls outside the supported owned range.
    #[error("input sample rate is outside the supported range")]
    InvalidSampleRate,
    /// Channel count is zero or exceeds the owned bound.
    #[error("input channel count is outside the supported range")]
    InvalidChannelCount,
    /// Selected channel does not exist in the input configuration.
    #[error("selected input channel is out of range")]
    InvalidSelectedChannel,
    /// A descriptor had zero or too many configurations.
    #[error("input descriptor configuration count is invalid")]
    InvalidConfigurationCount,
    /// A process or stream generation was zero.
    #[error("capture generations must be non-zero")]
    InvalidGeneration,
    /// UTC capture evidence predates the supported epoch.
    #[error("capture UTC timestamp must not precede the Unix epoch")]
    InvalidUtcTimestamp,
    /// Callback-delay evidence exceeded the owned representation.
    #[error("callback delay is outside the supported range")]
    InvalidCallbackDelay,
    /// Input-latency evidence exceeded the owned representation.
    #[error("input latency is outside the supported range")]
    InvalidLatency,
    /// Drift evidence exceeded the owned representation.
    #[error("input drift is outside the supported range")]
    InvalidDrift,
    /// Batch samples were empty or not aligned to the channel count.
    #[error("capture batch sample shape is invalid")]
    InvalidBatchShape,
    /// Batch exceeded the callback-to-worker bound.
    #[error("capture batch exceeds the bounded frame count")]
    BatchTooLarge,
    /// Clipped-sample count exceeded samples in the batch.
    #[error("clipped-sample count exceeds batch samples")]
    InvalidClippedSampleCount,
    /// Discontinuity position differed from the batch start.
    #[error("capture discontinuity does not match the batch start")]
    DiscontinuityPositionMismatch,
    /// Advancing a source-frame position overflowed.
    #[error("capture position overflow")]
    PositionOverflow,
    /// FT8 window start was not a non-negative 15-second boundary.
    #[error("FT8 receive window is not UTC-slot aligned")]
    MisalignedFt8Window,
    /// FT8 window did not contain exactly one canonical slot.
    #[error("FT8 receive window must contain exactly 180000 mono samples")]
    InvalidFt8WindowSize,
}

fn validate_bounded_text(value: &str, max_bytes: usize) -> Result<(), ()> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(channels: u16) -> InputConfiguration {
        InputConfiguration::new(48_000, channels, InputSampleFormat::Float32, 0).unwrap()
    }

    fn batch(
        process: u64,
        stream: u64,
        position: u64,
        monotonic: u64,
        frames: usize,
        discontinuity: Option<CaptureDiscontinuity>,
    ) -> CaptureBatch {
        CaptureBatch::new(
            ProcessGeneration::new(process).unwrap(),
            StreamGeneration::new(stream).unwrap(),
            config(1),
            CaptureTimeEvidence::new(CapturePosition::from_frames(position), 30_000, monotonic)
                .unwrap(),
            discontinuity,
            CaptureDiagnostics::new(0, 1).unwrap(),
            vec![0; frames],
        )
        .unwrap()
    }

    #[test]
    fn display_metadata_cannot_be_used_as_stable_identity() {
        let identity = InputDeviceIdentity::new(InputPlatform::MacOsCoreAudio, "uid:123").unwrap();
        let display = InputDeviceDisplay::new("USB Audio", None).unwrap();
        let range =
            InputConfigurationRange::new(48_000, 48_000, 2, InputSampleFormat::Float32).unwrap();
        let descriptor =
            InputDeviceDescriptor::new(identity.clone(), display, vec![range]).unwrap();
        assert_eq!(descriptor.identity(), &identity);
        assert_eq!(descriptor.display().name(), "USB Audio");
        assert_ne!(
            descriptor.identity().opaque_id(),
            descriptor.display().name()
        );
    }

    #[test]
    fn invalid_rates_channels_formats_timestamps_and_sizes_are_typed() {
        assert_eq!(
            InputConfiguration::new(1, 1, InputSampleFormat::Signed16, 0),
            Err(ReceiveAudioError::InvalidSampleRate)
        );
        assert_eq!(
            InputConfiguration::new(48_000, 0, InputSampleFormat::Signed16, 0),
            Err(ReceiveAudioError::InvalidChannelCount)
        );
        assert_eq!(
            InputConfiguration::new(48_000, 1, InputSampleFormat::Signed16, 1),
            Err(ReceiveAudioError::InvalidSelectedChannel)
        );
        assert_eq!(
            CaptureTimeEvidence::new(CapturePosition::from_frames(0), -1, 0),
            Err(ReceiveAudioError::InvalidUtcTimestamp)
        );
        let oversized = vec![0; MAX_CAPTURE_BATCH_FRAMES as usize + 1];
        assert_eq!(
            CaptureBatch::new(
                ProcessGeneration::new(1).unwrap(),
                StreamGeneration::new(1).unwrap(),
                config(1),
                CaptureTimeEvidence::new(CapturePosition::from_frames(0), 0, 0).unwrap(),
                None,
                CaptureDiagnostics::new(0, 0).unwrap(),
                oversized,
            ),
            Err(ReceiveAudioError::BatchTooLarge)
        );
    }

    #[test]
    fn capture_sequence_rejects_regressions_and_unmarked_gaps() {
        let first = batch(1, 1, 0, 10, 4, None);
        let mut sequence = CaptureSequence::start(&first).unwrap();
        assert_eq!(
            sequence.observe(&batch(1, 1, 3, 11, 4, None)),
            Err(CaptureSequenceError::PositionRegressed)
        );
        assert_eq!(
            sequence.observe(&batch(1, 1, 5, 12, 4, None)),
            Err(CaptureSequenceError::UnmarkedGap)
        );
        assert_eq!(
            sequence.observe(&batch(1, 1, 4, 9, 4, None)),
            Err(CaptureSequenceError::MonotonicRegressed)
        );
    }

    #[test]
    fn explicit_discontinuity_allows_a_visible_gap() {
        let first = batch(1, 1, 0, 10, 4, None);
        let mut sequence = CaptureSequence::start(&first).unwrap();
        let discontinuity = CaptureDiscontinuity {
            at: CapturePosition::from_frames(8),
            kind: CaptureDiscontinuityKind::Overflow,
            dropped_frames: 4,
        };
        sequence
            .observe(&batch(1, 1, 8, 11, 4, Some(discontinuity)))
            .unwrap();
    }

    #[test]
    fn canonical_ft8_window_requires_exact_alignment_and_size() {
        let process = ProcessGeneration::new(1).unwrap();
        let stream = StreamGeneration::new(2).unwrap();
        let mapping =
            CaptureTimeEvidence::new(CapturePosition::from_frames(0), 30_000, 100).unwrap();
        let window = Ft8ReceiveWindow::new(
            process,
            stream,
            30_000,
            mapping,
            vec![0; FT8_RECEIVE_WINDOW_SAMPLES],
        )
        .unwrap();
        assert_eq!(window.sample_rate_hz(), 12_000);
        assert_eq!(window.samples().len(), 180_000);
        assert_eq!(
            Ft8ReceiveWindow::new(process, stream, 30_001, mapping, vec![0; 180_000]),
            Err(ReceiveAudioError::MisalignedFt8Window)
        );
    }
}
