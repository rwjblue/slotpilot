//! Reproducible offline RIFF/WAVE and PCM decode conformance checks.

use std::{fs, path::Path};

use serde_json::Value;
use slotpilot_domain::AudioFrequency;
use slotpilot_protocol::{
    ClassifiedFt8Message, FT8_PCM_SAMPLE_RATE_HZ, FT8_SLOT_SAMPLES, Ft8Decode, Ft8DecodeConfig,
    Ft8DecodeDepth, Ft8DecodeError, Ft8OfflineDecoder, OfflineFt8Decoder, PcmBuffer, PcmFormat,
    PcmSampleFormat, PcmWaveError, decode_pcm_wave, encode_pcm_wave,
};

#[test]
fn reviewed_recordings_meet_owned_recall_order_and_tolerance_contract() {
    let (root, manifest) = load_manifest();
    let decoder = OfflineFt8Decoder::new();
    let config = reviewed_config();

    for recording in manifest["recordings"].as_array().unwrap() {
        let bytes = fs::read(root.join(text(recording, "path"))).unwrap();
        let pcm = decode_pcm_wave(&bytes).unwrap();
        assert_eq!(pcm.format().sample_rate_hz(), FT8_PCM_SAMPLE_RATE_HZ);
        assert_eq!(pcm.format().channels(), 1);
        assert_eq!(pcm.duration().frames(), FT8_SLOT_SAMPLES);

        let first = decoder.decode(&pcm, config).unwrap();
        assert_is_deterministically_sorted(&first);
        assert_recording_contract(recording, &first);

        if text(recording, "id") == "clean-cq-k1abc-fn42" {
            let second = decoder.decode(&pcm, config).unwrap();
            assert_eq!(first, second, "{}", text(recording, "id"));
        }
    }
}

#[test]
fn malformed_truncated_wrong_format_and_wrong_window_are_typed_failures() {
    let (root, manifest) = load_manifest();
    let bytes =
        fs::read(root.join(text(&manifest["recordings"].as_array().unwrap()[0], "path"))).unwrap();

    assert_eq!(decode_pcm_wave(&bytes[..10]), Err(PcmWaveError::Truncated));

    let mut truncated = bytes.clone();
    truncated.truncate(truncated.len() - 1);
    assert_eq!(decode_pcm_wave(&truncated), Err(PcmWaveError::Truncated));

    let mut wrong_encoding = bytes.clone();
    wrong_encoding[20..22].copy_from_slice(&3_u16.to_le_bytes());
    assert_eq!(
        decode_pcm_wave(&wrong_encoding),
        Err(PcmWaveError::UnsupportedFormat)
    );

    let mut wrong_depth = bytes;
    wrong_depth[34..36].copy_from_slice(&8_u16.to_le_bytes());
    assert_eq!(
        decode_pcm_wave(&wrong_depth),
        Err(PcmWaveError::UnsupportedFormat)
    );

    let format = PcmFormat::new(
        FT8_PCM_SAMPLE_RATE_HZ,
        1,
        PcmSampleFormat::Signed16LittleEndian,
    )
    .unwrap();
    let oversized_window = PcmBuffer::new(format, vec![0; FT8_SLOT_SAMPLES as usize + 1]).unwrap();
    assert_eq!(
        OfflineFt8Decoder::new().decode(&oversized_window, reviewed_config()),
        Err(Ft8DecodeError::UnsupportedPcmWindow)
    );
}

#[test]
fn silence_is_an_explicit_empty_result_and_wave_round_trip_is_lossless() {
    let format = PcmFormat::new(
        FT8_PCM_SAMPLE_RATE_HZ,
        1,
        PcmSampleFormat::Signed16LittleEndian,
    )
    .unwrap();
    let silence = PcmBuffer::new(format, vec![0; FT8_SLOT_SAMPLES as usize]).unwrap();
    let wave = encode_pcm_wave(&silence).unwrap();
    let parsed = decode_pcm_wave(&wave).unwrap();
    assert_eq!(parsed, silence);

    let no_decodes = OfflineFt8Decoder::new()
        .decode(&parsed, minimal_no_decode_config())
        .unwrap();
    assert!(no_decodes.is_empty());
}

#[test]
fn invalid_search_bounds_thresholds_and_candidate_caps_are_rejected() {
    let low = AudioFrequency::from_hz(100).unwrap();
    let high = AudioFrequency::from_hz(3_000).unwrap();
    let above_nyquist = AudioFrequency::from_hz(6_000).unwrap();

    for result in [
        Ft8DecodeConfig::new(high, low, 1_000, Ft8DecodeDepth::Deep, 200),
        Ft8DecodeConfig::new(low, above_nyquist, 1_000, Ft8DecodeDepth::Deep, 200),
        Ft8DecodeConfig::new(low, high, 0, Ft8DecodeDepth::Deep, 200),
        Ft8DecodeConfig::new(low, high, 1_000, Ft8DecodeDepth::Deep, 0),
    ] {
        assert!(matches!(
            result,
            Err(Ft8DecodeError::InvalidConfiguration { .. })
        ));
    }
}

