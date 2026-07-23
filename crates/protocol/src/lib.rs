//! SlotPilot-owned offline protocol contracts.
//!
//! Phase 1 begins with FT8-only value types and traits. This crate contains no
//! DSP implementation, file parser, audio-device access, QSO policy, station
//! command, persistence, scheduling, PTT, or transmit authority.

use std::cmp::Ordering;

use slotpilot_domain::{AudioFrequency, FullCallsign};
use thiserror::Error;

/// Number of information bits in one packed FT8 message.
pub const FT8_MESSAGE_BITS: usize = 77;

/// Nominal duration of an FT8 scheduling slot.
pub const FT8_SLOT_MILLIS: u32 = 15_000;

/// Maximum number of interleaved samples accepted by an owned offline buffer.
pub const MAX_OFFLINE_PCM_SAMPLES: usize = 24_000_000;

/// A supported, resolved FT8 message class without QSO-stage semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ft8MessageClass {
    /// A general call such as `CQ K1ABC FN42`.
    GeneralCall,
    /// A directed exchange carrying a Maidenhead grid.
    DirectedGrid,
    /// A directed signal report.
    SignalReport,
    /// A directed roger plus signal report.
    RogerSignalReport,
    /// A directed acknowledgement.
    Roger,
    /// A directed `73` ending.
    Ending73,
    /// A directed `RR73` ending.
    EndingRr73,
}

impl Ft8MessageClass {
    const fn requires_recipient(self) -> bool {
        !matches!(self, Self::GeneralCall)
    }
}

/// Failure constructing or converting an owned FT8 message.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Ft8MessageError {
    /// Canonical text was empty, non-ASCII, or exceeded the bounded size.
    #[error("FT8 canonical text must be 1 to 64 printable ASCII bytes")]
    InvalidCanonicalText,
    /// A general call had a recipient or a directed message lacked one.
    #[error("FT8 sender/recipient shape does not match its message class")]
    InvalidAddressing,
    /// A bounded explanatory value was empty, non-ASCII, or too long.
    #[error("FT8 classification detail must be 1 to 128 printable ASCII bytes")]
    InvalidDetail,
    /// A caller requested a resolved message from a different outcome.
    #[error("FT8 outcome {actual:?} is not a resolved supported message")]
    NotResolved {
        /// Outcome that prevented the checked conversion.
        actual: Ft8OutcomeKind,
    },
}

/// A supported FT8 message whose callsign identities are resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFt8Message {
    canonical_text: String,
    sender: FullCallsign,
    recipient: Option<FullCallsign>,
    class: Ft8MessageClass,
}

impl ResolvedFt8Message {
    /// Constructs a checked resolved message.
    pub fn new(
        canonical_text: impl Into<String>,
        sender: FullCallsign,
        recipient: Option<FullCallsign>,
        class: Ft8MessageClass,
    ) -> Result<Self, Ft8MessageError> {
        let canonical_text = canonical_text.into();
        validate_text(&canonical_text, 64).map_err(|()| Ft8MessageError::InvalidCanonicalText)?;
        if class.requires_recipient() != recipient.is_some() {
            return Err(Ft8MessageError::InvalidAddressing);
        }
        Ok(Self {
            canonical_text,
            sender,
            recipient,
            class,
        })
    }

    /// Returns the reviewed canonical message text.
    #[must_use]
    pub fn canonical_text(&self) -> &str {
        &self.canonical_text
    }

    /// Returns the full sender identity without base-call normalization.
    #[must_use]
    pub const fn sender(&self) -> &FullCallsign {
        &self.sender
    }

    /// Returns the full recipient identity for a directed message.
    #[must_use]
    pub const fn recipient(&self) -> Option<&FullCallsign> {
        self.recipient.as_ref()
    }

    /// Returns the protocol classification, not a QSO transition.
    #[must_use]
    pub const fn class(&self) -> Ft8MessageClass {
        self.class
    }
}

/// A decoded message containing at least one unresolved callsign hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedHashFt8Message {
    canonical_text: String,
    detail: String,
}

