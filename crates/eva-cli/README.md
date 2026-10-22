# eva command-line tools

Headless macOS/Linux executable; no egui/wgpu dependency or display server required.

```sh
cargo build -p eva-cli --release
./target/release/eva filter -help
./target/release/eva filter -lowpass 40.0 -highpass 0.1 input.mff output.mff

# Optional: install into Cargo's bin directory (add it to PATH)
cargo install --path crates/eva-cli --locked
```

The viewer filter dialog also has **Copy CLI command**. It copies all draft
settings, with input/output placeholders to replace. The macOS bundle script
includes the CLI at `EVA Viewer.app/Contents/MacOS/eva` when it has been built for
the selected profile.

EVA uses single-dash options throughout, including `-help` and `-version`.
Double-dash option spellings are rejected. The standalone `--` still
ends option parsing; Cargo and other external tools retain their own syntax.

## Options and defaults

`eva filter -help` lists every option. The CLI defaults to no active cutoffs;
request `-highpass` and/or `-lowpass`, line noise removal, or average reference.
All options take values; booleans use `true` or `false`. `-key=value` is also
accepted. Duplicate/unknown options are errors. Use `--` before a path beginning
with `-`. Paths can contain spaces and need normal shell quoting.

Supported GUI settings: IIR/FIR/Auto; Butterworth/elliptic; independent HP/LP pole
counts; ripple/stopband; Hamming/Kaiser; three FIR applications; transition/design
rule/crossover; notch/CleanLine; frequency/harmonics/Q/FIR notch; CleanLine window
and strength; precision; EEG average reference and excluded channels; PNS filtering;
and EEGLAB, ERPLAB, MNE-Python and Net Station convention presets. Presets are
applied first; explicit options override them regardless of argument order.
They do not call or depend on those external applications.

IIR order is per pass: 4 poles give the GUI's effective 48 dB/oct zero-phase slope.
Reference exclusions use **1-based** channel numbers, e.g. `-exclude-reference 1,8`.
PNS uses its own rate and is filtered by default; use `-filter-pns false` to copy
its binary exactly. PNS never participates in EEG average reference.

## Output and provenance

Input and output are unpacked MFF directories. Output must not exist and its parent
must exist. The source stays untouched. Blocks, events, timing gaps, channel units,
calibration, and ancillary files are preserved. Calibrated processing results are
converted back through the source GCAL, so reopening applies calibration exactly
once. Flat packages containing regular files are supported; symlinks/subdirectories,
zero calibration, and segmented/averaged export are rejected.

`eva.xml` follows macOS `EVAProcessingScriptXML`: `evaProcessing` version 1 with
`step`, `param`, `note`, timestamps, app version and file type. Existing child
records are preserved, then a `filter` step and optional `reference` step are
appended. Filter parameter names follow macOS `FilterViewModel.parameters`;
`cliArguments` additionally records the full, exact Rust configuration. New steps
are **replayable=false** because numerical parity with Swift is not established;
this is compatible provenance, not a promise of identical macOS replay. A separate
`log_eva_*.txt` audit log contains the settings and complete provenance. Invalid or
unsupported existing eva.xml is an error rather than silently losing history.

Filtering is bounded, fixed-tile processing identical to the GUI, including its
edge/context behavior; it is not a new whole-file filter implementation. Acquired
NaNs and recording gaps remain missing. GPU processing and other output formats
are not supported. Ordinary failures remove the new output; a force-killed process
or power loss can leave a partial output directory, which must be removed before
retrying. No overwrite flag is provided.

Tests reopen exported EEG/PNS, compare calibrated samples against the GUI engine,
check gaps/events/calibration/history, protect existing outputs, and verify failed
export cleanup and CLI/GUI settings round-trips.

## Channel interpolation

```sh
eva interpolate 1,8,12 input.mff repaired.mff
eva interpolate 1,8 -exclude 12,20 input.mff repaired.mff
eva interpolate -help
```

Lists use 1-based channel numbers. Targets are repaired simultaneously using a
shared set of independent donors; `-exclude` names additional bad donors.
Uses scalp positions from `coordinates.xml`, normalized about the MFF origin,
with Perrin spherical splines (order 4, 40 Legendre terms, regularization 1e-5),
matching EVA macOS defaults. At least three good positioned scalp electrodes
and compatible units are required. Supports up to 512 donors. Missing/degenerate
geometry fails explicitly; no distance-average fallback is substituted.

No filtering is added. Other EEG channels and PNS are copied byte-for-byte.
NaN donor samples propagate, preserving gaps. `eva.xml` records an
`interpolateChannels` step, donor list, exclusions and spline parameters; explicit
additional exclusions are recorded as `markBad`. The audit log records the CLI
command. Source files and existing outputs are never overwritten.

## MRI gradient artifact correction

```sh
eva mri -method aas -trigger TREV input.mff cleaned.mff
eva mri -method fastr -slices 60 -multiband 2 -obs auto input.mff cleaned.mff
eva mri -method farm -groups 30 -anc true input.mff cleaned.mff
eva mri -method moosmann -motion motion.par -motion-format fsl input.mff cleaned.mff
eva mri -help
```

