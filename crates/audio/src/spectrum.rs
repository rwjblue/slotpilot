//! Bounded receive-only spectrum and waterfall model.

use std::{collections::VecDeque, sync::Arc};

use rustfft::{Fft, FftPlanner, num_complex::Complex, num_traits::Zero};
use thiserror::Error;

use crate::{
    FT8_RECEIVE_SAMPLE_RATE_HZ, FT8_RECEIVE_SLOT_MILLIS, FT8_RECEIVE_WINDOW_SAMPLES,
    Ft8ReceiveSlot, Ft8ReceiveWindow, ProcessGeneration, StreamGeneration,
};

/// Smallest supported FFT length.
pub const MIN_SPECTRUM_FFT_SIZE: usize = 256;
/// Largest supported FFT length.
pub const MAX_SPECTRUM_FFT_SIZE: usize = 4_096;
/// Default FFT length.
pub const DEFAULT_SPECTRUM_FFT_SIZE: usize = 1_024;
/// Default 50 percent overlap in canonical samples.
pub const DEFAULT_SPECTRUM_OVERLAP_SAMPLES: usize = 512;
/// Default upper passband edge.
pub const DEFAULT_SPECTRUM_MAX_FREQUENCY_HZ: u32 = 5_000;
/// Default retained waterfall capacity.
pub const DEFAULT_WATERFALL_ROWS: usize = 60;
/// Maximum retained waterfall capacity.
pub const MAX_WATERFALL_ROWS: usize = 120;
/// Maximum number of positive-frequency bins in one row.
pub const MAX_SPECTRUM_BINS: usize = MAX_SPECTRUM_FFT_SIZE / 2 + 1;
/// Maximum canonical samples accepted by one worker-side push.
pub const MAX_SPECTRUM_CHUNK_SAMPLES: usize = 4_096;
/// Maximum FFT rows computed by one worker-side push.
pub const MAX_SPECTRUM_ROWS_PER_PUSH: usize = 64;
/// Default minimum interval between publication tokens.
pub const DEFAULT_SPECTRUM_PUBLICATION_MILLIS: u32 = 250;

const MIN_PUBLICATION_MILLIS: u32 = 50;
const MAX_PUBLICATION_MILLIS: u32 = 2_000;
const MAX_MODEL_BYTES: usize = 2 * 1_024 * 1_024;
const MAX_FFT_SCRATCH_COMPLEX: usize = MAX_SPECTRUM_FFT_SIZE * 4;
const MAGNITUDE_FLOOR_MILLIDECIBELS: i32 = -120_000;
const MILLIDECIBELS_PER_DECIBEL: f32 = 1_000.0;
const PCM_FULL_SCALE: f32 = 32_768.0;

/// Window applied before each FFT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpectrumWindow {
    /// Periodic Hann window, `0.5 - 0.5 cos(2πn/N)`.
    Hann,
}

/// Exact center frequency of one spectrum bin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpectrumFrequency(u32);

impl SpectrumFrequency {
    /// Returns center frequency in integer millihertz.
    #[must_use]
    pub const fn millihertz(self) -> u32 {
        self.0
    }
}

/// Full-scale-relative magnitude with a documented -120 dBFS floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpectrumMagnitude(i32);

impl SpectrumMagnitude {
    /// Returns magnitude in integer millidecibels relative to full scale.
    #[must_use]
    pub const fn millidecibels_full_scale(self) -> i32 {
        self.0
    }
}

/// One owned positive-frequency spectrum bin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpectrumBin {
    /// Zero-based FFT bin index.
    pub index: u16,
    /// Exact rounded bin-center frequency.
    pub frequency: SpectrumFrequency,
    /// Coherent-gain-corrected peak magnitude.
    pub magnitude: SpectrumMagnitude,
}

/// Reset evidence attached to the first row after invalidation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpectrumDiscontinuity {
    /// Daemon process generation changed.
    ProcessGenerationChanged,
    /// Input stream/device generation changed.
    StreamGenerationChanged,
    /// Canonical sample position was non-contiguous.
    TimelineDiscontinuity,
    /// Independent clock health became unusable.
    ClockHealthLost,
}

/// Checked bounded spectrum/waterfall policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpectrumConfig {
    fft_size: usize,
    overlap_samples: usize,
    first_bin: usize,
    last_bin: usize,
    waterfall_rows: usize,
    publication_millis: u32,
    estimated_model_bytes: usize,
}

