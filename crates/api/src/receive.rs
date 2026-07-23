//! Version-2 receive-only wire vocabulary.

use serde::{Deserialize, Serialize};
use slotpilot_domain::ReceiveWindowId;

use crate::CanonicalizationError;

/// A public receive payload exceeded its fixed collection/text bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireBoundError {
    /// One or more fields exceeded a documented bound.
    Exceeded,
}

/// Maximum input devices returned by one discovery response.
pub const MAX_INPUT_DEVICES: usize = 64;
/// Maximum exact configurations returned for one device.
pub const MAX_INPUT_CONFIGURATIONS: usize = 64;
/// Maximum receive records in one history response.
pub const MAX_RECEIVE_HISTORY_PAGE: u16 = 100;
/// Maximum decode outcomes in one public receive record.
pub const MAX_RECEIVE_DECODES: usize = 128;
/// Maximum bins in one public waterfall frame.
pub const MAX_WATERFALL_BINS: usize = 2_048;

/// Portable platform family owning a stable input identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputPlatform {
    /// macOS Core Audio.
    MacOsCoreAudio,
    /// Windows WASAPI.
    WindowsWasapi,
    /// Linux ALSA.
    LinuxAlsa,
    /// Linux JACK.
    LinuxJack,
}

/// Stable device identity. Display metadata is structurally excluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDeviceIdentity {
    /// Platform family defining the opaque value.
    pub platform: InputPlatform,
    /// Stable platform-owned identity.
    pub opaque_id: String,
}

/// Supported input sample representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSampleFormat {
    /// Signed 8-bit integer.
    Signed8,
    /// Signed 16-bit integer.
    Signed16,
    /// Signed 24-bit integer.
    Signed24,
    /// Signed 32-bit integer.
    Signed32,
    /// Signed 64-bit integer.
    Signed64,
    /// Unsigned 8-bit integer.
    Unsigned8,
    /// Unsigned 16-bit integer.
    Unsigned16,
    /// Unsigned 24-bit integer.
    Unsigned24,
    /// Unsigned 32-bit integer.
    Unsigned32,
    /// Unsigned 64-bit integer.
    Unsigned64,
    /// IEEE 754 32-bit float.
    Float32,
    /// IEEE 754 64-bit float.
    Float64,
}

/// Exact selected input format and channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputConfiguration {
    /// Source frames per second.
    pub sample_rate_hz: u32,
    /// Interleaved source channels.
    pub channels: u16,
    /// Source sample representation.
    pub sample_format: InputSampleFormat,
    /// Zero-based selected receive channel.
    pub selected_channel: u16,
}

/// Explicit receive selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiveSelection {
    /// Stable device identity.
    pub device_identity: InputDeviceIdentity,
    /// Exact input configuration.
    pub configuration: InputConfiguration,
}

impl ReceiveSelection {
    pub(crate) fn validate(&self) -> Result<(), CanonicalizationError> {
        let id = self.device_identity.opaque_id.as_bytes();
        if id.is_empty()
            || id.len() > 256
            || id.iter().any(|byte| byte.is_ascii_control())
            || !(8_000..=384_000).contains(&self.configuration.sample_rate_hz)
            || self.configuration.channels == 0
            || self.configuration.channels > 32
            || self.configuration.selected_channel >= self.configuration.channels
        {
            return Err(CanonicalizationError::InvalidReceiveCommand);
        }
        Ok(())
    }
}

/// One discovered device with bounded display metadata and configurations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDevice {
    /// Stable identity used for selection.
    pub identity: InputDeviceIdentity,
    /// Non-identifying display label.
    pub display_name: String,
    /// Optional display manufacturer.
    pub manufacturer: Option<String>,
    /// Bounded selectable configuration ranges.
    pub configuration_ranges: Vec<InputConfigurationRange>,
}

/// One bounded selectable input configuration range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputConfigurationRange {
    /// Inclusive minimum sample rate.
    pub min_sample_rate_hz: u32,
    /// Inclusive maximum sample rate.
    pub max_sample_rate_hz: u32,
    /// Interleaved channel count.
    pub channels: u16,
    /// Source sample representation.
    pub sample_format: InputSampleFormat,
}

/// Bounded discovery result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDevicePage {
    /// Devices in deterministic adapter order.
    pub devices: Vec<InputDevice>,
}

impl InputDevicePage {
    /// Validates discovery response bounds.
    pub fn validate(&self) -> Result<(), WireBoundError> {
        if self.devices.len() > MAX_INPUT_DEVICES
            || self.devices.iter().any(|device| {
                device.identity.opaque_id.is_empty()
                    || device.identity.opaque_id.len() > 256
                    || device.display_name.is_empty()
                    || device.display_name.len() > 256
                    || device
                        .manufacturer
                        .as_ref()
                        .is_some_and(|value| value.len() > 256)
                    || device.configuration_ranges.is_empty()
                    || device.configuration_ranges.len() > MAX_INPUT_CONFIGURATIONS
            })
        {
            return Err(WireBoundError::Exceeded);
        }
        Ok(())
    }
}

