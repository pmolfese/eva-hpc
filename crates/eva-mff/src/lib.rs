//! Read-only MFF metadata and bounded continuous-EEG sample windows.
//! No GUI, GPU, async runtime, or Python dependency. See the crate README.
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]
mod binary;
mod error;
mod metadata;
pub use error::{Error, Result};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    ops::Range,
    path::{Path, PathBuf},
};

/// Resource ceilings applied before allocating data controlled by input files.
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_channels: usize,
    pub max_blocks: usize,
    pub max_index_bytes: usize,
    pub max_xml_bytes: usize,
    pub max_window_bytes: usize,
    pub max_package_files: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_channels: 4096,
            max_blocks: 1_000_000,
            max_index_bytes: 128 << 20,
            max_xml_bytes: 32 << 20,
            max_window_bytes: 64 << 20,
            max_package_files: 4096,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingKind {
    Continuous,
    Discontinuous,
    SegmentedOrAveraged,
}
#[derive(Debug, Clone)]
pub struct Stream {
    pub file_name: String,
    /// Metadata declaration, e.g. EEG or PNSData.
    pub kind: String,
}
#[derive(Debug, Clone)]
pub struct Channel {
    /// Unit-sphere scalp position from coordinates.xml (type-0 electrodes only).
    pub position: Option<[f64; 3]>,
    /// Zero-based position in the binary stream, stable under display reordering.
    pub index: usize,
    pub label: String,
    /// Explicit metadata only; None means unspecified, not dimensionless.
    pub unit: Option<String>,
    /// GCAL multiplier; raw reads never apply this, calibrated reads apply it once.
    pub calibration: f32,
    /// Display convention only. Samples are never polarity-inverted by the reader.
    pub positive_up: Option<bool>,
}
#[derive(Debug, Clone)]
pub struct AcquisitionReference {
    pub sensor_number: usize,
    pub label: String,
    pub recorded_channel: Option<usize>,
}
#[derive(Debug, Clone)]
pub struct Event {
    pub code: String,
    pub label: Option<String>,
    pub raw_begin_time: String,
    pub source_file: String,
    pub onset_ns: i128,
    pub duration_ns: Option<i128>,
}
impl Event {
    /// Nearest sample, ties away from zero; negative pre-recording events stay negative.
    pub fn sample_index(&self, sample_rate: u32) -> Result<i64> {
        let v = self
            .onset_ns
            .checked_mul(i128::from(sample_rate))
            .ok_or_else(|| Error::Invalid("event time overflow".into()))?;
        let rounded = v
            .checked_add(if v < 0 { -500_000_000 } else { 500_000_000 })
            .ok_or_else(|| Error::Invalid("event time overflow".into()))?
            / 1_000_000_000;
        i64::try_from(rounded).map_err(|_| Error::Invalid("event sample index overflow".into()))
    }
}
#[derive(Debug, Clone)]
pub struct Metadata {
    pub streams: Vec<Stream>,
    /// Active stream index (EEG for the root recording, PNS for `pns()`).
    pub eeg_stream: usize,
    pub channels: Vec<Channel>,
    pub sample_rate: u32,
    pub sample_count: u64,
    pub recording_start: Option<String>,
    pub acquisition_reference: Option<AcquisitionReference>,
    pub acquisition_version: Option<String>,
    pub hardware_filter_adjusted: Option<bool>,
    pub hardware_filter_shift_microseconds: Option<i64>,
    pub kind: RecordingKind,
    pub events: Vec<Event>,
    pub warnings: Vec<String>,
}
impl Metadata {
    pub fn duration_seconds(&self) -> f64 {
        self.sample_count as f64 / f64::from(self.sample_rate)
    }
}
#[derive(Debug, Clone, Copy)]
pub enum SampleValues {
    Raw,
    Calibrated,
}
/// Channel-major samples in requested order; range is half-open and zero-based.
#[derive(Debug)]
pub struct SampleWindow {
    pub range: Range<u64>,
    pub channels: Vec<usize>,
    pub values: Vec<Vec<f32>>,
}
/// Synthetic acquisition annotation, separate from recorded stimulus events.
/// Sample bounds are authoritative, half-open, and use the active stream's clock.
#[derive(Debug, Clone, PartialEq)]
pub struct AcquisitionGap {
    pub range: Range<u64>,
    pub sample_rate: u32,
}
impl AcquisitionGap {
    pub const CODE: &'static str = "BAD_ACQ_SKIP";
    pub const LABEL: &'static str = "Acquisition gap";
    pub fn onset_seconds(&self) -> f64 {
        self.range.start as f64 / f64::from(self.sample_rate)
    }
    pub fn duration_seconds(&self) -> f64 {
        (self.range.end - self.range.start) as f64 / f64::from(self.sample_rate)
    }
}

