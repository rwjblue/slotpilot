//! Daemon-owned live FT8 receive composition.
//!
//! Native callbacks end at `slotpilot-audio`'s preallocated queue. Every method
//! in this module runs on a daemon worker/event-loop path: batch copying,
//! resampling, clock gating, offline decode, SQLite writes, and typed internal
//! event construction therefore never run in the callback.

use std::collections::VecDeque;

use sha2::{Digest, Sha256};
use slotpilot_audio::{
    CaptureBatch, Ft8ReceiveTimeline, InputCaptureError, InputConfiguration, InputDeviceIdentity,
    InputFault, InputFaultKind, InputHealth, InputPlatform, InputSampleFormat, ProcessGeneration,
    ReceiveTimelineError, StreamGeneration, SystemInputCapture,
};
use slotpilot_domain::{IdError, ReceiveWindowId, ServiceInstanceId};
use slotpilot_operations::{
    ClockGatedTimelineEvent, GenerationClockSample, MonotonicInstant, ReceiveClockConfig,
    ReceiveClockError, ReceiveClockFault, ReceiveClockMonitor, ReceiveClockState,
};
use slotpilot_protocol::{
    Ft8DecodeConfig, Ft8OfflineDecoder, PcmBuffer, PcmError, PcmFormat, PcmSampleFormat,
};
use slotpilot_storage::{
    ReceiveClockHealth, ReceiveDiagnosticSummary, ReceiveInsertOutcome, ReceiveRecord,
    ReceiveWindowContext, StorageError, Store,
};
use thiserror::Error;

/// Worker-side batch backlog owned by one coordinator.
///
/// The native adapter has its own independently bounded preallocated callback
/// queue. This second bound prevents an event-loop poll from admitting
/// unbounded worker work while decode or SQLite is slow.
pub const WORKER_BATCH_CAPACITY: usize = 4;

/// Exact receive-only input selection. Display names cannot enter this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveSelection {
    /// Stable platform device identity.
    pub device_identity: InputDeviceIdentity,
    /// Exact sample rate, channel, and source format.
    pub configuration: InputConfiguration,
}

/// Static owned configuration for one daemon receive coordinator.
#[derive(Debug, Clone)]
pub struct LiveReceiveCoordinatorConfig {
    /// Running daemon identity.
    pub service_instance_id: ServiceInstanceId,
    /// Capture and monotonic process generation.
    pub process_generation: ProcessGeneration,
    /// Exact stable input selection.
    pub selection: ReceiveSelection,
    /// Bounded owned FT8 decoder policy.
    pub decode: Ft8DecodeConfig,
    /// Receive UTC/monotonic monitoring policy.
    pub clock: ReceiveClockConfig,
}

/// Daemon-private live input ownership seam.
///
/// Implementations return worker-owned batches. No method is invoked by the
/// native audio callback.
pub trait DaemonReceiveInput {
    /// Opens the exact selected input for a fresh stream generation.
    fn start(
        &mut self,
        selection: &ReceiveSelection,
        process_generation: ProcessGeneration,
        stream_generation: StreamGeneration,
    ) -> Result<(), InputCaptureError>;
    /// Returns one already-owned bounded batch without blocking.
    fn next_batch(&mut self) -> Result<Option<CaptureBatch>, InputCaptureError>;
    /// Returns one typed callback/backend fault without blocking.
    fn next_fault(&mut self) -> Option<InputFault>;
    /// Returns current bounded capture diagnostics.
    fn health(&mut self) -> Result<InputHealth, InputCaptureError>;
    /// Stops input and discards its queued samples.
    fn stop(&mut self) -> Result<(), InputCaptureError>;
}

/// Daemon-private durable receive seam used only from the worker path.
pub trait DaemonReceiveStore {
    /// Atomically records one complete healthy window and its typed evidence.
    fn record_receive(
        &mut self,
        record: &ReceiveRecord,
    ) -> Result<ReceiveInsertOutcome, StorageError>;
}

impl DaemonReceiveStore for Store {
    fn record_receive(
        &mut self,
        record: &ReceiveRecord,
    ) -> Result<ReceiveInsertOutcome, StorageError> {
        Store::record_receive(self, record)
    }
}

/// Production input owner. It can hold at most one native input stream.
pub struct SystemReceiveInput {
    capture: Option<SystemInputCapture>,
    callback_queue_batches: usize,
}

impl SystemReceiveInput {
    /// Constructs an inactive native owner with an explicit callback queue bound.
    #[must_use]
    pub const fn new(callback_queue_batches: usize) -> Self {
        Self {
            capture: None,
            callback_queue_batches,
        }
    }
}

impl DaemonReceiveInput for SystemReceiveInput {
    fn start(
        &mut self,
        selection: &ReceiveSelection,
        process_generation: ProcessGeneration,
        stream_generation: StreamGeneration,
    ) -> Result<(), InputCaptureError> {
        if self.capture.is_some() {
            return Err(InputCaptureError::BackendFailure);
        }
        self.capture = Some(SystemInputCapture::start(
            &selection.device_identity,
            selection.configuration,
            process_generation,
            stream_generation,
            self.callback_queue_batches,
        )?);
        Ok(())
    }

    fn next_batch(&mut self) -> Result<Option<CaptureBatch>, InputCaptureError> {
        self.capture
            .as_ref()
            .ok_or(InputCaptureError::Stopped)?
            .next_batch()
    }

    fn next_fault(&mut self) -> Option<InputFault> {
        self.capture
            .as_ref()
            .and_then(SystemInputCapture::next_fault)
    }

    fn health(&mut self) -> Result<InputHealth, InputCaptureError> {
        self.capture
            .as_ref()
            .ok_or(InputCaptureError::Stopped)?
            .health()
    }