impl UnresolvedHashFt8Message {
    /// Constructs a bounded unresolved-hash outcome.
    pub fn new(
        canonical_text: impl Into<String>,
        detail: impl Into<String>,
    ) -> Result<Self, Ft8MessageError> {
        let canonical_text = canonical_text.into();
        let detail = detail.into();
        validate_text(&canonical_text, 64).map_err(|()| Ft8MessageError::InvalidCanonicalText)?;
        validate_text(&detail, 128).map_err(|()| Ft8MessageError::InvalidDetail)?;
        Ok(Self {
            canonical_text,
            detail,
        })
    }

    /// Returns the canonical decoded text, including unresolved notation.
    #[must_use]
    pub fn canonical_text(&self) -> &str {
        &self.canonical_text
    }

    /// Returns a stable owned explanation of what remains unresolved.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// A decoded structured FT8 message outside the supported Phase 1 matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedFt8Message {
    canonical_text: String,
    detail: String,
}

impl UnsupportedFt8Message {
    /// Constructs a bounded unsupported-structured outcome.
    pub fn new(
        canonical_text: impl Into<String>,
        detail: impl Into<String>,
    ) -> Result<Self, Ft8MessageError> {
        let canonical_text = canonical_text.into();
        let detail = detail.into();
        validate_text(&canonical_text, 64).map_err(|()| Ft8MessageError::InvalidCanonicalText)?;
        validate_text(&detail, 128).map_err(|()| Ft8MessageError::InvalidDetail)?;
        Ok(Self {
            canonical_text,
            detail,
        })
    }

    /// Returns the canonical decoded text.
    #[must_use]
    pub fn canonical_text(&self) -> &str {
        &self.canonical_text
    }

    /// Returns a stable owned explanation of the unsupported structure.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// A decoded FT8 payload with multiple plausible owned interpretations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguousFt8Message {
    canonical_text: String,
    detail: String,
}

impl AmbiguousFt8Message {
    /// Constructs a bounded ambiguous outcome.
    pub fn new(
        canonical_text: impl Into<String>,
        detail: impl Into<String>,
    ) -> Result<Self, Ft8MessageError> {
        let canonical_text = canonical_text.into();
        let detail = detail.into();
        validate_text(&canonical_text, 64).map_err(|()| Ft8MessageError::InvalidCanonicalText)?;
        validate_text(&detail, 128).map_err(|()| Ft8MessageError::InvalidDetail)?;
        Ok(Self {
            canonical_text,
            detail,
        })
    }

    /// Returns the canonical decoded text.
    #[must_use]
    pub fn canonical_text(&self) -> &str {
        &self.canonical_text
    }

    /// Returns a stable owned explanation of the ambiguity.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// Free text decoded from the FT8 payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeTextFt8Message(String);

impl FreeTextFt8Message {
    /// Constructs bounded printable free text.
    pub fn new(text: impl Into<String>) -> Result<Self, Ft8MessageError> {
        let text = text.into();
        validate_text(&text, 64).map_err(|()| Ft8MessageError::InvalidCanonicalText)?;
        Ok(Self(text))
    }

    /// Returns the decoded free text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.0
    }
}

/// Discriminant for a decoded FT8 outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ft8OutcomeKind {
    /// Supported and fully resolved.
    Resolved,
    /// At least one callsign hash is unresolved.
    UnresolvedHash,
    /// Structured but outside the supported matrix.
    Unsupported,
    /// Multiple plausible interpretations remain.
    Ambiguous,
    /// Arbitrary free text.
    FreeText,
}

/// Mutually exclusive owned FT8 decode classifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifiedFt8Message {
    /// Supported and fully resolved.
    Resolved(ResolvedFt8Message),
    /// At least one callsign hash is unresolved.
    UnresolvedHash(UnresolvedHashFt8Message),
    /// Structured but outside the supported matrix.
    Unsupported(UnsupportedFt8Message),
    /// Multiple plausible interpretations remain.
    Ambiguous(AmbiguousFt8Message),
    /// Arbitrary free text.
    FreeText(FreeTextFt8Message),
}

impl ClassifiedFt8Message {
    /// Returns the outcome discriminant.
    #[must_use]
    pub const fn kind(&self) -> Ft8OutcomeKind {
        match self {
            Self::Resolved(_) => Ft8OutcomeKind::Resolved,
            Self::UnresolvedHash(_) => Ft8OutcomeKind::UnresolvedHash,
            Self::Unsupported(_) => Ft8OutcomeKind::Unsupported,
            Self::Ambiguous(_) => Ft8OutcomeKind::Ambiguous,
            Self::FreeText(_) => Ft8OutcomeKind::FreeText,
        }
    }