#[derive(Debug)]
pub struct Recording {
    metadata: Metadata,
    index: binary::Index,
    signal_path: PathBuf,
    limits: Limits,
    pns: Option<std::sync::Arc<Recording>>,
}
impl Recording {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_limits(path, Limits::default())
    }
    pub fn open_with_limits(path: impl AsRef<Path>, limits: Limits) -> Result<Self> {
        metadata::open(path.as_ref(), limits)
    }
    /// Independently indexed PNS stream, with its own rate, channels, and range.
    pub fn pns(&self) -> Option<&std::sync::Arc<Recording>> {
        self.pns.as_ref()
    }
    pub fn is_pns(&self) -> bool {
        self.metadata.streams[self.metadata.eeg_stream]
            .kind
            .eq_ignore_ascii_case("PNSData")
    }
    /// Number of samples actually stored per channel, excluding timeline gaps.
    pub fn acquired_sample_count(&self) -> u64 {
        self.index.blocks.iter().map(|b| b.samples).sum()
    }
    /// Missing intervals on the active stream's acquisition sample grid.
    /// Includes a leading gap; adjacent binary blocks do not create gaps.
    pub fn gaps(&self) -> Vec<Range<u64>> {
        let mut end = 0;
        let mut gaps = Vec::new();
        for block in &self.index.blocks {
            if block.start > end {
                gaps.push(end..block.start);
            }
            end = block.start + block.samples;
        }
        gaps
    }
    /// Duration-bearing BAD_ACQ_SKIP annotations derived from acquisition timing.
    /// Kept separate from metadata().events so annotations cannot become triggers
    /// or change counts of recorded events. PNS uses its own sample grid.
    pub fn gap_annotations(&self) -> Vec<AcquisitionGap> {
        self.gaps()
            .into_iter()
            .map(|range| AcquisitionGap {
                range,
                sample_rate: self.metadata.sample_rate,
            })
            .collect()
    }
    /// Test a half-open interval against acquisition coverage without reading data.
    /// An empty in-bounds interval has no overlap. Invalid bounds and segmented/
    /// averaged recordings are errors, not silently accepted as gap-free.
    pub fn overlaps_gap(&self, range: Range<u64>) -> Result<bool> {
        if self.metadata.kind == RecordingKind::SegmentedOrAveraged {
            return Err(Error::Unsupported(
                "continuous acquisition coverage for segmented/averaged recording".into(),
            ));
        }
        if range.start > range.end || range.end > self.metadata.sample_count {
            return Err(Error::InvalidSelection(
                "sample range outside recording".into(),
            ));
        }
        if range.is_empty() {
            return Ok(false);
        }
        let first = self
            .index
            .blocks
            .partition_point(|b| b.start + b.samples <= range.start);
        let mut covered = range.start;
        for b in self.index.blocks[first..]
            .iter()
            .take_while(|b| b.start < range.end)
        {
            if b.start > covered {
                return Ok(true);
            }
            covered = b.start + b.samples;
            if covered >= range.end {
                return Ok(false);
            }
        }
        Ok(covered < range.end)
    }
    /// Read a candidate analysis epoch. None means reject the epoch: it overlaps
    /// an acquisition gap or contains nonfinite samples on any requested channel.
    /// Empty selections and invalid bounds are errors. This never fills, repairs,
    /// shifts, or joins missing data; ordinary read_window still returns NaNs.
    /// Include any required baseline/filter support in the requested range.
    pub fn read_epoch(
        &self,
        channels: &[usize],
        range: Range<u64>,
        mode: SampleValues,
    ) -> Result<Option<SampleWindow>> {
        if channels.is_empty()
            || range.is_empty()
            || channels.iter().any(|c| *c >= self.metadata.channels.len())
        {
            return Err(Error::InvalidSelection(
                "epoch requires valid channels and a nonempty sample range".into(),
            ));
        }
        if self.overlaps_gap(range.clone())? {
            return Ok(None);
        }
        let window = self.read_window(channels, range, mode)?;
        if window.values.iter().flatten().any(|v| !v.is_finite()) {
            return Ok(None);
        }
        Ok(Some(window))
    }
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
    /// Reads only the selected channel spans. Requires an unchanged source file.
    /// Acquisition pauses are returned as NaN gaps; segmented/averaged data are rejected.
    pub fn read_window(
        &self,
        channels: &[usize],
        range: Range<u64>,
        mode: SampleValues,
    ) -> Result<SampleWindow> {
        if self.metadata.kind == RecordingKind::SegmentedOrAveraged {
            return Err(Error::Unsupported(
                "sample-window timeline for noncontinuous recording".into(),
            ));
        }
        if range.start > range.end
            || range.end > self.index.samples
            || channels.iter().any(|c| *c >= self.index.channels)
        {
            return Err(Error::InvalidSelection(
                "channel or sample range outside recording".into(),
            ));
        }
        let count = usize::try_from(range.end - range.start)
            .map_err(|_| Error::Limit("window size".into()))?;
        let bytes = count
            .checked_mul(channels.len())
            .and_then(|x| x.checked_mul(4))
            .and_then(|x| {
                channels
                    .len()
                    .checked_mul(std::mem::size_of::<Vec<f32>>() + std::mem::size_of::<usize>())
                    .and_then(|y| x.checked_add(y))
            })
            .ok_or_else(|| Error::Limit("window size overflow".into()))?;
        if bytes > self.limits.max_window_bytes {
            return Err(Error::Limit(
                "sample window exceeds max_window_bytes".into(),
            ));
        }
        let mut f = File::open(&self.signal_path).map_err(|e| error::io(&self.signal_path, e))?;
        if f.metadata()
            .map_err(|e| error::io(&self.signal_path, e))?
            .len()
            != self.index.file_len
        {
            return Err(Error::Invalid(
                "signal file size changed since indexing".into(),
            ));
        }
        let mut values = vec![vec![f32::NAN; count]; channels.len()];
        let mut buf = [0u8; 8192];
        let first = self
            .index
            .blocks
            .partition_point(|b| b.start + b.samples <= range.start);
        for b in self.index.blocks[first..]
            .iter()
            .take_while(|b| b.start < range.end)
        {
            let start = range.start.max(b.start);
            let end = range.end.min(b.start + b.samples);
            for (row, &ch) in channels.iter().enumerate() {
                f.seek(SeekFrom::Start(
                    b.data + b.offsets[ch] + (start - b.start) * 4,
                ))
                .map_err(|e| error::io(&self.signal_path, e))?;
                let mut dest = (start - range.start) as usize;
                let mut remaining = (end - start) as usize;
                while remaining > 0 {
                    let n = remaining.min(buf.len() / 4);
                    f.read_exact(&mut buf[..n * 4])
                        .map_err(|e| error::io(&self.signal_path, e))?;
                    for (i, chunk) in buf[..n * 4].as_chunks::<4>().0.iter().enumerate() {
                        let raw = f32::from_le_bytes(*chunk);
                        values[row][dest + i] = match mode {
                            SampleValues::Raw => raw,
                            SampleValues::Calibrated => {
                                raw * self.metadata.channels[ch].calibration
                            }
                        };
                    }
                    dest += n;
                    remaining -= n;
                }
            }
        }
        Ok(SampleWindow {
            range,
            channels: channels.to_vec(),
            values,
        })
    }
}