fn reviewed_config() -> Ft8DecodeConfig {
    Ft8DecodeConfig::new(
        AudioFrequency::from_hz(600).unwrap(),
        AudioFrequency::from_hz(1_800).unwrap(),
        1_000,
        Ft8DecodeDepth::Normal,
        20,
    )
    .unwrap()
}

fn minimal_no_decode_config() -> Ft8DecodeConfig {
    Ft8DecodeConfig::new(
        AudioFrequency::from_hz(1).unwrap(),
        AudioFrequency::from_hz(100).unwrap(),
        5_000,
        Ft8DecodeDepth::Normal,
        1,
    )
    .unwrap()
}

fn assert_recording_contract(recording: &Value, actual: &[Ft8Decode]) {
    let expected = recording["expected_decodes"].as_array().unwrap();
    let tolerance = &recording["tolerance"];
    let mut matched = vec![false; actual.len()];
    let mut recall = 0;

    for wanted in expected {
        if let Some((index, found)) = actual.iter().enumerate().find(|(index, decode)| {
            !matched[*index] && decode.message.canonical_text() == text(wanted, "canonical_text")
        }) {
            assert_decode_fields(wanted, tolerance, found);
            matched[index] = true;
            recall += 1;
        }
    }

    let minimum_recall = recording["minimum_recall"].as_u64().unwrap() as usize;
    assert!(
        recall >= minimum_recall,
        "{} recalled {recall}/{minimum_recall}: {actual:#?}",
        text(recording, "id")
    );

    let permitted = recording["permitted_extra_messages"].as_array().unwrap();
    let extras: Vec<_> = actual
        .iter()
        .zip(matched)
        .filter_map(|(decode, matched)| (!matched).then_some(decode))
        .collect();
    assert!(
        extras.len() <= recording["maximum_extras"].as_u64().unwrap() as usize,
        "{} produced unbounded extras: {extras:#?}",
        text(recording, "id")
    );
    for extra in extras {
        assert!(
            permitted
                .iter()
                .any(|allowed| allowed.as_str() == Some(extra.message.canonical_text())),
            "{} produced an unreviewed extra: {extra:#?}",
            text(recording, "id")
        );
    }
}

fn assert_decode_fields(expected: &Value, tolerance: &Value, actual: &Ft8Decode) {
    assert_eq!(
        actual.message.kind(),
        slotpilot_protocol::Ft8OutcomeKind::Resolved
    );
    let ClassifiedFt8Message::Resolved(message) = &actual.message else {
        panic!("expected a resolved fixture decode");
    };
    assert_eq!(message.sender().original(), text(expected, "sender"));
    assert_eq!(
        message.recipient().map(|call| call.original()),
        expected["recipient"].as_str()
    );
    assert_eq!(
        message_class_name(message.class()),
        text(expected, "message_class")
    );
    assert_within(
        actual.metadata.audio_frequency_hz,
        expected["audio_frequency_hz"].as_u64().unwrap() as u32,
        tolerance["audio_frequency_hz"].as_u64().unwrap() as u32,
    );
    assert_within_signed(
        actual.metadata.start_offset_millis,
        expected["start_offset_millis"].as_i64().unwrap() as i32,
        tolerance["start_offset_millis"].as_u64().unwrap() as i32,
    );
    assert_within_signed(
        i32::from(actual.metadata.signal_to_noise_db),
        expected["signal_to_noise_db"].as_i64().unwrap() as i32,
        tolerance["signal_to_noise_db"].as_u64().unwrap() as i32,
    );
}

fn message_class_name(class: slotpilot_protocol::Ft8MessageClass) -> &'static str {
    match class {
        slotpilot_protocol::Ft8MessageClass::GeneralCall => "general_call",
        slotpilot_protocol::Ft8MessageClass::DirectedGrid => "directed_grid",
        slotpilot_protocol::Ft8MessageClass::SignalReport => "signal_report",
        slotpilot_protocol::Ft8MessageClass::RogerSignalReport => "roger_signal_report",
        slotpilot_protocol::Ft8MessageClass::Roger => "roger",
        slotpilot_protocol::Ft8MessageClass::Ending73 => "ending_73",
        slotpilot_protocol::Ft8MessageClass::EndingRr73 => "ending_rr73",
    }
}

fn assert_is_deterministically_sorted(results: &[Ft8Decode]) {
    let mut sorted = results.to_vec();
    Ft8Decode::sort_deterministically(&mut sorted);
    assert_eq!(results, sorted);
}

fn assert_within(actual: u32, expected: u32, tolerance: u32) {
    assert!(
        actual.abs_diff(expected) <= tolerance,
        "{actual} is not within {tolerance} of {expected}"
    );
}

fn assert_within_signed(actual: i32, expected: i32, tolerance: i32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} is not within {tolerance} of {expected}"
    );
}

fn load_manifest() -> (std::path::PathBuf, Value) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/ft8/v1")
        .canonicalize()
        .unwrap();
    let manifest = serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    (root, manifest)
}

fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field].as_str().unwrap()
}