    /// Returns the owned canonical text for deterministic comparison.
    #[must_use]
    pub fn canonical_text(&self) -> &str {
        match self {
            Self::Resolved(message) => message.canonical_text(),
            Self::UnresolvedHash(message) => message.canonical_text(),
            Self::Unsupported(message) => message.canonical_text(),
            Self::Ambiguous(message) => message.canonical_text(),
            Self::FreeText(message) => message.text(),
        }
    }

    /// Performs the explicit checked conversion required by automatic consumers.
    pub fn try_into_resolved(self) -> Result<ResolvedFt8Message, Ft8MessageError> {
        let actual = self.kind();
        match self {
            Self::Resolved(message) => Ok(message),
            _ => Err(Ft8MessageError::NotResolved { actual }),
        }
    }
}

impl TryFrom<ClassifiedFt8Message> for ResolvedFt8Message {
    type Error = Ft8MessageError;

    fn try_from(value: ClassifiedFt8Message) -> Result<Self, Self::Error> {
        value.try_into_resolved()
    }
}

/// Explicit metadata for one offline FT8 decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ft8DecodeMetadata {
    /// Start offset from the beginning of the supplied PCM window.
    pub start_offset_millis: i32,
    /// Audio-frequency offset in integer hertz.
    pub audio_frequency_hz: u32,
    /// Decoder-reported signal-to-noise ratio in integer decibels.
    pub signal_to_noise_db: i16,
}

/// One owned offline FT8 decode result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ft8Decode {
    /// Explicit signal metadata.
    pub metadata: Ft8DecodeMetadata,
    /// Checked protocol classification.
    pub message: ClassifiedFt8Message,
}

impl Ft8Decode {
    /// Sorts results by time, frequency, text, classification, then SNR.
    ///
    /// This ordering is independent of upstream container or worker order.
    pub fn sort_deterministically(results: &mut [Self]) {
        results.sort_by(Self::deterministic_cmp);
    }

    fn deterministic_cmp(left: &Self, right: &Self) -> Ordering {
        (
            left.metadata.start_offset_millis,
            left.metadata.audio_frequency_hz,
            left.message.canonical_text(),
            left.message.kind(),
            left.metadata.signal_to_noise_db,
        )
            .cmp(&(
                right.metadata.start_offset_millis,
                right.metadata.audio_frequency_hz,
                right.message.canonical_text(),
                right.message.kind(),
                right.metadata.signal_to_noise_db,
            ))
    }
}

/// SlotPilot-owned packed FT8 information bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedFt8Bits([bool; FT8_MESSAGE_BITS]);

impl PackedFt8Bits {
    /// Constructs a packed FT8 bit vector.
    #[must_use]
    pub const fn new(bits: [bool; FT8_MESSAGE_BITS]) -> Self {
        Self(bits)
    }

    /// Returns the 77 information bits in transmission order.
    #[must_use]
    pub const fn bits(&self) -> &[bool; FT8_MESSAGE_BITS] {
        &self.0
    }
}

/// Request to pack a supported resolved FT8 message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ft8EncodeRequest {
    /// Message eligible for protocol encoding, not automatic transition.
    pub message: ResolvedFt8Message,
}

/// Request to unpack one owned 77-bit FT8 payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ft8DecodeBitsRequest {
    /// Dependency-independent packed information bits.
    pub bits: PackedFt8Bits,
}

/// Typed FT8 message-codec failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Ft8CodecError {
    /// The resolved message is not representable by the reviewed codec.
    #[error("resolved FT8 message is not representable: {detail}")]
    NotRepresentable {
        /// Stable owned explanation.
        detail: String,
    },
    /// The packed payload could not be classified safely.
    #[error("packed FT8 message is invalid: {detail}")]
    InvalidPackedMessage {
        /// Stable owned explanation.
        detail: String,
    },
    /// The private adapter failed without a safe classified result.
    #[error("FT8 codec failed: {detail}")]
    Adapter {
        /// Stable owned explanation.
        detail: String,
    },
}

