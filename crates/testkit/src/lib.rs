//! Deterministic fakes that cannot discover or access physical hardware.
//!
//! These in-memory implementations contain no CPAL, Hamlib, rigctld, device
//! discovery, OS transport, network, file, PTT, or RF path.

use std::collections::VecDeque;

use slotpilot_domain::TransmissionId;
use slotpilot_operations::{
    AudioFault, AudioHealth, AudioPort, EmergencyUnkeyError, ProtocolPort, RigCommand, RigFault,
    RigPort, RigState, TransmitSupervisorPort, TxInhibition,
};
use slotpilot_protocol::{Ft8Decode, Ft8WaveformError, Ft8WaveformRequest, PcmBuffer, PcmError};

/// Deterministic in-memory rig port.
#[derive(Debug, Clone)]
pub struct FakeRig {
    state: RigState,
    faults: VecDeque<RigFault>,
}

impl FakeRig {
    /// Creates a fake with one initial verified state.
    #[must_use]
    pub fn new(state: RigState) -> Self {
        Self {
            state,
            faults: VecDeque::new(),
        }
    }

    /// Queues one failure for the next operation.
    pub fn inject(&mut self, fault: RigFault) {
        self.faults.push_back(fault);
    }
}

impl RigPort for FakeRig {
    fn read_state(&mut self) -> Result<RigState, RigFault> {
        if let Some(fault) = self.faults.pop_front() {
            return Err(fault);
        }
        Ok(self.state.clone())
    }

    fn apply(&mut self, command: RigCommand) -> Result<RigState, RigFault> {
        if let Some(fault) = self.faults.pop_front() {
            return Err(fault);
        }
        match command {
            RigCommand::SetDialFrequency(value) => self.state.dial_frequency = value,
            RigCommand::SetMode(value) => self.state.mode = value,
        }
        Ok(self.state.clone())
    }
}

/// Deterministic in-memory audio-health port.
#[derive(Debug, Clone)]
pub struct FakeAudio {
    health: AudioHealth,
    faults: VecDeque<AudioFault>,
}

impl FakeAudio {
    /// Creates a fake with initial health.
    #[must_use]
    pub fn new(health: AudioHealth) -> Self {
        Self {
            health,
            faults: VecDeque::new(),
        }
    }

    /// Queues one timestamped failure for the next health query.
    pub fn inject(&mut self, fault: AudioFault) {
        self.faults.push_back(fault);
    }
}

impl AudioPort for FakeAudio {
    fn health(&mut self) -> Result<AudioHealth, AudioFault> {
        if let Some(fault) = self.faults.pop_front() {
            return Err(fault);
        }
        Ok(self.health.clone())
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
    use slotpilot_domain::{DialFrequency, OperatingMode, Power};
    use slotpilot_operations::{
        AudioFaultKind, Clock, ClockSample, MonotonicInstant, UtcInstant, VirtualClock,
    };
    use slotpilot_protocol::{
        ClassifiedFt8Message, Ft8DecodeMetadata, Ft8MessageClass, Ft8WaveformPlacement,
        PcmAmplitudePermille, PcmFormat, PcmSampleFormat, ResolvedFt8Message,
    };

    use super::*;

    fn rig_state() -> RigState {
        RigState {
            dial_frequency: DialFrequency::from_hz(14_074_000).unwrap(),
            mode: OperatingMode::Ft8,
            power: Power::from_milliwatts(5_000).unwrap(),
            ptt_asserted: false,
        }
    }

    #[test]
    fn fake_rig_injects_every_required_failure() {
        let mut rig = FakeRig::new(rig_state());
        let contradictory = RigFault::ContradictoryReadback {
            expected: rig_state(),
            observed: RigState {
                ptt_asserted: true,
                ..rig_state()
            },
        };
        for fault in [
            RigFault::Disconnected,
            RigFault::StaleReadback,
            contradictory,
            RigFault::CommandRejected,
            RigFault::UnexpectedMovement(rig_state()),
            RigFault::PttStuck,
        ] {
            rig.inject(fault.clone());
            assert_eq!(rig.read_state(), Err(fault));
        }
    }

    #[test]
    fn fake_audio_injects_timestamped_failures_under_virtual_time() {
        let clock = VirtualClock::new(ClockSample {
            utc: UtcInstant::from_unix_millis(1_000).unwrap(),
            monotonic: MonotonicInstant::from_millis(10),
        });
        let mut audio = FakeAudio::new(AudioHealth {
            latency_millis: 20,
            drift_parts_per_million: 0,
        });
        let kinds = [
            AudioFaultKind::DeviceLost,
            AudioFaultKind::Overrun,
            AudioFaultKind::Underrun,
            AudioFaultKind::Clipping,
            AudioFaultKind::Drift {
                parts_per_million: 50,
            },
            AudioFaultKind::Latency { millis: 200 },
            AudioFaultKind::CallbackDelay { millis: 40 },
        ];
        for kind in kinds {
            let fault = AudioFault {
                occurred_at: clock.sample().monotonic,
                kind,
            };
            audio.inject(fault.clone());
            assert_eq!(audio.health(), Err(fault));
            clock.advance(1).unwrap();
        }
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
