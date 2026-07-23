//! Private conversion layer for the reviewed offline FT8 message dependency.

use std::str::FromStr;

use mfsk_core::ft8::{
    decode::DecodeDepth,
    decode_block::decode_block,
    wave_gen::{message_to_tones, tones_to_i16},
};
use mfsk_core::msg::{
    CallsignHashTable,
    hash_table::ihashcall,
    wsjt77::{pack77, pack77_type1, pack77_type4, unpack77_with_hash},
};
use slotpilot_domain::FullCallsign;

use crate::{
    AmbiguousFt8Message, ClassifiedFt8Message, FT8_FRAME_SAMPLES, FT8_PCM_SAMPLE_RATE_HZ,
    FT8_SLOT_SAMPLES, FreeTextFt8Message, Ft8CodecError, Ft8Decode, Ft8DecodeBitsRequest,
    Ft8DecodeConfig, Ft8DecodeDepth, Ft8DecodeError, Ft8DecodeMetadata, Ft8EncodeRequest,
    Ft8MessageClass, Ft8MessageCodec, Ft8OfflineDecoder, Ft8WaveformError, Ft8WaveformPlacement,
    Ft8WaveformRequest, Ft8WaveformSynthesizer, PackedFt8Bits, PcmBuffer, ResolvedFt8Message,
    UnresolvedHashFt8Message, UnsupportedFt8Message,
};

const PHASE1_SNR_CALIBRATION_DB: f32 = 8.0;

/// Reviewed offline FT8 message codec behind SlotPilot-owned values.
///
/// Known calls populate only the private hash-resolution context. Constructing
/// this value opens no device, performs no I/O, and grants no operating or
/// transmit authority.
#[derive(Debug, Clone, Default)]
pub struct OfflineFt8Codec {
    hashes: CallsignHashTable,
}

/// Deterministic, in-memory-only FT8 waveform synthesizer.
#[derive(Debug, Clone, Default)]
pub struct OfflineFt8Synthesizer {
    codec: OfflineFt8Codec,
}

/// Deterministic, in-memory-only FT8 slot decoder.
#[derive(Debug, Clone, Default)]
pub struct OfflineFt8Decoder {
    codec: OfflineFt8Codec,
}

impl OfflineFt8Decoder {
    /// Constructs an offline decoder with an empty callsign-hash context.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Constructs an offline decoder whose owned results may resolve known hashes.
    #[must_use]
    pub fn with_known_calls<'a>(known_calls: impl IntoIterator<Item = &'a FullCallsign>) -> Self {
        Self {
            codec: OfflineFt8Codec::with_known_calls(known_calls),
        }
    }
}

impl Ft8OfflineDecoder for OfflineFt8Decoder {
    fn decode(
        &self,
        pcm: &PcmBuffer,
        config: Ft8DecodeConfig,
    ) -> Result<Vec<Ft8Decode>, Ft8DecodeError> {
        if pcm.format().sample_rate_hz() != FT8_PCM_SAMPLE_RATE_HZ
            || pcm.format().channels() != 1
            || pcm.duration().frames() != FT8_SLOT_SAMPLES
        {
            return Err(Ft8DecodeError::UnsupportedPcmWindow);
        }
        if [
            "MFSK_RATIO_EPS",
            "MFSK_SYNC_LAG_S",
            "MFSK_TRACE_PHANTOM",
            "MFSK_PASS1_LIMIT",
            "MFSK_BP_KIND",
        ]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
        {
            return Err(Ft8DecodeError::InvalidConfiguration {
                detail: "private decoder tuning environment must be unset".to_owned(),
            });
        }

        let depth = match config.depth() {
            Ft8DecodeDepth::Normal => DecodeDepth::BpAll,
            Ft8DecodeDepth::Deep => DecodeDepth::BpAllOsd,
        };
        let decoded = decode_block(
            pcm.samples(),
            config.minimum_audio_frequency().hz() as f32,
            config.maximum_audio_frequency().hz() as f32,
            f32::from(config.sync_threshold_milli()) / 1_000.0,
            depth,
            usize::from(config.maximum_candidates()),
        );
        let mut owned = Vec::with_capacity(decoded.len());
        for result in decoded {
            let frequency = rounded_u32(result.freq_hz)?;
            let start_offset = rounded_i32(result.dt_sec * 1_000.0)?;
            let signal_to_noise = rounded_i16(result.snr_db + PHASE1_SNR_CALIBRATION_DB)?;
            let message = self.codec.decode_bits(&Ft8DecodeBitsRequest {
                bits: PackedFt8Bits::new(result.message77.map(|bit| bit != 0)),
            })?;
            let candidate = Ft8Decode {
                metadata: Ft8DecodeMetadata {
                    start_offset_millis: start_offset,
                    audio_frequency_hz: frequency,
                    signal_to_noise_db: signal_to_noise,
                },
                message,
            };
            if let Some(existing) = owned.iter_mut().find(|existing: &&mut Ft8Decode| {
                existing.metadata.start_offset_millis == candidate.metadata.start_offset_millis
                    && existing.metadata.audio_frequency_hz == candidate.metadata.audio_frequency_hz
                    && existing.message == candidate.message
            }) {
                if candidate.metadata.signal_to_noise_db > existing.metadata.signal_to_noise_db {
                    *existing = candidate;
                }
            } else {
                owned.push(candidate);
            }
        }
        Ft8Decode::sort_deterministically(&mut owned);
        Ok(owned)
    }
}

