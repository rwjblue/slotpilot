//! Hardware-free Phase 2 receive conformance through the public CLI and IPC.

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::thread;

use slotpilot_api::{
    API_VERSION, Command, EventCursor, EventPayload, Ft8Classification,
    InputConfiguration as ApiInputConfiguration, InputConfigurationRange, InputDevice,
    InputDeviceIdentity as ApiInputDeviceIdentity, InputDevicePage,
    InputPlatform as ApiInputPlatform, InputSampleFormat as ApiInputSampleFormat,
    ReceiveAudioHealth, ReceiveClockHealth as ApiReceiveClockHealth, ReceiveHistoryPage,
    ReceiveLifecycleSnapshot, ReceiveSelection as ApiReceiveSelection, ReceiveStatus,
    ResponseOutcome, ResultBody, SubscriptionOutcome, SubscriptionRequest,
};
use slotpilot_audio::{
    CaptureBatch, CaptureDiagnostics, CapturePosition, CaptureTimeEvidence,
    FT8_RECEIVE_WINDOW_SAMPLES, InputCaptureError, InputConfiguration, InputDeviceIdentity,
    InputFault, InputHealth, InputPlatform, InputSampleFormat, ProcessGeneration, SpectrumConfig,
    StreamGeneration,
};
use slotpilot_domain::{AudioFrequency, EventId, RequestId, ServiceInstanceId};
use slotpilot_ipc::{CancellationToken, EndpointAddress, LocalServer};
use slotpilot_operations::{
    ClockProcessGeneration, ClockSample, GenerationClockSample, MonotonicInstant,
    ReceiveClockConfig, UtcInstant,
};
use slotpilot_protocol::{
    ClassifiedFt8Message, Ft8Decode, Ft8DecodeConfig, Ft8DecodeDepth, Ft8DecodeError,
    Ft8DecodeMetadata, Ft8MessageClass, Ft8OfflineDecoder, PcmBuffer, ResolvedFt8Message,
};
use slotpilot_storage::{
    ReceiveInsertOutcome, ReceiveRecord, SequencedReceiveRecord, StorageError, Store,
};
use slotpilotd::{
    DaemonReceiveInput, DaemonReceiveStore, EventService, LiveReceiveCoordinator,
    LiveReceiveCoordinatorConfig, ReceiveApiFault, ReceiveApiPort, ReceiveApiProcessor,
    ReceiveLifecycleState, ReceivePollEvent, ReceiveSelection, ReceiveStopReason, map_lifecycle,
    map_receive_page, map_receive_record, map_waterfall_snapshot, serve_receive_once,
};

const PROCESS_GENERATION: u64 = 7;
const SERVICE_ID: &str = "svc_conformance";

struct ReplayInput {
    batches: VecDeque<CaptureBatch>,
    active: bool,
}

impl DaemonReceiveInput for ReplayInput {
    fn start(
        &mut self,
        selection: &ReceiveSelection,
        process_generation: ProcessGeneration,
        stream_generation: StreamGeneration,
    ) -> Result<(), InputCaptureError> {
        if selection != &receive_selection()
            || process_generation.get() != PROCESS_GENERATION
            || stream_generation.get() != 1
        {
            return Err(InputCaptureError::UnsupportedConfiguration);
        }
        self.active = true;
        Ok(())
    }

    fn next_batch(&mut self) -> Result<Option<CaptureBatch>, InputCaptureError> {
        Ok(self.batches.pop_front())
    }

    fn next_fault(&mut self) -> Option<InputFault> {
        None
    }

    fn health(&mut self) -> Result<InputHealth, InputCaptureError> {
        if !self.active {
            return Err(InputCaptureError::Stopped);
        }
        InputHealth::new(10, 0, 0, 0, 5).map_err(|_| InputCaptureError::BackendFailure)
    }

