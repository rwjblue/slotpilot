//! Private CPAL receive-only device-discovery adapter.

use std::str::FromStr;

use cpal::{
    DeviceId, Error, ErrorKind, SampleFormat, SupportedStreamConfigRange,
    traits::{DeviceTrait, HostTrait},
};

use crate::{
    InputConfigurationRange, InputDeviceDescriptor, InputDeviceDiscovery, InputDeviceDisplay,
    InputDeviceIdentity, InputDiscoveryError, InputPlatform, InputSampleFormat,
};

/// Cross-platform receive-only discovery through the system's default host.
///
/// Discovery never opens a stream and never consults a default device. Every
/// returned device carries the backend's stable host-qualified identifier.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemInputDiscovery;

impl SystemInputDiscovery {
    /// Constructs the stateless system discovery adapter.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl InputDeviceDiscovery for SystemInputDiscovery {
    fn enumerate(&self) -> Result<Vec<InputDeviceDescriptor>, InputDiscoveryError> {
        let host = cpal::default_host();
        let platform = platform_for_host(host.id())?;
        let devices = host.devices().map_err(map_discovery_error)?;
        let mut descriptors = Vec::new();

        for device in devices {
            match descriptor_for_device(&device, platform) {
                Ok(Some(descriptor)) => descriptors.push(descriptor),
                Ok(None) => {}
                Err(error) => return Err(error),
            }
        }

        finish_descriptors(descriptors)
    }

    fn find(
        &self,
        identity: &InputDeviceIdentity,
    ) -> Result<InputDeviceDescriptor, InputDiscoveryError> {
        let host = cpal::default_host();
        let platform = platform_for_host(host.id())?;
        if identity.platform() != platform {
            return Err(InputDiscoveryError::DeviceDisappeared);
        }
        let device_id = DeviceId::from_str(identity.opaque_id())
            .map_err(|_| InputDiscoveryError::IdentityUnavailable)?;
        let device = host
            .device_by_id(&device_id)
            .ok_or(InputDiscoveryError::DeviceDisappeared)?;
        descriptor_for_device(&device, platform)?
            .ok_or(InputDiscoveryError::UnsupportedConfiguration)
    }
}

fn descriptor_for_device(
    device: &cpal::Device,
    platform: InputPlatform,
) -> Result<Option<InputDeviceDescriptor>, InputDiscoveryError> {
    let supported = match device.supported_input_configs() {
        Ok(configurations) => configurations,
        Err(error) if error.kind() == ErrorKind::UnsupportedOperation => return Ok(None),
        Err(error) => return Err(map_discovery_error(error)),
    };
    let mut ranges = supported
        .map(configuration_range)
        .collect::<Result<Vec<_>, _>>()?;
    ranges.sort_unstable();
    ranges.dedup();
    if ranges.is_empty() {
        return Ok(None);
    }

    let id = device
        .id()
        .map_err(|_| InputDiscoveryError::IdentityUnavailable)?;
    let description = device.description().map_err(map_discovery_error)?;
    let identity = owned_identity(platform, &id.to_string())?;
    let display = InputDeviceDisplay::new(
        description.name(),
        description.manufacturer().map(str::to_owned),
    )
    .map_err(|_| InputDiscoveryError::BackendFailure)?;
    InputDeviceDescriptor::new(identity, display, ranges)
        .map(Some)
        .map_err(|_| InputDiscoveryError::UnsupportedConfiguration)
}

fn finish_descriptors(
    mut descriptors: Vec<InputDeviceDescriptor>,
) -> Result<Vec<InputDeviceDescriptor>, InputDiscoveryError> {
    if descriptors.is_empty() {
        return Err(InputDiscoveryError::NoInputDevices);
    }
    descriptors.sort_by(|left, right| left.identity().cmp(right.identity()));
    Ok(descriptors)
}

fn owned_identity(
    platform: InputPlatform,
    value: &str,
) -> Result<InputDeviceIdentity, InputDiscoveryError> {
    InputDeviceIdentity::new(platform, value).map_err(|_| InputDiscoveryError::IdentityUnavailable)
}

