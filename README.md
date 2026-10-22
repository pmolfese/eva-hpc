# EVA-HPC

**EVA-HPC is a Rust EEG analysis application and reusable processing toolkit for
macOS and Linux.** It combines a native waveform viewer with headless command-line
tools for inspecting recordings, filtering, MRI gradient-artifact correction,
channel interpolation, segmentation, and event-related averaging.

The project ports capabilities from the macOS EVA application into a modular Rust
codebase. The first supported recording format is **EGI MFF**. Reading and
processing are independent of the graphical application, so the same components
can be used by other Rust software or run on a cluster without a display server.

EVA-HPC does **not** depend on MNE-Python, MATLAB, or Python at runtime. The viewer
uses **egui/eframe and wgpu** for its interface and rendering. Signal processing
currently runs on the **CPU**; “HPC” describes the project's direction, not a claim
that GPU computation, distributed execution, or MPI support is already available.

## Project status

This is an actively developed, pre-release port. The crates are not yet published
to crates.io, and APIs and analysis-bundle formats may change. The Rust and Swift
applications are separate builds; numerical and feature parity is not assumed.

- **macOS:** native Metal-backed viewer launch has been verified locally.
- **Linux:** supported build target with X11/Wayland features and CI jobs;
  native desktop interaction testing remains pending.
- **Headless operation:** the `eva` CLI, reader, and processing engine do not need
  egui, wgpu, or a display server.
- **Data integrity:** input MFF recordings are read-only. Processing commands write
  new outputs and refuse to overwrite existing output directories.

See the [roadmap](ROADMAP_eva-hpc.md) for the original port stages and deferred work.
The capability descriptions below describe the current implementation.

## What is included

| Capability | Current support |
| --- | --- |
| MFF reading | Indexed, bounded sample windows; calibrated EEG; independently timed PNS; events; channel metadata and scalp coordinates |
| Acquisition pauses | Original acquisition clock retained; explicit gap ranges and `BAD_ACQ_SKIP` annotations; missing samples represented as NaN |
| Waveform viewer | Channel navigation, amplitude/time scaling, event inspection, exact sample values, EEG/PNS switching, channel hiding and bad-channel marking |
| Filtering | IIR/FIR/Auto, Butterworth/elliptic designs, high/low-pass, notch, CleanLine-style line-noise removal, average reference, and advanced design controls |
| MRI correction | Allen AAS, FASTR, FARM, and Moosmann; volume markers, slice/multiband or explicit acquisition timing, alignment, scaling, OBS, ANC, and motion-related controls |
| Interpolation | Perrin spherical splines using MFF scalp coordinates; interactive channel repair and headless export |
| PSA | Event-locked segmentation, baseline correction, average reference, category grouping/averaging, channel thresholds, global/per-epoch interpolation, escalation, and rejection reports |
| Recording inspection | AFNI-style selected-column tables, CSV/JSON export, event counts/timing, sampling and duration metrics, channel locations, gaps, and processing history |
| Provenance | Processing settings in `eva.xml`, audit logs, and preservation of existing source provenance |

Algorithm names describe EVA's implementations. They do not imply that external
toolboxes are linked or that their results will be numerically identical.

## Repository layout

| Crate | Role | Graphics dependency |
| --- | --- | --- |
| [`eva-mff`](crates/eva-mff/README.md) | Reusable MFF reader, metadata, acquisition coverage, sample-window and checked epoch APIs, and signal-export primitives | None |
| `eva-core` | Filtering, MRI correction, spherical splines, and PSA processing shared by the GUI and CLI | None |
| [`eva-viewer`](crates/eva-viewer/README.md) | Native waveform viewer and processing dialogs; builds `eva-viewer` | egui/eframe + wgpu |
| [`eva-cli`](crates/eva-cli/README.md) | Headless commands; builds `eva` | None |

Other directories:

- `scripts/`: macOS development bundling and fixture/consumer verification tools.
- `.github/workflows/`: macOS/Linux build, test, MSRV, formatting, and lint jobs.
- `docs/`: implementation investigations and supporting documentation.
- `crates/eva-mff/tests/fixtures/`: small attributed upstream MFF test fixtures.
- `target/`: generated binaries, app bundles, and local test outputs; ignored by Git.