impl SpectrumConfig {
    /// Constructs a checked receive spectrum policy.
    pub fn new(
        fft_size: usize,
        overlap_samples: usize,
        min_frequency_hz: u32,
        max_frequency_hz: u32,
        waterfall_rows: usize,
        publication_millis: u32,
    ) -> Result<Self, SpectrumModelError> {
        if !(MIN_SPECTRUM_FFT_SIZE..=MAX_SPECTRUM_FFT_SIZE).contains(&fft_size)
            || !fft_size.is_power_of_two()
        {
            return Err(SpectrumModelError::InvalidFftSize);
        }
        if overlap_samples >= fft_size || overlap_samples > fft_size * 3 / 4 {
            return Err(SpectrumModelError::InvalidOverlap);
        }
        if min_frequency_hz >= max_frequency_hz || max_frequency_hz > FT8_RECEIVE_SAMPLE_RATE_HZ / 2
        {
            return Err(SpectrumModelError::InvalidPassband);
        }
        if !(1..=MAX_WATERFALL_ROWS).contains(&waterfall_rows) {
            return Err(SpectrumModelError::InvalidRowCapacity);
        }
        if !(MIN_PUBLICATION_MILLIS..=MAX_PUBLICATION_MILLIS).contains(&publication_millis) {
            return Err(SpectrumModelError::InvalidPublicationCadence);
        }

        let first_bin = usize::try_from(min_frequency_hz)
            .map_err(|_| SpectrumModelError::ArithmeticOverflow)?
            .checked_mul(fft_size)
            .ok_or(SpectrumModelError::ArithmeticOverflow)?
            .div_ceil(FT8_RECEIVE_SAMPLE_RATE_HZ as usize);
        let last_bin = usize::try_from(max_frequency_hz)
            .map_err(|_| SpectrumModelError::ArithmeticOverflow)?
            .checked_mul(fft_size)
            .ok_or(SpectrumModelError::ArithmeticOverflow)?
            / FT8_RECEIVE_SAMPLE_RATE_HZ as usize;
        let bin_count = last_bin
            .checked_sub(first_bin)
            .and_then(|value| value.checked_add(1))
            .ok_or(SpectrumModelError::InvalidPassband)?;
        if bin_count == 0 || bin_count > MAX_SPECTRUM_BINS {
            return Err(SpectrumModelError::InvalidPassband);
        }

        let hop = fft_size - overlap_samples;
        if MAX_SPECTRUM_CHUNK_SAMPLES.div_ceil(hop) > MAX_SPECTRUM_ROWS_PER_PUSH {
            return Err(SpectrumModelError::CpuBoundExceeded);
        }
        let estimated_model_bytes = waterfall_rows
            .checked_mul(
                bin_count
                    .checked_mul(16)
                    .and_then(|value| value.checked_add(128))
                    .ok_or(SpectrumModelError::ArithmeticOverflow)?,
            )
            .and_then(|value| value.checked_add(fft_size.checked_mul(64)?))
            .ok_or(SpectrumModelError::ArithmeticOverflow)?;
        if estimated_model_bytes > MAX_MODEL_BYTES {
            return Err(SpectrumModelError::MemoryBoundExceeded);
        }

        Ok(Self {
            fft_size,
            overlap_samples,
            first_bin,
            last_bin,
            waterfall_rows,
            publication_millis,
            estimated_model_bytes,
        })
    }

    /// Returns the FFT length in canonical samples.
    #[must_use]
    pub const fn fft_size(self) -> usize {
        self.fft_size
    }

    /// Returns overlap between adjacent rows in canonical samples.
    #[must_use]
    pub const fn overlap_samples(self) -> usize {
        self.overlap_samples
    }

    /// Returns the row-to-row hop in canonical samples.
    #[must_use]
    pub const fn hop_samples(self) -> usize {
        self.fft_size - self.overlap_samples
    }

    /// Returns the number of retained positive-frequency bins.
    #[must_use]
    pub const fn bin_count(self) -> usize {
        self.last_bin - self.first_bin + 1
    }

    /// Returns retained waterfall capacity.
    #[must_use]
    pub const fn waterfall_rows(self) -> usize {
        self.waterfall_rows
    }

    /// Returns minimum publication-token cadence in milliseconds.
    #[must_use]
    pub const fn publication_millis(self) -> u32 {
        self.publication_millis
    }

    /// Returns the conservative checked allocation estimate.
    #[must_use]
    pub const fn estimated_model_bytes(self) -> usize {
        self.estimated_model_bytes
    }
}