fn configuration_range(
    range: SupportedStreamConfigRange,
) -> Result<InputConfigurationRange, InputDiscoveryError> {
    let sample_format = owned_sample_format(range.sample_format())
        .ok_or(InputDiscoveryError::UnsupportedConfiguration)?;
    InputConfigurationRange::new(
        range.min_sample_rate(),
        range.max_sample_rate(),
        range.channels(),
        sample_format,
    )
    .map_err(|_| InputDiscoveryError::UnsupportedConfiguration)
}

fn owned_sample_format(format: SampleFormat) -> Option<InputSampleFormat> {
    match format {
        SampleFormat::I8 => Some(InputSampleFormat::Signed8),
        SampleFormat::I16 => Some(InputSampleFormat::Signed16),
        SampleFormat::I24 => Some(InputSampleFormat::Signed24),
        SampleFormat::I32 => Some(InputSampleFormat::Signed32),
        SampleFormat::I64 => Some(InputSampleFormat::Signed64),
        SampleFormat::U8 => Some(InputSampleFormat::Unsigned8),
        SampleFormat::U16 => Some(InputSampleFormat::Unsigned16),
        SampleFormat::U24 => Some(InputSampleFormat::Unsigned24),
        SampleFormat::U32 => Some(InputSampleFormat::Unsigned32),
        SampleFormat::U64 => Some(InputSampleFormat::Unsigned64),
        SampleFormat::F32 => Some(InputSampleFormat::Float32),
        SampleFormat::F64 => Some(InputSampleFormat::Float64),
        SampleFormat::DsdU8 | SampleFormat::DsdU16 | SampleFormat::DsdU32 => None,
        _ => None,
    }
}

fn platform_for_host(host: cpal::HostId) -> Result<InputPlatform, InputDiscoveryError> {
    match host.to_string().as_str() {
        "coreaudio" => Ok(InputPlatform::MacOsCoreAudio),
        "wasapi" => Ok(InputPlatform::WindowsWasapi),
        "alsa" => Ok(InputPlatform::LinuxAlsa),
        "jack" => Ok(InputPlatform::LinuxJack),
        _ => Err(InputDiscoveryError::IdentityUnavailable),
    }
}

fn map_discovery_error(error: Error) -> InputDiscoveryError {
    match error.kind() {
        ErrorKind::HostUnavailable => InputDiscoveryError::HostUnavailable,
        ErrorKind::PermissionDenied => InputDiscoveryError::PermissionDenied,
        ErrorKind::DeviceNotAvailable | ErrorKind::DeviceChanged | ErrorKind::StreamInvalidated => {
            InputDiscoveryError::DeviceDisappeared
        }
        ErrorKind::UnsupportedConfig
        | ErrorKind::UnsupportedOperation
        | ErrorKind::InvalidInput => InputDiscoveryError::UnsupportedConfiguration,
        ErrorKind::DeviceBusy
        | ErrorKind::RealtimeDenied
        | ErrorKind::ResourceExhausted
        | ErrorKind::Xrun
        | ErrorKind::BackendError
        | ErrorKind::Other => InputDiscoveryError::BackendFailure,
        _ => InputDiscoveryError::BackendFailure,
    }
}

#[cfg(test)]
mod tests {
    use cpal::{DeviceDescriptionBuilder, SupportedBufferSize};

    use super::*;

