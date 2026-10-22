# EVA Viewer

A native egui/eframe application using wgpu, backed by the standalone `eva-mff`
reader. Supports continuous EEG and recordings with pauses in unpacked MFF directories. Source recordings
are read-only. Optional viewer filters change the displayed signal without writing
to the recording. Average reference and interpolation are applied only when
selected; the source samples remain unchanged.

## Run

Rust 1.95 or newer is required by the GUI dependencies (the reader remains Rust 1.88).

```sh
cargo run -p eva-viewer -- /path/to/recording.mff
cargo run -p eva-viewer -- crates/eva-mff/tests/fixtures/example_3.mff
cargo run -p eva-viewer
```

Use **Open MFF…**, Command/Ctrl+O, a command-line path, or drag a directory into the
window. On macOS the picker accepts both file packages and ordinary directories, so
registered `.mff` packages remain selectable. Linux uses a directory picker.
The reader validates the selected path as an unpacked MFF recording.

For a local unsigned macOS app bundle:

```sh
cargo build -p eva-viewer
python3 scripts/bundle-macos.py
```

Open `target/EVA Viewer.app` in Finder. The script also accepts `--release` after a
release build. This is a development bundle, without signing/notarization or file
associations. It does not replace the original Swift EVA application.

On Linux, run in an X11 or Wayland graphical session with a working Vulkan driver.
The native dialog uses the desktop's XDG portal. A headless cluster node cannot
show this window without a graphical session. The active adapter/backend appears
in the status bar and startup stderr. `WGPU_BACKEND=vulkan` (Linux) or
`WGPU_BACKEND=metal` (macOS) can select the backend explicitly.

## Controls

- Drag traces horizontally to pan. Left/Right arrows move half a window.
- The top **Time scale** slider changes the span (1× is the initial window).
  Keyboard **+ / −** and trackpad pinch also zoom. The horizontal slider and
  arrow buttons below the waveforms move through the recording.
- Scroll vertically to move through channels; Shift-scroll/horizontal scrolling
  pans time. **Rows** controls the number of visible channels (up to 128).
- The top **Scale** slider sets the amplitude above/below each row's baseline. **Fit amplitude** in the EEG panel
  fits the current visible data. The scale is in declared channel units; missing
  units remain explicitly unspecified.
- **Center traces** subtracts each channel's visible-window mean for display only.
  Disable it to display absolute values. Exact sample readings are never centered.
- Click a trace for an exact calibrated sample value, channel, index, and time.
  The vertical cursor is retained while navigating. Clicking a coarse envelope
  reads the selected sample through the active filter chain rather than reporting
  an envelope extremum. The status bar labels filtered versus unfiltered values.
- **Events** supports code/label search and jumps to event onset. Events outside
  the recording are listed but cannot be selected as jump targets.
- **EEG** opens the display/channel panel, where **Channels** filters and shows/hides channels. Clicking a waveform label hides
  that channel. **Reset** restores visibility, navigation, centering, and scale.
- **Recording info** / **Info** lists streams, reader warnings, graphics details, and interaction help.
- **Close** releases the current recording/view. **Cancel** invalidates pending
  work; use **Retry**, change the view, or reopen to resume.

The light layout follows the macOS EVA reference: scale controls at the upper
left, MRI / FILTER / ARTIFACTS / PROCESS / EEG / EVENTS buttons across the top,
a separate event lane, blue traces, and a bottom timeline. ARTIFACTS
and PROCESS are disabled placeholders; they do not run analysis. MRI, FILTER, EEG, and EVENTS
toggle the inspector, which is closed by default to give waveforms more room.

## Filtering

Click **FILTER** to open a focused modal with macOS-style field grouping.
Hover over a **?** button for contextual help, or click it (Tab then Space/Enter
with the keyboard) to open the explanation. Escape dismisses help before the
filter dialog. Help covers design choices, cutoffs, FIR timing, line noise,
reference, PNS, presets, precision, and Preview/Apply behavior.
Edits remain a draft until **Preview** or **Apply Filter**. Preview updates the
waveforms; **Cancel** (or Escape) restores the previous filter settings. Apply
commits and closes; **Remove filters** clears filtering and closes. Outside clicks
do not dismiss the dialog. **Reset 0.1–30 Hz** edits the draft only.
The enlarged two-column dialog fits expanded controls at the default 1280×820
window size. Smaller windows retain a scrolling fallback so nothing is clipped.
Opening a new recording starts unfiltered. **Reset** in EEG resets the view, not filters.