impl Ft8CodecError {
    /// Constructs a bounded not-representable failure.
    pub fn not_representable(detail: impl Into<String>) -> Result<Self, Ft8MessageError> {
        bounded_error(|detail| Self::NotRepresentable { detail }, detail)
    }

    /// Constructs a bounded invalid-payload failure.
    pub fn invalid_packed_message(detail: impl Into<String>) -> Result<Self, Ft8MessageError> {
        bounded_error(|detail| Self::InvalidPackedMessage { detail }, detail)
    }

    /// Constructs a bounded private-adapter failure.
    pub fn adapter(detail: impl Into<String>) -> Result<Self, Ft8MessageError> {
        bounded_error(|detail| Self::Adapter { detail }, detail)
    }
}

/// Owned FT8 message packing and unpacking boundary.
pub trait Ft8MessageCodec {
    /// Packs one supported resolved message.
    fn encode(&self, request: &Ft8EncodeRequest) -> Result<PackedFt8Bits, Ft8CodecError>;

    /// Unpacks and classifies one 77-bit payload.
    fn decode_bits(
        &self,
        request: &Ft8DecodeBitsRequest,
    ) -> Result<ClassifiedFt8Message, Ft8CodecError>;
}

/// Signed 16-bit interleaved PCM, with byte order explicit for file adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PcmSampleFormat {
    /// Signed little-endian 16-bit PCM.
    Signed16LittleEndian,
}

/// Failure validating offline PCM metadata or contents.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PcmError {
    /// The sample rate was zero or outside the offline safety bound.
    #[error("PCM sample rate must be between 1 and 384,000 Hz")]
    InvalidSampleRate,
    /// Channel count was zero or outside the offline safety bound.
    #[error("PCM channel count must be between 1 and 8")]
    InvalidChannelCount,
    /// Interleaved samples did not contain complete frames.
    #[error("PCM sample count must be divisible by its channel count")]
    IncompleteFrame,
    /// The buffer exceeded the bounded offline sample count.
    #[error("PCM buffer exceeds the offline sample limit")]
    BufferTooLarge,
}

/// Explicit format of an offline interleaved PCM buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PcmFormat {
    sample_rate_hz: u32,
    channels: u16,
    sample_format: PcmSampleFormat,
}

impl PcmFormat {
    /// Constructs checked offline PCM metadata.
    pub fn new(
        sample_rate_hz: u32,
        channels: u16,
        sample_format: PcmSampleFormat,
    ) -> Result<Self, PcmError> {
        if !(1..=384_000).contains(&sample_rate_hz) {
            return Err(PcmError::InvalidSampleRate);
        }
        if !(1..=8).contains(&channels) {
            return Err(PcmError::InvalidChannelCount);
        }
        Ok(Self {
            sample_rate_hz,
            channels,
            sample_format,
        })
    }

    /// Returns the sample rate in frames per second.
    #[must_use]
    pub const fn sample_rate_hz(self) -> u32 {
        self.sample_rate_hz
    }

    /// Returns the number of interleaved channels.
    #[must_use]
    pub const fn channels(self) -> u16 {
        self.channels
    }

    /// Returns the explicit sample representation.
    #[must_use]
    pub const fn sample_format(self) -> PcmSampleFormat {
        self.sample_format
    }
}

/// Explicit duration metadata derived from complete PCM frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PcmDuration {
    frames: u32,
    microseconds: u64,
}

impl PcmDuration {
    /// Returns complete sample frames per channel.
    #[must_use]
    pub const fn frames(self) -> u32 {
        self.frames
    }

    /// Returns the truncated duration in integer microseconds.
    #[must_use]
    pub const fn microseconds(self) -> u64 {
        self.microseconds
    }
}

/// Bounded owned offline PCM samples and explicit metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmBuffer {
    format: PcmFormat,
    duration: PcmDuration,
    samples: Vec<i16>,
}