impl Default for SpectrumConfig {
    fn default() -> Self {
        Self::new(
            DEFAULT_SPECTRUM_FFT_SIZE,
            DEFAULT_SPECTRUM_OVERLAP_SAMPLES,
            0,
            DEFAULT_SPECTRUM_MAX_FREQUENCY_HZ,
            DEFAULT_WATERFALL_ROWS,
            DEFAULT_SPECTRUM_PUBLICATION_MILLIS,
        )
        .unwrap_or(Self {
            fft_size: DEFAULT_SPECTRUM_FFT_SIZE,
            overlap_samples: DEFAULT_SPECTRUM_OVERLAP_SAMPLES,
            first_bin: 0,
            last_bin: 426,
            waterfall_rows: DEFAULT_WATERFALL_ROWS,
            publication_millis: DEFAULT_SPECTRUM_PUBLICATION_MILLIS,
            estimated_model_bytes: 520_192,
        })
    }
}

/// One bounded owned canonical sample chunk for visualization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalSpectrumChunk {
    /// Daemon-process generation.
    pub process_generation: ProcessGeneration,
    /// Input-stream/device generation.
    pub stream_generation: StreamGeneration,
    /// Exact FT8 slot containing every sample.
    pub slot: Ft8ReceiveSlot,
    /// Zero-based canonical sample offset within the slot.
    pub start_sample_offset: u32,
    /// Explicit upstream invalidation, if present.
    pub discontinuity: Option<SpectrumDiscontinuity>,
    samples: Vec<i16>,
}

impl CanonicalSpectrumChunk {
    /// Constructs one bounded 12 kHz mono worker-side chunk.
    pub fn new(
        process_generation: ProcessGeneration,
        stream_generation: StreamGeneration,
        slot: Ft8ReceiveSlot,
        start_sample_offset: u32,
        discontinuity: Option<SpectrumDiscontinuity>,
        samples: Vec<i16>,
    ) -> Result<Self, SpectrumModelError> {
        if samples.is_empty() {
            return Err(SpectrumModelError::EmptyChunk);
        }
        if samples.len() > MAX_SPECTRUM_CHUNK_SAMPLES {
            return Err(SpectrumModelError::ChunkTooLarge);
        }
        let end = usize::try_from(start_sample_offset)
            .map_err(|_| SpectrumModelError::ArithmeticOverflow)?
            .checked_add(samples.len())
            .ok_or(SpectrumModelError::ArithmeticOverflow)?;
        if end > FT8_RECEIVE_WINDOW_SAMPLES {
            return Err(SpectrumModelError::ChunkOutsideSlot);
        }
        Ok(Self {
            process_generation,
            stream_generation,
            slot,
            start_sample_offset,
            discontinuity,
            samples,
        })
    }

    /// Copies a bounded range from one owned canonical FT8 window.
    pub fn from_window(
        window: &Ft8ReceiveWindow,
        start_sample_offset: u32,
        sample_count: u32,
        discontinuity: Option<SpectrumDiscontinuity>,
    ) -> Result<Self, SpectrumModelError> {
        let start = usize::try_from(start_sample_offset)
            .map_err(|_| SpectrumModelError::ArithmeticOverflow)?;
        let count =
            usize::try_from(sample_count).map_err(|_| SpectrumModelError::ArithmeticOverflow)?;
        let end = start
            .checked_add(count)
            .ok_or(SpectrumModelError::ArithmeticOverflow)?;
        let samples = window
            .samples()
            .get(start..end)
            .ok_or(SpectrumModelError::ChunkOutsideSlot)?
            .to_vec();
        Self::new(
            window.process_generation,
            window.stream_generation,
            window.slot(),
            start_sample_offset,
            discontinuity,
            samples,
        )
    }

    /// Returns canonical mono signed-16-bit samples.
    #[must_use]
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }
}

/// One deterministic bounded waterfall row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpectrumRow {
    /// Monotonic model-local row sequence.
    pub sequence: u64,
    /// Daemon-process generation.
    pub process_generation: ProcessGeneration,
    /// Input-stream/device generation.
    pub stream_generation: StreamGeneration,
    /// Exact containing FT8 slot.
    pub slot: Ft8ReceiveSlot,
    /// First canonical sample represented by the FFT.
    pub start_sample_offset: u32,
    /// Exact row start in integer UTC microseconds.
    pub start_utc_unix_micros: i64,
    /// Window applied before the FFT.
    pub window: SpectrumWindow,
    /// Explicit reset evidence on the first row after invalidation.
    pub discontinuity: Option<SpectrumDiscontinuity>,
    /// Ordered owned passband bins.
    pub bins: Vec<SpectrumBin>,
}