- **IIR:** Butterworth or elliptic; independent HP/LP slopes of 24/48/72/96 dB/oct
  (effective two-pass slopes). Elliptic ripple and stopband attenuation are editable.
- **Auto:** IIR high-pass below (or at/below) the crossover; FIR elsewhere.
- **FIR:** Hamming or Kaiser, automatic/manual transitions, EVA or EEGLAB/MNE
  cutoff conventions, and zero-phase two-pass, delay-compensated single-pass,
  or causal forward application. Kaiser attenuation is editable.
- **Presets:** EEGLAB, ERPLAB, MNE-Python, and EGI Net Station mechanics ported
  from EVA. These are approximations, not dependencies on those packages or
  claims of bit-for-bit equivalence. Existing cutoffs, reference, and line settings
  are preserved when applying a preset.
- **Precision:** Float and Double recursive arithmetic; Auto conservatively uses
  Double. FIR uses Double in every mode, as in EVA. Unsafe Float designs are
  rejected rather than silently emitting unstable results.
- **Average reference:** applied after filtering to EEG only. All EEG channels,
  including hidden traces, contribute unless explicitly excluded. Units must be
  known and identical; nonfinite reference samples propagate gaps. Mean buffers
  are cached for one bounded tile, independent of which channels are displayed.
- **Notch:** IIR with adjustable Q or FIR (1 Hz stopband per harmonic), at the
  selected frequency and harmonics. FIR notch uses the FIR window/application.