The `mri` command uses the same `eva-core::mri` recording pipeline as the viewer.
Default method is Allen AAS, trigger is TREV, group count is 1, and PNS correction
is enabled. Supply the acquisition timing appropriate to the scan. All options
and defaults are listed in `-help`: group/slice/multiband timing or BIDS
SliceTiming, trigger position, alignment/upsampling, donor selection/scaling,
Allen AAS sections, OBS, ANC, motion metrics/censoring, and channel exclusions.
Method-specific options have the same meaning as in the GUI.

Choose one timing mode; uniform slices must divide evenly by the multiband factor.
`-offsets` takes comma-separated seconds; `-slice-timing` reads a JSON
`SliceTiming` array and collapses repeated simultaneous times. Motion files use
explicit AFNI, FSL or SPM conventions, with normalized motion rows recorded in
provenance. Moosmann requires motion data. Numeric channel/volume lists are
1-based: `-exclude-channels 1,8`, `-template-only 12`, `-censored 2,3`.
Excluded EEG channels are copied without rewriting their samples. These channel
lists do not apply to PNS. `-pns false` copies PNS unchanged.

Whole-recording correction currently accepts at most 512 MiB of combined input
sample values and enforces the same epoch workspace limits as the viewer. It is
CPU-only, does not require a display, and adds no temporal filtering, referencing,
or interpolation. Gaps remain missing; epochs crossing gaps are skipped and ANC
restarts after gaps. Segmented/averaged recordings are unsupported.

The new MFF contains an appended `mriGradientCorrection` step in `eva.xml` and a
`log_eva_mri_*.txt` audit log. `rustConfigJSON` holds every normalized setting,
including resolved offsets and motion rows, so external sidecars need not remain
at their original paths to interpret the record. Diagnostics include skipped
channel-epochs, fallbacks, motion censoring and warnings. Numerical equivalence
with the Swift implementation is not established; automatic macOS replay is
therefore disabled. Existing outputs and source files are never overwritten.

## Inspect recordings and compare event timing

`eva info -help` lists all metrics. This read-only command indexes MFF metadata
without loading waveform samples. Options use a single dash. Tables
use one row per input, with columns in flag order; headers are opt-in.

```sh
eva info -hdr -prefix -nchan -sfreq -nt -acquired_samples -duration -ngaps -gap_seconds *.mff
eva info -hdr -prefix -event_count DIN1 -event_count DIN2 -event_count TREV *.mff
eva info -hdr -prefix -timing DIN1 TREV -match next -max_lag 250 *.mff
eva info -json -prefix -timing_details DIN1 TREV -max_lag 250 input.mff > timing.json
eva info -json input.mff > metadata.json
eva info -format csv -hdr -prefix -history -gaps *.mff > comparison.csv
```

`-nt` counts timeline samples per channel **including gaps**; `-acquired_samples`
excludes gaps. `-data_points` counts acquired scalar values across active channels.
`-duration` includes gaps; `-acquired_duration` excludes them. `-stream pns` selects
the PNS timebase. `-streams` reports both streams. Gap bounds are half-open. Each `-gaps` entry includes the synthetic annotation
code `BAD_ACQ_SKIP` and label `Acquisition gap`; these are not included in recorded
event counts or timing matches.

`-event_types` reports counts for every exact event code. Timing pairs use signed
**B minus A** onset delays in milliseconds, with mean, median, population standard
deviation, range and matching counts. The default is nearest B within 100 ms;
`-match next`, `previous`, or `ordinal` changes pairing. Nearest ties choose the
earlier B. Independent matching may reuse a B event; the report exposes reuse,
unmatched A and unused B counts. Ordinal pairing never shifts after rejection.
Timing includes events outside signal coverage; `-events` identifies those and
events inside gaps. `-intervals CODE` summarizes consecutive same-code intervals.

`-labels`, `-channels`, `-locations`, and `-reference` expose channel metadata.
Original coordinates retain declared units (null when unspecified); normalized
unit-sphere positions are also reported and are not millimeters. `-history` reads
ordered `eva.xml` steps, settings, notes and original step XML. Missing history is
empty, whereas malformed history is null with a warning; no history does not prove
that samples are unprocessed. `-warnings` exposes reader and history diagnostics.

TSV is the default, with structured cells encoded as compact JSON. `-format csv`
quotes cells for spreadsheet import. `-json` produces structured JSON, defaulting
to all standard metrics unless metric flags are supplied. Repeated identical JSON
keys collapse to one key; table columns retain repetitions. `-NA_flag` controls
missing table values; JSON uses null. `-atr_delim` sets a custom table separator
(verbatim; choose one absent from cell contents). Unreadable inputs produce error
rows and stderr diagnostics; remaining inputs are inspected and exit status is 1.
Export with shell redirection; input packages are never modified.

## Batch filtering and MRI correction

```sh
eva filter -lowpass 40 -highpass 0.1 -inputs file1.mff file2.mff -suffix _filt
eva mri -method aas -trigger TREV -inputs file1.mff file2.mff -suffix _aas
```