/// Latest bounded producer status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpectrumPushResult {
    /// Rows computed by this push.
    pub produced_rows: u16,
    /// Rows currently retained by the model.
    pub retained_rows: u16,
    /// Whether a consumer snapshot token is pending.
    pub publication_pending: bool,
    /// Total due publications coalesced behind a slow/absent consumer.
    pub coalesced_publications: u64,
    /// Total oldest rows evicted by the fixed-capacity model.
    pub evicted_rows: u64,
}

/// Consumer-side copy of the latest bounded waterfall state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterfallSnapshot {
    /// Configuration that produced every row.
    pub config: SpectrumConfig,
    /// Oldest-to-newest retained rows.
    pub rows: Vec<SpectrumRow>,
    /// Total due publications coalesced before this snapshot.
    pub coalesced_publications: u64,
    /// Total fixed-capacity row evictions before this snapshot.
    pub evicted_rows: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkCursor {
    process_generation: ProcessGeneration,
    stream_generation: StreamGeneration,
    slot: Ft8ReceiveSlot,
    next_sample_offset: u32,
}

/// Pure worker-side bounded spectrum and waterfall producer.
///
/// Construction allocates the FFT plan and reusable work buffers. `push`
/// performs bounded worker-side DSP and never opens a device, blocks on a
/// client, decodes, persists, renders, schedules, controls a rig, or transmits.
pub struct SpectrumModel {
    config: SpectrumConfig,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    window_sum: f32,
    fft_buffer: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    pending_samples: VecDeque<i16>,
    pending_start_offset: u32,
    cursor: Option<ChunkCursor>,
    rows: VecDeque<SpectrumRow>,
    pending_discontinuity: Option<SpectrumDiscontinuity>,
    next_sequence: u64,
    last_publication_utc_micros: Option<i64>,
    publication_pending: bool,
    coalesced_publications: u64,
    evicted_rows: u64,
}

impl SpectrumModel {
    /// Allocates one bounded model and private reviewed FFT plan.
    pub fn new(config: SpectrumConfig) -> Result<Self, SpectrumModelError> {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(config.fft_size);
        let scratch_len = fft.get_inplace_scratch_len();
        if scratch_len > MAX_FFT_SCRATCH_COMPLEX {
            return Err(SpectrumModelError::DependencyScratchExceeded);
        }
        let window = hann_window(config.fft_size)?;
        let window_sum = window.iter().sum();
        Ok(Self {
            config,
            fft,
            window,
            window_sum,
            fft_buffer: vec![Complex::zero(); config.fft_size],
            scratch: vec![Complex::zero(); scratch_len],
            pending_samples: VecDeque::with_capacity(
                config
                    .fft_size
                    .checked_add(MAX_SPECTRUM_CHUNK_SAMPLES)
                    .ok_or(SpectrumModelError::ArithmeticOverflow)?,
            ),
            pending_start_offset: 0,
            cursor: None,
            rows: VecDeque::with_capacity(config.waterfall_rows),
            pending_discontinuity: None,
            next_sequence: 0,
            last_publication_utc_micros: None,
            publication_pending: false,
            coalesced_publications: 0,
            evicted_rows: 0,
        })
    }

    /// Returns the exact checked model policy.
    #[must_use]
    pub const fn config(&self) -> SpectrumConfig {
        self.config
    }

    /// Invalidates overlap/history and preserves the reason for the next row.
    pub fn reset(&mut self, reason: SpectrumDiscontinuity) {
        self.pending_samples.clear();
        self.cursor = None;
        self.rows.clear();
        self.pending_discontinuity = Some(reason);
        self.last_publication_utc_micros = None;
        self.publication_pending = false;
    }