- **CleanLine (EVA):** ports the existing EVA sinusoid-regression mechanics:
  demeaned windows, sine/cosine least squares, explained-energy gating, 50%
  overlap, Hann-weighted correction, configurable window/strength, and sequential
  harmonics. The gate is `max(0.0002, 0.002 / max(strength, 0.1))`. Unlike
  [EEGLAB CleanLine](https://eeglab.org/plugins/cleanline/), this does not use
  multitapers, an F-test, or a frequency drift search. It fits the selected fixed
  frequencies while allowing amplitude/phase to vary across windows.

Notch and CleanLine are alternative line-noise modes and can be combined with
IIR or FIR passband filtering. IIR sections (including notch, if selected) run
first, followed by FIR edges when selected, then CleanLine, then EEG average reference. `eva-mff` remains a
standalone reader; display-filter DSP and MRI correction live in `eva-core`.

Validation rejects invalid cutoffs/order, reversed passbands, harmonics at or
above Nyquist, FIR transitions outside DC–Nyquist, excessive tap counts, and
excessive boundary context. Designs are never silently shortened. Limits are
32,767 FIR taps and 1,048,576 context samples per side, with additional alignment
for CleanLine windows. Very narrow high-pass/transition settings may require IIR
or a less aggressive design.

Filtering uses **fixed recording-aligned tiles**, not viewport-relative windows.
Each tile is computed with neighboring samples and reflection at recording/gap
edges, then cropped. IIR context is based on pole decay to `1e-10`; this is a
finite-context approximation to whole-recording forward/backward filtering,
not bit-for-bit parity with EVA's whole-array implementation. Boundary transients
can remain near file edges and nonfinite gaps. CleanLine regression windows align
to the recording sample grid. A sample is processed identically when read for a
trace, at another zoom, or for the cursor; timestamps are not shifted. Causal FIR intentionally retains its group delay.

NaN/infinity split independent finite runs; no recursive state crosses a gap.
Runs shorter than three samples are blanked when filtering is active; CleanLine
skips runs shorter than 16 samples. The original calibrated values return exactly
when bypassed. Processing is bounded per tile/channel; FFT scratch and padded
buffers add memory beyond the reduced display cache. Cancellation is checked
between tiles, sections, convolutions, and regression windows.

Reference: EVA `EVA/Filtering/EEGSignalFilter.swift` at commit
`2c941d08e12f9baafde213d711570bfde2508139`; biquad equations follow the
[W3C Audio EQ Cookbook](https://www.w3.org/TR/audio-eq-cookbook/).
Tests check pass/stop bands, line/harmonic suppression, zero-phase impulse
alignment, missing data, fixed-tile/cursor consistency, bypass restoration, and
rejection of invalid designs. Native interaction and Linux graphics verification
remain pending; these tests verify DSP and headless UI integration.

## Loading and rendering

A single worker owns disk reads and display reduction. A one-slot request mailbox
keeps only the latest pending request; generation checks discard stale results.
Reduction checks cancellation between chunks. MFF opening/indexing itself cannot
be interrupted mid-call, but its result is discarded after cancellation or replacement.

Only visible channels are read, one at a time in 16,384-sample tiles. Active filters
read additional neighboring context as described below.
The retained display is capped at 2,048 buckets per channel and 128 channels. At
fine zoom it renders sample traces; at coarse zoom it renders separate min/max
vertical envelopes so narrow spikes remain visible. Buckets containing NaN or
infinity are left blank, and fine traces never connect across these gaps.

The current reduced view is reused for amplitude changes and repainting. Navigation
requests a new reduction; there is no recording-wide cache or precomputed overview
pyramid. Long windows may require substantial I/O. The old view is cleared while a
new window is loading so stale data cannot be drawn under updated axes. Metadata
and block-index overhead remain governed by `eva-mff` limits. Cached repaint memory
is bounded by the display limits; no full-recording sample upload is performed.

A maximum of 250 event markers and 12 marker labels are drawn per window to bound
clutter and drawing work; the event list remains complete. Use **EEG stream / PNS stream** in the toolbar to switch streams while preserving
view time in seconds. PNS uses its own sampling rate, duration, channel names,
units, calibration, and polarity. **Filter PNS** controls whether the same temporal
filter settings apply there; settings are validated against both sampling rates.
PNS never participates in EEG average reference. Streams are displayed separately;
no resampling or acquisition timing correction is invented. Recording pauses remain blank on the acquisition timeline, preserving event positions.
Segmented and averaged recordings show metadata instead of flattened waveforms.
MRI correction accepts recordings with pauses: gaps remain blank, acquisition
epochs crossing gaps are skipped, and ANC resets on each finite segment.

## MRI gradient correction

**MRI** opens a focused dialog. Each algorithm and settings section has a **?**
button: hover for help or click to keep the explanation open. Select the volume
marker (default `TREV`), verify the reported count/TR, configure acquisition groups,
then choose **Apply MRI Correction**. Work runs on the reader worker with progress
and cancellation. Reapplying always starts from calibrated source samples. A failed
or cancelled job retains the last completed correction. **Bypass correction** and
**Remove correction** return to source samples; temporal display filters remain
independent. Closing/reopening a recording releases correction data.

Implemented choices follow the macOS controls:

- **Allen AAS:** fixed sections, running-template correlation gate, initial accepted
  epochs, volume or slice/group timing. The deprecated Fast AAS is not offered.
- **FASTR:** temporal donor templates with common alignment, scale fitting, optional
  OBS and ANC. **FARM** substitutes correlation-ranked donors with reported temporal
  fallback; optional squared-correlation ranking permits self-donation.
- **Moosmann:** motion-informed neighborhoods and high-motion donor exclusion,
  reported barrier relaxation when insufficient neighbors remain. Requires motion.
- Slices/multiband, direct group count, or explicit offsets; BIDS `SliceTiming`
  JSON import sorts and deduplicates simultaneous acquisitions. Trigger position
  and internal upsampling are configurable.
- Scanner-slaved, independent-clock or custom alignment; unscaled, median-smoothed
  drift-tracking or per-epoch least-squares scaling, with a bounded fitted range.
- OBS off/automatic/fixed components, variance target, chunk duration, basis-epoch
  cap and residual-energy floor. ANC tap count, NLMS step and high-pass policy.
- Explicit six-column AFNI/FSL/SPM motion formats, L1 translation, Euclidean
  translation or six-parameter FD metrics, threshold and head radius. Short files
  are front-padded and longer files truncated with warnings, matching EVA's rule.
- EEG exclusions and template-only channels; optional PNS correction on its own
  sampling grid. MRI output feeds the same waveform/cursor path and then filtering.

This is a **first CPU port**, not a verified replacement for the macOS pipeline.
No Metal/wgpu compute backend, processing-history export, automatic unreliable-span
annotation events, or run-grade plots are included yet. Diagnostics report skipped
channel-epochs, uncovered samples, correlation fallbacks, motion-barrier crossings,
rejected scale fits, OBS counts and ANC use. Numerical parity across real scanner
recordings remains to be established; alignment interpolation and the OBS eigensolver
are native Rust implementations and are not bit-identical to Swift/Accelerate.

The current job loads up to **512 MiB of combined EEG/PNS float32 input** and retains
corrected arrays in memory. Additional workspace and the previous correction consume
memory too; this is not a total-memory ceiling. Per-channel aligned-epoch workspace
is limited to 16 million values; upsampling raises that cost. Oversized jobs fail
explicitly. Disk-backed and parallel/GPU correction remain future work.
Nonfinite epochs do not donate; missing-trigger gaps are not filled with invented
triggers, and uncovered samples remain unchanged. Source files are never written.

## Try a synthetic recording

```sh
python3 scripts/make-demo-mff.py target/demo.mff
cargo run -p eva-viewer -- target/demo.mff
```

The generator refuses to overwrite an existing directory. It creates 32 channels
at 1 kHz for 60 seconds, with six event-linked spikes and one nonfinite gap. This
is synthetic data, not a participant recording.

## Validation status

- Local macOS build and native launch succeeded: wgpu reported Metal on Apple M3 Ultra.
- Headless egui tests exercise loading, waveform geometry generation, navigation,
  exact sample reads, hidden channels, panel resizing, replacement, errors, and close.
- Reduction tests check extrema, missing-data buckets, and cancellation.
- Reader tests, Clippy, and fixture hash checks are included in repository validation.
- Computer Use permissions were unavailable, so native visual inspection and
  physical mouse/trackpad/Retina checks remain manual. Headless tests do not prove
  native rendering quality or frame rate.
- Linux build/tests are configured in CI but have not been executed locally.
  Linux graphics, portal behavior, platform minimums, and the roadmap's 30 FPS
  target remain to be measured on target machines.

## Command-line filtering

The filter dialog's **Copy CLI command** button copies every draft setting as an
`eva filter` command. Replace the quoted input/output placeholders. The CLI and
viewer use the same `eva-core::filter` implementation and fixed processing tiles.
Build with `cargo build -p eva-cli`; see `../eva-cli/README.md` for export details.
When built, the bundle script includes `Contents/MacOS/eva` inside the macOS app.

## Channel actions

- Left-click a channel label to hide/show its trace; its row and label remain.
- Right-click an EEG label for **Interpolate** or **Mark bad / Unmark bad**.
  **Remove interpolation** restores the channel's unrepaired signal.
- Interpolated labels show `[I]`; explicitly marked channels show `[bad]`.
  Bad marks are independent of repair, so explicitly marked repaired channels
  remain in the reported bad-channel list until unmarked.
- On normal quit or recording close/replacement, marked EEG channels print to
  standard output as a sorted, comma-separated **1-based** list; recording identity
  is printed to standard error. Launch from a terminal to capture this output.

Interpolation uses `eva-core` Perrin splines and requires MFF scalp coordinates.
Bad and interpolated channels never donate. Hidden good channels can still donate.
Replacement samples are evaluated after current MRI correction, display filters,
and reference, so changing upstream settings never leaves stale repaired traces.
Bad/repaired channels are excluded from EEG average reference. Decisions are
session-local; CLI interpolation exports repairs and provenance into a new MFF.

MRI correction is also available headlessly through `eva mri -help`. It shares
the viewer's recording-level engine and writes a new MFF plus processing history.

## PSA

The **PSA** toolbar button opens a focused configuration dialog for EEG segmentation,
threshold rejection, global/per-epoch spherical-spline interpolation, baseline,
average reference and category means. It processes the current signal snapshot in
a cancellable worker; opening another recording discards stale work. Hover the
question marks for units, donor rules and threshold behavior. The result window
selects individual accepted epochs or averages, shows rejection reasons, and
exports a new `.psa` analysis directory. Averages retain their contributing epochs.

This first port does not yet reopen saved PSA bundles or implement macOS's Decision
Matrix, eye detectors or segmented-MFF export. See the CLI README for the exact
shared options, limitations and binary layout. PSA does not modify the source MFF
or replace the continuous waveform used by other tools.