impl OfflineFt8Synthesizer {
    /// Constructs an offline synthesizer with no device or station behavior.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Ft8WaveformSynthesizer for OfflineFt8Synthesizer {
    fn synthesize(&self, request: &Ft8WaveformRequest) -> Result<PcmBuffer, Ft8WaveformError> {
        if request.format.sample_rate_hz() != FT8_PCM_SAMPLE_RATE_HZ
            || request.format.channels() != 1
        {
            return Err(invalid_waveform(
                "FT8 synthesis requires mono signed 16-bit PCM at 12,000 Hz",
            ));
        }

        let base_frequency = request.audio_frequency.hz();
        if base_frequency > 5_956 {
            return Err(invalid_waveform(
                "FT8 tone 7 must remain below the 6,000 Hz Nyquist frequency",
            ));
        }

        let bits = self.codec.encode(&Ft8EncodeRequest {
            message: request.message.clone(),
        })?;
        let message_bits = bits.bits().map(u8::from);
        let tones = message_to_tones(&message_bits);
        let amplitude = i16::try_from(
            32_767_u32
                .checked_mul(u32::from(request.amplitude.value()))
                .ok_or_else(|| invalid_waveform("PCM amplitude calculation overflowed"))?
                / 1_000,
        )
        .map_err(|_| invalid_waveform("PCM amplitude exceeded signed 16-bit range"))?;
        let frame = tones_to_i16(&tones, base_frequency as f32, amplitude);
        if frame.len() != FT8_FRAME_SAMPLES as usize
            || frame
                .iter()
                .any(|sample| sample.unsigned_abs() > amplitude as u16)
        {
            return Err(invalid_waveform(
                "private FT8 synthesis violated the owned PCM bounds",
            ));
        }

        let samples = match request.placement {
            Ft8WaveformPlacement::FrameOnly => frame,
            Ft8WaveformPlacement::FullSlot { start_frame } => {
                let end_frame = start_frame
                    .checked_add(FT8_FRAME_SAMPLES)
                    .ok_or_else(|| invalid_waveform("full-slot frame placement overflowed"))?;
                if end_frame > FT8_SLOT_SAMPLES {
                    return Err(invalid_waveform(
                        "full-slot placement cannot contain the complete FT8 frame",
                    ));
                }
                let mut slot = vec![0; FT8_SLOT_SAMPLES as usize];
                let start = start_frame as usize;
                let end = end_frame as usize;
                slot[start..end].copy_from_slice(&frame);
                slot
            }
        };
        PcmBuffer::new(request.format, samples).map_err(Ft8WaveformError::from)
    }
}

