//! Private native input-stream and bounded callback-queue adapter.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use cpal::{
    BufferSize, Error, ErrorKind, I24, InputCallbackInfo, SampleFormat, StreamConfig, U24,
    traits::{DeviceTrait, StreamTrait},
};
use crossbeam_queue::ArrayQueue;
use thiserror::Error;

use crate::{
    CaptureBatch, CaptureDiagnostics, CaptureDiscontinuity, CaptureDiscontinuityKind,
    CapturePosition, CaptureTimeEvidence, InputConfiguration, InputDeviceIdentity, InputFault,
    InputFaultKind, InputHealth, InputSampleFormat, MAX_CAPTURE_BATCH_FRAMES, ProcessGeneration,
    StreamGeneration, discovery::device_for_identity,
};

/// Default number of preallocated callback buffers.
pub const DEFAULT_CAPTURE_QUEUE_BATCHES: usize = 8;
/// Largest supported callback queue, bounding preallocated memory.
pub const MAX_CAPTURE_QUEUE_BATCHES: usize = 128;
/// Delay at which the callback emits a health fault.
pub const CALLBACK_DELAY_FAULT_MILLIS: u32 = 250;

/// Typed failure while configuring or controlling a receive-only input stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum InputCaptureError {
    /// The preallocated queue size is outside the bounded contract.
    #[error("capture queue capacity is invalid")]
    InvalidQueueCapacity,
    /// The exact device does not support the requested configuration.
    #[error("input capture configuration is unsupported")]
    UnsupportedConfiguration,
    /// The platform denied input capture permission.
    #[error("input capture permission denied")]
    PermissionDenied,
    /// The exact selected device disappeared.
    #[error("input capture device disappeared")]
    DeviceLost,
    /// The stream has already stopped.
    #[error("input capture stream is stopped")]
    Stopped,
    /// The native input backend failed.
    #[error("input capture backend failed")]
    BackendFailure,
}

/// One daemon-owned native receive-only input stream.
///
/// Construction requires an exact stable identity and exact checked
/// configuration. Dropping or stopping the value disables callback processing
/// and discards queued samples; a later open must use a new stream generation.
pub struct SystemInputCapture {
    stream: cpal::Stream,
    shared: Arc<SharedCapture>,
}

impl SystemInputCapture {
    /// Opens and starts one exact input device with a preallocated bounded queue.
    pub fn start(
        identity: &InputDeviceIdentity,
        configuration: InputConfiguration,
        process_generation: ProcessGeneration,
        stream_generation: StreamGeneration,
        queue_batches: usize,
    ) -> Result<Self, InputCaptureError> {
        let shared = SharedCapture::new(
            configuration,
            process_generation,
            stream_generation,
            queue_batches,
        )?;
        let device = device_for_identity(identity).map_err(map_discovery_error)?;
        ensure_configuration_supported(&device, configuration)?;

        let clock: Arc<dyn CallbackClock> = Arc::new(SystemCallbackClock::new());
        let callback = CallbackState::new(Arc::clone(&shared), Arc::clone(&clock));
        let error_shared = Arc::clone(&shared);
        let error_clock = Arc::clone(&clock);
        let native_config = StreamConfig {
            channels: configuration.channels(),
            sample_rate: configuration.sample_rate_hz(),
            buffer_size: BufferSize::Default,
        };
        let stream = build_input_stream(
            &device,
            native_config,
            configuration.sample_format(),
            callback,
            move |error| error_shared.stop_for_backend_error(error, error_clock.as_ref()),
        )
        .map_err(map_open_error)?;
        stream.play().map_err(map_open_error)?;
        Ok(Self { stream, shared })
    }

    /// Returns the next worker-owned batch, allocating only on the caller thread.
    pub fn next_batch(&self) -> Result<Option<CaptureBatch>, InputCaptureError> {
        self.shared.next_batch()
    }

    /// Returns the next typed diagnostic without blocking.
    #[must_use]
    pub fn next_fault(&self) -> Option<InputFault> {
        self.shared.next_fault()
    }

    /// Returns a snapshot of bounded callback health counters.
    pub fn health(&self) -> Result<InputHealth, InputCaptureError> {
        InputHealth::new(
            self.shared
                .max_callback_delay_millis
                .load(Ordering::Acquire),
            0,
            self.shared.overflow_count.load(Ordering::Acquire),
            self.shared.clipped_sample_count.load(Ordering::Acquire),
            self.shared
                .max_callback_delay_millis
                .load(Ordering::Acquire),
        )
        .map_err(|_| InputCaptureError::BackendFailure)
    }