/// Stable receive inhibition/fault category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveInhibitionKind {
    /// Receive service is unavailable in this composition.
    ServiceUnavailable,
    /// Exact input could not start or disappeared.
    DeviceUnavailable,
    /// Callback or worker capacity overflowed.
    Overflow,
    /// Capture continuity was invalidated.
    Discontinuity,
    /// UTC/monotonic mapping is unhealthy.
    ClockUnhealthy,
    /// Canonical timeline rejected evidence.
    TimelineInvalid,
    /// Offline decoder failed.
    DecoderFailed,
    /// Durable receive transaction failed.
    StorageFailed,
    /// Cancellation stopped receive.
    Cancelled,
}

/// Public receive lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ReceiveLifecycleSnapshot {
    /// No input resource exists.
    Stopped {
        /// Last issued stream generation.
        last_stream_generation: u64,
    },
    /// Exact input is opening.
    Starting {
        /// Reserved fresh stream generation.
        stream_generation: u64,
    },
    /// Receive worker is active.
    Receiving {
        /// Current stream generation.
        stream_generation: u64,
    },
    /// Receive stopped on explicit failure.
    Inhibited {
        /// Failed stream generation.
        stream_generation: u64,
        /// Stable fault category.
        reason: ReceiveInhibitionKind,
    },
    /// Input resources are being released.
    Stopping {
        /// Stream generation being stopped.
        stream_generation: u64,
    },
}

/// Bounded audio health in explicit integer units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiveAudioHealth {
    /// Estimated input latency.
    pub latency_millis: u32,
    /// Estimated signed drift.
    pub drift_parts_per_million: i32,
    /// Callback queue overflow count.
    pub overflow_count: u64,
    /// Clipped sample count.
    pub clipped_sample_count: u64,
    /// Greatest callback delay.
    pub max_callback_delay_millis: u32,
}

/// Public clock health.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ReceiveClockHealth {
    /// Mapping is fresh and usable.
    Healthy {
        /// Age of accepted mapping.
        mapping_age_millis: u64,
    },
    /// Mapping is latched unhealthy.
    Unhealthy {
        /// Stable fault category.
        reason: ReceiveInhibitionKind,
        /// Recovery observations completed.
        recovery_progress: u8,
        /// Recovery observations required.
        recovery_required: u8,
        /// Age of last accepted mapping.
        mapping_age_millis: u64,
    },
}

/// Current receive state and optional live health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiveStatus {
    /// Current lifecycle.
    pub lifecycle: ReceiveLifecycleSnapshot,
    /// Exact configured selection, if any.
    pub selection: Option<ReceiveSelection>,
    /// Latest audio health, if capture has produced evidence.
    pub audio: Option<ReceiveAudioHealth>,
    /// Latest clock health, if a monitor exists.
    pub clock: Option<ReceiveClockHealth>,
}

impl ReceiveStatus {
    /// Inactive restart-safe status.
    #[must_use]
    pub const fn stopped() -> Self {
        Self {
            lifecycle: ReceiveLifecycleSnapshot::Stopped {
                last_stream_generation: 0,
            },
            selection: None,
            audio: None,
            clock: None,
        }
    }
}

/// FT8 supported message class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ft8MessageClass {
    /// General call.
    GeneralCall,
    /// Directed grid exchange.
    DirectedGrid,
    /// Directed signal report.
    SignalReport,
    /// Roger plus signal report.
    RogerSignalReport,
    /// Roger acknowledgement.
    Roger,
    /// `73` ending.
    Ending73,
    /// `RR73` ending.
    EndingRr73,
}

/// Exact owned FT8 classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ft8Classification {
    /// Supported resolved structured message.
    Resolved {
        /// Canonical message text.
        canonical_text: String,
        /// Full sender identity.
        sender: String,
        /// Full recipient identity, if directed.
        recipient: Option<String>,
        /// Supported message class.
        message_class: Ft8MessageClass,
    },
    /// At least one callsign hash remains unresolved.
    UnresolvedHash {
        /// Canonical text.
        canonical_text: String,
        /// Bounded explanation.
        detail: String,
    },
    /// Structured but unsupported message.
    Unsupported {
        /// Canonical text.
        canonical_text: String,
        /// Bounded explanation.
        detail: String,
    },
    /// Multiple interpretations remain plausible.
    Ambiguous {
        /// Canonical text.
        canonical_text: String,
        /// Bounded explanation.
        detail: String,
    },
    /// Decoded free text.
    FreeText {
        /// Text.
        text: String,
    },
}

/// One typed FT8 decode with integer metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ft8Decode {
    /// Start offset in milliseconds.
    pub start_offset_millis: i32,
    /// Audio frequency in hertz.
    pub audio_frequency_hz: u32,
    /// Signal-to-noise ratio in dB.
    pub signal_to_noise_db: i16,
    /// Exact classification.
    pub classification: Ft8Classification,
}