impl OfflineFt8Codec {
    /// Constructs a codec with an empty callsign-hash context.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Constructs a codec whose decoder may resolve hashes for known calls.
    #[must_use]
    pub fn with_known_calls<'a>(known_calls: impl IntoIterator<Item = &'a FullCallsign>) -> Self {
        let mut hashes = CallsignHashTable::new();
        for call in known_calls {
            hashes.insert(call.original());
        }
        Self { hashes }
    }

    fn encode_owned(&self, message: &ResolvedFt8Message) -> Result<[u8; 77], Ft8CodecError> {
        let words: Vec<&str> = message.canonical_text().split_whitespace().collect();
        let encoded = match (message.class(), words.as_slice()) {
            (Ft8MessageClass::GeneralCall, ["CQ", sender])
                if same_identity(sender, message.sender()) && sender.contains('/') =>
            {
                pack77_type4(sender, "", "", true).map(|mut bits| {
                    write_bits(&mut bits, 0, 12, ihashcall(sender, 12));
                    bits
                })
            }
            (Ft8MessageClass::GeneralCall, ["CQ", sender, grid])
                if same_identity(sender, message.sender()) && is_grid(grid) =>
            {
                pack77("CQ", sender, grid)
            }
            (class, [sender, recipient, payload])
                if class != Ft8MessageClass::GeneralCall
                    && same_identity(sender, message.sender())
                    && message
                        .recipient()
                        .is_some_and(|expected| same_identity(recipient, expected))
                    && payload_matches(class, payload) =>
            {
                if class == Ft8MessageClass::DirectedGrid {
                    pack77_type1(sender, recipient, payload)
                } else {
                    pack77(sender, recipient, payload)
                }
            }
            _ => {
                return Err(not_representable(
                    "canonical text does not exactly match its owned FT8 class and identities",
                ));
            }
        }
        .ok_or_else(|| {
            not_representable("the reviewed FT8 message encoder cannot represent this message")
        })?;

        let mut verification_hashes = self.hashes.clone();
        verification_hashes.insert(message.sender().original());
        if let Some(recipient) = message.recipient() {
            verification_hashes.insert(recipient.original());
        }
        let round_trip = normalize_standard_report(
            unpack77_with_hash(&encoded, &verification_hashes).ok_or_else(|| {
                adapter_error("the reviewed FT8 encoder produced a payload it could not unpack")
            })?,
        );
        if round_trip != message.canonical_text() {
            return Err(not_representable(
                "encoding would normalize or discard part of the owned message identity",
            ));
        }
        Ok(encoded)
    }
}

impl Ft8MessageCodec for OfflineFt8Codec {
    fn encode(&self, request: &Ft8EncodeRequest) -> Result<PackedFt8Bits, Ft8CodecError> {
        let encoded = self.encode_owned(&request.message)?;
        Ok(PackedFt8Bits::new(encoded.map(|bit| bit != 0)))
    }

    fn decode_bits(
        &self,
        request: &Ft8DecodeBitsRequest,
    ) -> Result<ClassifiedFt8Message, Ft8CodecError> {
        let bits = request.bits.bits().map(u8::from);
        classify_bits(&bits, &self.hashes)
    }
}

fn classify_bits(
    bits: &[u8; 77],
    hashes: &CallsignHashTable,
) -> Result<ClassifiedFt8Message, Ft8CodecError> {
    let n3 = read_bits(bits, 71, 3);
    let i3 = read_bits(bits, 74, 3);
    let decoded = unpack77_with_hash(bits, hashes);

    if i3 == 0 && n3 == 0 {
        let text = decoded.ok_or_else(|| invalid_message("empty FT8 free-text payload"))?;
        return FreeTextFt8Message::new(text)
            .map(ClassifiedFt8Message::FreeText)
            .map_err(|_| adapter_error("decoded FT8 free text exceeded the owned boundary"));
    }

    if !matches!(i3, 1 | 2 | 4) {
        let text = decoded.unwrap_or_else(|| format!("UNSUPPORTED FT8 TYPE {i3}/{n3}"));
        return UnsupportedFt8Message::new(
            text,
            format!("FT8 type {i3}/{n3} is outside the reviewed Phase 1 message matrix"),
        )
        .map(ClassifiedFt8Message::Unsupported)
        .map_err(|_| adapter_error("unsupported FT8 message exceeded the owned boundary"));
    }

    let text = normalize_standard_report(
        decoded.ok_or_else(|| invalid_message("FT8 payload could not be unpacked"))?,
    );
    if text.contains("<...>") {
        return UnresolvedHashFt8Message::new(
            text,
            "at least one FT8 callsign hash is not present in the supplied resolution context",
        )
        .map(ClassifiedFt8Message::UnresolvedHash)
        .map_err(|_| adapter_error("unresolved FT8 message exceeded the owned boundary"));
    }

    classify_supported(&text, directed_class_from_bits(bits, i3))
}

