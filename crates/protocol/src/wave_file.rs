//! Bounded in-memory RIFF/WAVE serialization and parsing.

use crate::{MAX_OFFLINE_PCM_SAMPLES, PcmBuffer, PcmFormat, PcmSampleFormat, PcmWaveError};

const HEADER_BYTES: usize = 12;
const CHUNK_HEADER_BYTES: usize = 8;

/// Encodes a checked PCM buffer as an in-memory RIFF/WAVE file.
///
/// This helper performs no file, device, playback, or permission operation.
pub fn encode_pcm_wave(buffer: &PcmBuffer) -> Result<Vec<u8>, PcmWaveError> {
    let sample_bytes = buffer
        .samples()
        .len()
        .checked_mul(size_of::<i16>())
        .and_then(|length| u32::try_from(length).ok())
        .ok_or(PcmWaveError::FileTooLarge)?;
    let riff_size = 36_u32
        .checked_add(sample_bytes)
        .ok_or(PcmWaveError::FileTooLarge)?;
    let format = buffer.format();
    let block_align = format
        .channels()
        .checked_mul(2)
        .ok_or(PcmWaveError::FileTooLarge)?;
    let byte_rate = format
        .sample_rate_hz()
        .checked_mul(u32::from(block_align))
        .ok_or(PcmWaveError::FileTooLarge)?;
    let capacity = usize::try_from(riff_size)
        .ok()
        .and_then(|size| size.checked_add(8))
        .ok_or(PcmWaveError::FileTooLarge)?;

    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&format.channels().to_le_bytes());
    bytes.extend_from_slice(&format.sample_rate_hz().to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&sample_bytes.to_le_bytes());
    for sample in buffer.samples() {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(bytes)
}

/// Parses bounded signed 16-bit PCM from an in-memory RIFF/WAVE file.
///
/// Unknown chunks are skipped according to their declared size. This helper
/// performs no file, device, capture, playback, or permission operation.
pub fn decode_pcm_wave(bytes: &[u8]) -> Result<PcmBuffer, PcmWaveError> {
    if bytes.len() < HEADER_BYTES {
        return Err(PcmWaveError::Truncated);
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(PcmWaveError::InvalidStructure);
    }
    let riff_size = usize::try_from(read_u32(bytes, 4)?)
        .map_err(|_| PcmWaveError::FileTooLarge)?
        .checked_add(8)
        .ok_or(PcmWaveError::FileTooLarge)?;
    if riff_size > bytes.len() {
        return Err(PcmWaveError::Truncated);
    }
    if riff_size < HEADER_BYTES {
        return Err(PcmWaveError::InvalidStructure);
    }

    let mut offset = HEADER_BYTES;
    let mut parsed_format = None;
    let mut data = None;
    while offset < riff_size {
        let header_end = offset
            .checked_add(CHUNK_HEADER_BYTES)
            .ok_or(PcmWaveError::InvalidStructure)?;
        if header_end > riff_size {
            return Err(PcmWaveError::Truncated);
        }
        let id = &bytes[offset..offset + 4];
        let chunk_size = usize::try_from(read_u32(bytes, offset + 4)?)
            .map_err(|_| PcmWaveError::FileTooLarge)?;
        let chunk_start = header_end;
        let chunk_end = chunk_start
            .checked_add(chunk_size)
            .ok_or(PcmWaveError::FileTooLarge)?;
        if chunk_end > riff_size {
            return Err(PcmWaveError::Truncated);
        }

        match id {
            b"fmt " => {
                if parsed_format.is_some() {
                    return Err(PcmWaveError::MissingOrDuplicateChunk);
                }
                parsed_format = Some(parse_format(&bytes[chunk_start..chunk_end])?);
            }
            b"data" => {
                if data.is_some() {
                    return Err(PcmWaveError::MissingOrDuplicateChunk);
                }
                data = Some(&bytes[chunk_start..chunk_end]);
            }
            _ => {}
        }

        offset = chunk_end
            .checked_add(chunk_size % 2)
            .ok_or(PcmWaveError::FileTooLarge)?;
        if offset > riff_size {
            return Err(PcmWaveError::Truncated);
        }
    }

    let format = parsed_format.ok_or(PcmWaveError::MissingOrDuplicateChunk)?;
    let data = data.ok_or(PcmWaveError::MissingOrDuplicateChunk)?;
    if data.len() % size_of::<i16>() != 0 || data.len() / size_of::<i16>() > MAX_OFFLINE_PCM_SAMPLES
    {
        return Err(PcmWaveError::FileTooLarge);
    }
    let samples = data
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
        .collect();
    PcmBuffer::new(format, samples).map_err(PcmWaveError::from)
}

fn parse_format(bytes: &[u8]) -> Result<PcmFormat, PcmWaveError> {
    if bytes.len() < 16 {
        return Err(PcmWaveError::Truncated);
    }
    let encoding = read_u16(bytes, 0)?;
    let channels = read_u16(bytes, 2)?;
    let sample_rate = read_u32(bytes, 4)?;
    let byte_rate = read_u32(bytes, 8)?;
    let block_align = read_u16(bytes, 12)?;
    let bits_per_sample = read_u16(bytes, 14)?;
    let expected_align = channels
        .checked_mul(2)
        .ok_or(PcmWaveError::UnsupportedFormat)?;
    let expected_rate = sample_rate
        .checked_mul(u32::from(expected_align))
        .ok_or(PcmWaveError::UnsupportedFormat)?;
    if encoding != 1
        || bits_per_sample != 16
        || block_align != expected_align
        || byte_rate != expected_rate
    {
        return Err(PcmWaveError::UnsupportedFormat);
    }
    PcmFormat::new(sample_rate, channels, PcmSampleFormat::Signed16LittleEndian)
        .map_err(PcmWaveError::from)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, PcmWaveError> {
    let end = offset.checked_add(2).ok_or(PcmWaveError::Truncated)?;
    let value = bytes.get(offset..end).ok_or(PcmWaveError::Truncated)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, PcmWaveError> {
    let end = offset.checked_add(4).ok_or(PcmWaveError::Truncated)?;
    let value = bytes.get(offset..end).ok_or(PcmWaveError::Truncated)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}
