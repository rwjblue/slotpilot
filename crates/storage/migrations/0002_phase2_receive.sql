CREATE TABLE receive_windows (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    receive_window_id TEXT NOT NULL UNIQUE CHECK (
        length(receive_window_id) BETWEEN 12 AND 68
        AND substr(receive_window_id, 1, 4) = 'rxw_'
        AND substr(receive_window_id, 5) NOT GLOB '*[^a-z0-9]*'
    ),
    service_instance_id TEXT NOT NULL CHECK (
        length(service_instance_id) BETWEEN 12 AND 68
        AND substr(service_instance_id, 1, 4) = 'svc_'
        AND substr(service_instance_id, 5) NOT GLOB '*[^a-z0-9]*'
    ),
    process_generation INTEGER NOT NULL CHECK (process_generation > 0),
    stream_generation INTEGER NOT NULL CHECK (stream_generation > 0),
    slot_start_utc_millis INTEGER NOT NULL CHECK (
        slot_start_utc_millis >= 0
        AND slot_start_utc_millis % 15000 = 0
    ),
    device_platform TEXT NOT NULL CHECK (
        device_platform IN ('macos_core_audio', 'windows_wasapi', 'linux_alsa', 'linux_jack')
    ),
    device_opaque_id TEXT NOT NULL CHECK (
        length(device_opaque_id) BETWEEN 1 AND 256
        AND instr(device_opaque_id, char(0)) = 0
    ),
    sample_rate_hz INTEGER NOT NULL CHECK (sample_rate_hz BETWEEN 8000 AND 384000),
    channels INTEGER NOT NULL CHECK (channels BETWEEN 1 AND 32),
    sample_format TEXT NOT NULL CHECK (
        sample_format IN (
            'signed_8', 'signed_16', 'signed_24', 'signed_32', 'signed_64',
            'unsigned_8', 'unsigned_16', 'unsigned_24', 'unsigned_32', 'unsigned_64',
            'float_32', 'float_64'
        )
    ),
    selected_channel INTEGER NOT NULL CHECK (
        selected_channel >= 0 AND selected_channel < channels
    ),
    capture_position_frames INTEGER NOT NULL CHECK (capture_position_frames >= 0),
    capture_utc_millis INTEGER NOT NULL CHECK (capture_utc_millis >= 0),
    capture_monotonic_millis INTEGER NOT NULL CHECK (capture_monotonic_millis >= 0),
    recorded_utc_millis INTEGER NOT NULL CHECK (recorded_utc_millis >= 0),
    UNIQUE (
        service_instance_id,
        process_generation,
        stream_generation,
        slot_start_utc_millis,
        device_platform,
        device_opaque_id,
        sample_rate_hz,
        channels,
        sample_format,
        selected_channel
    )
) STRICT;

CREATE INDEX receive_windows_slot_order
ON receive_windows(slot_start_utc_millis, sequence);

CREATE INDEX receive_windows_service_order
ON receive_windows(service_instance_id, sequence);

CREATE TABLE receive_diagnostics (
    receive_window_id TEXT PRIMARY KEY
        REFERENCES receive_windows(receive_window_id) ON DELETE CASCADE,
    audio_latency_millis INTEGER NOT NULL CHECK (
        audio_latency_millis BETWEEN 0 AND 60000
    ),
    audio_drift_parts_per_million INTEGER NOT NULL CHECK (
        audio_drift_parts_per_million BETWEEN -100000 AND 100000
    ),
    audio_overflow_count INTEGER NOT NULL CHECK (audio_overflow_count >= 0),
    audio_clipped_sample_count INTEGER NOT NULL CHECK (audio_clipped_sample_count >= 0),
    audio_max_callback_delay_millis INTEGER NOT NULL CHECK (
        audio_max_callback_delay_millis BETWEEN 0 AND 60000
    ),
    timeline_max_jitter_millis INTEGER NOT NULL CHECK (
        timeline_max_jitter_millis BETWEEN 0 AND 4294967295
    ),
    timeline_drift_parts_per_million INTEGER NOT NULL CHECK (
        timeline_drift_parts_per_million BETWEEN -100000 AND 100000
    ),
    timeline_incomplete_slot_count INTEGER NOT NULL CHECK (
        timeline_incomplete_slot_count >= 0
    ),
    timeline_late_batch_count INTEGER NOT NULL CHECK (timeline_late_batch_count >= 0),
    clock_state TEXT NOT NULL CHECK (clock_state IN ('healthy', 'unhealthy')),
    clock_fault_kind TEXT CHECK (
        clock_fault_kind IS NULL OR clock_fault_kind IN (
            'process_generation_changed',
            'timeline_regressed',
            'utc_jump',
            'stale_mapping',
            'sampling_delayed',
            'sample_gap',
            'window_misaligned',
            'arithmetic_overflow'
        )
    ),
    clock_fault_value INTEGER,
    clock_recovery_progress INTEGER NOT NULL CHECK (
        clock_recovery_progress BETWEEN 0 AND 10
    ),
    clock_recovery_required INTEGER NOT NULL CHECK (
        clock_recovery_required BETWEEN 0 AND 10
    ),
    clock_mapping_age_millis INTEGER NOT NULL CHECK (
        clock_mapping_age_millis BETWEEN 0 AND 86400000
    ),
    CHECK (
        (
            clock_state = 'healthy'
            AND clock_fault_kind IS NULL
            AND clock_fault_value IS NULL
            AND clock_recovery_progress = 0
            AND clock_recovery_required = 0
        )
        OR
        (
            clock_state = 'unhealthy'
            AND clock_fault_kind IS NOT NULL
            AND clock_recovery_required BETWEEN 2 AND 10
            AND clock_recovery_progress < clock_recovery_required
        )
    ),
    CHECK (
        (
            clock_fault_kind IN (
                'utc_jump',
                'stale_mapping',
                'sampling_delayed',
                'sample_gap',
                'window_misaligned'
            )
            AND clock_fault_value IS NOT NULL
        )
        OR
        (
            (
                clock_fault_kind IS NULL
                OR clock_fault_kind IN (
                    'process_generation_changed',
                    'timeline_regressed',
                    'arithmetic_overflow'
                )
            )
            AND clock_fault_value IS NULL
        )
    )
) STRICT;

