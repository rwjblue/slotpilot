//! Deterministic fakes that cannot discover or access physical hardware.
//!
//! These in-memory implementations contain no CPAL, Hamlib, rigctld, device
//! discovery, OS transport, network, file, PTT, or RF path.

use std::collections::VecDeque;

use slotpilot_audio::{CaptureBatch, InputFault, InputHealth};
use slotpilot_domain::{ProfileRevisionId, RigProfile, TransmissionId};
use slotpilot_operations::{
    Clock, EmergencyUnkeyError, ProtocolPort, ReadOnlyRigPort, ReceiveAudioPort,
    RigCapabilityReport, RigConnectionGeneration, RigFault, RigFreshnessPolicy, RigLifecycleState,
    RigObservation, RigObservationAge, TransmitSupervisorPort, TxInhibition, VirtualClock,
};
use slotpilot_protocol::{Ft8Decode, Ft8WaveformError, Ft8WaveformRequest, PcmBuffer, PcmError};
use thiserror::Error;

/// Maximum scripted calls retained by one deterministic rig fake.
pub const MAX_FAKE_RIG_STEPS: usize = 64;

/// Failure scripting the bounded deterministic rig fake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FakeRigScriptError {
    /// The fixed script bound was reached.
    #[error("fake rig script exceeds its fixed bound")]
    ScriptFull,
}

#[derive(Debug, Clone)]
enum FakeRigStep {
    Connect(Result<RigConnectionGeneration, RigFault>),
    Probe(Result<RigCapabilityReport, RigFault>),
    Read(Result<RigObservation, RigFault>),
}

/// Deterministic in-memory read-only rig port with injected time.
#[derive(Debug, Clone)]
pub struct FakeReadOnlyRig {
    clock: VirtualClock,
    lifecycle: RigLifecycleState,
    profile_revision_id: Option<ProfileRevisionId>,
    steps: VecDeque<FakeRigStep>,
}

impl FakeReadOnlyRig {
    /// Creates a disconnected fake driven by the supplied virtual clock.
    #[must_use]
    pub fn new(clock: VirtualClock) -> Self {
        Self {
            clock,
            lifecycle: RigLifecycleState::Disconnected,
            profile_revision_id: None,
            steps: VecDeque::new(),
        }
    }

    /// Queues one deterministic connect result.
    pub fn queue_connect(
        &mut self,
        result: Result<RigConnectionGeneration, RigFault>,
    ) -> Result<(), FakeRigScriptError> {
        self.push(FakeRigStep::Connect(result))
    }

    /// Queues one deterministic capability-probe result.
    pub fn queue_probe(
        &mut self,
        result: Result<RigCapabilityReport, RigFault>,
    ) -> Result<(), FakeRigScriptError> {
        self.push(FakeRigStep::Probe(result))
    }

    /// Queues one deterministic observation result.
    pub fn queue_read(
        &mut self,
        result: Result<RigObservation, RigFault>,
    ) -> Result<(), FakeRigScriptError> {
        self.push(FakeRigStep::Read(result))
    }

    /// Reads once and checks freshness against the injected virtual clock.
    pub fn read_fresh(
        &mut self,
        policy: RigFreshnessPolicy,
    ) -> Result<(RigObservation, RigObservationAge), RigFault> {
        let observation = self.read()?;
        match observation.fresh_at(self.clock.sample(), policy) {
            Ok(age) => Ok((observation, age)),
            Err(fault) => {
                self.fail(&fault);
                Err(fault)
            }
        }
    }

    /// Returns the injected clock so tests can advance it without sleeping.
    #[must_use]
    pub fn clock(&self) -> &VirtualClock {
        &self.clock
    }

    /// Returns the number of unconsumed bounded script steps.
    #[must_use]
    pub fn remaining_steps(&self) -> usize {
        self.steps.len()
    }

    fn push(&mut self, step: FakeRigStep) -> Result<(), FakeRigScriptError> {
        if self.steps.len() == MAX_FAKE_RIG_STEPS {
            return Err(FakeRigScriptError::ScriptFull);
        }
        self.steps.push_back(step);
        Ok(())
    }