/// Public summary of one durable receive record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiveRecordSummary {
    /// Global receive sequence.
    pub sequence: u64,
    /// Stable receive identity.
    pub receive_window_id: ReceiveWindowId,
    /// UTC FT8 slot start.
    pub slot_start_utc_millis: i64,
    /// Exact stable input selection.
    pub selection: ReceiveSelection,
    /// Audio health recorded atomically with decodes.
    pub audio: ReceiveAudioHealth,
    /// Clock health recorded atomically with decodes.
    pub clock: ReceiveClockHealth,
    /// Deterministically ordered typed outcomes.
    pub decodes: Vec<Ft8Decode>,
}

impl ReceiveRecordSummary {
    pub(crate) fn validate(&self) -> Result<(), WireBoundError> {
        if self.decodes.len() > MAX_RECEIVE_DECODES {
            return Err(WireBoundError::Exceeded);
        }
        Ok(())
    }
}

/// Bounded durable history response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiveHistoryPage {
    /// Ordered records after the requested sequence.
    pub records: Vec<ReceiveRecordSummary>,
    /// Next global sequence cursor.
    pub next_sequence: u64,
    /// Whether more committed records exist.
    pub has_more: bool,
}

impl ReceiveHistoryPage {
    /// Validates page and per-record bounds.
    pub fn validate(&self) -> Result<(), WireBoundError> {
        if self.records.len() > usize::from(MAX_RECEIVE_HISTORY_PAGE) {
            return Err(WireBoundError::Exceeded);
        }
        self.records
            .iter()
            .try_for_each(ReceiveRecordSummary::validate)
    }
}

/// Receive health event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiveHealthSnapshot {
    /// Current lifecycle.
    pub lifecycle: ReceiveLifecycleSnapshot,
    /// Audio health, if present.
    pub audio: Option<ReceiveAudioHealth>,
    /// Clock health, if present.
    pub clock: Option<ReceiveClockHealth>,
}

/// Explicit discontinuity event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiveDiscontinuity {
    /// Failed stream generation.
    pub stream_generation: u64,
    /// Stable reason.
    pub reason: ReceiveInhibitionKind,
    /// Known dropped frames.
    pub dropped_frames: u64,
}

/// One integer waterfall bin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaterfallBin {
    /// Center frequency in millihertz.
    pub frequency_millihz: u64,
    /// Magnitude in milli-dBFS.
    pub magnitude_millidbfs: i32,
}

/// One bounded coalesced waterfall frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaterfallFrame {
    /// Stream generation.
    pub stream_generation: u64,
    /// UTC row timestamp.
    pub utc_unix_millis: i64,
    /// Fixed publication sequence.
    pub frame_sequence: u64,
    /// Whether intermediate frames were coalesced for a slow consumer.
    pub coalesced: bool,
    /// Bounded bins.
    pub bins: Vec<WaterfallBin>,
}

impl WaterfallFrame {
    /// Validates the fixed public bin bound.
    pub fn validate(&self) -> Result<(), WireBoundError> {
        if self.bins.len() > MAX_WATERFALL_BINS {
            return Err(WireBoundError::Exceeded);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_collection_bounds_reject_oversized_values() {
        let frame = WaterfallFrame {
            stream_generation: 1,
            utc_unix_millis: 1,
            frame_sequence: 1,
            coalesced: false,
            bins: vec![
                WaterfallBin {
                    frequency_millihz: 1,
                    magnitude_millidbfs: -1,
                };
                MAX_WATERFALL_BINS + 1
            ],
        };
        assert_eq!(frame.validate(), Err(WireBoundError::Exceeded));
        let page = InputDevicePage {
            devices: (0..=MAX_INPUT_DEVICES)
                .map(|index| InputDevice {
                    identity: InputDeviceIdentity {
                        platform: InputPlatform::LinuxAlsa,
                        opaque_id: format!("device-{index}"),
                    },
                    display_name: "Input".into(),
                    manufacturer: None,
                    configuration_ranges: vec![InputConfigurationRange {
                        min_sample_rate_hz: 48_000,
                        max_sample_rate_hz: 48_000,
                        channels: 1,
                        sample_format: InputSampleFormat::Signed16,
                    }],
                })
                .collect(),
        };
        assert_eq!(page.validate(), Err(WireBoundError::Exceeded));
    }

    #[test]
    fn exact_selection_excludes_display_identity_and_checks_shape() {
        let selection = ReceiveSelection {
            device_identity: InputDeviceIdentity {
                platform: InputPlatform::MacOsCoreAudio,
                opaque_id: "stable-id".into(),
            },
            configuration: InputConfiguration {
                sample_rate_hz: 48_000,
                channels: 2,
                sample_format: InputSampleFormat::Signed16,
                selected_channel: 1,
            },
        };
        assert!(selection.validate().is_ok());
        let mut invalid = selection;
        invalid.configuration.selected_channel = 2;
        assert!(invalid.validate().is_err());
    }
}