CREATE TABLE receive_decodes (
    receive_window_id TEXT NOT NULL
        REFERENCES receive_windows(receive_window_id) ON DELETE CASCADE,
    decode_index INTEGER NOT NULL CHECK (decode_index BETWEEN 0 AND 127),
    start_offset_millis INTEGER NOT NULL CHECK (
        start_offset_millis BETWEEN -60000 AND 60000
    ),
    audio_frequency_hz INTEGER NOT NULL CHECK (
        audio_frequency_hz BETWEEN 0 AND 6000
    ),
    signal_to_noise_db INTEGER NOT NULL CHECK (
        signal_to_noise_db BETWEEN -100 AND 100
    ),
    outcome_kind TEXT NOT NULL CHECK (
        outcome_kind IN (
            'resolved',
            'unresolved_hash',
            'unsupported',
            'ambiguous',
            'free_text'
        )
    ),
    canonical_text TEXT NOT NULL CHECK (
        length(canonical_text) BETWEEN 1 AND 64
        AND instr(canonical_text, char(0)) = 0
    ),
    classification_detail TEXT CHECK (
        classification_detail IS NULL
        OR (
            length(classification_detail) BETWEEN 1 AND 128
            AND instr(classification_detail, char(0)) = 0
        )
    ),
    message_class TEXT CHECK (
        message_class IS NULL OR message_class IN (
            'general_call',
            'directed_grid',
            'signal_report',
            'roger_signal_report',
            'roger',
            'ending_73',
            'ending_rr73'
        )
    ),
    sender_callsign TEXT CHECK (
        sender_callsign IS NULL
        OR length(sender_callsign) BETWEEN 1 AND 32
    ),
    recipient_callsign TEXT CHECK (
        recipient_callsign IS NULL
        OR length(recipient_callsign) BETWEEN 1 AND 32
    ),
    PRIMARY KEY (receive_window_id, decode_index),
    CHECK (
        (
            outcome_kind = 'resolved'
            AND classification_detail IS NULL
            AND message_class IS NOT NULL
            AND sender_callsign IS NOT NULL
        )
        OR
        (
            outcome_kind IN ('unresolved_hash', 'unsupported', 'ambiguous')
            AND classification_detail IS NOT NULL
            AND message_class IS NULL
            AND sender_callsign IS NULL
            AND recipient_callsign IS NULL
        )
        OR
        (
            outcome_kind = 'free_text'
            AND classification_detail IS NULL
            AND message_class IS NULL
            AND sender_callsign IS NULL
            AND recipient_callsign IS NULL
        )
    ),
    CHECK (
        message_class IS NULL
        OR (message_class = 'general_call' AND recipient_callsign IS NULL)
        OR (message_class != 'general_call' AND recipient_callsign IS NOT NULL)
    )
) STRICT;

CREATE INDEX receive_decodes_deterministic_order
ON receive_decodes(
    receive_window_id,
    start_offset_millis,
    audio_frequency_hz,
    canonical_text,
    outcome_kind,
    signal_to_noise_db,
    decode_index
);