    fn stop(&mut self) -> Result<(), InputCaptureError> {
        let mut capture = self.capture.take().ok_or(InputCaptureError::Stopped)?;
        capture.stop()
    }
}

/// Stable reason receive entered an inhibited/faulted state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveInhibition {
    /// Exact input could not start or failed after start.
    Input(InputFaultKind),
    /// Capture continuity or canonical timeline validation failed.
    Timeline(ReceiveTimelineError),
    /// UTC/monotonic evidence cannot safely align receive.
    Clock(ReceiveClockFault),
    /// The bounded worker backlog could not admit more work.
    WorkerBackpressure,
    /// The owned FT8 decoder rejected or failed a complete window.
    DecoderFailure,
    /// Atomic durable receive persistence failed.
    StorageFailure,
}

/// Explicit reason an active coordinator was stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveStopReason {
    /// A daemon-internal caller explicitly requested shutdown.
    Requested,
    /// Daemon cancellation requested shutdown.
    Cancelled,
}

/// Serialized lifecycle state for one daemon process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveLifecycleState {
    /// No input exists. This is also the only restart state.
    Stopped {
        /// Daemon process generation.
        process_generation: ProcessGeneration,
        /// Last issued stream generation, or zero before the first start.
        last_stream_generation: u64,
    },
    /// Exact selected input is being opened.
    Starting {
        /// Fresh stream generation reserved for this attempt.
        stream_generation: StreamGeneration,
    },
    /// Input and worker coordinator are active.
    Receiving {
        /// Current stream generation.
        stream_generation: StreamGeneration,
    },
    /// Input was stopped and no work will continue implicitly.
    Inhibited {
        /// Generation whose evidence became unsafe.
        stream_generation: StreamGeneration,
        /// Exact owned reason.
        reason: ReceiveInhibition,
    },
    /// Input shutdown is in progress.
    Stopping {
        /// Generation being stopped.
        stream_generation: StreamGeneration,
        /// Explicit shutdown source.
        reason: ReceiveStopReason,
    },
}

/// Internal lifecycle transition seam for the later public receive API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiveLifecycleEvent {
    /// State before the serialized transition.
    pub previous: ReceiveLifecycleState,
    /// State after the serialized transition.
    pub current: ReceiveLifecycleState,
}

/// Typed worker result. This is deliberately not a public wire event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceivePollEvent {
    /// One partial slot was deliberately withheld from decode.
    Incomplete {
        /// Exact slot start.
        slot_start_utc_millis: i64,
        /// Canonical samples accumulated before discard.
        accumulated_samples: u32,
    },
    /// One complete healthy window was atomically persisted.
    Persisted {
        /// Deterministic durable identity.
        receive_window_id: ReceiveWindowId,
        /// Idempotent insert/retry result.
        outcome: ReceiveInsertOutcome,
        /// Number of typed decode outcomes retained.
        decode_count: usize,
    },
    /// Receive transitioned to an inhibited state.
    Lifecycle(ReceiveLifecycleEvent),
}

