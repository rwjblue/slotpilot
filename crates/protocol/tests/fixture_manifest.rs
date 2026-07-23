//! Schema, provenance, checksum, and unit checks for the reviewed FT8 corpus.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Manifest {
    schema_version: u32,
    compatibility_claim: String,
    reference: Reference,
    supplemental_observations: Vec<serde_json::Value>,
    message_vectors: Vec<MessageVector>,
    recordings: Vec<Recording>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Reference {
    authority: String,
    program: String,
    version: String,
    source_project_url: String,
    release_artifact_url: String,
    release_artifact_sha256: String,
    license: String,
    reviewed_on: String,
    message_settings: String,
    decode_settings: String,
    notes: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MessageVector {
    id: String,
    reference_authority: String,
    input_text: String,
    canonical_text: String,
    outcome: String,
    message_class: Option<String>,
    sender: Option<String>,
    recipient: Option<String>,
    exactly_representable: bool,
    payload_bits: Option<String>,
    notes: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Recording {
    id: String,
    path: String,
    purpose: String,
    origin: String,
    generation_command: String,
    license: String,
    sha256: String,
    byte_length: u64,
    format: AudioFormat,
    expected_decodes: Vec<ExpectedDecode>,
    tolerance: DecodeTolerance,
    minimum_recall: usize,
    maximum_extras: usize,
    permitted_extra_messages: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AudioFormat {
    sample_rate_hz: u32,
    channels: u16,
    bits_per_sample: u16,
    duration_millis: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ExpectedDecode {
    canonical_text: String,
    outcome: String,
    message_class: Option<String>,
    sender: Option<String>,
    recipient: Option<String>,
    audio_frequency_hz: u32,
    start_offset_millis: i32,
    signal_to_noise_db: i16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct DecodeTolerance {
    audio_frequency_hz: u32,
    start_offset_millis: u32,
    signal_to_noise_db: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WaveMetadata {
    sample_rate_hz: u32,
    channels: u16,
    bits_per_sample: u16,
    duration_millis: u32,
}

#[test]
fn checked_in_manifest_and_recordings_are_valid() {
    let (root, manifest) = load_manifest();
    validate_manifest(&root, &manifest).unwrap();
}

#[test]
fn validation_rejects_duplicate_identity_missing_provenance_invalid_units_and_drift() {
    let (root, manifest) = load_manifest();

    let mut duplicate = manifest.clone();
    duplicate.recordings[1].id = duplicate.recordings[0].id.clone();
    assert!(validate_manifest(&root, &duplicate).is_err());

    let mut missing_provenance = manifest.clone();
    missing_provenance.recordings[0].origin.clear();
    assert!(validate_manifest(&root, &missing_provenance).is_err());

    let mut invalid_units = manifest.clone();
    invalid_units.recordings[0].format.sample_rate_hz = 0;
    assert!(validate_manifest(&root, &invalid_units).is_err());

    let mut checksum_drift = manifest;
    checksum_drift.recordings[0].sha256 = "0".repeat(64);
    assert!(validate_manifest(&root, &checksum_drift).is_err());
}

#[test]
fn malformed_or_dependency_specific_golden_data_is_rejected() {
    assert!(serde_json::from_str::<Manifest>("{}").is_err());
    let (root, mut manifest) = load_manifest();
    manifest.message_vectors[0].notes = "mfsk_core debug output".to_owned();
    assert!(validate_manifest(&root, &manifest).is_err());
}

fn load_manifest() -> (PathBuf, Manifest) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/ft8/v1")
        .canonicalize()
        .unwrap();
    let bytes = fs::read(root.join("manifest.json")).unwrap();
    let manifest = serde_json::from_slice(&bytes).unwrap();
    (root, manifest)
}

fn validate_manifest(root: &Path, manifest: &Manifest) -> Result<(), String> {
    if manifest.schema_version != 1 {
        return Err("unsupported fixture schema".to_owned());
    }
    require_text(&manifest.compatibility_claim, "compatibility claim")?;
    validate_reference(&manifest.reference)?;
    if !manifest.supplemental_observations.is_empty() {
        return Err("v1 records no supplemental decoder observations".to_owned());
    }

    let encoded = serde_json::to_string(manifest).map_err(|error| error.to_string())?;
    if encoded.contains("mfsk_core") || encoded.contains("mfsk-core") {
        return Err("dependency-specific golden data is forbidden".to_owned());
    }

    let mut message_ids = HashSet::new();
    for vector in &manifest.message_vectors {
        if !message_ids.insert(&vector.id) {
            return Err(format!("duplicate message vector id: {}", vector.id));
        }
        validate_message_vector(vector)?;
    }
    require_matrix_case(&manifest.message_vectors, "ordinary-cq-grid", "resolved")?;
    require_matrix_case(&manifest.message_vectors, "special-compound-cq", "resolved")?;
    require_matrix_case(
        &manifest.message_vectors,
        "special-hashed-resolved",
        "resolved",
    )?;
    require_matrix_case(
        &manifest.message_vectors,
        "special-hashed-unresolved",
        "unresolved_hash",
    )?;
    require_matrix_case(&manifest.message_vectors, "free-text", "free_text")?;
    require_matrix_case(
        &manifest.message_vectors,
        "unsupported-telemetry",
        "unsupported",
    )?;
    require_matrix_case(
        &manifest.message_vectors,
        "owned-ambiguous-boundary",
        "ambiguous",
    )?;

    let mut recording_ids = HashSet::new();
    let mut recording_paths = HashSet::new();
    for recording in &manifest.recordings {
        if !recording_ids.insert(&recording.id) {
            return Err(format!("duplicate recording id: {}", recording.id));
        }
        if !recording_paths.insert(&recording.path) {
            return Err(format!("duplicate recording path: {}", recording.path));
        }
        validate_recording(root, recording)?;
    }
    if manifest.recordings.len() != 2 {
        return Err("v1 must remain a deliberately bounded two-recording corpus".to_owned());
    }
    Ok(())
}

fn validate_reference(reference: &Reference) -> Result<(), String> {
    for (value, name) in [
        (&reference.authority, "reference authority"),
        (&reference.program, "reference program"),
        (&reference.version, "reference version"),
        (&reference.source_project_url, "reference source URL"),
        (&reference.release_artifact_url, "reference artifact URL"),
        (&reference.license, "reference license"),
        (&reference.reviewed_on, "reference review date"),
        (&reference.message_settings, "message settings"),
        (&reference.decode_settings, "decode settings"),
        (&reference.notes, "reference notes"),
    ] {
        require_text(value, name)?;
    }
    validate_sha256(&reference.release_artifact_sha256)?;
    if reference.authority != "wsjt_x_authoritative" {
        return Err("WSJT-X must remain the v1 authority".to_owned());
    }
    Ok(())
}

fn validate_message_vector(vector: &MessageVector) -> Result<(), String> {
    for (value, name) in [
        (&vector.id, "message id"),
        (&vector.reference_authority, "message authority"),
        (&vector.input_text, "message input"),
        (&vector.canonical_text, "canonical text"),
        (&vector.outcome, "message outcome"),
        (&vector.notes, "message notes"),
    ] {
        require_text(value, name)?;
    }
    if !matches!(
        vector.reference_authority.as_str(),
        "wsjt_x_authoritative" | "slotpilot_boundary_only"
    ) {
        return Err(format!(
            "invalid reference authority: {}",
            vector.reference_authority
        ));
    }
    if !matches!(
        vector.outcome.as_str(),
        "resolved" | "unresolved_hash" | "unsupported" | "ambiguous" | "free_text"
    ) {
        return Err(format!("invalid outcome: {}", vector.outcome));
    }
    if vector.outcome == "resolved" && (vector.message_class.is_none() || vector.sender.is_none()) {
        return Err(format!("resolved vector lacks owned fields: {}", vector.id));
    }
    if vector.reference_authority == "wsjt_x_authoritative" {
        let bits = vector
            .payload_bits
            .as_deref()
            .ok_or_else(|| format!("authoritative vector lacks bits: {}", vector.id))?;
        if bits.len() != slotpilot_protocol::FT8_MESSAGE_BITS
            || !bits.bytes().all(|byte| matches!(byte, b'0' | b'1'))
        {
            return Err(format!("invalid 77-bit payload: {}", vector.id));
        }
    }
    if vector.exactly_representable
        && vector.outcome == "resolved"
        && vector.reference_authority != "wsjt_x_authoritative"
    {
        return Err("only an authoritative vector may claim representation".to_owned());
    }
    if vector.recipient.is_some() && vector.sender.is_none() && vector.outcome == "resolved" {
        return Err("resolved recipient lacks sender".to_owned());
    }
    Ok(())
}

fn validate_recording(root: &Path, recording: &Recording) -> Result<(), String> {
    for (value, name) in [
        (&recording.id, "recording id"),
        (&recording.path, "recording path"),
        (&recording.purpose, "recording purpose"),
        (&recording.origin, "recording origin"),
        (&recording.generation_command, "generation command"),
        (&recording.license, "recording license"),
    ] {
        require_text(value, name)?;
    }
    validate_sha256(&recording.sha256)?;
    if recording.format.sample_rate_hz == 0
        || recording.format.channels == 0
        || recording.format.bits_per_sample != 16
        || recording.format.duration_millis == 0
    {
        return Err(format!("invalid recording units: {}", recording.id));
    }
    if recording.minimum_recall > recording.expected_decodes.len()
        || recording.maximum_extras < recording.permitted_extra_messages.len()
    {
        return Err(format!("inconsistent recall/extra rules: {}", recording.id));
    }
    if recording.tolerance.audio_frequency_hz > 20
        || recording.tolerance.start_offset_millis > 500
        || recording.tolerance.signal_to_noise_db > 10
    {
        return Err(format!("unreviewed broad tolerance: {}", recording.id));
    }
    for expected in &recording.expected_decodes {
        validate_expected_decode(expected, &recording.format)?;
    }

    let path = root.join(&recording.path);
    let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    if u64::try_from(bytes.len()).map_err(|error| error.to_string())? != recording.byte_length {
        return Err(format!("byte length drift: {}", recording.id));
    }
    let actual_hash = format!("{:x}", Sha256::digest(&bytes));
    if actual_hash != recording.sha256 {
        return Err(format!("checksum drift: {}", recording.id));
    }
    let metadata = parse_pcm_wave_metadata(&bytes)?;
    let expected_metadata = WaveMetadata {
        sample_rate_hz: recording.format.sample_rate_hz,
        channels: recording.format.channels,
        bits_per_sample: recording.format.bits_per_sample,
        duration_millis: recording.format.duration_millis,
    };
    if metadata != expected_metadata {
        return Err(format!(
            "WAV metadata drift for {}: {metadata:?}",
            recording.id
        ));
    }
    Ok(())
}

fn validate_expected_decode(expected: &ExpectedDecode, format: &AudioFormat) -> Result<(), String> {
    require_text(&expected.canonical_text, "expected canonical text")?;
    if expected.outcome != "resolved"
        || expected.message_class.is_none()
        || expected.sender.is_none()
    {
        return Err("recording expectations must be resolved owned results".to_owned());
    }
    if expected.audio_frequency_hz >= format.sample_rate_hz / 2
        || expected.start_offset_millis.unsigned_abs() > format.duration_millis
        || !(-50..=50).contains(&expected.signal_to_noise_db)
    {
        return Err(format!(
            "invalid expected decode units: {}",
            expected.canonical_text
        ));
    }
    if expected.recipient.is_some() && expected.sender.is_none() {
        return Err("expected recipient lacks sender".to_owned());
    }
    Ok(())
}

fn parse_pcm_wave_metadata(bytes: &[u8]) -> Result<WaveMetadata, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("fixture is not a RIFF/WAVE file".to_owned());
    }
    let mut cursor = 12_usize;
    let mut format = None;
    let mut data_bytes = None;
    while cursor.checked_add(8).is_some_and(|end| end <= bytes.len()) {
        let id = &bytes[cursor..cursor + 4];
        let size = usize::try_from(u32::from_le_bytes(
            bytes[cursor + 4..cursor + 8].try_into().unwrap(),
        ))
        .map_err(|error| error.to_string())?;
        let body_start = cursor + 8;
        let body_end = body_start
            .checked_add(size)
            .ok_or_else(|| "WAV chunk overflow".to_owned())?;
        if body_end > bytes.len() {
            return Err("truncated WAV chunk".to_owned());
        }
        if id == b"fmt " {
            if size < 16 {
                return Err("short WAV fmt chunk".to_owned());
            }
            let audio_format =
                u16::from_le_bytes(bytes[body_start..body_start + 2].try_into().unwrap());
            let channels =
                u16::from_le_bytes(bytes[body_start + 2..body_start + 4].try_into().unwrap());
            let sample_rate_hz =
                u32::from_le_bytes(bytes[body_start + 4..body_start + 8].try_into().unwrap());
            let bits_per_sample =
                u16::from_le_bytes(bytes[body_start + 14..body_start + 16].try_into().unwrap());
            if audio_format != 1 {
                return Err("fixture WAV is not integer PCM".to_owned());
            }
            format = Some((sample_rate_hz, channels, bits_per_sample));
        } else if id == b"data" {
            data_bytes = Some(size);
        }
        cursor = body_end + (size % 2);
    }
    let (sample_rate_hz, channels, bits_per_sample) =
        format.ok_or_else(|| "WAV lacks fmt chunk".to_owned())?;
    let data_bytes = data_bytes.ok_or_else(|| "WAV lacks data chunk".to_owned())?;
    let bytes_per_frame = usize::from(channels)
        .checked_mul(usize::from(bits_per_sample) / 8)
        .ok_or_else(|| "WAV frame size overflow".to_owned())?;
    if bytes_per_frame == 0 || !data_bytes.is_multiple_of(bytes_per_frame) {
        return Err("WAV data contains incomplete frames".to_owned());
    }
    let frames = data_bytes / bytes_per_frame;
    let duration_millis = u32::try_from(
        u64::try_from(frames)
            .map_err(|error| error.to_string())?
            .saturating_mul(1_000)
            / u64::from(sample_rate_hz),
    )
    .map_err(|error| error.to_string())?;
    Ok(WaveMetadata {
        sample_rate_hz,
        channels,
        bits_per_sample,
        duration_millis,
    })
}

fn validate_sha256(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid SHA-256".to_owned());
    }
    Ok(())
}

fn require_text(value: &str, name: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("missing {name}"));
    }
    Ok(())
}

fn require_matrix_case(vectors: &[MessageVector], id: &str, outcome: &str) -> Result<(), String> {
    if vectors
        .iter()
        .any(|vector| vector.id == id && vector.outcome == outcome)
    {
        Ok(())
    } else {
        Err(format!("missing matrix case {id}/{outcome}"))
    }
}