An `eva-io` umbrella crate and additional EEG formats remain future work.

## Build and run

Install a Rust toolchain with Cargo. The viewer requires **Rust 1.95 or newer**;
`eva-mff`, `eva-core`, and `eva-cli` declare a **Rust 1.88** minimum. Use a current
stable toolchain to build the complete workspace. Keep `Cargo.lock` when building
this application repository.

From the repository root:

```sh
# Build both applications.
cargo build --release --locked -p eva-viewer -p eva-cli

# Launch the graphical viewer, with or without a recording.
./target/release/eva-viewer
./target/release/eva-viewer /path/to/recording.mff

# Inspect available headless commands.
./target/release/eva -help
```

The executables currently have separate entry points: **`eva-viewer` opens the
GUI; `eva` runs the CLI and shows help when invoked without arguments.** Launching
the viewer from a bare `eva` command is a future integration step.

For a headless machine, build only the CLI:

```sh
cargo build --release --locked -p eva-cli
# Optional installation into Cargo's bin directory:
cargo install --path crates/eva-cli --locked
```

### macOS application bundle

```sh
cargo build --release --locked -p eva-viewer -p eva-cli
python3 scripts/bundle-macos.py --release
open "target/EVA Viewer.app"
```

This creates a local unsigned development bundle. The CLI is also included in
`Contents/MacOS/eva` when built for the selected profile. Signing, notarization,
and installer distribution are not implemented. Python 3 is used by this optional
bundling script, not by the applications themselves.

### Linux desktop and cluster use

The GUI needs a graphical session, working graphics drivers, and the desktop
services used by native file dialogs. The configured viewer supports X11 and
Wayland; its Linux file picker uses the XDG portal. See the
[viewer platform notes](crates/eva-viewer/README.md) for backend details.

On a compute node without a graphical session, use the headless `eva` commands.
Jobs can be submitted through your scheduler as ordinary command-line processes.
The application does not currently distribute a job across nodes, schedule jobs,
or perform processing on GPUs. Batch input lists run sequentially.

## Viewer workflow

1. Use **Open MFF…**, drag a recording into the window, or pass its path at launch.
   MFF inputs are unpacked recording directories; on macOS they may appear as
   file packages.
2. Navigate the waveform with the time/amplitude controls, channel scrolling,
   and bottom timeline. Inspect markers with **EVENTS** and channel controls with
   **EEG**. PNS retains its own sampling rate and timebase.
3. Use **FILTER** and **MRI** to configure processing in focused dialogs. Hover
   question-mark controls for explanations. Viewer processing leaves the original
   recording unchanged.
4. Left-click a channel name to hide its trace while retaining the row. Right-click
   to interpolate or mark it bad. Bad-channel numbers are printed as a comma-separated,
   one-based list when the recording is closed or the program exits.
5. Use **PSA** for event selection, epoch windows, baseline/reference settings,
   thresholds, interpolation, and category averaging. Results appear in an
   epoch/average window and can be exported as a new `.psa` directory.

The GUI's PSA engine uses the current MRI/filter/interpolation snapshot. The CLI
uses its input file's samples, so export the required preprocessing first when
reproducing that workflow headlessly. The viewer's broader **ARTIFACTS** tool and
several macOS EVA workspaces are not yet ported.

## Command-line workflows

EVA uses **single-dash options**, following the AFNI convention. Double-dash option
aliases are not accepted. The standalone `--` still ends option parsing where
supported. Cargo and other external tools retain their own command syntax.
Use `eva <command> -help` for the complete option list.

### Inspect and compare recordings

```sh
eva info -hdr -prefix -nchan -sfreq -nt -duration -ngaps *.mff
eva info -hdr -prefix -event_count DIN1 -event_count TREV *.mff
eva info -hdr -prefix -timing DIN1 TREV -max_lag 250 *.mff
eva info -json recording.mff > metadata.json
eva info -format csv -hdr -prefix -history -gaps *.mff > comparison.csv
```