impl Recording {
    /// Create a new signal binary with identical block headers and layout.
    /// The callback supplies calibrated samples on the acquisition timeline.
    /// Values are divided by the original GCAL so the original metadata remains valid.
    /// Only acquired samples are written: recording gaps are never filled.
    /// On failure the newly created destination is removed. Existing files are refused.
    pub fn export_signal(
        &self,
        destination: impl AsRef<Path>,
        mut samples: impl FnMut(usize, Range<u64>) -> std::result::Result<Vec<f32>, String>,
    ) -> Result<()> {
        self.export_signal_channels(
            destination,
            &(0..self.index.channels).collect::<Vec<_>>(),
            &mut samples,
        )
    }
    /// Like export_signal, but byte-copies unselected channels without conversion.
    pub fn export_signal_channels(
        &self,
        destination: impl AsRef<Path>,
        selected: &[usize],
        mut samples: impl FnMut(usize, Range<u64>) -> std::result::Result<Vec<f32>, String>,
    ) -> Result<()> {
        use std::io::Write;
        if selected.iter().any(|&c| c >= self.index.channels) {
            return Err(Error::InvalidSelection(
                "export channel outside recording".into(),
            ));
        }
        let destination = destination.as_ref();
        if self.metadata.kind == RecordingKind::SegmentedOrAveraged {
            return Err(Error::Unsupported(
                "export of segmented/averaged recordings".into(),
            ));
        }
        if self
            .metadata
            .channels
            .iter()
            .any(|c| !c.calibration.is_finite() || c.calibration == 0.)
        {
            return Err(Error::Unsupported(
                "export requires finite nonzero GCAL".into(),
            ));
        }
        let mut source =
            File::open(&self.signal_path).map_err(|e| error::io(&self.signal_path, e))?;
        if source
            .metadata()
            .map_err(|e| error::io(&self.signal_path, e))?
            .len()
            != self.index.file_len
        {
            return Err(Error::Invalid("signal size changed since indexing".into()));
        }
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|e| error::io(destination, e))?;
        let result = (|| {
            std::io::copy(&mut source, &mut out).map_err(|e| error::io(destination, e))?;
            for block in &self.index.blocks {
                let mut start = block.start;
                while start < block.start + block.samples {
                    let end = (start + 16384).min(block.start + block.samples);
                    for &ch in selected {
                        let values = samples(ch, start..end).map_err(Error::Invalid)?;
                        if values.len() != (end - start) as usize {
                            return Err(Error::Invalid(
                                "export callback returned wrong sample count".into(),
                            ));
                        }
                        let mut bytes = Vec::with_capacity(values.len() * 4);
                        for v in values {
                            let raw = v / self.metadata.channels[ch].calibration;
                            if v.is_finite() && !raw.is_finite() {
                                return Err(Error::Invalid("export calibration overflow".into()));
                            }
                            bytes.extend_from_slice(&raw.to_le_bytes());
                        }
                        out.seek(SeekFrom::Start(
                            block.data + block.offsets[ch] + (start - block.start) * 4,
                        ))
                        .map_err(|e| error::io(destination, e))?;
                        out.write_all(&bytes)
                            .map_err(|e| error::io(destination, e))?;
                    }
                    start = end;
                }
            }
            out.sync_all().map_err(|e| error::io(destination, e))?;
            Ok(())
        })();
        drop(out);
        if result.is_err() {
            let _ = std::fs::remove_file(destination);
        }
        result
    }
}