    /// Reports whether callback processing has stopped after an explicit stop or backend error.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.shared.stopped.load(Ordering::Acquire)
    }

    /// Stops callback processing, pauses the native stream, and discards queued samples.
    pub fn stop(&mut self) -> Result<(), InputCaptureError> {
        if self.shared.stopped.swap(true, Ordering::AcqRel) {
            return Err(InputCaptureError::Stopped);
        }
        self.stream.pause().map_err(map_open_error)?;
        self.shared.discard_ready();
        Ok(())
    }
}

impl Drop for SystemInputCapture {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        let _ = self.stream.pause();
        self.shared.discard_ready();
    }
}

struct SharedCapture {
    configuration: InputConfiguration,
    process_generation: ProcessGeneration,
    stream_generation: StreamGeneration,
    free: ArrayQueue<Box<[i16]>>,
    ready: ArrayQueue<RawBatch>,
    faults: ArrayQueue<InputFault>,
    stopped: AtomicBool,
    overflow_count: AtomicU64,
    clipped_sample_count: AtomicU64,
    max_callback_delay_millis: AtomicU32,
    terminal_fault: AtomicU32,
    terminal_fault_time: AtomicU64,
    terminal_fault_reported: AtomicBool,
}

impl SharedCapture {
    fn new(
        configuration: InputConfiguration,
        process_generation: ProcessGeneration,
        stream_generation: StreamGeneration,
        queue_batches: usize,
    ) -> Result<Arc<Self>, InputCaptureError> {
        if !(2..=MAX_CAPTURE_QUEUE_BATCHES).contains(&queue_batches) {
            return Err(InputCaptureError::InvalidQueueCapacity);
        }
        let free = ArrayQueue::new(queue_batches);
        for _ in 0..queue_batches {
            free.push(vec![0; MAX_CAPTURE_BATCH_FRAMES as usize].into_boxed_slice())
                .map_err(|_| InputCaptureError::BackendFailure)?;
        }
        Ok(Arc::new(Self {
            configuration,
            process_generation,
            stream_generation,
            free,
            ready: ArrayQueue::new(queue_batches),
            faults: ArrayQueue::new(queue_batches),
            stopped: AtomicBool::new(false),
            overflow_count: AtomicU64::new(0),
            clipped_sample_count: AtomicU64::new(0),
            max_callback_delay_millis: AtomicU32::new(0),
            terminal_fault: AtomicU32::new(0),
            terminal_fault_time: AtomicU64::new(0),
            terminal_fault_reported: AtomicBool::new(false),
        }))
    }

    fn next_batch(&self) -> Result<Option<CaptureBatch>, InputCaptureError> {
        if self.stopped.load(Ordering::Acquire) {
            self.discard_ready();
            return Ok(None);
        }
        let Some(raw) = self.ready.pop() else {
            return Ok(None);
        };
        let samples = raw.samples[..raw.frame_count].to_vec();
        let batch = CaptureBatch::new(
            self.process_generation,
            self.stream_generation,
            self.configuration,
            raw.first_frame,
            raw.discontinuity,
            raw.diagnostics,
            samples,
        )
        .map_err(|_| InputCaptureError::BackendFailure);
        self.free
            .push(raw.samples)
            .map_err(|_| InputCaptureError::BackendFailure)?;
        batch.map(Some)
    }

    fn discard_ready(&self) {
        while let Some(raw) = self.ready.pop() {
            let _ = self.free.push(raw.samples);
        }
    }

    fn emit_fault(&self, monotonic_millis: u64, kind: InputFaultKind) {
        let _ = self.faults.push(InputFault {
            process_generation: self.process_generation,
            stream_generation: Some(self.stream_generation),
            monotonic_millis,
            kind,
        });
    }

    fn next_fault(&self) -> Option<InputFault> {
        let terminal = self.terminal_fault.load(Ordering::Acquire);
        if terminal != 0 && !self.terminal_fault_reported.swap(true, Ordering::AcqRel) {
            let kind = match terminal {
                1 => InputFaultKind::PermissionDenied,
                2 => InputFaultKind::DeviceLost,
                _ => InputFaultKind::BackendFailure,
            };
            return Some(InputFault {
                process_generation: self.process_generation,
                stream_generation: Some(self.stream_generation),
                monotonic_millis: self.terminal_fault_time.load(Ordering::Acquire),
                kind,
            });
        }
        self.faults.pop()
    }