impl PcmBuffer {
    /// Constructs and validates a complete interleaved PCM buffer.
    pub fn new(format: PcmFormat, samples: Vec<i16>) -> Result<Self, PcmError> {
        if samples.len() > MAX_OFFLINE_PCM_SAMPLES {
            return Err(PcmError::BufferTooLarge);
        }
        let channels = usize::from(format.channels);
        if !samples.len().is_multiple_of(channels) {
            return Err(PcmError::IncompleteFrame);
        }
        let frames =
            u32::try_from(samples.len() / channels).map_err(|_| PcmError::BufferTooLarge)?;
        let microseconds =
            u64::from(frames).saturating_mul(1_000_000) / u64::from(format.sample_rate_hz);
        Ok(Self {
            format,
            duration: PcmDuration {
                frames,
                microseconds,
            },
            samples,
        })
    }

    /// Returns the explicit PCM format.
    #[must_use]
    pub const fn format(&self) -> PcmFormat {
        self.format
    }

    /// Returns duration derived from complete frames.
    #[must_use]
    pub const fn duration(&self) -> PcmDuration {
        self.duration
    }

    /// Returns interleaved signed samples.
    #[must_use]
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }
}

/// Waveform placement within a purely offline FT8 buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ft8WaveformPlacement {
    /// Return only the encoded FT8 frame.
    FrameOnly,
    /// Place the frame at an explicit offset in a nominal 15-second buffer.
    FullSlot {
        /// Silent samples before the frame, per channel.
        start_frame: u32,
    },
}

/// Validated relative amplitude in per-mille of signed PCM full scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PcmAmplitudePermille(u16);

impl PcmAmplitudePermille {
    /// Constructs an amplitude from 1 through 1,000 per-mille.
    pub fn new(value: u16) -> Result<Self, Ft8WaveformError> {
        if !(1..=1_000).contains(&value) {
            return Err(Ft8WaveformError::InvalidAmplitude);
        }
        Ok(Self(value))
    }

    /// Returns the amplitude in per-mille.
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }
}

/// Owned request for deterministic offline FT8 waveform synthesis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ft8WaveformRequest {
    /// Supported resolved message to synthesize.
    pub message: ResolvedFt8Message,
    /// Explicit output format.
    pub format: PcmFormat,
    /// Tone-base audio frequency in integer hertz.
    pub audio_frequency: AudioFrequency,
    /// Relative full-scale amplitude.
    pub amplitude: PcmAmplitudePermille,
    /// Frame-only or explicit full-slot placement.
    pub placement: Ft8WaveformPlacement,
}

