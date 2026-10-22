# eva-mff

A synchronous, read-only Rust library for EGI MFF directories. Intended for EVA,
Sumaru, and other applications; no egui, wgpu, Python, or async runtime dependency.

## Use from another application

```toml
[dependencies]
eva-mff = { path = "/path/to/eva-hpc/crates/eva-mff" }
```

```rust,no_run
use eva_mff::{Recording, SampleValues};

fn main() -> Result<(), eva_mff::Error> {
    let recording = Recording::open("recording.mff")?;
    let metadata = recording.metadata();
    println!("{} channels at {} Hz", metadata.channels.len(), metadata.sample_rate);
    let end = metadata.sample_count.min(1000);
    let window = recording.read_window(&[0], 0..end, SampleValues::Raw)?;
    println!("{} samples", window.values[0].len());
    Ok(())
}
```

Use `open_with_limits` to set resource ceilings. Calls are synchronous: GUI callers
should use their own worker thread. `Recording` owns immutable metadata/indexes;
each read opens its own file handle, so concurrent reads do not share a seek cursor.
The window memory ceiling applies per call, not globally across concurrent callers.

## Supported now

- Unpacked MFF directories with exactly one metadata-declared EEG stream.
- Little-endian float32 channel-major data, full/reused block headers, changing
  block lengths, uniform channel counts/rates, and checked channel offsets.
- Metadata inspection, sensor labels, reference identity, optional explicit sensor
  units, constant per-channel GCAL, event onsets/durations, and stream inventory.
- Selected-channel reads across blocks on the acquisition timeline, with NaN gaps for recording pauses. Empty windows and
  repeated/reordered channel selections are valid.
- One optional metadata-declared PNSData stream, independently indexed with its
  own sampling rate/count, zero-based pnsSet sensor labels, explicit units,
  positive-up display flags, stream-specific GCAL and hardware timing metadata.
- Unknown additional streams and noncontinuous/segmented/averaged recordings are detected.
  Their metadata may be inspected when the EEG encoding is supported; sample reads
  for segmented/averaged recordings return `Unsupported`. Categories are conservatively
  classified as `SegmentedOrAveraged`, without promising a finer classification.

## PNS windows

`recording.pns()` returns an optional shared PNS recording with the same bounded
`read_window` API. Select ranges in **PNS samples**, using its own metadata rate;
EEG sample indices must not be reused when rates differ. `metadata().eeg_stream`
is the active stream index on either handle (retained for API compatibility).
`is_pns()` distinguishes the handles. PNS polarity is metadata only: raw and
calibrated samples retain their stored signs. PNSSet gain/display-filter fields
are not additional calibration factors. Timing shifts are preserved, not applied.
Malformed PNS metadata/binary data produces an explicit open error.

```rust,no_run
# fn main() -> Result<(), eva_mff::Error> {
let recording = eva_mff::Recording::open("recording.mff")?;
if let Some(pns) = recording.pns() {
    let end = pns.metadata().sample_count.min(1000);
    let samples = pns.read_window(&[0], 0..end, eva_mff::SampleValues::Calibrated)?;
    println!("{} PNS samples at {} Hz", samples.values[0].len(), pns.metadata().sample_rate);
}
# Ok(()) }
```

## Value and time contracts

Channels and sample ranges are zero-based. Ranges are half-open. Returned data is
channel-major in the requested channel order. `Raw` preserves stored float values;
`Calibrated` multiplies by GCAL exactly once. Missing GCAL entries use identity.
No referencing, filtering, polarity change, or implicit unit conversion occurs.
Missing unit metadata is `None`; applications must not label such values as volts
or microvolts without an independently established convention. Blank numbered
sensor names become `E{number}`; missing layouts use `Ch {number}` placeholders.
An acquisition reference outside the recorded channel count is metadata only.

Absolute event timestamps are parsed as RFC3339, preserving nanoseconds and timezone
offsets. Onsets are integer nanoseconds relative to `recordTime`; durations are
converted from MFF microseconds. `Event::sample_index` rounds to the nearest sample,
with ties away from zero. Duplicate events are retained. `relativeBeginTime` does
not override the absolute timestamp. Non-RFC3339/numeric event times are explicitly
unsupported in this first version. Absolute events require `recordTime`.