fn classify_supported(
    text: &str,
    bit_class: Option<Ft8MessageClass>,
) -> Result<ClassifiedFt8Message, Ft8CodecError> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let resolved = match words.as_slice() {
        ["CQ", sender] => resolved(text, sender, None, Ft8MessageClass::GeneralCall),
        ["CQ", sender, grid] if is_grid(grid) => {
            resolved(text, sender, None, Ft8MessageClass::GeneralCall)
        }
        [sender, recipient, payload] => {
            let class = bit_class.or_else(|| classify_payload(payload));
            match class {
                Some(class) => resolved(text, sender, Some(recipient), class),
                None => {
                    return UnsupportedFt8Message::new(
                        text,
                        "directed FT8 payload is outside the reviewed Phase 1 message classes",
                    )
                    .map(ClassifiedFt8Message::Unsupported)
                    .map_err(|_| {
                        adapter_error("unsupported FT8 message exceeded the owned boundary")
                    });
                }
            }
        }
        _ => {
            return UnsupportedFt8Message::new(
                text,
                "FT8 message shape is outside the reviewed Phase 1 message matrix",
            )
            .map(ClassifiedFt8Message::Unsupported)
            .map_err(|_| adapter_error("unsupported FT8 message exceeded the owned boundary"));
        }
    };

    match resolved {
        Ok(message) => Ok(ClassifiedFt8Message::Resolved(message)),
        Err(()) => AmbiguousFt8Message::new(
            text,
            "decoded FT8 identity cannot be mapped unambiguously to owned full callsigns",
        )
        .map(ClassifiedFt8Message::Ambiguous)
        .map_err(|_| adapter_error("ambiguous FT8 message exceeded the owned boundary")),
    }
}

fn directed_class_from_bits(bits: &[u8; 77], i3: u32) -> Option<Ft8MessageClass> {
    match i3 {
        1 | 2 => {
            let grid_or_report = read_bits(bits, 59, 15);
            if grid_or_report <= 32_400 {
                return Some(Ft8MessageClass::DirectedGrid);
            }
            let report_code = grid_or_report - 32_400;
            match (bits[58], report_code) {
                (0, 2) => Some(Ft8MessageClass::Roger),
                (0, 3) => Some(Ft8MessageClass::EndingRr73),
                (0, 4) => Some(Ft8MessageClass::Ending73),
                (0, 5..) => Some(Ft8MessageClass::SignalReport),
                (1, 5..) => Some(Ft8MessageClass::RogerSignalReport),
                _ => None,
            }
        }
        4 if bits[73] == 0 => match read_bits(bits, 71, 2) {
            1 => Some(Ft8MessageClass::Roger),
            2 => Some(Ft8MessageClass::EndingRr73),
            3 => Some(Ft8MessageClass::Ending73),
            _ => None,
        },
        _ => None,
    }
}

fn resolved(
    text: &str,
    sender: &str,
    recipient: Option<&&str>,
    class: Ft8MessageClass,
) -> Result<ResolvedFt8Message, ()> {
    let sender = parse_identity(sender)?;
    let recipient = recipient.map(|value| parse_identity(value)).transpose()?;
    ResolvedFt8Message::new(text, sender, recipient, class).map_err(|_| ())
}

fn parse_identity(token: &str) -> Result<FullCallsign, ()> {
    let unbracketed = token
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        .unwrap_or(token);
    FullCallsign::from_str(unbracketed).map_err(|_| ())
}

fn same_identity(token: &str, expected: &FullCallsign) -> bool {
    token
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        .unwrap_or(token)
        == expected.original()
}