/// Typed offline waveform synthesis failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Ft8WaveformError {
    /// Relative amplitude was zero or exceeded full scale.
    #[error("PCM amplitude must be between 1 and 1,000 per-mille")]
    InvalidAmplitude,
    /// Frequency, format, duration, or placement cannot contain the FT8 frame.
    #[error("invalid FT8 waveform configuration: {detail}")]
    InvalidConfiguration {
        /// Stable owned explanation.
        detail: String,
    },
    /// Message packing failed.
    #[error(transparent)]
    Codec(#[from] Ft8CodecError),
    /// Owned PCM construction failed.
    #[error(transparent)]
    Pcm(#[from] PcmError),
}

impl Ft8WaveformError {
    /// Constructs a bounded configuration failure.
    pub fn invalid_configuration(detail: impl Into<String>) -> Result<Self, Ft8MessageError> {
        bounded_error(|detail| Self::InvalidConfiguration { detail }, detail)
    }
}

/// Owned deterministic offline FT8 waveform boundary.
pub trait Ft8WaveformSynthesizer {
    /// Produces samples only; it never selects or opens an audio device.
    fn synthesize(&self, request: &Ft8WaveformRequest) -> Result<PcmBuffer, Ft8WaveformError>;
}

fn bounded_error<T>(
    constructor: impl FnOnce(String) -> T,
    detail: impl Into<String>,
) -> Result<T, Ft8MessageError> {
    let detail = detail.into();
    validate_text(&detail, 128).map_err(|()| Ft8MessageError::InvalidDetail)?;
    Ok(constructor(detail))
}

fn validate_text(value: &str, maximum: usize) -> Result<(), ()> {
    if value.is_empty()
        || value.len() > maximum
        || !value
            .bytes()
            .all(|byte| byte.is_ascii() && !byte.is_ascii_control())
    {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directed_message() -> ResolvedFt8Message {
        ResolvedFt8Message::new(
            "K1ABC W1AW -12",
            "K1ABC".parse().unwrap(),
            Some("W1AW".parse().unwrap()),
            Ft8MessageClass::SignalReport,
        )
        .unwrap()
    }

    #[test]
    fn full_callsign_identity_is_preserved() {
        let message = ResolvedFt8Message::new(
            "CQ W1AW/1 FN31",
            "W1AW/1".parse().unwrap(),
            None,
            Ft8MessageClass::GeneralCall,
        )
        .unwrap();
        assert_eq!(message.sender().original(), "W1AW/1");
        assert_eq!(message.sender().base().as_str(), "W1AW");
    }

    #[test]
    fn resolved_addressing_shape_is_checked() {
        assert_eq!(
            ResolvedFt8Message::new(
                "K1ABC W1AW -12",
                "K1ABC".parse().unwrap(),
                None,
                Ft8MessageClass::SignalReport,
            ),
            Err(Ft8MessageError::InvalidAddressing)
        );
        assert_eq!(
            ResolvedFt8Message::new(
                "CQ K1ABC FN42",
                "K1ABC".parse().unwrap(),
                Some("W1AW".parse().unwrap()),
                Ft8MessageClass::GeneralCall,
            ),
            Err(Ft8MessageError::InvalidAddressing)
        );
    }

    #[test]
    fn non_resolved_outcomes_require_a_failing_checked_conversion() {
        let outcomes = [
            ClassifiedFt8Message::UnresolvedHash(
                UnresolvedHashFt8Message::new("<...> W1AW -12", "sender hash unresolved").unwrap(),
            ),
            ClassifiedFt8Message::Unsupported(
                UnsupportedFt8Message::new("K1ABC W1AW 123456", "unsupported structured subtype")
                    .unwrap(),
            ),
            ClassifiedFt8Message::Ambiguous(
                AmbiguousFt8Message::new("K1ABC W1AW", "two interpretations remain").unwrap(),
            ),
            ClassifiedFt8Message::FreeText(FreeTextFt8Message::new("HELLO WORLD").unwrap()),
        ];
        for outcome in outcomes {
            let kind = outcome.kind();
            assert_eq!(
                ResolvedFt8Message::try_from(outcome),
                Err(Ft8MessageError::NotResolved { actual: kind })
            );
        }
    }

    #[test]
    fn decode_results_have_an_owned_deterministic_order() {
        let mut results = vec![
            Ft8Decode {
                metadata: Ft8DecodeMetadata {
                    start_offset_millis: 200,
                    audio_frequency_hz: 1_000,
                    signal_to_noise_db: -20,
                },
                message: ClassifiedFt8Message::Resolved(directed_message()),
            },
            Ft8Decode {
                metadata: Ft8DecodeMetadata {
                    start_offset_millis: 100,
                    audio_frequency_hz: 1_500,
                    signal_to_noise_db: -10,
                },
                message: ClassifiedFt8Message::FreeText(
                    FreeTextFt8Message::new("HELLO WORLD").unwrap(),
                ),
            },
            Ft8Decode {
                metadata: Ft8DecodeMetadata {
                    start_offset_millis: 100,
                    audio_frequency_hz: 500,
                    signal_to_noise_db: -5,
                },
                message: ClassifiedFt8Message::Resolved(directed_message()),
            },
        ];
        Ft8Decode::sort_deterministically(&mut results);
        assert_eq!(results[0].metadata.audio_frequency_hz, 500);
        assert_eq!(results[1].metadata.audio_frequency_hz, 1_500);
        assert_eq!(results[2].metadata.start_offset_millis, 200);
    }

    #[test]
    fn pcm_metadata_has_explicit_units_and_complete_frames() {
        let format = PcmFormat::new(12_000, 2, PcmSampleFormat::Signed16LittleEndian).unwrap();
        let buffer = PcmBuffer::new(format, vec![1, -1, 2, -2]).unwrap();
        assert_eq!(buffer.format().sample_rate_hz(), 12_000);
        assert_eq!(buffer.format().channels(), 2);
        assert_eq!(buffer.duration().frames(), 2);
        assert_eq!(buffer.duration().microseconds(), 166);
        assert_eq!(
            PcmBuffer::new(format, vec![1, 2, 3]),
            Err(PcmError::IncompleteFrame)
        );
    }
}