Acquisition version and `hardwareFilterAdjusted`/`shiftMicroseconds` are preserved.
No additional timing correction is applied: metadata describing an existing
hardware adjustment must not trigger a second shift. A warning makes this policy
visible. This follows the inspected EVA read path, which exposes adjustment
metadata separately from raw event parsing.

If epochs exist, their block coverage and times are validated. Gaps, a nonzero
start cause a discontinuous classification. Block positions preserve these gaps,
and sample_count includes them. Epoch duration/sample-count disagreement or
overlap is rejected (allowing microsecond serialization rounding).
Absent epochs produce a warning and a continuous block-timeline assumption.

## Resource limits and errors

Default ceilings: 4096 channels, 1 million blocks, 128 MiB block-index accounting,
32 MiB per XML file/aggregate event XML, 64 MiB returned sample-window accounting,
and 4096 package entries. Reads use an additional fixed 8 KiB byte buffer. XML parser
and metadata allocations have overhead beyond input bytes, as do vector capacities
and retained block headers; these are input-derived ceilings, not a process RSS
limit. Only the selected sample spans are read, not entire blocks or recordings.

`Error` distinguishes I/O failures, invalid input, unsupported variants, resource
limits, and invalid selections. Unknown optional XML fields are ignored; malformed
required data is not silently skipped. XML is parsed with namespace-resolved local
names, and DTD processing is disabled by the parser's default configuration.

The source must remain unchanged while a `Recording` is in use. Each read checks
file size and checks short reads; equal-size in-place modifications are not detected.
Reopen after any source change. Source files are never written.

## Validation and limits

Tests include synthetic known-value signals, reused and changing-length blocks,
channel ordering, calibration, all subranges of a small signal, exact event timing,
malformed inputs, resource limits, a sparse 1 GiB signal read with a 1 KiB window
budget, and two attributed EVA/mffpy fixtures. `scripts/check-consumer.py` verifies a
separate Cargo workspace can consume the library without graphics dependencies.

macOS tests were run locally. CI is configured for macOS/Linux and Rust 1.88/stable;
Linux execution and the minimum toolchain are pending CI verification. Rust APIs
are initial and may evolve before publication. MFF writing, other sample encodings,
multiple EEG/PNS streams and epoch-aware reads remain unsupported.

## Writing a processed signal copy

`Recording::export_signal(destination, callback)` creates a **new binary signal
file**, preserving block headers/layout. The callback returns calibrated samples
for the requested channel and acquisition-time range. Export reverses GCAL once
so the original calibration metadata remains valid. Gaps have no binary samples
and are never synthesized. Existing output files, zero GCAL, and segmented/averaged
signals are rejected. Failed exports remove the newly created signal file.

This is a low-level API, not a full package writer. The `eva-cli` package orchestrates
metadata copying, EEG/PNS export, `eva.xml` history, and audit logs. Input MFF files
are never edited.

### Acquisition gaps and analysis epochs

Missing acquisition intervals remain **NaN**, never measured zeros. The sample
clock and original event timestamps include elapsed pause time. `gaps()` exposes
half-open sample ranges, and `gap_annotations()` provides duration-bearing
`AcquisitionGap` records with code `BAD_ACQ_SKIP` and label `Acquisition gap`.
These synthetic annotations are separate from `metadata().events`: they cannot
be selected accidentally as stimulus/MRI triggers or change recorded event counts.
Each stream, including PNS, uses its own sample rate and gap ranges.

`overlaps_gap(range)` checks coverage without loading samples. A window ending at
the start of a gap, or beginning at its end, is valid; crossing any gap is not.
Out-of-range selections and segmented/averaged recordings return errors.

For future epoching/segmentation consumers, use `read_epoch(channels, range, mode)`.
It returns `Ok(None)` to reject any epoch crossing an acquisition gap or containing
nonfinite data on a requested channel, `Ok(Some(window))` for valid data, and an
error for invalid/empty selections or I/O failures. Include the baseline and any
required processing context in the requested range. This API does not implement
an epoching GUI, repair missing data, or change the semantics of `read_window`.

NaNs acquired in the original signal are also rejected by `read_epoch`, but do not
create synthetic acquisition annotations. If a downstream format requires zero
padding, convert only at that boundary and carry the explicit gap mask alongside
it; do not feed unmasked zeros back into processing. Existing MFF export preserves
the stored acquisition blocks and epochs.xml rather than writing gap placeholders.