    /// Processes one bounded canonical chunk outside the real-time callback.
    pub fn push(
        &mut self,
        chunk: CanonicalSpectrumChunk,
    ) -> Result<SpectrumPushResult, SpectrumModelError> {
        self.prepare_chunk(&chunk)?;
        if self.pending_samples.is_empty() {
            self.pending_start_offset = chunk.start_sample_offset;
        }
        let chunk_sample_count = chunk.samples.len();
        self.pending_samples.extend(chunk.samples);

        let mut produced = 0_usize;
        while self.pending_samples.len() >= self.config.fft_size {
            if produced >= MAX_SPECTRUM_ROWS_PER_PUSH {
                return Err(SpectrumModelError::CpuBoundExceeded);
            }
            self.produce_row(
                chunk.process_generation,
                chunk.stream_generation,
                chunk.slot,
            )?;
            produced += 1;
        }

        let end = usize::try_from(chunk.start_sample_offset)
            .map_err(|_| SpectrumModelError::ArithmeticOverflow)?
            .checked_add(chunk_sample_count)
            .ok_or(SpectrumModelError::ArithmeticOverflow)?;
        self.cursor = Some(ChunkCursor {
            process_generation: chunk.process_generation,
            stream_generation: chunk.stream_generation,
            slot: chunk.slot,
            next_sample_offset: u32::try_from(end)
                .map_err(|_| SpectrumModelError::ArithmeticOverflow)?,
        });
        Ok(SpectrumPushResult {
            produced_rows: u16::try_from(produced)
                .map_err(|_| SpectrumModelError::ArithmeticOverflow)?,
            retained_rows: u16::try_from(self.rows.len())
                .map_err(|_| SpectrumModelError::ArithmeticOverflow)?,
            publication_pending: self.publication_pending,
            coalesced_publications: self.coalesced_publications,
            evicted_rows: self.evicted_rows,
        })
    }

    /// Takes the latest bounded snapshot only when a publication token is due.
    ///
    /// A slow or absent consumer causes tokens to coalesce; producer-side work
    /// never waits for this method to be called.
    pub fn take_snapshot(&mut self) -> Option<WaterfallSnapshot> {
        if !self.publication_pending {
            return None;
        }
        self.publication_pending = false;
        Some(WaterfallSnapshot {
            config: self.config,
            rows: self.rows.iter().cloned().collect(),
            coalesced_publications: self.coalesced_publications,
            evicted_rows: self.evicted_rows,
        })
    }

    fn prepare_chunk(&mut self, chunk: &CanonicalSpectrumChunk) -> Result<(), SpectrumModelError> {
        if let Some(reason) = chunk.discontinuity {
            self.reset(reason);
        }
        let Some(cursor) = self.cursor else {
            return Ok(());
        };
        if cursor.process_generation != chunk.process_generation {
            self.reset(SpectrumDiscontinuity::ProcessGenerationChanged);
            return Ok(());
        }
        if cursor.stream_generation != chunk.stream_generation {
            self.reset(SpectrumDiscontinuity::StreamGenerationChanged);
            return Ok(());
        }
        if cursor.slot == chunk.slot && cursor.next_sample_offset == chunk.start_sample_offset {
            return Ok(());
        }
        let next_slot = cursor
            .slot
            .start_utc_unix_millis()
            .checked_add(FT8_RECEIVE_SLOT_MILLIS)
            .ok_or(SpectrumModelError::ArithmeticOverflow)?;
        if cursor.next_sample_offset as usize == FT8_RECEIVE_WINDOW_SAMPLES
            && chunk.start_sample_offset == 0
            && chunk.slot.start_utc_unix_millis() == next_slot
        {
            self.pending_samples.clear();
            self.pending_start_offset = 0;
            return Ok(());
        }
        self.reset(SpectrumDiscontinuity::TimelineDiscontinuity);
        Ok(())
    }

    fn produce_row(
        &mut self,
        process_generation: ProcessGeneration,
        stream_generation: StreamGeneration,
        slot: Ft8ReceiveSlot,
    ) -> Result<(), SpectrumModelError> {
        for (index, (sample, weight)) in self
            .pending_samples
            .iter()
            .take(self.config.fft_size)
            .zip(self.window.iter())
            .enumerate()
        {
            self.fft_buffer[index] =
                Complex::new(f32::from(*sample) / PCM_FULL_SCALE * weight, 0.0);
        }
        self.fft
            .process_with_scratch(&mut self.fft_buffer, &mut self.scratch);

        let mut bins = Vec::with_capacity(self.config.bin_count());
        for index in self.config.first_bin..=self.config.last_bin {
            let edge_bin = index == 0 || index == self.config.fft_size / 2;
            let scale = if edge_bin { 1.0 } else { 2.0 };
            let amplitude = self.fft_buffer[index].norm() * scale / self.window_sum;
            let millidecibels = if amplitude <= 0.0 {
                MAGNITUDE_FLOOR_MILLIDECIBELS
            } else {
                (20.0 * amplitude.log10() * MILLIDECIBELS_PER_DECIBEL)
                    .round()
                    .clamp(MAGNITUDE_FLOOR_MILLIDECIBELS as f32, i32::MAX as f32)
                    as i32
            };
            bins.push(SpectrumBin {
                index: u16::try_from(index).map_err(|_| SpectrumModelError::ArithmeticOverflow)?,
                frequency: SpectrumFrequency(bin_frequency_millihertz(
                    index,
                    self.config.fft_size,
                )?),
                magnitude: SpectrumMagnitude(millidecibels),
            });
        }

        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(SpectrumModelError::ArithmeticOverflow)?;
        let start_utc_unix_micros = row_start_utc_micros(slot, self.pending_start_offset)?;
        let row = SpectrumRow {
            sequence,
            process_generation,
            stream_generation,
            slot,
            start_sample_offset: self.pending_start_offset,
            start_utc_unix_micros,
            window: SpectrumWindow::Hann,
            discontinuity: self.pending_discontinuity.take(),
            bins,
        };
        if self.rows.len() == self.config.waterfall_rows {
            self.rows.pop_front();
            self.evicted_rows = self.evicted_rows.saturating_add(1);
        }
        self.rows.push_back(row);
        self.mark_publication(start_utc_unix_micros)?;

        let hop = self.config.hop_samples();
        self.pending_samples.drain(..hop);
        self.pending_start_offset = self
            .pending_start_offset
            .checked_add(u32::try_from(hop).map_err(|_| SpectrumModelError::ArithmeticOverflow)?)
            .ok_or(SpectrumModelError::ArithmeticOverflow)?;
        Ok(())
    }