    fn stop(&mut self) -> Result<(), InputCaptureError> {
        self.active = false;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct ReplayDecoder;

impl Ft8OfflineDecoder for ReplayDecoder {
    fn decode(
        &self,
        _pcm: &PcmBuffer,
        _config: Ft8DecodeConfig,
    ) -> Result<Vec<Ft8Decode>, Ft8DecodeError> {
        Ok(vec![Ft8Decode {
            metadata: Ft8DecodeMetadata {
                start_offset_millis: 0,
                audio_frequency_hz: 1_000,
                signal_to_noise_db: -10,
            },
            message: ClassifiedFt8Message::Resolved(
                ResolvedFt8Message::new(
                    "CQ K1ABC FN42",
                    "K1ABC".parse().expect("fixture callsign"),
                    None,
                    Ft8MessageClass::GeneralCall,
                )
                .expect("fixture message"),
            ),
        }])
    }
}

#[derive(Clone)]
struct CoupledStore {
    path: PathBuf,
}

impl DaemonReceiveStore for CoupledStore {
    fn record_receive(
        &mut self,
        record: &ReceiveRecord,
    ) -> Result<ReceiveInsertOutcome, StorageError> {
        let suffix = record
            .context()
            .receive_window_id
            .as_str()
            .strip_prefix("rxw_")
            .ok_or(StorageError::InvalidReceiveEventPayload)?;
        let event_id: EventId = format!("evt_{suffix}").parse()?;
        let mut store = Store::open(&self.path)?;
        let commit = store.record_receive_with_event_builder(
            record,
            &event_id,
            record.recorded_utc_millis(),
            |sequence| {
                serde_json::to_string(&EventPayload::ReceiveDecode(map_receive_record(
                    &SequencedReceiveRecord {
                        sequence,
                        record: record.clone(),
                    },
                )))
                .map_err(|_| StorageError::InvalidReceiveEventPayload)
            },
        )?;
        Ok(commit.receive)
    }
}

struct ReplayPort {
    coordinator: LiveReceiveCoordinator<ReplayInput, ReplayDecoder, CoupledStore>,
    database: PathBuf,
    selected: Option<ApiReceiveSelection>,
    next_waterfall_sequence: u64,
}

impl ReplayPort {
    fn new(database: PathBuf) -> Self {
        let input = ReplayInput {
            batches: replay_batches(),
            active: false,
        };
        let mut coordinator = LiveReceiveCoordinator::new(
            input,
            ReplayDecoder,
            CoupledStore {
                path: database.clone(),
            },
            LiveReceiveCoordinatorConfig {
                service_instance_id: service_id(),
                process_generation: process_generation(),
                selection: receive_selection(),
                decode: decode_config(),
                clock: ReceiveClockConfig::default(),
            },
        );
        coordinator
            .enable_spectrum(SpectrumConfig::default())
            .expect("spectrum model");
        Self {
            coordinator,
            database,
            selected: None,
            next_waterfall_sequence: 0,
        }
    }

    fn publish_waterfall(
        &mut self,
        snapshot: &slotpilot_audio::WaterfallSnapshot,
    ) -> Result<(), ReceiveApiFault> {
        self.next_waterfall_sequence += 1;
        let frame = map_waterfall_snapshot(snapshot, self.next_waterfall_sequence)
            .ok_or(ReceiveApiFault::InvalidRequest)?;
        let mut events = EventService::open(&self.database, service_id())
            .map_err(|_| ReceiveApiFault::StorageUnavailable)?;
        events
            .publish_event(
                "evt_waterfall01".parse().expect("fixture event id"),
                EventPayload::WaterfallFrame(frame),
                30_000,
            )
            .map_err(|_| ReceiveApiFault::StorageUnavailable)?;
        Ok(())
    }

    fn drive_replay(&mut self) -> Result<(), ReceiveApiFault> {
        for step in 0..40_u64 {
            let observed = 1_000 + step * 700;
            self.coordinator
                .observe_clock(clock_at(observed), MonotonicInstant::from_millis(observed))
                .map_err(|_| ReceiveApiFault::InvalidRequest)?;
            let events = self
                .coordinator
                .poll(
                    MonotonicInstant::from_millis(observed),
                    30_000
                        + i64::try_from(observed - 1_000)
                            .map_err(|_| ReceiveApiFault::InvalidRequest)?,
                )
                .map_err(|_| ReceiveApiFault::InvalidRequest)?;
            for event in events {
                if let ReceivePollEvent::Waterfall(snapshot) = event {
                    self.publish_waterfall(&snapshot)?;
                }
            }
        }
        Ok(())
    }

    fn current_status(&self) -> ReceiveStatus {
        let active = matches!(
            self.coordinator.state(),
            ReceiveLifecycleState::Receiving { .. }
        );
        ReceiveStatus {
            lifecycle: map_lifecycle(self.coordinator.state()),
            selection: self.selected.clone(),
            audio: active.then_some(ReceiveAudioHealth {
                latency_millis: 10,
                drift_parts_per_million: 0,
                overflow_count: 0,
                clipped_sample_count: 0,
                max_callback_delay_millis: 5,
            }),
            clock: active.then_some(ApiReceiveClockHealth::Healthy {
                mapping_age_millis: 0,
            }),
        }
    }
}

impl ReceiveApiPort for ReplayPort {
    fn input_devices(&mut self) -> Result<InputDevicePage, ReceiveApiFault> {
        Ok(InputDevicePage {
            devices: vec![InputDevice {
                identity: api_selection().device_identity,
                display_name: "Replay Input".into(),
                manufacturer: Some("SlotPilot test".into()),
                configuration_ranges: vec![InputConfigurationRange {
                    min_sample_rate_hz: 12_000,
                    max_sample_rate_hz: 12_000,
                    channels: 1,
                    sample_format: ApiInputSampleFormat::Signed16,
                }],
            }],
        })
    }

    fn status(&mut self) -> Result<ReceiveStatus, ReceiveApiFault> {
        Ok(self.current_status())
    }

    fn start(
        &mut self,
        _request_id: &RequestId,
        selection: &ApiReceiveSelection,
    ) -> Result<ReceiveStatus, ReceiveApiFault> {
        if selection != &api_selection() {
            return Err(ReceiveApiFault::InvalidRequest);
        }
        self.coordinator
            .start(clock_at(1_000))
            .map_err(|_| ReceiveApiFault::InvalidRequest)?;
        self.selected = Some(selection.clone());
        self.drive_replay()?;
        Ok(self.current_status())
    }

    fn stop(&mut self, _request_id: &RequestId) -> Result<ReceiveStatus, ReceiveApiFault> {
        self.coordinator
            .stop(ReceiveStopReason::Requested)
            .map_err(|_| ReceiveApiFault::InvalidRequest)?;
        Ok(self.current_status())
    }

    fn history(
        &mut self,
        after_sequence: u64,
        limit: u16,
    ) -> Result<ReceiveHistoryPage, ReceiveApiFault> {
        let store = Store::open(&self.database).map_err(|_| ReceiveApiFault::StorageUnavailable)?;
        let page = store
            .receive_page(after_sequence, usize::from(limit))
            .map_err(|_| ReceiveApiFault::StorageUnavailable)?;
        Ok(map_receive_page(page, after_sequence))
    }
}

#[test]
fn public_api_cli_ipc_replay_conformance_is_bounded_and_restart_safe() {
    let directory = unique_directory("phase2-conformance");
    let database = directory.join("receive.sqlite3");
    let journal = directory.join("journal.sqlite3");
    let address = EndpointAddress::for_user(&directory, "phase0-dev").expect("endpoint");
    let server = LocalServer::bind(&address).expect("bind command endpoint");
    let processor =
        ReceiveApiProcessor::open(&journal, service_id(), ReplayPort::new(database.clone()))
            .expect("processor");
    let cancellation = CancellationToken::new();
    let server_cancellation = cancellation.clone();
    let handle = thread::spawn(move || {
        let mut processor = processor;
        for accepted in 1..=5 {
            serve_receive_once(&server, &mut processor, accepted, &server_cancellation)
                .expect("serve receive command");
        }
    });

    let devices = request(&address, "req_devices01", Command::ListInputDevices);
    assert!(matches!(
        devices.outcome,
        ResponseOutcome::Success(ResultBody::InputDevices(InputDevicePage { ref devices }))
            if devices.len() == 1
    ));

    let started = request(
        &address,
        "req_start0001",
        Command::ReceiveStart {
            selection: api_selection(),
        },
    );
    assert!(matches!(
        started.outcome,
        ResponseOutcome::Success(ResultBody::ReceiveStarted(
            ReceiveLifecycleSnapshot::Receiving {
                stream_generation: 1
            }
        ))
    ));

    let status = request(&address, "req_status001", Command::GetReceiveStatus);
    assert!(slotpilot::render_table(&status).contains("receiving"));
    assert!(
        slotpilot::render_json(&status)
            .expect("render status JSON")
            .contains("\"stream_generation\":1")
    );

    let history = request(
        &address,
        "req_history01",
        Command::QueryReceiveHistory {
            after_sequence: 0,
            limit: 100,
        },
    );
    assert!(matches!(
        history.outcome,
        ResponseOutcome::Success(ResultBody::ReceiveHistory(ReceiveHistoryPage {
            ref records,
            has_more: false,
            ..
        })) if records.len() == 1
            && matches!(
                records[0].decodes[0].classification,
                Ft8Classification::Resolved { .. }
            )
    ));

    let stopped = request(&address, "req_stop00001", Command::ReceiveStop);
    assert!(matches!(
        stopped.outcome,
        ResponseOutcome::Success(ResultBody::ReceiveStopped(
            ReceiveLifecycleSnapshot::Stopped {
                last_stream_generation: 1
            }
        ))
    ));
    handle.join().expect("command server");

    let event_server = LocalServer::bind(&address).expect("rebind event endpoint");
    let events = EventService::open(&database, service_id()).expect("event service");
    let event_cancellation = cancellation.clone();
    let event_handle = thread::spawn(move || {
        event_server
            .serve_exchange(&event_cancellation, |request: SubscriptionRequest| {
                events.subscribe(request).expect("subscribe events")
            })
            .expect("serve event replay");
    });
    let replay = slotpilot::request_events(
        &address,
        &SubscriptionRequest {
            api_version: API_VERSION,
            after: Some(EventCursor {
                service_instance_id: service_id(),
                sequence: 0,
            }),
            limit: 256,
        },
        &cancellation,
    )
    .expect("request event replay");
    event_handle.join().expect("event server");
    assert!(matches!(
        replay.outcome,
        SubscriptionOutcome::Events {
            ref events,
            has_more: false,
            ..
        } if events.len() == 2
            && matches!(events[0].event, EventPayload::ReceiveDecode(_))
            && matches!(events[1].event, EventPayload::WaterfallFrame(_))
    ));
    assert_eq!(
        slotpilot::render_jsonl(&replay)
            .expect("render JSONL")
            .lines()
            .count(),
        2
    );

    let restart_server = LocalServer::bind(&address).expect("rebind restart endpoint");
    let restart_processor = ReceiveApiProcessor::open(
        &journal,
        "svc_conformance2".parse().expect("restart service id"),
        ReplayPort::new(database),
    )
    .expect("restart processor");
    let restart_cancellation = cancellation.clone();
    let restart_handle = thread::spawn(move || {
        let mut processor = restart_processor;
        serve_receive_once(&restart_server, &mut processor, 9, &restart_cancellation)
            .expect("serve restart status");
    });
    let restarted = request(&address, "req_restart01", Command::GetReceiveStatus);
    restart_handle.join().expect("restart server");
    assert!(matches!(
        restarted.outcome,
        ResponseOutcome::Success(ResultBody::ReceiveStatus(ReceiveStatus {
            lifecycle: ReceiveLifecycleSnapshot::Stopped {
                last_stream_generation: 0
            },
            ..
        }))
    ));

    fs::remove_dir_all(directory).expect("remove test runtime");
}

fn request(
    address: &EndpointAddress,
    request_id: &str,
    command: Command,
) -> slotpilot_api::ResponseEnvelope {
    slotpilot::request_command(
        address,
        request_id.parse().expect("request id"),
        command,
        &CancellationToken::new(),
    )
    .expect("request command")
}

fn replay_batches() -> VecDeque<CaptureBatch> {
    let mut batches = VecDeque::new();
    let mut position = 0_u64;
    while position < FT8_RECEIVE_WINDOW_SAMPLES as u64 {
        let remaining = FT8_RECEIVE_WINDOW_SAMPLES as u64 - position;
        let count = remaining.min(8_192) as usize;
        batches.push_back(
            CaptureBatch::new(
                process_generation(),
                StreamGeneration::new(1).expect("stream generation"),
                receive_selection().configuration,
                CaptureTimeEvidence::new(
                    CapturePosition::from_frames(position),
                    30_000 + i64::try_from(position / 12).expect("UTC offset"),
                    1_000 + position / 12,
                )
                .expect("capture time"),
                None,
                CaptureDiagnostics::new(0, 1).expect("capture diagnostics"),
                vec![0; count],
            )
            .expect("capture batch"),
        );
        position += u64::try_from(count).expect("batch length");
    }
    batches
}

fn process_generation() -> ProcessGeneration {
    ProcessGeneration::new(PROCESS_GENERATION).expect("process generation")
}

fn service_id() -> ServiceInstanceId {
    SERVICE_ID.parse().expect("service id")
}

fn receive_selection() -> ReceiveSelection {
    ReceiveSelection {
        device_identity: InputDeviceIdentity::new(
            InputPlatform::MacOsCoreAudio,
            "stable-replay-input",
        )
        .expect("device identity"),
        configuration: InputConfiguration::new(12_000, 1, InputSampleFormat::Signed16, 0)
            .expect("input configuration"),
    }
}

fn api_selection() -> ApiReceiveSelection {
    ApiReceiveSelection {
        device_identity: ApiInputDeviceIdentity {
            platform: ApiInputPlatform::MacOsCoreAudio,
            opaque_id: "stable-replay-input".into(),
        },
        configuration: ApiInputConfiguration {
            sample_rate_hz: 12_000,
            channels: 1,
            sample_format: ApiInputSampleFormat::Signed16,
            selected_channel: 0,
        },
    }
}

fn decode_config() -> Ft8DecodeConfig {
    Ft8DecodeConfig::new(
        AudioFrequency::from_hz(600).expect("minimum audio frequency"),
        AudioFrequency::from_hz(1_800).expect("maximum audio frequency"),
        1_000,
        Ft8DecodeDepth::Normal,
        20,
    )
    .expect("decode config")
}

fn clock_at(monotonic_millis: u64) -> GenerationClockSample {
    GenerationClockSample {
        generation: ClockProcessGeneration::new(PROCESS_GENERATION).expect("clock generation"),
        sample: ClockSample {
            utc: UtcInstant::from_unix_millis(
                30_000 + i64::try_from(monotonic_millis - 1_000).expect("clock offset"),
            )
            .expect("UTC instant"),
            monotonic: MonotonicInstant::from_millis(monotonic_millis),
        },
    }
}

fn unique_directory(label: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    #[cfg(unix)]
    let base = PathBuf::from("/tmp");
    #[cfg(not(unix))]
    let base = std::env::temp_dir();
    base.join(format!("{label}-{}-{nonce:x}", std::process::id()))
}