    fn current_generation(&self) -> Option<RigConnectionGeneration> {
        match self.lifecycle {
            RigLifecycleState::Connected { generation }
            | RigLifecycleState::Probing { generation }
            | RigLifecycleState::Ready { generation } => Some(generation),
            RigLifecycleState::Faulted { generation, .. } => generation,
            RigLifecycleState::Disconnected | RigLifecycleState::Connecting => None,
        }
    }

    fn fail(&mut self, fault: &RigFault) {
        self.lifecycle = RigLifecycleState::Faulted {
            generation: self.current_generation(),
            fault: fault.kind(),
        };
    }

    fn malformed(&mut self) -> RigFault {
        let fault = RigFault::MalformedResponse;
        self.fail(&fault);
        fault
    }
}

impl ReadOnlyRigPort for FakeReadOnlyRig {
    fn lifecycle(&self) -> RigLifecycleState {
        self.lifecycle
    }

    fn connect(&mut self, profile: &RigProfile) -> Result<RigConnectionGeneration, RigFault> {
        self.lifecycle = RigLifecycleState::Connecting;
        let Some(FakeRigStep::Connect(result)) = self.steps.pop_front() else {
            return Err(self.malformed());
        };
        match result {
            Ok(generation) => {
                self.profile_revision_id = Some(profile.revision_id().clone());
                self.lifecycle = RigLifecycleState::Connected { generation };
                Ok(generation)
            }
            Err(fault) => {
                self.fail(&fault);
                Err(fault)
            }
        }
    }

    fn probe(&mut self) -> Result<RigCapabilityReport, RigFault> {
        let Some(generation) = self.current_generation() else {
            return Err(RigFault::NotConnected);
        };
        if !matches!(self.lifecycle, RigLifecycleState::Connected { .. }) {
            return Err(RigFault::NotConnected);
        }
        self.lifecycle = RigLifecycleState::Probing { generation };
        let Some(FakeRigStep::Probe(result)) = self.steps.pop_front() else {
            return Err(self.malformed());
        };
        match result {
            Ok(report) if report.generation() == generation => {
                self.lifecycle = RigLifecycleState::Ready { generation };
                Ok(report)
            }
            Ok(report) => {
                let fault = RigFault::GenerationChanged {
                    expected: generation,
                    observed: report.generation(),
                };
                self.fail(&fault);
                Err(fault)
            }
            Err(fault) => {
                self.fail(&fault);
                Err(fault)
            }
        }
    }

    fn read(&mut self) -> Result<RigObservation, RigFault> {
        let RigLifecycleState::Ready { generation } = self.lifecycle else {
            return Err(if self.current_generation().is_some() {
                RigFault::NotProbed
            } else {
                RigFault::NotConnected
            });
        };
        let Some(FakeRigStep::Read(result)) = self.steps.pop_front() else {
            return Err(self.malformed());
        };
        match result {
            Ok(observation)
                if observation.provenance.connection_generation == generation
                    && self.profile_revision_id.as_ref()
                        == Some(&observation.profile_revision_id) =>
            {
                Ok(observation)
            }
            Ok(observation) if observation.provenance.connection_generation != generation => {
                let fault = RigFault::GenerationChanged {
                    expected: generation,
                    observed: observation.provenance.connection_generation,
                };
                self.fail(&fault);
                Err(fault)
            }
            Ok(_) => {
                let fault = RigFault::ContradictoryReadback {
                    field: slotpilot_operations::RigObservedField::ProfileRevision,
                };
                self.fail(&fault);
                Err(fault)
            }
            Err(fault) => {
                self.fail(&fault);
                Err(fault)
            }
        }
    }
}

/// Deterministic in-memory receive-audio port.
#[derive(Debug, Clone)]
pub struct FakeInputAudio {
    health: InputHealth,
    events: VecDeque<Result<CaptureBatch, InputFault>>,
}

impl FakeInputAudio {
    /// Creates a fake with initial health.
    #[must_use]
    pub fn new(health: InputHealth) -> Self {
        Self {
            health,
            events: VecDeque::new(),
        }
    }

    /// Queues one normal bounded capture batch.
    pub fn emit(&mut self, batch: CaptureBatch) {
        self.events.push_back(Ok(batch));
    }