    fn stop_with_terminal_fault(&self, monotonic_millis: u64, kind: InputFaultKind) {
        let code = match kind {
            InputFaultKind::PermissionDenied => 1,
            InputFaultKind::DeviceLost => 2,
            _ => 3,
        };
        self.terminal_fault_time
            .store(monotonic_millis, Ordering::Release);
        self.terminal_fault.store(code, Ordering::Release);
        self.stopped.store(true, Ordering::Release);
    }

    fn stop_for_backend_error(&self, error: Error, clock: &dyn CallbackClock) {
        let kind = match error.kind() {
            ErrorKind::PermissionDenied => InputFaultKind::PermissionDenied,
            ErrorKind::DeviceNotAvailable
            | ErrorKind::DeviceChanged
            | ErrorKind::StreamInvalidated => InputFaultKind::DeviceLost,
            _ => InputFaultKind::BackendFailure,
        };
        self.stop_with_terminal_fault(clock.reading().monotonic_millis, kind);
    }
}

struct RawBatch {
    samples: Box<[i16]>,
    frame_count: usize,
    first_frame: CaptureTimeEvidence,
    discontinuity: Option<CaptureDiscontinuity>,
    diagnostics: CaptureDiagnostics,
}

#[derive(Debug, Clone, Copy)]
struct ClockReading {
    utc_unix_millis: i64,
    monotonic_millis: u64,
}

trait CallbackClock: Send + Sync {
    fn reading(&self) -> ClockReading;
}

struct SystemCallbackClock {
    monotonic_origin: Instant,
}

impl SystemCallbackClock {
    fn new() -> Self {
        Self {
            monotonic_origin: Instant::now(),
        }
    }
}

impl CallbackClock for SystemCallbackClock {
    fn reading(&self) -> ClockReading {
        let utc_unix_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|value| i64::try_from(value.as_millis()).ok())
            .unwrap_or_default();
        let monotonic_millis =
            u64::try_from(self.monotonic_origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        ClockReading {
            utc_unix_millis,
            monotonic_millis,
        }
    }
}

struct CallbackState {
    shared: Arc<SharedCapture>,
    clock: Arc<dyn CallbackClock>,
    position: CapturePosition,
    pending_dropped_frames: u64,
    first_batch: bool,
}

impl CallbackState {
    fn new(shared: Arc<SharedCapture>, clock: Arc<dyn CallbackClock>) -> Self {
        Self {
            shared,
            clock,
            position: CapturePosition::from_frames(0),
            pending_dropped_frames: 0,
            first_batch: true,
        }
    }

    // BEGIN REALTIME CALLBACK
    fn process_native<T: NormalizeSample>(&mut self, data: &[T], info: &InputCallbackInfo) {
        let timestamp = info.timestamp();
        let delay = timestamp
            .callback
            .checked_duration_since(timestamp.capture)
            .unwrap_or_default();
        let callback_delay_millis = u32::try_from(delay.as_millis())
            .unwrap_or(u32::MAX)
            .min(60_000);
        let reading = self.clock.reading();
        let delay_millis = i64::from(callback_delay_millis);
        let timing = CallbackTiming {
            utc_unix_millis: reading.utc_unix_millis.saturating_sub(delay_millis),
            monotonic_millis: reading
                .monotonic_millis
                .saturating_sub(u64::from(callback_delay_millis)),
            callback_delay_millis,
        };
        self.process(data, timing);
    }

