//! Conformance checks for the private message adapter against owned fixtures.

use std::{fs, path::Path};

use serde_json::Value;
use slotpilot_domain::FullCallsign;
use slotpilot_protocol::{
    ClassifiedFt8Message, Ft8CodecError, Ft8DecodeBitsRequest, Ft8EncodeRequest, Ft8MessageClass,
    Ft8MessageCodec, OfflineFt8Codec, PackedFt8Bits, ResolvedFt8Message,
};

const AUTHORITATIVE_ROUND_TRIPS: &[&str] = &[
    "ordinary-cq-grid",
    "ordinary-directed-grid",
    "ordinary-signal-report",
    "ordinary-roger-report",
    "ordinary-rrr",
    "ordinary-73",
    "special-compound-cq",
];

#[test]
fn supported_owned_messages_match_authoritative_bits_and_decode_fields() {
    let manifest = load_manifest();
    let codec = OfflineFt8Codec::new();

    for id in AUTHORITATIVE_ROUND_TRIPS {
        let vector = vector(&manifest, id);
        let message = owned_resolved(vector);
        let encoded = codec
            .encode(&Ft8EncodeRequest {
                message: message.clone(),
            })
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        assert_eq!(bits_text(&encoded), text(vector, "payload_bits"), "{id}");

        let decoded = codec
            .decode_bits(&Ft8DecodeBitsRequest { bits: encoded })
            .unwrap();
        assert_eq!(decoded, ClassifiedFt8Message::Resolved(message), "{id}");
    }
}

#[test]
fn rr73_grid_collision_is_classified_from_bits_not_text() {
    let manifest = load_manifest();
    let vector = vector(&manifest, "ordinary-rr73");
    let codec = OfflineFt8Codec::new();
    let decoded = decode_vector(&codec, vector);
    assert_eq!(
        decoded,
        ClassifiedFt8Message::Resolved(owned_resolved(vector))
    );

    let ending = ResolvedFt8Message::new(
        text(vector, "canonical_text"),
        "K1ABC".parse().unwrap(),
        Some("W1AW".parse().unwrap()),
        Ft8MessageClass::EndingRr73,
    )
    .unwrap();
    let ending_bits = codec
        .encode(&Ft8EncodeRequest {
            message: ending.clone(),
        })
        .unwrap();
    assert_ne!(bits_text(&ending_bits), text(vector, "payload_bits"));
    assert_eq!(
        codec
            .decode_bits(&Ft8DecodeBitsRequest { bits: ending_bits })
            .unwrap(),
        ClassifiedFt8Message::Resolved(ending)
    );
}

#[test]
fn known_and_unknown_hashes_produce_distinct_owned_outcomes() {
    let manifest = load_manifest();
    let resolved_vector = vector(&manifest, "special-hashed-resolved");
    let bits = parse_bits(text(resolved_vector, "payload_bits"));

    let unresolved = OfflineFt8Codec::new()
        .decode_bits(&Ft8DecodeBitsRequest { bits: bits.clone() })
        .unwrap();
    assert!(matches!(
        unresolved,
        ClassifiedFt8Message::UnresolvedHash(_)
    ));
    assert_eq!(
        unresolved.canonical_text(),
        text(
            vector(&manifest, "special-hashed-unresolved"),
            "canonical_text"
        )
    );

    let compound: FullCallsign = "W1AW/1".parse().unwrap();
    let resolved = OfflineFt8Codec::with_known_calls([&compound])
        .decode_bits(&Ft8DecodeBitsRequest { bits })
        .unwrap();
    assert_eq!(
        resolved,
        ClassifiedFt8Message::Resolved(owned_resolved(resolved_vector))
    );
}

#[test]
fn free_text_and_unsupported_structures_never_become_resolved() {
    let manifest = load_manifest();
    let codec = OfflineFt8Codec::new();

    let free_text = decode_vector(&codec, vector(&manifest, "free-text"));
    assert!(matches!(free_text, ClassifiedFt8Message::FreeText(_)));
    assert!(free_text.try_into_resolved().is_err());

    let telemetry = decode_vector(&codec, vector(&manifest, "unsupported-telemetry"));
    assert!(matches!(telemetry, ClassifiedFt8Message::Unsupported(_)));
    assert!(telemetry.try_into_resolved().is_err());
}

#[test]
fn identity_loss_and_unexposed_hash_packing_are_typed_encode_failures() {
    let manifest = load_manifest();
    let codec = OfflineFt8Codec::new();

    let lossy = ResolvedFt8Message::new(
        text(
            vector(&manifest, "special-plain-not-representable"),
            "input_text",
        ),
        "W1AW/1".parse().unwrap(),
        None,
        Ft8MessageClass::GeneralCall,
    )
    .unwrap();
    assert!(matches!(
        codec.encode(&Ft8EncodeRequest { message: lossy }),
        Err(Ft8CodecError::NotRepresentable { .. })
    ));

    let hashed = owned_resolved(vector(&manifest, "special-hashed-resolved"));
    assert!(matches!(
        codec.encode(&Ft8EncodeRequest { message: hashed }),
        Err(Ft8CodecError::NotRepresentable { .. })
    ));
}

fn decode_vector(codec: &OfflineFt8Codec, vector: &Value) -> ClassifiedFt8Message {
    codec
        .decode_bits(&Ft8DecodeBitsRequest {
            bits: parse_bits(text(vector, "payload_bits")),
        })
        .unwrap()
}

fn owned_resolved(vector: &Value) -> ResolvedFt8Message {
    let sender = text(vector, "sender").parse().unwrap();
    let recipient = vector["recipient"]
        .as_str()
        .map(str::parse)
        .transpose()
        .unwrap();
    ResolvedFt8Message::new(
        text(vector, "canonical_text"),
        sender,
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

fn parse_bits(value: &str) -> PackedFt8Bits {
    let parsed: Vec<bool> = value.bytes().map(|byte| byte == b'1').collect();
    PackedFt8Bits::new(parsed.try_into().unwrap())
}

fn bits_text(bits: &PackedFt8Bits) -> String {
    bits.bits()
        .iter()
        .map(|bit| if *bit { '1' } else { '0' })
        .collect()
}

fn load_manifest() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ft8/v1/manifest.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn vector<'a>(manifest: &'a Value, id: &str) -> &'a Value {
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