Tables contain one row per recording, with columns in the requested order. Event
pair delays are **second-event onset minus first-event onset**, in milliseconds;
matching methods and unmatched/reused counts are explicit. `-nt` and `-duration`
include acquisition gaps. `-acquired_samples` and `-acquired_duration` exclude them.
`-data_points` counts acquired scalar values across the selected stream's channels.

### Filter

```sh
eva filter -highpass 0.1 -lowpass 40 input.mff filtered.mff
eva filter -line-noise notch -line-frequency 60 input.mff notched.mff
```

The GUI's **Copy CLI command** action copies its filter settings. The CLI exposes
independent cutoff orders, IIR/FIR design choices, notch/CleanLine controls,
precision, reference exclusions, and optional PNS processing. Filtering defaults
to no active cutoffs; request the processing you need explicitly.

### Remove MRI gradient artifacts

```sh
eva mri -method aas -trigger TREV input.mff cleaned.mff
eva mri -method fastr -slice-timing scan.json -obs auto input.mff cleaned.mff
eva mri -method moosmann -motion motion.par -motion-format fsl input.mff cleaned.mff
```

Choose acquisition timing that actually belongs to the recording. A slice count
and multiband factor determine the number of groups, but do not describe all
scanner dead time or unequal group spacing. Explicit timing uses BIDS `SliceTiming`.
Moosmann requires motion data. These methods retain implementation and validation
limitations described below and in the CLI documentation.

### Interpolate channels

```sh
eva interpolate 1,8 -exclude 12,20 input.mff repaired.mff
```

Channels are one-based. Targets and excluded bad channels never act as donors to
one another. Interpolation requires scalp coordinates and at least three usable
donors. This command changes selected EEG channels; PNS is copied unchanged.

### Batch filtering or MRI correction

```sh
eva filter -highpass 0.1 -lowpass 40 -inputs run1.mff run2.mff -suffix _filt
eva mri -method aas -inputs run1.mff run2.mff -suffix _aas
```

The suffix is appended literally before `.mff`, beside each input. All output
names are checked for collisions before processing. Jobs use the same settings,
including any MRI sidecars, and stop at the first failure; completed outputs
remain. Use separate commands when recordings need different timing or motion files.

### Segment and average with PSA

```sh
eva psa -events Shap,Fear -pre 0.2 -post 0.8 input.mff epochs.psa
eva psa -events Shap,Fear -average true input.mff averages.psa
eva psa -events Shap,Fear -categories Shap=visual,Fear=visual \
  -bad-channels 1,8 -average true input.mff grouped.psa
```

PSA exposes amplitude, slope and acceleration thresholds; bad-channel count or
percentage limits; global/per-epoch spline repair; recurrent-channel escalation;
explicit artifact-event and candidate-epoch exclusions; baseline correction;
average reference; and category means. Thresholds require declared signal units.

A `.psa` directory is an analysis bundle, **not an MFF recording**:

| Contents | Purpose |
| --- | --- |
| `manifest.json` | Settings, channel metadata, timebase, array layout, candidate identities, decisions, warnings and provenance summary |
| `epochs.f32le` | Accepted epochs in epoch × channel × sample order |
| `averages.f32le`, `averages.csv` | Category averages and contributing trial counts; empty/header-only if averaging is off or all trials are rejected |
| `decisions.csv` | Acceptance/rejection reasons and flagged/repaired channels for every candidate |
| `eva.xml`, `log_eva_psa.txt` | PSA processing settings and audit summary |
| `source-eva.xml` | Original source provenance, when present |