    fn process<T: NormalizeSample>(&mut self, data: &[T], timing: CallbackTiming) {
        if self.shared.stopped.load(Ordering::Acquire) {
            return;
        }
        let channels = usize::from(self.shared.configuration.channels());
        if data.is_empty() || !data.len().is_multiple_of(channels) {
            self.stop_with_fault(timing.monotonic_millis, InputFaultKind::BackendFailure);
            return;
        }

        self.shared
            .max_callback_delay_millis
            .fetch_max(timing.callback_delay_millis, Ordering::AcqRel);
        if timing.callback_delay_millis > CALLBACK_DELAY_FAULT_MILLIS {
            self.shared.emit_fault(
                timing.monotonic_millis,
                InputFaultKind::CallbackDelay {
                    millis: timing.callback_delay_millis,
                },
            );
        }

        let total_frames = data.len() / channels;
        let mut frame_offset = 0usize;
        while frame_offset < total_frames {
            let frame_count = (total_frames - frame_offset).min(MAX_CAPTURE_BATCH_FRAMES as usize);
            let Some(mut samples) = self.shared.free.pop() else {
                self.drop_frames(frame_count, timing.monotonic_millis);
                frame_offset += frame_count;
                continue;
            };

            let mut clipped_samples = 0u32;
            for (output_index, source_frame) in
                (frame_offset..frame_offset + frame_count).enumerate()
            {
                let source_index = source_frame * channels
                    + usize::from(self.shared.configuration.selected_channel());
                let Some((sample, clipped)) = data[source_index].normalize() else {
                    let _ = self.shared.free.push(samples);
                    self.stop_with_fault(timing.monotonic_millis, InputFaultKind::BackendFailure);
                    return;
                };
                samples[output_index] = sample;
                clipped_samples = clipped_samples.saturating_add(u32::from(clipped));
            }

            let offset_millis = frame_offset
                .saturating_mul(1_000)
                .checked_div(self.shared.configuration.sample_rate_hz() as usize)
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or(u64::MAX);
            let first_frame = match CaptureTimeEvidence::new(
                self.position,
                timing
                    .utc_unix_millis
                    .saturating_add(i64::try_from(offset_millis).unwrap_or(i64::MAX)),
                timing.monotonic_millis.saturating_add(offset_millis),
            ) {
                Ok(value) => value,
                Err(_) => {
                    let _ = self.shared.free.push(samples);
                    self.stop_with_fault(timing.monotonic_millis, InputFaultKind::BackendFailure);
                    return;
                }
            };
            let discontinuity = if self.pending_dropped_frames > 0 {
                let dropped_frames = self.pending_dropped_frames;
                self.pending_dropped_frames = 0;
                Some(CaptureDiscontinuity {
                    at: self.position,
                    kind: CaptureDiscontinuityKind::Overflow,
                    dropped_frames,
                })
            } else if self.first_batch {
                Some(CaptureDiscontinuity {
                    at: self.position,
                    kind: CaptureDiscontinuityKind::StreamRestart,
                    dropped_frames: 0,
                })
            } else {
                None
            };
            self.first_batch = false;
            let diagnostics =
                match CaptureDiagnostics::new(clipped_samples, timing.callback_delay_millis) {
                    Ok(value) => value,
                    Err(_) => {
                        let _ = self.shared.free.push(samples);
                        self.stop_with_fault(
                            timing.monotonic_millis,
                            InputFaultKind::BackendFailure,
                        );
                        return;
                    }
                };
            let raw = RawBatch {
                samples,
                frame_count,
                first_frame,
                discontinuity,
                diagnostics,
            };
            if let Err(raw) = self.shared.ready.push(raw) {
                let _ = self.shared.free.push(raw.samples);
                self.drop_frames(frame_count, timing.monotonic_millis);
            } else {
                if clipped_samples > 0 {
                    self.shared
                        .clipped_sample_count
                        .fetch_add(u64::from(clipped_samples), Ordering::AcqRel);
                    self.shared.emit_fault(
                        timing.monotonic_millis,
                        InputFaultKind::Clipping {
                            sample_count: clipped_samples,
                        },
                    );
                }
                match self
                    .position
                    .checked_advance(u32::try_from(frame_count).unwrap_or(u32::MAX))
                {
                    Ok(position) => self.position = position,
                    Err(_) => {
                        self.stop_with_fault(
                            timing.monotonic_millis,
                            InputFaultKind::BackendFailure,
                        );
                        return;
                    }
                }
            }
            frame_offset += frame_count;
        }
    }

    fn drop_frames(&mut self, frame_count: usize, monotonic_millis: u64) {
        let frames = u64::try_from(frame_count).unwrap_or(u64::MAX);
        self.pending_dropped_frames = self.pending_dropped_frames.saturating_add(frames);
        self.shared.overflow_count.fetch_add(1, Ordering::AcqRel);
        self.shared.emit_fault(
            monotonic_millis,
            InputFaultKind::Overflow {
                dropped_frames: frames,
            },
        );
        match self
            .position
            .checked_advance(u32::try_from(frame_count).unwrap_or(u32::MAX))
        {
            Ok(position) => self.position = position,
            Err(_) => self.stop_with_fault(monotonic_millis, InputFaultKind::BackendFailure),
        }
    }

