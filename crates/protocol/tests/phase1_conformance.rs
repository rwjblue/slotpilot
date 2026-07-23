//! Phase 1 cross-boundary conformance through only SlotPilot-owned APIs.

use std::{fs, path::Path};

use serde_json::Value;
use slotpilot_domain::AudioFrequency;
use slotpilot_protocol::{
    AmbiguousFt8Message, ClassifiedFt8Message, FT8_PCM_SAMPLE_RATE_HZ, Ft8DecodeBitsRequest,
    Ft8DecodeConfig, Ft8DecodeDepth, Ft8EncodeRequest, Ft8MessageClass, Ft8MessageCodec,
    Ft8OfflineDecoder, Ft8WaveformPlacement, Ft8WaveformRequest, Ft8WaveformSynthesizer,
    OfflineFt8Codec, OfflineFt8Decoder, OfflineFt8Synthesizer, PackedFt8Bits, PcmAmplitudePermille,
    PcmFormat, PcmSampleFormat, ResolvedFt8Message,
};

const SYNTHESIZABLE_GOLDEN_CASES: &[&str] = &[
    "ordinary-cq-grid",
    "ordinary-directed-grid",
    "ordinary-signal-report",
    "ordinary-roger-report",
    "ordinary-rrr",
    "ordinary-73",
    "ordinary-rr73",
    "special-compound-cq",
];

#[test]
fn supported_golden_messages_encode_synthesize_and_decode_through_owned_types() {
    let manifest = load_manifest();
    std::thread::scope(|scope| {
        let handles: Vec<_> = SYNTHESIZABLE_GOLDEN_CASES
            .iter()
            .map(|id| scope.spawn(|| assert_synthesized_round_trip(&manifest, id)))
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
}

#[test]
fn every_non_resolved_matrix_outcome_fails_the_checked_resolved_path() {
    let manifest = load_manifest();
    let codec = OfflineFt8Codec::new();
    let unresolved = codec
        .decode_bits(&Ft8DecodeBitsRequest {
            bits: vector_bits(fixture_vector(&manifest, "special-hashed-unresolved")),
        })
        .unwrap();
    let unsupported = codec
        .decode_bits(&Ft8DecodeBitsRequest {
            bits: vector_bits(fixture_vector(&manifest, "unsupported-telemetry")),
        })
        .unwrap();
    let free_text = codec
        .decode_bits(&Ft8DecodeBitsRequest {
            bits: vector_bits(fixture_vector(&manifest, "free-text")),
        })
        .unwrap();
    let ambiguous = ClassifiedFt8Message::Ambiguous(
        AmbiguousFt8Message::new(
            text(
                fixture_vector(&manifest, "owned-ambiguous-boundary"),
                "canonical_text",
            ),
            "reviewed boundary has insufficient protocol meaning",
        )
        .unwrap(),
    );

    assert!(matches!(
        unresolved,
        ClassifiedFt8Message::UnresolvedHash(_)
    ));
    assert!(matches!(unsupported, ClassifiedFt8Message::Unsupported(_)));
    assert!(matches!(free_text, ClassifiedFt8Message::FreeText(_)));
    for outcome in [unresolved, unsupported, free_text, ambiguous] {
        assert!(outcome.try_into_resolved().is_err());
    }
}

fn assert_synthesized_round_trip(manifest: &Value, id: &str) {
    let codec = OfflineFt8Codec::new();
    let synthesizer = OfflineFt8Synthesizer::new();
    let decoder = OfflineFt8Decoder::new();
    let format = PcmFormat::new(
        FT8_PCM_SAMPLE_RATE_HZ,
        1,
        PcmSampleFormat::Signed16LittleEndian,
    )
    .unwrap();
    let decode_config = Ft8DecodeConfig::new(
        AudioFrequency::from_hz(50).unwrap(),
        AudioFrequency::from_hz(160).unwrap(),
        1_000,
        Ft8DecodeDepth::Normal,
        20,
    )
    .unwrap();
    let vector = fixture_vector(manifest, id);
    let expected = resolved_vector(vector);
    let packed = codec
        .encode(&Ft8EncodeRequest {
            message: expected.clone(),
        })
        .unwrap_or_else(|error| panic!("{id}: {error}"));
    assert_eq!(
        bits_text(packed.bits()),
        text(vector, "payload_bits"),
        "{id}"
    );

    let pcm = synthesizer
        .synthesize(&Ft8WaveformRequest {
            message: expected.clone(),
            format,
            audio_frequency: AudioFrequency::from_hz(100).unwrap(),
            amplitude: PcmAmplitudePermille::new(750).unwrap(),
            placement: Ft8WaveformPlacement::FullSlot { start_frame: 6_000 },
        })
        .unwrap_or_else(|error| panic!("{id}: {error}"));
    let decoded = decoder
        .decode(&pcm, decode_config)
        .unwrap_or_else(|error| panic!("{id}: {error}"));
    assert!(
        decoded
            .iter()
            .any(|result| result.message == ClassifiedFt8Message::Resolved(expected.clone())),
        "{id}: {decoded:#?}"
    );
}

fn resolved_vector(vector: &Value) -> ResolvedFt8Message {
    let recipient = vector["recipient"]
        .as_str()
        .map(str::parse)
        .transpose()
        .unwrap();
    ResolvedFt8Message::new(
        text(vector, "canonical_text"),
        text(vector, "sender").parse().unwrap(),
        recipient,
        message_class(text(vector, "message_class")),
    )
    .unwrap()
}

fn message_class(value: &str) -> Ft8MessageClass {
    match value {
        "general_call" => Ft8MessageClass::GeneralCall,
        "directed_grid" => Ft8MessageClass::DirectedGrid,
        "signal_report" => Ft8MessageClass::SignalReport,
        "roger_signal_report" => Ft8MessageClass::RogerSignalReport,
        "roger" => Ft8MessageClass::Roger,
        "ending_73" => Ft8MessageClass::Ending73,
        "ending_rr73" => Ft8MessageClass::EndingRr73,
        other => panic!("unreviewed fixture message class: {other}"),
    }
}

fn vector_bits(vector: &Value) -> PackedFt8Bits {
    let bits: Vec<bool> = text(vector, "payload_bits")
        .bytes()
        .map(|byte| byte == b'1')
        .collect();
    PackedFt8Bits::new(bits.try_into().unwrap())
}

fn bits_text(bits: &[bool; 77]) -> String {
    bits.iter()
        .map(|bit| if *bit { '1' } else { '0' })
        .collect()
}

fn load_manifest() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ft8/v1/manifest.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn fixture_vector<'a>(manifest: &'a Value, id: &str) -> &'a Value {
    manifest["message_vectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["id"] == id)
        .unwrap()
}

fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field].as_str().unwrap()
}
