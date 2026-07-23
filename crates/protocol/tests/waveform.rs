//! Offline FT8 synthesis and in-memory RIFF/WAVE export checks.

use std::{fs, path::Path};

use serde_json::Value;
use slotpilot_domain::AudioFrequency;
use slotpilot_protocol::{
    ClassifiedFt8Message, FT8_FRAME_SAMPLES, FT8_PCM_SAMPLE_RATE_HZ, FT8_SLOT_SAMPLES,
    FreeTextFt8Message, Ft8CodecError, Ft8EncodeRequest, Ft8MessageClass, Ft8MessageCodec,
    Ft8WaveformError, Ft8WaveformPlacement, Ft8WaveformRequest, Ft8WaveformSynthesizer,
    OfflineFt8Codec, OfflineFt8Synthesizer, PcmAmplitudePermille, PcmFormat, PcmSampleFormat,
    ResolvedFt8Message, encode_pcm_wave,
};

#[test]
fn ordinary_and_compound_golden_messages_synthesize_reproducibly() {
    let manifest = load_manifest();
    let codec = OfflineFt8Codec::new();
    let synthesizer = OfflineFt8Synthesizer::new();

    for id in ["ordinary-cq-grid", "special-compound-cq"] {
        let vector = fixture_vector(&manifest, id);
        let message = resolved_vector(vector);
        let bits = codec
            .encode(&Ft8EncodeRequest {
                message: message.clone(),
            })
            .unwrap();
        assert_eq!(bits_text(bits.bits()), text(vector, "payload_bits"), "{id}");

        let request = waveform_request(message, Ft8WaveformPlacement::FrameOnly);
        let first = synthesizer.synthesize(&request).unwrap();
        let second = synthesizer.synthesize(&request).unwrap();
        assert_eq!(first, second, "{id}");
        assert_eq!(first.format(), canonical_format());
        assert_eq!(first.duration().frames(), FT8_FRAME_SAMPLES);
        assert_eq!(first.duration().microseconds(), 12_640_000);
        assert!(first.samples().iter().any(|sample| *sample != 0));
        assert!(
            first
                .samples()
                .iter()
                .all(|sample| sample.unsigned_abs() <= 16_383)
        );
    }
}

#[test]
fn full_slot_placement_is_explicit_silence_around_the_same_frame() {
    let manifest = load_manifest();
    let message = resolved_vector(fixture_vector(&manifest, "ordinary-cq-grid"));
    let synthesizer = OfflineFt8Synthesizer::new();
    let start_frame = 4_800;

    let frame = synthesizer
        .synthesize(&waveform_request(
            message.clone(),
            Ft8WaveformPlacement::FrameOnly,
        ))
        .unwrap();
    let slot = synthesizer
        .synthesize(&waveform_request(
            message,
            Ft8WaveformPlacement::FullSlot { start_frame },
        ))
        .unwrap();

    assert_eq!(slot.duration().frames(), FT8_SLOT_SAMPLES);
    assert_eq!(slot.duration().microseconds(), 15_000_000);
    assert!(
        slot.samples()[..start_frame as usize]
            .iter()
            .all(|sample| *sample == 0)
    );
    let end = start_frame as usize + FT8_FRAME_SAMPLES as usize;
    assert_eq!(&slot.samples()[start_frame as usize..end], frame.samples());
    assert!(slot.samples()[end..].iter().all(|sample| *sample == 0));
}