    fn mark_publication(&mut self, row_utc_micros: i64) -> Result<(), SpectrumModelError> {
        let due = self.last_publication_utc_micros.is_none_or(|previous| {
            row_utc_micros.saturating_sub(previous)
                >= i64::from(self.config.publication_millis) * 1_000
        });
        if due {
            if self.publication_pending {
                self.coalesced_publications = self.coalesced_publications.saturating_add(1);
            }
            self.publication_pending = true;
            self.last_publication_utc_micros = Some(row_utc_micros);
        }
        Ok(())
    }
}

/// Checked model construction or processing failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SpectrumModelError {
    /// FFT length is outside the supported power-of-two range.
    #[error("spectrum FFT size is invalid")]
    InvalidFftSize,
    /// Overlap is not less than the FFT or exceeds 75 percent.
    #[error("spectrum overlap is invalid")]
    InvalidOverlap,
    /// Passband is empty or exceeds the canonical Nyquist frequency.
    #[error("spectrum passband is invalid")]
    InvalidPassband,
    /// Retained row capacity is zero or too large.
    #[error("waterfall row capacity is invalid")]
    InvalidRowCapacity,
    /// Publication cadence is outside the bounded range.
    #[error("spectrum publication cadence is invalid")]
    InvalidPublicationCadence,
    /// Configuration would exceed the per-push FFT work bound.
    #[error("spectrum CPU work bound would be exceeded")]
    CpuBoundExceeded,
    /// Configuration would exceed the conservative allocation bound.
    #[error("spectrum memory bound would be exceeded")]
    MemoryBoundExceeded,
    /// The reviewed FFT requested more scratch than the adapter permits.
    #[error("FFT scratch request exceeded the reviewed bound")]
    DependencyScratchExceeded,
    /// A worker-side chunk contained no samples.
    #[error("spectrum chunk is empty")]
    EmptyChunk,
    /// A worker-side chunk exceeded the fixed sample bound.
    #[error("spectrum chunk is too large")]
    ChunkTooLarge,
    /// A worker-side chunk extended outside its declared slot.
    #[error("spectrum chunk extends outside its FT8 slot")]
    ChunkOutsideSlot,
    /// Checked time, size, or sequence arithmetic overflowed.
    #[error("spectrum model arithmetic overflowed")]
    ArithmeticOverflow,
}

fn hann_window(size: usize) -> Result<Vec<f32>, SpectrumModelError> {
    let size_u32 = u32::try_from(size).map_err(|_| SpectrumModelError::ArithmeticOverflow)?;
    Ok((0..size_u32)
        .map(|index| {
            0.5 - 0.5 * (2.0 * std::f32::consts::PI * index as f32 / size_u32 as f32).cos()
        })
        .collect())
}

fn bin_frequency_millihertz(index: usize, fft_size: usize) -> Result<u32, SpectrumModelError> {
    let numerator = index
        .checked_mul(FT8_RECEIVE_SAMPLE_RATE_HZ as usize)
        .and_then(|value| value.checked_mul(1_000))
        .and_then(|value| value.checked_add(fft_size / 2))
        .ok_or(SpectrumModelError::ArithmeticOverflow)?;
    u32::try_from(numerator / fft_size).map_err(|_| SpectrumModelError::ArithmeticOverflow)
}