With averaging enabled, **accepted epochs and averages are both retained**. Rejected
trials remain in the decision report, but their waveform arrays are not exported.
The binary arrays are little-endian float32. The
[CLI documentation](crates/eva-cli/README.md#psa-directory-contents) includes a Python
loading example and exact indexing conventions.

## Acquisition gaps and provenance

Recording pauses are represented on the acquisition clock rather than removed by
joining the surrounding samples. This keeps later event markers aligned with EEG.
Missing samples remain **NaN** internally. Explicit gap ranges and duration-bearing
`BAD_ACQ_SKIP` annotations are separate from recorded stimulus events.

- Filters respect missing-data boundaries; adaptive state does not bridge gaps.
- MRI correction skips invalid acquisition windows and preserves missing samples.
- PSA always rejects epochs crossing gaps or containing nonfinite samples.
  Interpolation cannot rescue a missing acquisition interval.
- MFF signal export preserves stored blocks and acquisition metadata rather than
  writing placeholder samples into the original binary layout.

MFF processing exports append `eva.xml` records and write audit logs. Existing
history is preserved; invalid or unsupported provenance is not silently discarded.
Rust-generated steps are marked non-replayable where numerical compatibility with
Swift replay has not been established. PSA stores its source history separately,
as described above. These records document what ran; they do not establish that
artifact removal or analysis choices were scientifically appropriate.

## Reuse the Rust libraries

The reader and processing engine are intended for reuse outside EVA. Until public
crate releases are available, use path dependencies from another Rust project:

```toml
[dependencies]
eva-mff = { path = "../eva-hpc/crates/eva-mff" }
eva-core = { path = "../eva-hpc/crates/eva-core" }
```

`eva-mff` exposes recording metadata, independent EEG/PNS streams, bounded raw or
calibrated sample windows, acquisition-gap checks, and checked epoch reads.
`eva-core` contains the GUI-independent algorithms. Neither requires Python or a
GPU. See the [reader API documentation](crates/eva-mff/README.md) and generate local
Rust API docs with `cargo doc -p eva-mff -p eva-core --no-deps --locked --open`.

## Current limitations

- MFF support focuses on unpacked continuous EEG and discontinuous acquisitions.
  Categorized/segmented and averaged MFF metadata can be inspected, but their
  sample-window processing and export workflows are not implemented.
- GPU processing, distributed execution, compressed MFF input, and additional EEG
  formats are not implemented.
- MRI correction currently has a 512 MiB combined input-sample limit plus temporary
  epoch-workspace limits. PSA caps the candidate epoch payload at 256 MiB.
- Slice-based MRI correction still needs work on AAS donor grouping and explicit
  timing-window coverage. Do not assume that importing slice timings fixes those
  issues or that a successful run guarantees adequate artifact removal.
- PSA is a first port of the main processing controls. Saved `.psa` reopening in
  the GUI, segmented/averaged MFF export, Decision Matrix editing, automated
  eye/health detectors, regex categories, and DIN anchor reassignment are pending.
- Packaging is developmental; there are no published crate releases, signed
  installers, or established cross-version API/format guarantees.

## Development and verification

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo doc -p eva-mff -p eva-core --no-deps --locked
python3 scripts/check-fixtures.py
python3 scripts/check-consumer.py
```

Tests cover binary reading, calibration, PNS timing, acquisition gaps, event-time
precision, filter and MRI behavior, interpolation, PSA decisions and means, export
provenance, and headless GUI interactions. The GitHub Actions workflow configures
stable macOS/Linux jobs and separate Rust 1.88 tests for the non-GUI crates. A
configured job is not a claim that every platform interaction has been verified.

Reference fixtures have documented upstream provenance and checksums; most tests
build synthetic recordings in temporary directories. See
[fixture attribution](crates/eva-mff/tests/fixtures/README.md). Build products and
local analysis outputs under `target/` should not be committed.

When reporting a problem, include the command/settings, OS and Rust versions,
relevant error text, and whether the recording contains gaps or PNS. Prefer a small
synthetic reproduction or an appropriately shareable recording. Algorithm changes
should include a numerical regression test and preserve source data and timing.

## License and third-party material

EVA-HPC uses EVA's [License and Public Work Notice](LICENSE). It identifies EVA's
Government-authored code as work prepared by P. Molfese, National Institutes of
Health, as part of official United States Government duties, and states the
applicable public-work terms, international-rights qualification, and warranty
and liability provisions.

**Third-party material retains its own licenses.** In particular, the bundled
mffpy example recordings retain their Apache-2.0 license and attribution; Rust
and GUI dependencies retain their respective terms. See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and the fixture license. The root
notice does not convert those dependencies or datasets into Government works.