/// Failure validating or running the daemon-internal coordinator.
#[derive(Debug, Error)]
pub enum ReceiveCoordinatorError {
    /// A lifecycle operation is invalid from the current state.
    #[error("receive lifecycle transition is invalid from {0:?}")]
    InvalidLifecycle(ReceiveLifecycleState),
    /// The stream generation counter overflowed.
    #[error("receive stream generation overflowed")]
    StreamGenerationOverflow,
    /// Audio generation construction failed.
    #[error("invalid receive stream generation")]
    InvalidStreamGeneration,
    /// Receive clock construction failed.
    #[error(transparent)]
    Clock(#[from] ReceiveClockError),
    /// Canonical PCM construction failed.
    #[error(transparent)]
    Pcm(#[from] PcmError),
    /// Deterministic receive identity construction failed.
    #[error(transparent)]
    Identity(#[from] IdError),
}

/// Sole live FT8 receive coordinator for one daemon process generation.
///
/// The value is intentionally inactive after construction or process restart.
/// No fault path switches devices, advances generations, or starts capture.
pub struct LiveReceiveCoordinator<I, D, S = Store> {
    input: I,
    decoder: D,
    store: S,
    service_instance_id: ServiceInstanceId,
    process_generation: ProcessGeneration,
    selection: ReceiveSelection,
    decode_config: Ft8DecodeConfig,
    clock_config: ReceiveClockConfig,
    state: ReceiveLifecycleState,
    last_stream_generation: u64,
    timeline: Option<Ft8ReceiveTimeline>,
    clock: Option<ReceiveClockMonitor>,
    worker_batches: VecDeque<CaptureBatch>,
}

impl<I: DaemonReceiveInput, D: Ft8OfflineDecoder, S: DaemonReceiveStore>
    LiveReceiveCoordinator<I, D, S>
{
    /// Composes an inactive coordinator. This never opens input.
    #[must_use]
    pub fn new(input: I, decoder: D, store: S, config: LiveReceiveCoordinatorConfig) -> Self {
        Self {
            input,
            decoder,
            store,
            service_instance_id: config.service_instance_id,
            process_generation: config.process_generation,
            selection: config.selection,
            decode_config: config.decode,
            clock_config: config.clock,
            state: ReceiveLifecycleState::Stopped {
                process_generation: config.process_generation,
                last_stream_generation: 0,
            },
            last_stream_generation: 0,
            timeline: None,
            clock: None,
            worker_batches: VecDeque::with_capacity(WORKER_BATCH_CAPACITY),
        }
    }

    /// Returns current serialized lifecycle state.
    #[must_use]
    pub const fn state(&self) -> ReceiveLifecycleState {
        self.state
    }

    /// Starts the exact selected input with a new stream/time generation.
    ///
    /// The supplied initial clock sample must be from the same numeric process
    /// generation. On any open failure, receive stays inhibited and inactive.
    pub fn start(
        &mut self,
        initial_clock: GenerationClockSample,
    ) -> Result<Vec<ReceiveLifecycleEvent>, ReceiveCoordinatorError> {
        if !matches!(self.state, ReceiveLifecycleState::Stopped { .. }) {
            return Err(ReceiveCoordinatorError::InvalidLifecycle(self.state));
        }
        let next = self
            .last_stream_generation
            .checked_add(1)
            .ok_or(ReceiveCoordinatorError::StreamGenerationOverflow)?;
        let stream_generation = StreamGeneration::new(next)
            .map_err(|_| ReceiveCoordinatorError::InvalidStreamGeneration)?;
        // Reserve every attempt so failed input or clock setup cannot cause a
        // later explicit restart to reuse stale lifecycle identity.
        self.last_stream_generation = next;
        let starting = ReceiveLifecycleState::Starting { stream_generation };
        let first = self.transition(starting);

        if initial_clock.generation.get() != self.process_generation.get() {
            let inhibited = ReceiveLifecycleState::Inhibited {
                stream_generation,
                reason: ReceiveInhibition::Clock(ReceiveClockFault::ProcessGenerationChanged),
            };
            return Ok(vec![first, self.transition(inhibited)]);
        }
        if let Err(error) =
            self.input
                .start(&self.selection, self.process_generation, stream_generation)
        {
            let inhibited = ReceiveLifecycleState::Inhibited {
                stream_generation,
                reason: ReceiveInhibition::Input(capture_error_kind(error)),
            };
            return Ok(vec![first, self.transition(inhibited)]);
        }

        let monitor = ReceiveClockMonitor::new(initial_clock, self.clock_config)?;
        self.timeline = Some(Ft8ReceiveTimeline::new(
            self.process_generation,
            stream_generation,
            self.selection.configuration,
        ));
        self.clock = Some(monitor);
        self.worker_batches.clear();
        let receiving = ReceiveLifecycleState::Receiving { stream_generation };
        Ok(vec![first, self.transition(receiving)])
    }

    /// Observes one explicit clock sample while active.
    ///
    /// Any unhealthy transition stops input and remains inhibited until an
    /// explicit stop followed by a fresh start.
    pub fn observe_clock(
        &mut self,
        observation: GenerationClockSample,
        observed_at: MonotonicInstant,
    ) -> Result<Option<ReceiveLifecycleEvent>, ReceiveCoordinatorError> {
        self.require_receiving()?;
        let transition = self
            .clock
            .as_mut()
            .ok_or(ReceiveCoordinatorError::InvalidLifecycle(self.state))?
            .observe(observation, observed_at);
        let snapshot = match transition {
            slotpilot_operations::ReceiveClockTransition::Healthy(snapshot)
            | slotpilot_operations::ReceiveClockTransition::Recovered(snapshot) => snapshot,
            slotpilot_operations::ReceiveClockTransition::BecameUnhealthy(snapshot)
            | slotpilot_operations::ReceiveClockTransition::Recovering(snapshot)
            | slotpilot_operations::ReceiveClockTransition::Unhealthy(snapshot) => snapshot,
        };
        if let ReceiveClockState::Unhealthy { fault, .. } = snapshot.state {
            return Ok(Some(self.inhibit(ReceiveInhibition::Clock(fault))));
        }
        Ok(None)
    }

    /// Performs one bounded worker poll without sleeping.
    ///
    /// At most `WORKER_BATCH_CAPACITY` batches are admitted and at most one is
    /// resampled/decoded/persisted per call.
    pub fn poll(
        &mut self,
        observed_at: MonotonicInstant,
        recorded_utc_millis: i64,
    ) -> Result<Vec<ReceivePollEvent>, ReceiveCoordinatorError> {
        self.require_receiving()?;

        if let Some(fault) = self.input.next_fault() {
            return Ok(vec![ReceivePollEvent::Lifecycle(
                self.inhibit(ReceiveInhibition::Input(fault.kind)),
            )]);
        }
        let audio_health = match self.input.health() {
            Ok(health) => health,
            Err(error) => {
                return Ok(vec![ReceivePollEvent::Lifecycle(
                    self.inhibit(ReceiveInhibition::Input(capture_error_kind(error))),
                )]);
            }
        };
        if audio_health.overflow_count > 0 {
            return Ok(vec![ReceivePollEvent::Lifecycle(self.inhibit(
                ReceiveInhibition::Input(InputFaultKind::Overflow { dropped_frames: 0 }),
            ))]);
        }

        let snapshot = self
            .clock
            .as_mut()
            .ok_or(ReceiveCoordinatorError::InvalidLifecycle(self.state))?
            .snapshot(observed_at);
        if let ReceiveClockState::Unhealthy { fault, .. } = snapshot.state {
            return Ok(vec![ReceivePollEvent::Lifecycle(
                self.inhibit(ReceiveInhibition::Clock(fault)),
            )]);
        }

        while self.worker_batches.len() < WORKER_BATCH_CAPACITY {
            match self.input.next_batch() {
                Ok(Some(batch)) => self.worker_batches.push_back(batch),
                Ok(None) => break,
                Err(error) => {
                    return Ok(vec![ReceivePollEvent::Lifecycle(
                        self.inhibit(ReceiveInhibition::Input(capture_error_kind(error))),
                    )]);
                }
            }
        }
        let Some(batch) = self.worker_batches.pop_front() else {
            return Ok(Vec::new());
        };
        self.process_batch(batch, audio_health, observed_at, recorded_utc_millis)
    }

    /// Explicitly stops or cancels receive. Inhibited input must pass through
    /// this boundary before it can be restarted.
    pub fn stop(
        &mut self,
        reason: ReceiveStopReason,
    ) -> Result<Vec<ReceiveLifecycleEvent>, ReceiveCoordinatorError> {
        let stream_generation = match self.state {
            ReceiveLifecycleState::Receiving { stream_generation }
            | ReceiveLifecycleState::Inhibited {
                stream_generation, ..
            } => stream_generation,
            _ => return Err(ReceiveCoordinatorError::InvalidLifecycle(self.state)),
        };
        let stopping = ReceiveLifecycleState::Stopping {
            stream_generation,
            reason,
        };
        let first = self.transition(stopping);
        let _ = self.input.stop();
        self.timeline = None;
        self.clock = None;
        self.worker_batches.clear();
        let stopped = ReceiveLifecycleState::Stopped {
            process_generation: self.process_generation,
            last_stream_generation: self.last_stream_generation,
        };
        Ok(vec![first, self.transition(stopped)])
    }

    fn process_batch(
        &mut self,
        batch: CaptureBatch,
        audio_health: InputHealth,
        observed_at: MonotonicInstant,
        recorded_utc_millis: i64,
    ) -> Result<Vec<ReceivePollEvent>, ReceiveCoordinatorError> {
        let events = match self
            .timeline
            .as_mut()
            .ok_or(ReceiveCoordinatorError::InvalidLifecycle(self.state))?
            .push(&batch, observed_at.millis())
        {
            Ok(events) => events,
            Err(error) => {
                return Ok(vec![ReceivePollEvent::Lifecycle(
                    self.inhibit(ReceiveInhibition::Timeline(error)),
                )]);
            }
        };
        let mut results = Vec::new();
        for event in events {
            let gated = self
                .clock
                .as_mut()
                .ok_or(ReceiveCoordinatorError::InvalidLifecycle(self.state))?
                .gate(event, observed_at);
            match gated {
                ClockGatedTimelineEvent::Incomplete(incomplete) => {
                    results.push(ReceivePollEvent::Incomplete {
                        slot_start_utc_millis: incomplete.slot.start_utc_unix_millis(),
                        accumulated_samples: incomplete.accumulated_samples,
                    });
                }
                ClockGatedTimelineEvent::WindowRejected { fault, .. } => {
                    results.push(ReceivePollEvent::Lifecycle(
                        self.inhibit(ReceiveInhibition::Clock(fault)),
                    ));
                    break;
                }
                ClockGatedTimelineEvent::WindowReady { window, alignment } => {
                    let format = PcmFormat::new(
                        window.sample_rate_hz(),
                        1,
                        PcmSampleFormat::Signed16LittleEndian,
                    )?;
                    let pcm = PcmBuffer::new(format, window.samples().to_vec())?;
                    let decodes = match self.decoder.decode(&pcm, self.decode_config) {
                        Ok(decodes) => decodes,
                        Err(_) => {
                            results.push(ReceivePollEvent::Lifecycle(
                                self.inhibit(ReceiveInhibition::DecoderFailure),
                            ));
                            break;
                        }
                    };
                    let receive_window_id = receive_window_id(
                        &self.service_instance_id,
                        &self.selection,
                        window.process_generation,
                        window.stream_generation,
                        window.slot_start_utc_millis,
                    )?;
                    let clock_health = ReceiveClockHealth::Healthy {
                        mapping_age_millis: alignment.mapping_age_millis,
                    };
                    let context = ReceiveWindowContext {
                        receive_window_id: receive_window_id.clone(),
                        service_instance_id: self.service_instance_id.clone(),
                        process_generation: window.process_generation,
                        stream_generation: window.stream_generation,
                        slot: window.slot(),
                        device_identity: self.selection.device_identity.clone(),
                        configuration: self.selection.configuration,
                        capture_mapping: window.mapping,
                    };
                    let diagnostics = ReceiveDiagnosticSummary {
                        audio: audio_health,
                        timeline: self
                            .timeline
                            .as_ref()
                            .ok_or(ReceiveCoordinatorError::InvalidLifecycle(self.state))?
                            .health(),
                        clock: clock_health,
                    };
                    let record = match ReceiveRecord::new(
                        context,
                        diagnostics,
                        decodes,
                        recorded_utc_millis,
                    ) {
                        Ok(record) => record,
                        Err(_) => {
                            results.push(ReceivePollEvent::Lifecycle(
                                self.inhibit(ReceiveInhibition::StorageFailure),
                            ));
                            break;
                        }
                    };
                    let decode_count = record.decodes().len();
                    match self.store.record_receive(&record) {
                        Ok(outcome) => results.push(ReceivePollEvent::Persisted {
                            receive_window_id,
                            outcome,
                            decode_count,
                        }),
                        Err(_) => {
                            results.push(ReceivePollEvent::Lifecycle(
                                self.inhibit(ReceiveInhibition::StorageFailure),
                            ));
                            break;
                        }
                    }
                }
            }
        }
        Ok(results)
    }

    fn require_receiving(&self) -> Result<StreamGeneration, ReceiveCoordinatorError> {
        match self.state {
            ReceiveLifecycleState::Receiving { stream_generation } => Ok(stream_generation),
            _ => Err(ReceiveCoordinatorError::InvalidLifecycle(self.state)),
        }
    }

    fn transition(&mut self, current: ReceiveLifecycleState) -> ReceiveLifecycleEvent {
        let previous = self.state;
        self.state = current;
        ReceiveLifecycleEvent { previous, current }
    }

    fn inhibit(&mut self, reason: ReceiveInhibition) -> ReceiveLifecycleEvent {
        let stream_generation = match self.state {
            ReceiveLifecycleState::Receiving { stream_generation }
            | ReceiveLifecycleState::Starting { stream_generation }
            | ReceiveLifecycleState::Inhibited {
                stream_generation, ..
            }
            | ReceiveLifecycleState::Stopping {
                stream_generation, ..
            } => stream_generation,
            ReceiveLifecycleState::Stopped { .. } => {
                return self.transition(self.state);
            }
        };
        let _ = self.input.stop();
        self.worker_batches.clear();
        self.timeline = None;
        self.clock = None;
        self.transition(ReceiveLifecycleState::Inhibited {
            stream_generation,
            reason,
        })
    }
}

fn capture_error_kind(error: InputCaptureError) -> InputFaultKind {
    match error {
        InputCaptureError::PermissionDenied => InputFaultKind::PermissionDenied,
        InputCaptureError::DeviceLost => InputFaultKind::DeviceLost,
        InputCaptureError::UnsupportedConfiguration
        | InputCaptureError::InvalidQueueCapacity
        | InputCaptureError::Stopped
        | InputCaptureError::BackendFailure => InputFaultKind::BackendFailure,
    }
}

fn receive_window_id(
    service_instance_id: &ServiceInstanceId,
    selection: &ReceiveSelection,
    process_generation: ProcessGeneration,
    stream_generation: StreamGeneration,
    slot_start_utc_millis: i64,
) -> Result<ReceiveWindowId, IdError> {
    let mut digest = Sha256::new();
    digest.update(service_instance_id.as_str().as_bytes());
    digest.update(process_generation.get().to_be_bytes());
    digest.update(stream_generation.get().to_be_bytes());
    digest.update(slot_start_utc_millis.to_be_bytes());
    digest.update([platform_code(selection.device_identity.platform())]);
    digest.update(selection.device_identity.opaque_id().as_bytes());
    digest.update(selection.configuration.sample_rate_hz().to_be_bytes());
    digest.update(selection.configuration.channels().to_be_bytes());
    digest.update([sample_format_code(selection.configuration.sample_format())]);
    digest.update(selection.configuration.selected_channel().to_be_bytes());
    format!("rxw_{:x}", digest.finalize()).parse()
}

const fn platform_code(platform: InputPlatform) -> u8 {
    match platform {
        InputPlatform::MacOsCoreAudio => 1,
        InputPlatform::WindowsWasapi => 2,
        InputPlatform::LinuxAlsa => 3,
        InputPlatform::LinuxJack => 4,
    }
}

const fn sample_format_code(format: InputSampleFormat) -> u8 {
    match format {
        InputSampleFormat::Signed8 => 1,
        InputSampleFormat::Signed16 => 2,
        InputSampleFormat::Signed24 => 3,
        InputSampleFormat::Signed32 => 4,
        InputSampleFormat::Signed64 => 5,
        InputSampleFormat::Unsigned8 => 6,
        InputSampleFormat::Unsigned16 => 7,
        InputSampleFormat::Unsigned24 => 8,
        InputSampleFormat::Unsigned32 => 9,
        InputSampleFormat::Unsigned64 => 10,
        InputSampleFormat::Float32 => 11,
        InputSampleFormat::Float64 => 12,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use slotpilot_audio::{
        CaptureDiagnostics, CapturePosition, CaptureTimeEvidence, FT8_RECEIVE_WINDOW_SAMPLES,
        InputSampleFormat,
    };
    use slotpilot_domain::AudioFrequency;
    use slotpilot_operations::{ClockProcessGeneration, ClockSample, UtcInstant};
    use slotpilot_protocol::{
        AmbiguousFt8Message, ClassifiedFt8Message, FreeTextFt8Message, Ft8Decode, Ft8DecodeDepth,
        Ft8DecodeError, Ft8DecodeMetadata, Ft8MessageClass, ResolvedFt8Message,
        UnresolvedHashFt8Message, UnsupportedFt8Message,
    };

    use super::*;

    #[derive(Default)]
    struct FakeInput {
        active: bool,
        starts: Vec<StreamGeneration>,
        stops: usize,
        batches: VecDeque<CaptureBatch>,
        faults: VecDeque<InputFault>,
        start_error: Option<InputCaptureError>,
        next_error: Option<InputCaptureError>,
        health_error: Option<InputCaptureError>,
        health: Option<InputHealth>,
    }

    impl DaemonReceiveInput for FakeInput {
        fn start(
            &mut self,
            _selection: &ReceiveSelection,
            _process_generation: ProcessGeneration,
            stream_generation: StreamGeneration,
        ) -> Result<(), InputCaptureError> {
            if let Some(error) = self.start_error.take() {
                return Err(error);
            }
            self.active = true;
            self.starts.push(stream_generation);
            Ok(())
        }

        fn next_batch(&mut self) -> Result<Option<CaptureBatch>, InputCaptureError> {
            if let Some(error) = self.next_error.take() {
                return Err(error);
            }
            Ok(self.batches.pop_front())
        }

        fn next_fault(&mut self) -> Option<InputFault> {
            self.faults.pop_front()
        }

        fn health(&mut self) -> Result<InputHealth, InputCaptureError> {
            if let Some(error) = self.health_error.take() {
                return Err(error);
            }
            Ok(self.health.unwrap_or_else(healthy_audio))
        }

        fn stop(&mut self) -> Result<(), InputCaptureError> {
            self.active = false;
            self.stops += 1;
            Ok(())
        }
    }

    #[derive(Clone)]
    struct FakeDecoder {
        results: Result<Vec<Ft8Decode>, Ft8DecodeError>,
    }

    struct FailingStore;

    impl DaemonReceiveStore for FailingStore {
        fn record_receive(
            &mut self,
            _record: &ReceiveRecord,
        ) -> Result<ReceiveInsertOutcome, StorageError> {
            Err(StorageError::InvalidReceiveRecord("injected failure"))
        }
    }

    impl Ft8OfflineDecoder for FakeDecoder {
        fn decode(
            &self,
            _pcm: &PcmBuffer,
            _config: Ft8DecodeConfig,
        ) -> Result<Vec<Ft8Decode>, Ft8DecodeError> {
            self.results.clone()
        }
    }

    fn process_generation() -> ProcessGeneration {
        ProcessGeneration::new(7).unwrap()
    }

    fn clock_generation() -> ClockProcessGeneration {
        ClockProcessGeneration::new(7).unwrap()
    }

    fn selection() -> ReceiveSelection {
        ReceiveSelection {
            device_identity: InputDeviceIdentity::new(
                InputPlatform::MacOsCoreAudio,
                "stable-input-1",
            )
            .unwrap(),
            configuration: InputConfiguration::new(12_000, 1, InputSampleFormat::Signed16, 0)
                .unwrap(),
        }
    }

    fn decode_config() -> Ft8DecodeConfig {
        Ft8DecodeConfig::new(
            AudioFrequency::from_hz(600).unwrap(),
            AudioFrequency::from_hz(1_800).unwrap(),
            1_000,
            Ft8DecodeDepth::Normal,
            20,
        )
        .unwrap()
    }

    fn initial_clock() -> GenerationClockSample {
        clock_at(1_000)
    }

    fn clock_at(monotonic_millis: u64) -> GenerationClockSample {
        GenerationClockSample {
            generation: clock_generation(),
            sample: ClockSample {
                utc: UtcInstant::from_unix_millis(
                    30_000 + i64::try_from(monotonic_millis - 1_000).unwrap(),
                )
                .unwrap(),
                monotonic: MonotonicInstant::from_millis(monotonic_millis),
            },
        }
    }

    fn healthy_audio() -> InputHealth {
        InputHealth::new(10, 0, 0, 0, 5).unwrap()
    }

    fn coordinator(
        input: FakeInput,
        decoder: FakeDecoder,
    ) -> LiveReceiveCoordinator<FakeInput, FakeDecoder> {
        LiveReceiveCoordinator::new(
            input,
            decoder,
            Store::open_in_memory().unwrap(),
            LiveReceiveCoordinatorConfig {
                service_instance_id: "svc_phase2001".parse().unwrap(),
                process_generation: process_generation(),
                selection: selection(),
                decode: decode_config(),
                clock: ReceiveClockConfig::default(),
            },
        )
    }

    fn silence_batch(stream_generation: u64) -> CaptureBatch {
        CaptureBatch::new(
            process_generation(),
            StreamGeneration::new(stream_generation).unwrap(),
            selection().configuration,
            CaptureTimeEvidence::new(CapturePosition::from_frames(0), 30_000, 1_000).unwrap(),
            None,
            CaptureDiagnostics::new(0, 1).unwrap(),
            vec![0; FT8_RECEIVE_WINDOW_SAMPLES.min(8_192)],
        )
        .unwrap()
    }

    fn silence_batches(stream_generation: u64) -> VecDeque<CaptureBatch> {
        let mut batches = VecDeque::new();
        let mut position = 0_u64;
        while position < FT8_RECEIVE_WINDOW_SAMPLES as u64 {
            let remaining = FT8_RECEIVE_WINDOW_SAMPLES as u64 - position;
            let count = remaining.min(8_192) as usize;
            batches.push_back(
                CaptureBatch::new(
                    process_generation(),
                    StreamGeneration::new(stream_generation).unwrap(),
                    selection().configuration,
                    CaptureTimeEvidence::new(
                        CapturePosition::from_frames(position),
                        30_000 + i64::try_from(position / 12).unwrap(),
                        1_000 + position / 12,
                    )
                    .unwrap(),
                    None,
                    CaptureDiagnostics::new(0, 1).unwrap(),
                    vec![0; count],
                )
                .unwrap(),
            );
            position += count as u64;
        }
        batches
    }

    fn all_classifications() -> Vec<Ft8Decode> {
        let metadata = |offset| Ft8DecodeMetadata {
            start_offset_millis: offset,
            audio_frequency_hz: 1_000,
            signal_to_noise_db: -10,
        };
        vec![
            Ft8Decode {
                metadata: metadata(0),
                message: ClassifiedFt8Message::Resolved(
                    ResolvedFt8Message::new(
                        "CQ K1ABC FN42",
                        "K1ABC".parse().unwrap(),
                        None,
                        Ft8MessageClass::GeneralCall,
                    )
                    .unwrap(),
                ),
            },
            Ft8Decode {
                metadata: metadata(1),
                message: ClassifiedFt8Message::UnresolvedHash(
                    UnresolvedHashFt8Message::new("<HASH> K1ABC", "unresolved sender").unwrap(),
                ),
            },
            Ft8Decode {
                metadata: metadata(2),
                message: ClassifiedFt8Message::Unsupported(
                    UnsupportedFt8Message::new("K1ABC RR73", "unsupported structure").unwrap(),
                ),
            },
            Ft8Decode {
                metadata: metadata(3),
                message: ClassifiedFt8Message::Ambiguous(
                    AmbiguousFt8Message::new("CQ TEST", "multiple interpretations").unwrap(),
                ),
            },
            Ft8Decode {
                metadata: metadata(4),
                message: ClassifiedFt8Message::FreeText(
                    FreeTextFt8Message::new("HELLO WORLD").unwrap(),
                ),
            },
        ]
    }

    #[test]
    fn construction_and_restart_are_inactive_and_generations_advance() {
        let mut coordinator = coordinator(
            FakeInput::default(),
            FakeDecoder {
                results: Ok(Vec::new()),
            },
        );
        assert!(matches!(
            coordinator.state(),
            ReceiveLifecycleState::Stopped {
                last_stream_generation: 0,
                ..
            }
        ));
        assert_eq!(coordinator.start(initial_clock()).unwrap().len(), 2);
        assert!(matches!(
            coordinator.state(),
            ReceiveLifecycleState::Receiving { stream_generation }
                if stream_generation.get() == 1
        ));
        assert_eq!(
            coordinator
                .stop(ReceiveStopReason::Requested)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(coordinator.start(initial_clock()).unwrap().len(), 2);
        assert!(matches!(
            coordinator.state(),
            ReceiveLifecycleState::Receiving { stream_generation }
                if stream_generation.get() == 2
        ));
    }

    #[test]
    fn complete_healthy_window_preserves_all_classifications_and_retries_identity() {
        let input = FakeInput {
            batches: silence_batches(1),
            ..FakeInput::default()
        };
        let mut coordinator = coordinator(
            input,
            FakeDecoder {
                results: Ok(all_classifications()),
            },
        );
        coordinator.start(initial_clock()).unwrap();
        let mut persisted = None;
        for step in 0..40 {
            let observed_millis = 1_000 + step * 700;
            coordinator
                .observe_clock(
                    clock_at(observed_millis),
                    MonotonicInstant::from_millis(observed_millis),
                )
                .unwrap();
            let events = coordinator
                .poll(
                    MonotonicInstant::from_millis(observed_millis),
                    30_000 + i64::try_from(observed_millis - 1_000).unwrap(),
                )
                .unwrap();
            for event in events {
                if let ReceivePollEvent::Persisted {
                    receive_window_id,
                    outcome,
                    decode_count,
                } = event
                {
                    assert!(matches!(outcome, ReceiveInsertOutcome::Inserted { .. }));
                    assert_eq!(decode_count, 5);
                    persisted = Some(receive_window_id);
                }
            }
        }
        let id = persisted.expect("window should persist");
        let stored = coordinator
            .store
            .receive_record(&id)
            .unwrap()
            .expect("stored record");
        assert_eq!(stored.record.decodes().len(), 5);
        assert!(matches!(
            stored.record.decodes()[0].message,
            ClassifiedFt8Message::Resolved(_)
        ));
        let exact = stored.record.clone();
        assert!(matches!(
            coordinator.store.record_receive(&exact).unwrap(),
            ReceiveInsertOutcome::Existing { .. }
        ));
    }

    #[test]
    fn input_faults_inhibit_without_fallback_or_automatic_restart() {
        for kind in [
            InputFaultKind::DeviceLost,
            InputFaultKind::Overflow { dropped_frames: 12 },
            InputFaultKind::Discontinuity(slotpilot_audio::CaptureDiscontinuityKind::BackendGap),
        ] {
            let mut input = FakeInput::default();
            input.faults.push_back(InputFault {
                process_generation: process_generation(),
                stream_generation: Some(StreamGeneration::new(1).unwrap()),
                monotonic_millis: 1_001,
                kind,
            });
            let mut coordinator = coordinator(
                input,
                FakeDecoder {
                    results: Ok(Vec::new()),
                },
            );
            coordinator.start(initial_clock()).unwrap();
            let event = coordinator
                .poll(MonotonicInstant::from_millis(1_001), 30_001)
                .unwrap();
            assert!(matches!(
                event.as_slice(),
                [ReceivePollEvent::Lifecycle(ReceiveLifecycleEvent {
                    current: ReceiveLifecycleState::Inhibited {
                        reason: ReceiveInhibition::Input(observed),
                        ..
                    },
                    ..
                })] if *observed == kind
            ));
            assert!(matches!(
                coordinator.start(initial_clock()),
                Err(ReceiveCoordinatorError::InvalidLifecycle(_))
            ));
        }
    }

    #[test]
    fn stale_clock_decoder_storage_shutdown_and_cancellation_are_explicit() {
        let mut stale = coordinator(
            FakeInput::default(),
            FakeDecoder {
                results: Ok(Vec::new()),
            },
        );
        stale.start(initial_clock()).unwrap();
        let events = stale
            .poll(MonotonicInstant::from_millis(3_501), 32_501)
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ReceivePollEvent::Lifecycle(ReceiveLifecycleEvent {
                current: ReceiveLifecycleState::Inhibited {
                    reason: ReceiveInhibition::Clock(ReceiveClockFault::StaleMapping { .. }),
                    ..
                },
                ..
            })]
        ));
        let cancelled = stale.stop(ReceiveStopReason::Cancelled).unwrap();
        assert!(matches!(
            cancelled[0].current,
            ReceiveLifecycleState::Stopping {
                reason: ReceiveStopReason::Cancelled,
                ..
            }
        ));

        let decoder_error = Ft8DecodeError::InvalidConfiguration {
            detail: "injected".into(),
        };
        let input = FakeInput {
            batches: silence_batches(1),
            ..FakeInput::default()
        };
        let mut failed = coordinator(
            input,
            FakeDecoder {
                results: Err(decoder_error),
            },
        );
        failed.start(initial_clock()).unwrap();
        let mut saw_failure = false;
        for step in 0..40 {
            let observed_millis = 1_000 + step * 700;
            failed
                .observe_clock(
                    clock_at(observed_millis),
                    MonotonicInstant::from_millis(observed_millis),
                )
                .unwrap();
            let events = failed
                .poll(
                    MonotonicInstant::from_millis(observed_millis),
                    30_000 + i64::try_from(observed_millis - 1_000).unwrap(),
                )
                .unwrap();
            saw_failure |= events.iter().any(|event| {
                matches!(
                    event,
                    ReceivePollEvent::Lifecycle(ReceiveLifecycleEvent {
                        current: ReceiveLifecycleState::Inhibited {
                            reason: ReceiveInhibition::DecoderFailure,
                            ..
                        },
                        ..
                    })
                )
            });
            if saw_failure {
                break;
            }
        }
        assert!(saw_failure);
    }

    #[test]
    fn worker_admission_is_bounded_and_timeline_failure_inhibits() {
        let mut input = FakeInput::default();
        for _ in 0..20 {
            input.batches.push_back(silence_batch(1));
        }
        let mut coordinator = coordinator(
            input,
            FakeDecoder {
                results: Ok(Vec::new()),
            },
        );
        coordinator.start(initial_clock()).unwrap();
        let _ = coordinator
            .poll(MonotonicInstant::from_millis(1_000), 30_000)
            .unwrap();
        assert!(coordinator.worker_batches.len() < WORKER_BATCH_CAPACITY);
        let events = coordinator
            .poll(MonotonicInstant::from_millis(1_001), 30_001)
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ReceivePollEvent::Lifecycle(ReceiveLifecycleEvent {
                current: ReceiveLifecycleState::Inhibited {
                    reason: ReceiveInhibition::Timeline(_),
                    ..
                },
                ..
            })]
        ));
    }

    #[test]
    fn wrong_clock_generation_and_start_failure_are_visible() {
        let mut wrong_clock = initial_clock();
        wrong_clock.generation = ClockProcessGeneration::new(8).unwrap();
        let mut wrong_generation_coordinator = coordinator(
            FakeInput::default(),
            FakeDecoder {
                results: Ok(Vec::new()),
            },
        );
        assert_eq!(
            wrong_generation_coordinator
                .start(wrong_clock)
                .unwrap()
                .len(),
            2
        );
        assert!(matches!(
            wrong_generation_coordinator.state(),
            ReceiveLifecycleState::Inhibited {
                reason: ReceiveInhibition::Clock(ReceiveClockFault::ProcessGenerationChanged),
                ..
            }
        ));

        let input = FakeInput {
            start_error: Some(InputCaptureError::DeviceLost),
            ..FakeInput::default()
        };
        let mut failed = coordinator(
            input,
            FakeDecoder {
                results: Ok(Vec::new()),
            },
        );
        failed.start(initial_clock()).unwrap();
        assert!(matches!(
            failed.state(),
            ReceiveLifecycleState::Inhibited {
                reason: ReceiveInhibition::Input(InputFaultKind::DeviceLost),
                ..
            }
        ));
    }

    #[test]
    fn storage_failure_stops_input_and_inhibits() {
        let input = FakeInput {
            batches: silence_batches(1),
            ..FakeInput::default()
        };
        let mut coordinator = LiveReceiveCoordinator::new(
            input,
            FakeDecoder {
                results: Ok(Vec::new()),
            },
            FailingStore,
            LiveReceiveCoordinatorConfig {
                service_instance_id: "svc_phase2001".parse().unwrap(),
                process_generation: process_generation(),
                selection: selection(),
                decode: decode_config(),
                clock: ReceiveClockConfig::default(),
            },
        );
        coordinator.start(initial_clock()).unwrap();
        let mut saw_failure = false;
        for step in 0..40 {
            let observed_millis = 1_000 + step * 700;
            coordinator
                .observe_clock(
                    clock_at(observed_millis),
                    MonotonicInstant::from_millis(observed_millis),
                )
                .unwrap();
            let events = coordinator
                .poll(
                    MonotonicInstant::from_millis(observed_millis),
                    30_000 + i64::try_from(observed_millis - 1_000).unwrap(),
                )
                .unwrap();
            saw_failure |= events.iter().any(|event| {
                matches!(
                    event,
                    ReceivePollEvent::Lifecycle(ReceiveLifecycleEvent {
                        current: ReceiveLifecycleState::Inhibited {
                            reason: ReceiveInhibition::StorageFailure,
                            ..
                        },
                        ..
                    })
                )
            });
            if saw_failure {
                break;
            }
        }
        assert!(saw_failure);
    }

    #[test]
    fn production_store_couples_committed_decode_to_ordered_event() {
        let input = FakeInput {
            batches: silence_batches(1),
            ..FakeInput::default()
        };
        let service_instance_id: ServiceInstanceId = "svc_phase2001".parse().unwrap();
        let mut coordinator = LiveReceiveCoordinator::new(
            input,
            FakeDecoder {
                results: Ok(Vec::new()),
            },
            crate::PublicReceiveStore::in_memory().unwrap(),
            LiveReceiveCoordinatorConfig {
                service_instance_id: service_instance_id.clone(),
                process_generation: process_generation(),
                selection: selection(),
                decode: decode_config(),
                clock: ReceiveClockConfig::default(),
            },
        );
        coordinator.start(initial_clock()).unwrap();
        for step in 0..40 {
            let observed_millis = 1_000 + step * 700;
            coordinator
                .observe_clock(
                    clock_at(observed_millis),
                    MonotonicInstant::from_millis(observed_millis),
                )
                .unwrap();
            let _ = coordinator
                .poll(
                    MonotonicInstant::from_millis(observed_millis),
                    30_000 + i64::try_from(observed_millis - 1_000).unwrap(),
                )
                .unwrap();
        }
        let store = coordinator.store.into_inner();
        let events = store
            .replay_events(&service_instance_id, 0, 10)
            .unwrap()
            .events;
        assert_eq!(events.len(), 1);
        let payload: slotpilot_api::EventPayload =
            serde_json::from_str(&events[0].event_json).unwrap();
        assert!(matches!(
            payload,
            slotpilot_api::EventPayload::ReceiveDecode(_)
        ));
    }
}