    fn stop_with_fault(&self, monotonic_millis: u64, kind: InputFaultKind) {
        self.shared.stop_with_terminal_fault(monotonic_millis, kind);
    }
    // END REALTIME CALLBACK
}

#[derive(Debug, Clone, Copy)]
struct CallbackTiming {
    utc_unix_millis: i64,
    monotonic_millis: u64,
    callback_delay_millis: u32,
}

trait NormalizeSample: Copy + Send + 'static {
    fn normalize(self) -> Option<(i16, bool)>;
}

macro_rules! signed_sample {
    ($sample:ty, $value:expr, $min:expr, $max:expr) => {
        impl NormalizeSample for $sample {
            fn normalize(self) -> Option<(i16, bool)> {
                let value = $value(self) as i128;
                let normalized = if value >= 0 {
                    value.saturating_mul(i128::from(i16::MAX)) / ($max as i128)
                } else {
                    value.saturating_mul(-i128::from(i16::MIN)) / -($min as i128)
                };
                Some((
                    i16::try_from(normalized).ok()?,
                    value == ($min as i128) || value == ($max as i128),
                ))
            }
        }
    };
}

macro_rules! unsigned_sample {
    ($sample:ty, $value:expr, $max:expr, $midpoint:expr) => {
        impl NormalizeSample for $sample {
            fn normalize(self) -> Option<(i16, bool)> {
                let value = $value(self) as u128;
                let midpoint = $midpoint as u128;
                let normalized = if value >= midpoint {
                    let positive = value - midpoint;
                    positive.saturating_mul(i16::MAX as u128) / (($max as u128) - midpoint)
                } else {
                    let negative = midpoint - value;
                    negative.saturating_mul((-i32::from(i16::MIN)) as u128) / midpoint
                };
                let normalized = if value >= midpoint {
                    i128::try_from(normalized).ok()?
                } else {
                    -i128::try_from(normalized).ok()?
                };
                Some((
                    i16::try_from(normalized).ok()?,
                    value == 0 || value == ($max as u128),
                ))
            }
        }
    };
}

signed_sample!(i8, |value: i8| value, i8::MIN, i8::MAX);
signed_sample!(i16, |value: i16| value, i16::MIN, i16::MAX);
signed_sample!(
    I24,
    |value: I24| value.inner(),
    -8_388_608_i32,
    8_388_607_i32
);
signed_sample!(i32, |value: i32| value, i32::MIN, i32::MAX);
signed_sample!(i64, |value: i64| value, i64::MIN, i64::MAX);
unsigned_sample!(u8, |value: u8| value, u8::MAX, 128_u8);
unsigned_sample!(u16, |value: u16| value, u16::MAX, 32_768_u16);
unsigned_sample!(
    U24,
    |value: U24| value.inner(),
    16_777_215_u32,
    8_388_608_u32
);
unsigned_sample!(u32, |value: u32| value, u32::MAX, 2_147_483_648_u32);
unsigned_sample!(
    u64,
    |value: u64| value,
    u64::MAX,
    9_223_372_036_854_775_808_u64
);

impl NormalizeSample for f32 {
    fn normalize(self) -> Option<(i16, bool)> {
        normalize_float(f64::from(self))
    }
}

impl NormalizeSample for f64 {
    fn normalize(self) -> Option<(i16, bool)> {
        normalize_float(self)
    }
}

fn normalize_float(value: f64) -> Option<(i16, bool)> {
    if !value.is_finite() {
        return None;
    }
    let clipped = !(-1.0..=1.0).contains(&value) || value == -1.0 || value == 1.0;
    let value = value.clamp(-1.0, 1.0);
    let scaled = if value >= 0.0 {
        value * f64::from(i16::MAX)
    } else {
        value * -f64::from(i16::MIN)
    };
    Some((scaled.round() as i16, clipped))
}