    /// Queues one timestamped failure.
    pub fn inject(&mut self, fault: InputFault) {
        self.events.push_back(Err(fault));
    }

    /// Replaces the deterministic health snapshot.
    pub fn set_health(&mut self, health: InputHealth) {
        self.health = health;
    }
}

impl ReceiveAudioPort for FakeInputAudio {
    fn health(&mut self) -> Result<InputHealth, InputFault> {
        if let Some(Err(fault)) = self.events.front() {
            return Err(fault.clone());
        }
        Ok(self.health)
    }

    fn next_batch(&mut self) -> Result<Option<CaptureBatch>, InputFault> {
        self.events.pop_front().transpose()
    }
}

/// Deterministic typed message and placeholder-waveform port.
#[derive(Debug, Default, Clone)]
pub struct FakeProtocol {
    decodes: VecDeque<Ft8Decode>,
}

impl FakeProtocol {
    /// Creates an empty fake protocol boundary.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues one typed decode.
    pub fn emit(&mut self, decode: Ft8Decode) {
        self.decodes.push_back(decode);
    }
}

impl ProtocolPort for FakeProtocol {
    fn drain_decodes(&mut self) -> Vec<Ft8Decode> {
        let mut decodes: Vec<_> = self.decodes.drain(..).collect();
        Ft8Decode::sort_deterministically(&mut decodes);
        decodes
    }

    fn prepare_waveform(
        &mut self,
        request: &Ft8WaveformRequest,
    ) -> Result<PcmBuffer, Ft8WaveformError> {
        let marker = i16::try_from(request.message.canonical_text().len())
            .map_err(|_| PcmError::BufferTooLarge)?;
        Ok(PcmBuffer::new(request.format, vec![marker; 8])?)
    }
}

/// Deterministic logical transmit-supervisor port with no PTT implementation.
#[derive(Debug, Default, Clone)]
pub struct FakeTransmitSupervisor {
    inhibition: Option<TxInhibition>,
    ptt_stuck: bool,
    emergency_unkey_calls: usize,
}

impl FakeTransmitSupervisor {
    /// Creates an uninhibited logical fake.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets or clears the next admission outcome.
    pub fn set_inhibition(&mut self, inhibition: Option<TxInhibition>) {
        self.inhibition = inhibition;
    }

    /// Controls the fake emergency-unkey result.
    pub fn set_ptt_stuck(&mut self, stuck: bool) {
        self.ptt_stuck = stuck;
    }

    /// Returns the number of logical emergency-unkey calls observed.
    #[must_use]
    pub const fn emergency_unkey_calls(&self) -> usize {
        self.emergency_unkey_calls
    }
}

impl TransmitSupervisorPort for FakeTransmitSupervisor {
    fn admit(&mut self, _transmission_id: &TransmissionId) -> Result<(), TxInhibition> {
        self.inhibition.clone().map_or(Ok(()), Err)
    }