#[test]
fn invalid_format_frequency_placement_and_message_are_typed_failures() {
    let manifest = load_manifest();
    let message = resolved_vector(fixture_vector(&manifest, "ordinary-cq-grid"));
    let synthesizer = OfflineFt8Synthesizer::new();

    for format in [
        PcmFormat::new(8_000, 1, PcmSampleFormat::Signed16LittleEndian).unwrap(),
        PcmFormat::new(12_000, 2, PcmSampleFormat::Signed16LittleEndian).unwrap(),
    ] {
        let mut request = waveform_request(message.clone(), Ft8WaveformPlacement::FrameOnly);
        request.format = format;
        assert!(matches!(
            synthesizer.synthesize(&request),
            Err(Ft8WaveformError::InvalidConfiguration { .. })
        ));
    }

    let mut high_frequency = waveform_request(message.clone(), Ft8WaveformPlacement::FrameOnly);
    high_frequency.audio_frequency = AudioFrequency::from_hz(5_957).unwrap();
    assert!(matches!(
        synthesizer.synthesize(&high_frequency),
        Err(Ft8WaveformError::InvalidConfiguration { .. })
    ));

    let bad_placement = waveform_request(
        message,
        Ft8WaveformPlacement::FullSlot {
            start_frame: FT8_SLOT_SAMPLES - FT8_FRAME_SAMPLES + 1,
        },
    );
    assert!(matches!(
        synthesizer.synthesize(&bad_placement),
        Err(Ft8WaveformError::InvalidConfiguration { .. })
    ));

    assert_eq!(
        PcmAmplitudePermille::new(0),
        Err(Ft8WaveformError::InvalidAmplitude)
    );
    assert_eq!(
        PcmAmplitudePermille::new(1_001),
        Err(Ft8WaveformError::InvalidAmplitude)
    );

    let free_text = ClassifiedFt8Message::FreeText(FreeTextFt8Message::new("HELLO WORLD").unwrap());
    assert!(free_text.try_into_resolved().is_err());

    let lossy = ResolvedFt8Message::new(
        "CQ W1AW/1 FN31",
        "W1AW/1".parse().unwrap(),
        None,
        Ft8MessageClass::GeneralCall,
    )
    .unwrap();
    assert!(matches!(
        synthesizer.synthesize(&waveform_request(lossy, Ft8WaveformPlacement::FrameOnly)),
        Err(Ft8WaveformError::Codec(
            Ft8CodecError::NotRepresentable { .. }
        ))
    ));
}

#[test]
fn wav_export_preserves_canonical_header_and_pcm_bytes_in_memory() {
    let manifest = load_manifest();
    let message = resolved_vector(fixture_vector(&manifest, "ordinary-cq-grid"));
    let buffer = OfflineFt8Synthesizer::new()
        .synthesize(&waveform_request(message, Ft8WaveformPlacement::FrameOnly))
        .unwrap();
    let wave = encode_pcm_wave(&buffer).unwrap();

    assert_eq!(&wave[0..4], b"RIFF");
    assert_eq!(&wave[8..12], b"WAVE");
    assert_eq!(&wave[12..16], b"fmt ");
    assert_eq!(u16::from_le_bytes([wave[20], wave[21]]), 1);
    assert_eq!(u16::from_le_bytes([wave[22], wave[23]]), 1);
    assert_eq!(
        u32::from_le_bytes([wave[24], wave[25], wave[26], wave[27]]),
        FT8_PCM_SAMPLE_RATE_HZ
    );
    assert_eq!(u16::from_le_bytes([wave[34], wave[35]]), 16);
    assert_eq!(&wave[36..40], b"data");
    assert_eq!(
        u32::from_le_bytes([wave[40], wave[41], wave[42], wave[43]]) as usize,
        buffer.samples().len() * 2
    );
    assert_eq!(wave.len(), 44 + buffer.samples().len() * 2);
    for (encoded, sample) in wave[44..].chunks_exact(2).zip(buffer.samples()) {
        assert_eq!(encoded, sample.to_le_bytes());
    }
}

fn waveform_request(
    message: ResolvedFt8Message,
    placement: Ft8WaveformPlacement,
) -> Ft8WaveformRequest {
    Ft8WaveformRequest {
        message,
        format: canonical_format(),
        audio_frequency: AudioFrequency::from_hz(1_500).unwrap(),
        amplitude: PcmAmplitudePermille::new(500).unwrap(),
        placement,
    }
}

fn canonical_format() -> PcmFormat {
    PcmFormat::new(
        FT8_PCM_SAMPLE_RATE_HZ,
        1,
        PcmSampleFormat::Signed16LittleEndian,
    )
    .unwrap()
}

fn resolved_vector(vector: &Value) -> ResolvedFt8Message {
    let class = match text(vector, "message_class") {
        "general_call" => Ft8MessageClass::GeneralCall,
        other => panic!("waveform fixture has unreviewed class: {other}"),
    };
    ResolvedFt8Message::new(
        text(vector, "canonical_text"),
        text(vector, "sender").parse().unwrap(),
        None,
        class,
    )
    .unwrap()
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