fn build_input_stream<E>(
    device: &cpal::Device,
    configuration: StreamConfig,
    sample_format: InputSampleFormat,
    callback: CallbackState,
    error_callback: E,
) -> Result<cpal::Stream, Error>
where
    E: FnMut(Error) + Send + 'static,
{
    macro_rules! build {
        ($sample:ty) => {{
            let mut callback = callback;
            device.build_input_stream::<$sample, _, _>(
                configuration,
                move |data, info| callback.process_native(data, info),
                error_callback,
                Some(Duration::from_secs(5)),
            )
        }};
    }
    match sample_format {
        InputSampleFormat::Signed8 => build!(i8),
        InputSampleFormat::Signed16 => build!(i16),
        InputSampleFormat::Signed24 => build!(I24),
        InputSampleFormat::Signed32 => build!(i32),
        InputSampleFormat::Signed64 => build!(i64),
        InputSampleFormat::Unsigned8 => build!(u8),
        InputSampleFormat::Unsigned16 => build!(u16),
        InputSampleFormat::Unsigned24 => build!(U24),
        InputSampleFormat::Unsigned32 => build!(u32),
        InputSampleFormat::Unsigned64 => build!(u64),
        InputSampleFormat::Float32 => build!(f32),
        InputSampleFormat::Float64 => build!(f64),
    }
}

fn ensure_configuration_supported(
    device: &cpal::Device,
    configuration: InputConfiguration,
) -> Result<(), InputCaptureError> {
    let expected_format = native_sample_format(configuration.sample_format());
    let supported = device
        .supported_input_configs()
        .map_err(map_open_error)?
        .any(|range| {
            range.channels() == configuration.channels()
                && range.sample_format() == expected_format
                && (range.min_sample_rate()..=range.max_sample_rate())
                    .contains(&configuration.sample_rate_hz())
        });
    if supported {
        Ok(())
    } else {
        Err(InputCaptureError::UnsupportedConfiguration)
    }
}

const fn native_sample_format(format: InputSampleFormat) -> SampleFormat {
    match format {
        InputSampleFormat::Signed8 => SampleFormat::I8,
        InputSampleFormat::Signed16 => SampleFormat::I16,
        InputSampleFormat::Signed24 => SampleFormat::I24,
        InputSampleFormat::Signed32 => SampleFormat::I32,
        InputSampleFormat::Signed64 => SampleFormat::I64,
        InputSampleFormat::Unsigned8 => SampleFormat::U8,
        InputSampleFormat::Unsigned16 => SampleFormat::U16,
        InputSampleFormat::Unsigned24 => SampleFormat::U24,
        InputSampleFormat::Unsigned32 => SampleFormat::U32,
        InputSampleFormat::Unsigned64 => SampleFormat::U64,
        InputSampleFormat::Float32 => SampleFormat::F32,
        InputSampleFormat::Float64 => SampleFormat::F64,
    }
}

fn map_open_error(error: Error) -> InputCaptureError {
    match error.kind() {
        ErrorKind::PermissionDenied => InputCaptureError::PermissionDenied,
        ErrorKind::DeviceNotAvailable | ErrorKind::DeviceChanged | ErrorKind::StreamInvalidated => {
            InputCaptureError::DeviceLost
        }
        ErrorKind::UnsupportedConfig
        | ErrorKind::UnsupportedOperation
        | ErrorKind::InvalidInput => InputCaptureError::UnsupportedConfiguration,
        _ => InputCaptureError::BackendFailure,
    }
}