    fn emergency_unkey(&mut self) -> Result<(), EmergencyUnkeyError> {
        self.emergency_unkey_calls += 1;
        if self.ptt_stuck {
            Err(EmergencyUnkeyError::PttStuck)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use slotpilot_audio::{
        CaptureDiagnostics, CaptureDiscontinuity, CaptureDiscontinuityKind, CapturePosition,
        CaptureTimeEvidence, InputConfiguration, InputFaultKind, InputSampleFormat,
        ProcessGeneration, StreamGeneration,
    };
    use slotpilot_domain::{
        DialFrequency, DownstreamRigEndpoint, HamlibModelId, HamlibVersionExpectation,
        RadioModulation, RadioPassband, RigVfo, RigctldMode, RigctldServiceEndpoint, SplitReadback,
    };
    use slotpilot_operations::{
        ClockSample, MonotonicInstant, RigCapability, RigCapabilityEvidence, RigCapabilityStatus,
        RigObservationFields, RigObservationProvenance, RigObservationSequence,
        RigObservationTimestamp, RigObservedField, RigOperation, RigReadback, UtcInstant,
    };
    use slotpilot_protocol::{
        ClassifiedFt8Message, Ft8DecodeMetadata, Ft8MessageClass, Ft8WaveformPlacement,
        PcmAmplitudePermille, PcmFormat, PcmSampleFormat, ResolvedFt8Message,
    };

    use super::*;

    fn virtual_clock() -> VirtualClock {
        VirtualClock::new(ClockSample {
            utc: UtcInstant::from_unix_millis(1_000).unwrap(),
            monotonic: MonotonicInstant::from_millis(10),
        })
    }

    fn rig_profile() -> RigProfile {
        RigProfile::new(
            "prv_01jrigprofile".parse().unwrap(),
            DownstreamRigEndpoint::new("k4.operator.lan", 12_345).unwrap(),
            RigctldServiceEndpoint::new("127.0.0.1", 4_532).unwrap(),
            RigctldMode::Managed,
            HamlibModelId::new(2_047).unwrap(),
            HamlibVersionExpectation::new("4.7.1").unwrap(),
            RadioModulation::DataUpperSideband,
            RadioPassband::from_hz(3_000).unwrap(),
        )
        .unwrap()
    }

    fn capability_report(
        generation: RigConnectionGeneration,
        partial: bool,
    ) -> RigCapabilityReport {
        let mut evidence = vec![
            RigCapabilityEvidence {
                capability: RigCapability::DialFrequency,
                status: RigCapabilityStatus::RuntimeProbed,
            },
            RigCapabilityEvidence {
                capability: RigCapability::Modulation,
                status: RigCapabilityStatus::RuntimeProbed,
            },
        ];
        if partial {
            evidence.push(RigCapabilityEvidence {
                capability: RigCapability::Power,
                status: RigCapabilityStatus::Unsupported,
            });
        }
        RigCapabilityReport::new(generation, evidence).unwrap()
    }

    fn observation(
        generation: RigConnectionGeneration,
        timestamp: RigObservationTimestamp,
    ) -> RigObservation {
        RigObservation {
            profile_revision_id: rig_profile().revision_id().clone(),
            provenance: RigObservationProvenance {
                service_instance_id: "svc_01jrigprocess".parse().unwrap(),
                connection_generation: generation,
                sequence: RigObservationSequence::new(1).unwrap(),
            },
            observed_at: timestamp,
            fields: RigObservationFields {
                dial_frequency: RigReadback::Observed(DialFrequency::from_hz(14_074_000).unwrap()),
                modulation: RigReadback::Observed(RadioModulation::DataUpperSideband),
                passband: RigReadback::Observed(RadioPassband::from_hz(3_000).unwrap()),
                vfo: RigReadback::Observed(RigVfo::A),
                split: RigReadback::Observed(SplitReadback::new(false, None)),
                power: RigReadback::Unavailable,
                ptt_asserted: RigReadback::Unsupported,
            },
        }
    }

    #[test]
    fn fake_rig_runs_normal_partial_reconnect_and_generation_sequences() {
        let clock = virtual_clock();
        let mut rig = FakeReadOnlyRig::new(clock.clone());
        let first = RigConnectionGeneration::new(1).unwrap();
        let second = RigConnectionGeneration::new(2).unwrap();
        rig.queue_connect(Ok(first)).unwrap();
        rig.queue_probe(Ok(capability_report(first, true))).unwrap();
        rig.queue_read(Ok(observation(
            first,
            RigObservationTimestamp::from_clock_sample(clock.sample()),
        )))
        .unwrap();
        rig.queue_read(Err(RigFault::Disconnected)).unwrap();
        rig.queue_connect(Ok(second)).unwrap();
        rig.queue_probe(Ok(capability_report(second, false)))
            .unwrap();
        rig.queue_read(Ok(observation(
            second,
            RigObservationTimestamp::from_clock_sample(clock.sample()),
        )))
        .unwrap();

        assert_eq!(rig.connect(&rig_profile()).unwrap(), first);
        assert_eq!(
            rig.probe().unwrap().status(RigCapability::Power),
            Some(RigCapabilityStatus::Unsupported)
        );
        assert!(rig.read().is_ok());
        assert_eq!(rig.read(), Err(RigFault::Disconnected));
        assert_eq!(rig.connect(&rig_profile()).unwrap(), second);
        assert_eq!(rig.probe().unwrap().generation(), second);
        assert_eq!(rig.read().unwrap().provenance.connection_generation, second);
        assert_eq!(rig.remaining_steps(), 0);
    }

    #[test]
    fn fake_rig_injects_all_bounded_read_only_failures() {
        let failures = [
            RigFault::Timeout {
                operation: RigOperation::Read,
            },
            RigFault::ContradictoryReadback {
                field: RigObservedField::Split,
            },
            RigFault::Unsupported {
                capability: RigCapability::Ptt,
            },
            RigFault::MalformedResponse,
            RigFault::UnexpectedChange {
                field: RigObservedField::DialFrequency,
            },
        ];
        for fault in failures {
            let clock = virtual_clock();
            let generation = RigConnectionGeneration::new(1).unwrap();
            let mut rig = FakeReadOnlyRig::new(clock);
            rig.queue_connect(Ok(generation)).unwrap();
            rig.queue_probe(Ok(capability_report(generation, false)))
                .unwrap();
            rig.queue_read(Err(fault.clone())).unwrap();
            rig.connect(&rig_profile()).unwrap();
            rig.probe().unwrap();
            assert_eq!(rig.read(), Err(fault.clone()));
            assert_eq!(
                rig.lifecycle(),
                RigLifecycleState::Faulted {
                    generation: Some(generation),
                    fault: fault.kind(),
                }
            );
        }
    }

    #[test]
    fn fake_rig_checks_stale_time_and_generation_without_sleeping() {
        let clock = virtual_clock();
        let first = RigConnectionGeneration::new(1).unwrap();
        let second = RigConnectionGeneration::new(2).unwrap();
        let mut stale = FakeReadOnlyRig::new(clock.clone());
        stale.queue_connect(Ok(first)).unwrap();
        stale
            .queue_probe(Ok(capability_report(first, false)))
            .unwrap();
        stale
            .queue_read(Ok(observation(
                first,
                RigObservationTimestamp::from_clock_sample(clock.sample()),
            )))
            .unwrap();
        stale.connect(&rig_profile()).unwrap();
        stale.probe().unwrap();
        clock.advance(101).unwrap();
        assert!(matches!(
            stale.read_fresh(RigFreshnessPolicy::new(100, 5).unwrap()),
            Err(RigFault::Stale { .. })
        ));

        let mut changed = FakeReadOnlyRig::new(virtual_clock());
        changed.queue_connect(Ok(first)).unwrap();
        changed
            .queue_probe(Ok(capability_report(first, false)))
            .unwrap();
        changed
            .queue_read(Ok(observation(
                second,
                RigObservationTimestamp::new(1_000, 10).unwrap(),
            )))
            .unwrap();
        changed.connect(&rig_profile()).unwrap();
        changed.probe().unwrap();
        assert_eq!(
            changed.read(),
            Err(RigFault::GenerationChanged {
                expected: first,
                observed: second,
            })
        );
    }

    #[test]
    fn fake_rig_script_has_a_fixed_bound() {
        let mut rig = FakeReadOnlyRig::new(virtual_clock());
        for _ in 0..MAX_FAKE_RIG_STEPS {
            rig.queue_connect(Err(RigFault::Disconnected)).unwrap();
        }
        assert_eq!(
            rig.queue_connect(Err(RigFault::Disconnected)),
            Err(FakeRigScriptError::ScriptFull)
        );
    }

    #[test]
    fn fake_audio_produces_batches_and_timestamped_failures_under_virtual_time() {
        let clock = VirtualClock::new(ClockSample {
            utc: UtcInstant::from_unix_millis(1_000).unwrap(),
            monotonic: MonotonicInstant::from_millis(10),
        });
        let process_generation = ProcessGeneration::new(1).unwrap();
        let stream_generation = StreamGeneration::new(1).unwrap();
        let configuration =
            InputConfiguration::new(48_000, 1, InputSampleFormat::Float32, 0).unwrap();
        let mut audio = FakeInputAudio::new(InputHealth::new(20, 0, 0, 0, 1).unwrap());
        let batch = CaptureBatch::new(
            process_generation,
            stream_generation,
            configuration,
            CaptureTimeEvidence::new(
                CapturePosition::from_frames(0),
                1_000,
                clock.sample().monotonic.millis(),
            )
            .unwrap(),
            None,
            CaptureDiagnostics::new(0, 1).unwrap(),
            vec![0; 64],
        )
        .unwrap();
        audio.emit(batch.clone());
        assert_eq!(audio.next_batch().unwrap(), Some(batch));
        let kinds = [
            InputFaultKind::DeviceLost,
            InputFaultKind::Overflow { dropped_frames: 64 },
            InputFaultKind::Discontinuity(CaptureDiscontinuityKind::BackendGap),
            InputFaultKind::Clipping { sample_count: 2 },
            InputFaultKind::Drift {
                parts_per_million: 50,
            },
            InputFaultKind::CallbackDelay { millis: 40 },
            InputFaultKind::BackendFailure,
        ];
        for kind in kinds {
            let fault = InputFault {
                process_generation,
                stream_generation: Some(stream_generation),
                monotonic_millis: clock.sample().monotonic.millis(),
                kind,
            };
            audio.inject(fault.clone());
            assert_eq!(audio.health(), Err(fault.clone()));
            assert_eq!(audio.next_batch(), Err(fault));
            clock.advance(1).unwrap();
        }
        let discontinuity = CaptureDiscontinuity {
            at: CapturePosition::from_frames(128),
            kind: CaptureDiscontinuityKind::Overflow,
            dropped_frames: 64,
        };
        let marked = CaptureBatch::new(
            process_generation,
            stream_generation,
            configuration,
            CaptureTimeEvidence::new(
                CapturePosition::from_frames(128),
                1_001,
                clock.sample().monotonic.millis(),
            )
            .unwrap(),
            Some(discontinuity),
            CaptureDiagnostics::new(2, 40).unwrap(),
            vec![i16::MAX; 64],
        )
        .unwrap();
        audio.emit(marked.clone());
        assert_eq!(audio.next_batch().unwrap(), Some(marked));
    }

    #[test]
    fn fake_protocol_emits_typed_messages_and_deterministic_samples() {
        let message = ResolvedFt8Message::new(
            "K1ABC W1AW -12",
            "K1ABC".parse().unwrap(),
            Some("W1AW".parse().unwrap()),
            Ft8MessageClass::SignalReport,
        )
        .unwrap();
        let decode = Ft8Decode {
            metadata: Ft8DecodeMetadata {
                start_offset_millis: 42,
                audio_frequency_hz: 1_000,
                signal_to_noise_db: -12,
            },
            message: ClassifiedFt8Message::Resolved(message.clone()),
        };
        let request = Ft8WaveformRequest {
            message,
            format: PcmFormat::new(12_000, 1, PcmSampleFormat::Signed16LittleEndian).unwrap(),
            audio_frequency: slotpilot_domain::AudioFrequency::from_hz(1_000).unwrap(),
            amplitude: PcmAmplitudePermille::new(500).unwrap(),
            placement: Ft8WaveformPlacement::FrameOnly,
        };
        let mut protocol = FakeProtocol::new();
        protocol.emit(decode.clone());
        assert_eq!(protocol.drain_decodes(), vec![decode]);
        assert_eq!(
            protocol.prepare_waveform(&request).unwrap().samples(),
            vec![14; 8]
        );
    }

    #[test]
    fn supervisor_exposes_inhibition_and_emergency_unkey() {
        let mut supervisor = FakeTransmitSupervisor::new();
        let transmission: TransmissionId = "txm_01jabcde9".parse().unwrap();
        supervisor.set_inhibition(Some(TxInhibition::ClockUnhealthy));
        assert_eq!(
            supervisor.admit(&transmission),
            Err(TxInhibition::ClockUnhealthy)
        );
        supervisor.set_ptt_stuck(true);
        assert_eq!(
            supervisor.emergency_unkey(),
            Err(EmergencyUnkeyError::PttStuck)
        );
        assert_eq!(supervisor.emergency_unkey_calls(), 1);
    }
}