fn classify_payload(payload: &str) -> Option<Ft8MessageClass> {
    if payload == "RRR" {
        Some(Ft8MessageClass::Roger)
    } else if payload == "73" {
        Some(Ft8MessageClass::Ending73)
    } else if payload == "RR73" {
        Some(Ft8MessageClass::EndingRr73)
    } else if is_grid(payload) {
        Some(Ft8MessageClass::DirectedGrid)
    } else if is_report(payload) {
        Some(Ft8MessageClass::SignalReport)
    } else if payload.strip_prefix('R').is_some_and(is_report) {
        Some(Ft8MessageClass::RogerSignalReport)
    } else {
        None
    }
}

fn normalize_standard_report(text: String) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let Some(payload) = words.get(2) else {
        return text;
    };
    let (prefix, numeric) = match payload.strip_prefix('R') {
        Some(numeric) => ("R", numeric),
        None => ("", *payload),
    };
    let Some((sign, digits)) = numeric.split_at_checked(1) else {
        return text;
    };
    if !matches!(sign, "+" | "-") || digits.len() >= 2 {
        return text;
    }
    let Ok(value) = digits.parse::<u8>() else {
        return text;
    };
    format!("{} {} {prefix}{sign}{value:02}", words[0], words[1])
}

fn payload_matches(class: Ft8MessageClass, payload: &str) -> bool {
    match class {
        Ft8MessageClass::DirectedGrid => is_grid(payload),
        Ft8MessageClass::SignalReport => is_report(payload),
        Ft8MessageClass::RogerSignalReport => payload.strip_prefix('R').is_some_and(is_report),
        Ft8MessageClass::Roger => payload == "RRR",
        Ft8MessageClass::Ending73 => payload == "73",
        Ft8MessageClass::EndingRr73 => payload == "RR73",
        Ft8MessageClass::GeneralCall => false,
    }
}

fn is_grid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 4
        && matches!(bytes[0], b'A'..=b'R')
        && matches!(bytes[1], b'A'..=b'R')
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_digit()
}

fn is_report(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 3
        && matches!(bytes[0], b'+' | b'-')
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
}

fn read_bits(bits: &[u8; 77], start: usize, length: usize) -> u32 {
    bits[start..start + length]
        .iter()
        .fold(0, |value, bit| (value << 1) | u32::from(*bit))
}

fn write_bits(bits: &mut [u8; 77], start: usize, length: usize, value: u32) {
    for index in 0..length {
        bits[start + index] = ((value >> (length - 1 - index)) & 1) as u8;
    }
}

fn not_representable(detail: &str) -> Ft8CodecError {
    Ft8CodecError::NotRepresentable {
        detail: detail.to_owned(),
    }
}

fn invalid_message(detail: &str) -> Ft8CodecError {
    Ft8CodecError::InvalidPackedMessage {
        detail: detail.to_owned(),
    }
}

fn adapter_error(detail: &str) -> Ft8CodecError {
    Ft8CodecError::Adapter {
        detail: detail.to_owned(),
    }
}

fn invalid_waveform(detail: &str) -> Ft8WaveformError {
    Ft8WaveformError::InvalidConfiguration {
        detail: detail.to_owned(),
    }
}

fn rounded_u32(value: f32) -> Result<u32, Ft8DecodeError> {
    if !value.is_finite() || value < 0.0 || value > u32::MAX as f32 {
        return Err(Ft8DecodeError::InvalidResultMetadata);
    }
    Ok(value.round() as u32)
}

fn rounded_i32(value: f32) -> Result<i32, Ft8DecodeError> {
    if !value.is_finite() || value < i32::MIN as f32 || value > i32::MAX as f32 {
        return Err(Ft8DecodeError::InvalidResultMetadata);
    }
    Ok(value.round() as i32)
}

fn rounded_i16(value: f32) -> Result<i16, Ft8DecodeError> {
    if !value.is_finite() || value < f32::from(i16::MIN) || value > f32::from(i16::MAX) {
        return Err(Ft8DecodeError::InvalidResultMetadata);
    }
    Ok(value.round() as i16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_resolved_identity_remains_ambiguous() {
        let classified = classify_supported("CQ BAD FN42", None).unwrap();
        assert!(matches!(classified, ClassifiedFt8Message::Ambiguous(_)));
    }
}