fn map_discovery_error(error: crate::InputDiscoveryError) -> InputCaptureError {
    match error {
        crate::InputDiscoveryError::PermissionDenied => InputCaptureError::PermissionDenied,
        crate::InputDiscoveryError::DeviceDisappeared => InputCaptureError::DeviceLost,
        crate::InputDiscoveryError::UnsupportedConfiguration
        | crate::InputDiscoveryError::IdentityUnavailable => {
            InputCaptureError::UnsupportedConfiguration
        }
        crate::InputDiscoveryError::HostUnavailable
        | crate::InputDiscoveryError::NoInputDevices
        | crate::InputDiscoveryError::BackendFailure => InputCaptureError::BackendFailure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration(
        channels: u16,
        selected_channel: u16,
        format: InputSampleFormat,
    ) -> InputConfiguration {
        InputConfiguration::new(48_000, channels, format, selected_channel).unwrap()
    }

    fn harness(
        capacity: usize,
        stream_generation: u64,
        configuration: InputConfiguration,
    ) -> (Arc<SharedCapture>, CallbackState) {
        let shared = SharedCapture::new(
            configuration,
            ProcessGeneration::new(1).unwrap(),
            StreamGeneration::new(stream_generation).unwrap(),
            capacity,
        )
        .unwrap();
        let clock: Arc<dyn CallbackClock> = Arc::new(FixedClock);
        let callback = CallbackState::new(Arc::clone(&shared), clock);
        (shared, callback)
    }

    struct FixedClock;

    impl CallbackClock for FixedClock {
        fn reading(&self) -> ClockReading {
            ClockReading {
                utc_unix_millis: 30_000,
                monotonic_millis: 1_000,
            }
        }
    }

    const fn timing() -> CallbackTiming {
        CallbackTiming {
            utc_unix_millis: 30_000,
            monotonic_millis: 1_000,
            callback_delay_millis: 4,
        }
    }

    #[test]
    fn queue_capacity_is_bounded_before_any_stream_can_open() {
        assert_eq!(
            SharedCapture::new(
                configuration(1, 0, InputSampleFormat::Signed16),
                ProcessGeneration::new(1).unwrap(),
                StreamGeneration::new(1).unwrap(),
                1,
            )
            .err(),
            Some(InputCaptureError::InvalidQueueCapacity)
        );
        assert_eq!(
            SharedCapture::new(
                configuration(1, 0, InputSampleFormat::Signed16),
                ProcessGeneration::new(1).unwrap(),
                StreamGeneration::new(1).unwrap(),
                MAX_CAPTURE_QUEUE_BATCHES + 1,
            )
            .err(),
            Some(InputCaptureError::InvalidQueueCapacity)
        );
    }

    #[test]
    fn selected_channel_is_scaled_into_mono_without_callback_allocation() {
        let (shared, mut callback) = harness(2, 1, configuration(2, 1, InputSampleFormat::Float32));
        callback.process(&[0.75_f32, -1.0, -0.75, 0.5], timing());
        let batch = shared.next_batch().unwrap().unwrap();
        assert_eq!(batch.samples(), &[i16::MIN, 16_384]);
        assert_eq!(batch.frame_count(), 2);
        assert_eq!(
            batch.discontinuity.unwrap().kind,
            CaptureDiscontinuityKind::StreamRestart
        );
        assert_eq!(batch.diagnostics.clipped_samples, 1);
        assert_eq!(
            shared.next_fault().unwrap().kind,
            InputFaultKind::Clipping { sample_count: 1 }
        );
    }

    #[test]
    fn sustained_pressure_drops_new_frames_and_marks_the_next_batch() {
        let (shared, mut callback) =
            harness(2, 1, configuration(1, 0, InputSampleFormat::Signed16));
        callback.process(&[1_i16, 2], timing());
        callback.process(&[3_i16, 4], timing());
        callback.process(&[5_i16, 6, 7], timing());
        assert_eq!(shared.overflow_count.load(Ordering::Acquire), 1);
        assert_eq!(
            shared.next_fault().unwrap().kind,
            InputFaultKind::Overflow { dropped_frames: 3 }
        );

        let first = shared.next_batch().unwrap().unwrap();
        assert_eq!(first.samples(), &[1, 2]);
        callback.process(&[8_i16, 9], timing());
        let second = shared.next_batch().unwrap().unwrap();
        assert_eq!(second.samples(), &[3, 4]);
        let resumed = shared.next_batch().unwrap().unwrap();
        assert_eq!(resumed.samples(), &[8, 9]);
        assert_eq!(
            resumed.discontinuity,
            Some(CaptureDiscontinuity {
                at: CapturePosition::from_frames(7),
                kind: CaptureDiscontinuityKind::Overflow,
                dropped_frames: 3,
            })
        );
    }

    #[test]
    fn every_supported_format_has_checked_endpoint_scaling() {
        assert_eq!(i8::MIN.normalize(), Some((i16::MIN, true)));
        assert_eq!(i8::MAX.normalize(), Some((i16::MAX, true)));
        assert_eq!(i16::MIN.normalize(), Some((i16::MIN, true)));
        assert_eq!(i16::MAX.normalize(), Some((i16::MAX, true)));
        assert_eq!(i32::MIN.normalize(), Some((i16::MIN, true)));
        assert_eq!(i32::MAX.normalize(), Some((i16::MAX, true)));
        assert_eq!(i64::MIN.normalize(), Some((i16::MIN, true)));
        assert_eq!(i64::MAX.normalize(), Some((i16::MAX, true)));
        assert_eq!(0_u8.normalize(), Some((i16::MIN, true)));
        assert_eq!(u8::MAX.normalize(), Some((i16::MAX, true)));
        assert_eq!(0_u16.normalize(), Some((i16::MIN, true)));
        assert_eq!(u16::MAX.normalize(), Some((i16::MAX, true)));
        assert_eq!(0_u32.normalize(), Some((i16::MIN, true)));
        assert_eq!(u32::MAX.normalize(), Some((i16::MAX, true)));
        assert_eq!(0_u64.normalize(), Some((i16::MIN, true)));
        assert_eq!(u64::MAX.normalize(), Some((i16::MAX, true)));
        assert_eq!(
            I24::new(-8_388_608).unwrap().normalize(),
            Some((i16::MIN, true))
        );
        assert_eq!(
            U24::new(16_777_215).unwrap().normalize(),
            Some((i16::MAX, true))
        );
        assert_eq!(f32::NAN.normalize(), None);
        assert_eq!(f64::INFINITY.normalize(), None);
        assert_eq!((-2.0_f64).normalize(), Some((i16::MIN, true)));
        assert_eq!(2.0_f64.normalize(), Some((i16::MAX, true)));
    }

    #[test]
    fn backend_error_stops_processing_and_is_typed() {
        let (shared, mut callback) = harness(2, 1, configuration(1, 0, InputSampleFormat::Float32));
        callback.process(&[f32::NAN], timing());
        assert!(shared.stopped.load(Ordering::Acquire));
        assert_eq!(
            shared.next_fault().unwrap().kind,
            InputFaultKind::BackendFailure
        );
        callback.process(&[0.5], timing());
        assert!(shared.ready.is_empty());
    }

    #[test]
    fn device_loss_stops_capture_without_discarding_the_exact_identity_contract() {
        let (shared, _) = harness(2, 1, configuration(1, 0, InputSampleFormat::Signed16));
        shared.emit_fault(999, InputFaultKind::Overflow { dropped_frames: 1 });
        shared.stop_for_backend_error(Error::new(ErrorKind::DeviceNotAvailable), &FixedClock);
        assert!(shared.stopped.load(Ordering::Acquire));
        let fault = shared.next_fault().unwrap();
        assert_eq!(fault.kind, InputFaultKind::DeviceLost);
        assert_eq!(fault.monotonic_millis, 1_000);
        assert_eq!(
            shared.next_fault().unwrap().kind,
            InputFaultKind::Overflow { dropped_frames: 1 }
        );
    }

    #[test]
    fn explicit_shutdown_discards_ready_batches() {
        let (shared, mut callback) =
            harness(2, 1, configuration(1, 0, InputSampleFormat::Signed16));
        callback.process(&[1_i16, 2], timing());
        shared.stopped.store(true, Ordering::Release);
        shared.discard_ready();
        assert!(shared.next_batch().unwrap().is_none());
        assert_eq!(shared.free.len(), 2);
    }

    #[test]
    fn callback_delay_is_bounded_observable_health() {
        let (shared, mut callback) =
            harness(2, 1, configuration(1, 0, InputSampleFormat::Signed16));
        callback.process(
            &[0_i16],
            CallbackTiming {
                callback_delay_millis: CALLBACK_DELAY_FAULT_MILLIS + 1,
                ..timing()
            },
        );
        assert_eq!(
            shared.next_fault().unwrap().kind,
            InputFaultKind::CallbackDelay {
                millis: CALLBACK_DELAY_FAULT_MILLIS + 1
            }
        );
        assert_eq!(
            shared.max_callback_delay_millis.load(Ordering::Acquire),
            CALLBACK_DELAY_FAULT_MILLIS + 1
        );
    }

    #[test]
    fn reopen_uses_fresh_generation_and_cannot_retain_stale_samples() {
        let (old_shared, mut old_callback) =
            harness(2, 1, configuration(1, 0, InputSampleFormat::Signed16));
        old_callback.process(&[1_i16], timing());
        old_shared.stopped.store(true, Ordering::Release);
        old_shared.discard_ready();

        let (new_shared, mut new_callback) =
            harness(2, 2, configuration(1, 0, InputSampleFormat::Signed16));
        new_callback.process(&[2_i16], timing());
        assert!(old_shared.next_batch().unwrap().is_none());
        let batch = new_shared.next_batch().unwrap().unwrap();
        assert_eq!(batch.stream_generation, StreamGeneration::new(2).unwrap());
        assert_eq!(batch.samples(), &[2]);
    }
}