fn row_start_utc_micros(
    slot: Ft8ReceiveSlot,
    sample_offset: u32,
) -> Result<i64, SpectrumModelError> {
    let slot_micros = i128::from(slot.start_utc_unix_millis())
        .checked_mul(1_000)
        .ok_or(SpectrumModelError::ArithmeticOverflow)?;
    let sample_micros = i128::from(sample_offset)
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_div(i128::from(FT8_RECEIVE_SAMPLE_RATE_HZ)))
        .ok_or(SpectrumModelError::ArithmeticOverflow)?;
    i64::try_from(slot_micros + sample_micros).map_err(|_| SpectrumModelError::ArithmeticOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process_generation(value: u64) -> ProcessGeneration {
        ProcessGeneration::new(value).unwrap()
    }

    fn stream_generation(value: u64) -> StreamGeneration {
        StreamGeneration::new(value).unwrap()
    }

    fn slot(value: i64) -> Ft8ReceiveSlot {
        Ft8ReceiveSlot::new(value).unwrap()
    }

    fn samples_for_tones(tones: &[(f32, f32)], count: usize, start: usize) -> Vec<i16> {
        (start..start + count)
            .map(|sample| {
                let time = sample as f32 / FT8_RECEIVE_SAMPLE_RATE_HZ as f32;
                let value = tones
                    .iter()
                    .map(|(frequency, amplitude)| {
                        amplitude * (2.0 * std::f32::consts::PI * frequency * time).sin()
                    })
                    .sum::<f32>()
                    .clamp(-1.0, 1.0);
                (value * f32::from(i16::MAX)).round() as i16
            })
            .collect()
    }

    fn chunk(
        stream: u64,
        offset: u32,
        discontinuity: Option<SpectrumDiscontinuity>,
        samples: Vec<i16>,
    ) -> CanonicalSpectrumChunk {
        CanonicalSpectrumChunk::new(
            process_generation(1),
            stream_generation(stream),
            slot(30_000),
            offset,
            discontinuity,
            samples,
        )
        .unwrap()
    }

    fn bin_at(row: &SpectrumRow, frequency_hz: u32) -> SpectrumBin {
        row.bins
            .iter()
            .copied()
            .min_by_key(|bin| {
                bin.frequency
                    .millihertz()
                    .abs_diff(frequency_hz.saturating_mul(1_000))
            })
            .unwrap()
    }

    #[test]
    fn configuration_rejects_unbounded_or_ambiguous_shapes() {
        assert_eq!(
            SpectrumConfig::new(1_000, 500, 0, 5_000, 60, 250),
            Err(SpectrumModelError::InvalidFftSize)
        );
        let maximum_overlap = SpectrumConfig::new(256, 192, 0, 5_000, 60, 250).unwrap();
        assert_eq!(
            MAX_SPECTRUM_CHUNK_SAMPLES.div_ceil(maximum_overlap.hop_samples()),
            MAX_SPECTRUM_ROWS_PER_PUSH
        );
        assert_eq!(
            SpectrumConfig::new(1_024, 1_024, 0, 5_000, 60, 250),
            Err(SpectrumModelError::InvalidOverlap)
        );
        assert_eq!(
            SpectrumConfig::new(1_024, 512, 5_000, 6_001, 60, 250),
            Err(SpectrumModelError::InvalidPassband)
        );
        assert_eq!(
            SpectrumConfig::new(1_024, 512, 0, 5_000, 0, 250),
            Err(SpectrumModelError::InvalidRowCapacity)
        );
        assert_eq!(
            SpectrumConfig::new(1_024, 512, 0, 5_000, 60, 49),
            Err(SpectrumModelError::InvalidPublicationCadence)
        );
        assert_eq!(
            SpectrumConfig::new(4_096, 2_048, 0, 6_000, 120, 250),
            Err(SpectrumModelError::MemoryBoundExceeded)
        );
    }

    #[test]
    fn silence_is_exact_floor_with_explicit_units_and_cadence() {
        let config = SpectrumConfig::default();
        let mut model = SpectrumModel::new(config).unwrap();
        let result = model
            .push(chunk(1, 0, None, vec![0; MAX_SPECTRUM_CHUNK_SAMPLES]))
            .unwrap();
        assert_eq!(result.produced_rows, 7);
        let snapshot = model.take_snapshot().unwrap();
        assert_eq!(snapshot.rows.len(), 7);
        assert_eq!(snapshot.rows[0].start_utc_unix_micros, 30_000_000);
        assert_eq!(snapshot.rows[1].start_sample_offset, 512);
        assert_eq!(snapshot.rows[1].start_utc_unix_micros, 30_042_666);
        assert_eq!(snapshot.rows[0].bins.len(), config.bin_count());
        assert!(snapshot.rows.iter().all(|row| row.bins.iter().all(|bin| {
            bin.magnitude.millidecibels_full_scale() == MAGNITUDE_FLOOR_MILLIDECIBELS
        })));
    }

    #[test]
    fn aligned_tone_has_deterministic_frequency_and_dbfs() {
        let mut model = SpectrumModel::new(SpectrumConfig::default()).unwrap();
        model
            .push(chunk(
                1,
                0,
                None,
                samples_for_tones(&[(1_500.0, 0.5)], MAX_SPECTRUM_CHUNK_SAMPLES, 0),
            ))
            .unwrap();
        let row = &model.take_snapshot().unwrap().rows[0];
        let tone = bin_at(row, 1_500);
        assert_eq!(tone.index, 128);
        assert_eq!(tone.frequency.millihertz(), 1_500_000);
        assert!(tone.magnitude.millidecibels_full_scale().abs_diff(-6_021) <= 2);
        assert!(
            row.bins
                .iter()
                .filter(|bin| bin.index.abs_diff(tone.index) > 1)
                .all(|bin| bin.magnitude.millidecibels_full_scale() < -70_000)
        );
    }

    #[test]
    fn multiple_tones_retain_independent_owned_peaks() {
        let mut model = SpectrumModel::new(SpectrumConfig::default()).unwrap();
        model
            .push(chunk(
                1,
                0,
                None,
                samples_for_tones(
                    &[(750.0, 0.25), (2_250.0, 0.5)],
                    MAX_SPECTRUM_CHUNK_SAMPLES,
                    0,
                ),
            ))
            .unwrap();
        let row = &model.take_snapshot().unwrap().rows[0];
        assert!(
            bin_at(row, 750)
                .magnitude
                .millidecibels_full_scale()
                .abs_diff(-12_041)
                <= 3
        );
        assert!(
            bin_at(row, 2_250)
                .magnitude
                .millidecibels_full_scale()
                .abs_diff(-6_021)
                <= 3
        );
    }

    #[test]
    fn fixed_capacity_and_single_pending_token_bound_slow_consumers() {
        let config = SpectrumConfig::new(1_024, 512, 0, 3_000, 2, 50).unwrap();
        let mut model = SpectrumModel::new(config).unwrap();
        let first = model
            .push(chunk(1, 0, None, vec![0; MAX_SPECTRUM_CHUNK_SAMPLES]))
            .unwrap();
        assert_eq!(first.retained_rows, 2);
        assert!(first.evicted_rows > 0);
        assert!(first.coalesced_publications > 0);
        let snapshot = model.take_snapshot().unwrap();
        assert_eq!(snapshot.rows.len(), 2);
        assert!(model.take_snapshot().is_none());
    }

    #[test]
    fn generation_timeline_and_clock_resets_remain_visible() {
        let mut model = SpectrumModel::new(SpectrumConfig::default()).unwrap();
        model.push(chunk(1, 0, None, vec![0; 1_024])).unwrap();
        model.push(chunk(2, 0, None, vec![0; 1_024])).unwrap();
        let snapshot = model.take_snapshot().unwrap();
        assert_eq!(snapshot.rows.len(), 1);
        assert_eq!(
            snapshot.rows[0].discontinuity,
            Some(SpectrumDiscontinuity::StreamGenerationChanged)
        );

        model.push(chunk(2, 4_000, None, vec![0; 1_024])).unwrap();
        assert_eq!(
            model.take_snapshot().unwrap().rows[0].discontinuity,
            Some(SpectrumDiscontinuity::TimelineDiscontinuity)
        );

        model.reset(SpectrumDiscontinuity::ClockHealthLost);
        model.push(chunk(2, 5_024, None, vec![0; 1_024])).unwrap();
        assert_eq!(
            model.take_snapshot().unwrap().rows[0].discontinuity,
            Some(SpectrumDiscontinuity::ClockHealthLost)
        );
    }

    #[test]
    fn one_push_cannot_exceed_the_documented_fft_work_bound() {
        let config = SpectrumConfig::new(256, 128, 0, 5_000, 1, 50).unwrap();
        let mut model = SpectrumModel::new(config).unwrap();
        let result = model
            .push(chunk(1, 0, None, vec![0; MAX_SPECTRUM_CHUNK_SAMPLES]))
            .unwrap();
        assert!(usize::from(result.produced_rows) <= MAX_SPECTRUM_ROWS_PER_PUSH);
    }
}