    #[test]
    fn every_pcm_format_maps_without_leaking_cpal_types() {
        let cases = [
            (SampleFormat::I8, InputSampleFormat::Signed8),
            (SampleFormat::I16, InputSampleFormat::Signed16),
            (SampleFormat::I24, InputSampleFormat::Signed24),
            (SampleFormat::I32, InputSampleFormat::Signed32),
            (SampleFormat::I64, InputSampleFormat::Signed64),
            (SampleFormat::U8, InputSampleFormat::Unsigned8),
            (SampleFormat::U16, InputSampleFormat::Unsigned16),
            (SampleFormat::U24, InputSampleFormat::Unsigned24),
            (SampleFormat::U32, InputSampleFormat::Unsigned32),
            (SampleFormat::U64, InputSampleFormat::Unsigned64),
            (SampleFormat::F32, InputSampleFormat::Float32),
            (SampleFormat::F64, InputSampleFormat::Float64),
        ];
        for (cpal_format, owned_format) in cases {
            assert_eq!(owned_sample_format(cpal_format), Some(owned_format));
        }
        assert_eq!(owned_sample_format(SampleFormat::DsdU8), None);
    }

    #[test]
    fn supported_configuration_ranges_map_with_checked_units() {
        let cpal_range = SupportedStreamConfigRange::new(
            2,
            44_100,
            48_000,
            SupportedBufferSize::Unknown,
            SampleFormat::F32,
        );
        let owned = configuration_range(cpal_range).unwrap();
        assert_eq!(owned.min_sample_rate_hz(), 44_100);
        assert_eq!(owned.max_sample_rate_hz(), 48_000);
        assert_eq!(owned.channels(), 2);
        assert_eq!(owned.sample_format(), InputSampleFormat::Float32);
        assert_eq!(owned.select(48_000, 1).unwrap().selected_channel(), 1);
        let dsd_range = SupportedStreamConfigRange::new(
            1,
            48_000,
            48_000,
            SupportedBufferSize::Unknown,
            SampleFormat::DsdU8,
        );
        assert_eq!(
            configuration_range(dsd_range),
            Err(InputDiscoveryError::UnsupportedConfiguration)
        );
    }

    #[test]
    fn discovery_errors_remain_typed_and_stable() {
        let cases = [
            (
                ErrorKind::HostUnavailable,
                InputDiscoveryError::HostUnavailable,
            ),
            (
                ErrorKind::PermissionDenied,
                InputDiscoveryError::PermissionDenied,
            ),
            (
                ErrorKind::DeviceNotAvailable,
                InputDiscoveryError::DeviceDisappeared,
            ),
            (
                ErrorKind::UnsupportedConfig,
                InputDiscoveryError::UnsupportedConfiguration,
            ),
            (ErrorKind::BackendError, InputDiscoveryError::BackendFailure),
        ];
        for (kind, expected) in cases {
            assert_eq!(map_discovery_error(Error::new(kind)), expected);
        }
    }

    #[test]
    fn duplicate_display_names_do_not_collapse_stable_identities() {
        let display = InputDeviceDisplay::new("USB Audio", Some("Example".into())).unwrap();
        let range =
            InputConfigurationRange::new(48_000, 48_000, 2, InputSampleFormat::Signed16).unwrap();
        let first = InputDeviceDescriptor::new(
            InputDeviceIdentity::new(InputPlatform::MacOsCoreAudio, "coreaudio:uid-1").unwrap(),
            display.clone(),
            vec![range],
        )
        .unwrap();
        let second = InputDeviceDescriptor::new(
            InputDeviceIdentity::new(InputPlatform::MacOsCoreAudio, "coreaudio:uid-2").unwrap(),
            display,
            vec![range],
        )
        .unwrap();
        assert_eq!(first.display().name(), second.display().name());
        assert_ne!(first.identity(), second.identity());
    }

    #[test]
    fn empty_discovery_and_missing_identity_fail_explicitly() {
        assert_eq!(
            finish_descriptors(Vec::new()),
            Err(InputDiscoveryError::NoInputDevices)
        );
        assert_eq!(
            owned_identity(InputPlatform::MacOsCoreAudio, ""),
            Err(InputDiscoveryError::IdentityUnavailable)
        );
    }

    #[test]
    fn cpal_description_fields_are_available_without_opening_a_stream() {
        let description = DeviceDescriptionBuilder::new("Input")
            .manufacturer("Example")
            .build();
        assert_eq!(description.name(), "Input");
        assert_eq!(description.manufacturer(), Some("Example"));
    }
}