`-inputs` takes one or more paths, stopping at the next option. `-suffix` is
required in batch mode and is appended **literally** before the extension:
`file1.mff` becomes `file1_filt.mff`. Outputs are placed beside their inputs.
Options may precede or follow the input list; shell wildcards work too. Quote paths
with spaces and use `./` before filenames starting with `-`. Batch mode cannot be
mixed with positional input/output arguments. The existing single-file syntax
remains available.

All output names are checked for existing files and duplicate destinations before
processing starts. Jobs run sequentially with identical settings. For MRI this
also means the same motion and slice-timing sidecars; use separate commands when
recordings need different sidecars. Processing stops on the first failure with a
nonzero exit code; completed outputs remain and ordinary failed-output cleanup
is unchanged. Each output gets its own `eva.xml` processing entry and audit log.

## PSA: epochs, rejection and averages

```sh
eva psa -help
eva psa -events Shap,Fear -pre 0.2 -post 0.8 -average true input.mff results.psa
eva psa -events Shap,Fear -categories Shap=visual,Fear=visual -bad-channels 1,8 input.mff epochs.psa
```

The shared `eva-core::psa` engine operates on EEG. The GUI PSA button uses the
current MRI/filter/interpolation snapshot; the CLI uses the input MFF samples.
For equivalent CLI results, run MRI correction and filtering first. Inputs are
read-only and output directories must be new and outside the input package.

The configuration follows macOS PSA's principal controls: pre/post/offset, code or
label selection, category pooling, baseline correction, average reference,
per-epoch min/max/slope/acceleration thresholds, bad-channel count or percentage,
global and per-epoch spherical-spline interpolation, recurrent-channel escalation,
and category averaging. Defaults match the macOS values: 0.2/0.8 s, baseline and
reference on, thresholds −150/+150 µV, 25 µV/sample and 15 µV/sample², 10% maximum
transient bad channels, 50% escalation, averaging off. `-baseline-start` defaults
to `-pre`; the GUI allows independent baseline bounds. All endpoints are half-open.

Detection uses baseline-adjusted amplitude (when enabled), with adjacent-sample
slope and second-difference acceleration. Global bad channels are excluded from
threshold detection, spline donors and average reference. Escalation uses the
fraction of measurable candidates before threshold-based rejection, pooling the
selected categories. Rejection/repair happens before average reference and final
baseline subtraction; accepted trials receive equal weight in each category mean.
All targets are reconstructed from independent donors. Coordinates are required
for repair; failures appear as explicit per-epoch rejection reasons.

Gap overlap, out-of-bounds epochs and nonfinite values cannot be rescued by
interpolation. Explicit `-reject-codes` rejects artifact-event overlap and
`-reject-epochs` rejects candidate numbers. Rejection reports include candidates
that never reached threshold detection. Thresholding requires known EEG units;
no silent µV assumption is made. Current in-memory epoch payload limit is 256 MiB.

### PSA directory contents

The `.psa` directory is an EVA analysis bundle, **not MFF**. Without averaging it
contains accepted epochs. With averaging it contains accepted epochs **and**
category means, preserving the link between the trials and their averages.

- `manifest.json`: format/version, sample rate, epoch length, pre-stimulus sample
  count, channel names/units, full settings, global bad channels, decisions, and
  binary array ordering. Configuration bad-channel indices are zero-based;
  decision/report channel numbers and candidate numbers are one-based.
- `epochs.f32le`: accepted epoch × channel × sample, little-endian float32.
- `averages.f32le`: category × channel × sample, little-endian float32; empty if
  averaging was not selected or no epochs survived.
- `averages.csv`: category, contributing trial count, channel/name/unit, relative
  time and value; header-only when there are no averages.
- `decisions.csv`: acceptance/rejection reason, original event index, anchor sample,
  flagged channels and repaired channels for every candidate.
- `eva.xml`, `log_eva_psa.txt`: PSA settings and audit summary. Existing source
  history is preserved verbatim in `source-eva.xml` when present. Numerical Swift
  replay parity is not claimed.

For example, read accepted epochs in Python without any EVA dependency:

```python
import json
from pathlib import Path
import numpy as np
p = Path("results.psa")
m = json.loads((p / "manifest.json").read_text())
x = np.fromfile(p / "epochs.f32le", dtype="<f4").reshape(
    len(m["epoch_candidates"]), len(m["channel_names"]), m["samples_per_epoch"])
t = (np.arange(m["samples_per_epoch"]) - m["pre_samples"]) / m["sample_rate"]
```

The GUI shows accepted epochs and averages in its PSA results window and exports
this bundle. Reopening a saved PSA bundle in the GUI, segmented/averaged MFF export,
Decision Matrix manual cell edits, automatic eye/health detectors, regex categories,
and DIN reassignment are not part of this first port. The options shown here are
implemented; this is not a claim of full macOS PSA parity. An all-rejected run writes
its audit report with empty data/average arrays instead of inventing an average.
