# EVA-HPC roadmap

Created: 2026-10-21  
Status: Stage 1 reader implemented and tested on macOS; Linux/MSRV verification pending CI. Stage 2 viewer implemented; macOS Metal launch and headless UI tests pass. Native visual/input checks and Linux verification remain pending.

## Immediate goal

Build a Rust MFF reader and get a waveform viewer working promptly. Develop and test primarily on macOS, with macOS and Linux support from the start.

The first two stages are fixed:

1. **`eva-mff`: build the framework for native EGI MFF reading.**
2. **`eva-viewer`: immediately use that reader to display and navigate EEG waveforms with egui and wgpu.**

Do not wait for comprehensive MFF variant coverage or a general-purpose I/O framework before building the viewer. Finish a correct continuous-EEG reading path, then put real waveforms on screen.

## Crate boundaries

| Crate | Responsibility | Dependencies and consumers |
| --- | --- | --- |
| `eva-mff` | MFF package metadata, stream/channel/event types, binary block indexing, and bounded sample reads | Standalone Rust library; no egui, wgpu, viewer, or Python runtime dependency. Intended for EVA, Sumaru, and other software. |
| `eva-viewer` | Application shell, loading state, waveform rendering, navigation, and event inspection | Uses `eva-mff`, egui, and wgpu. Owns display state and background work. |

Keep the reader and viewer in one Cargo workspace initially. Design `eva-mff` so another Rust application can depend on it without pulling in the viewer or its graphics dependencies. MFF-specific public types live in `eva-mff`; no speculative general-format abstraction is needed. A focused `eva-core` was subsequently added for the requested MRI processing port (below).

`eva-io`, if built later, will handle formats other than MFF. It will not require egui or wgpu. Its API, dependencies, relationship to `eva-mff`, and format coverage are decisions for later, not prerequisites for this work.

The existing EVA code and tests are implementation references. Do not assume EVA relies on MNE-Python: the user has identified that statement in its README as incorrect. No edits to the original EVA README are part of this task.

## Stage 1 — MFF reading framework

Deliver a small, reusable native Rust reader with enough validated functionality to power Stage 2.

- [x] Scaffold a Cargo workspace containing `eva-mff` and `eva-viewer`, select a stable Rust toolchain, and establish macOS/Linux build and reader-test checks.
- [x] Inspect EVA's current MFF implementation and regression tests; record the reference commit and select a small fixture corpus with documented provenance and redistribution rights.
- [x] Define a minimal public API: open an MFF directory, inspect recording metadata, list streams/channels/events, and read selected channels over a half-open sample range.
- [x] Discover the EEG stream through metadata associations rather than assuming the first binary file is always EEG.
- [x] Parse required XML and index binary blocks, including reused headers, offsets, sample counts, byte order, and supported sample encodings. Validate sizes and arithmetic before allocating or seeking.
- [x] Implement bounded reads across block boundaries, including the first and last samples, without decoding the entire recording into memory.
- [x] Expose channel labels, sample rate, sample count, units, recording start time, and available acquisition-reference information. Keep raw values, calibration, and display scaling distinguishable.
- [x] Preserve event timestamps without millisecond truncation and define timestamp-to-sample rounding. Review acquisition-specific timing corrections against EVA's tests and report any applied correction explicitly.
- [x] Detect unsupported stream layouts, discontinuities, segmented data, and averages. Return structured errors or capability warnings rather than silently treating them as continuous EEG.
- [x] Add fixture tests for metadata, exact sample windows, cross-block reads, event timing, and malformed/truncated input. Compare expected results with EVA and independently established fixture values where available.
- [x] Provide a small standalone Rust example that opens a recording and prints metadata and a sample window, without creating a display or GPU context.

**Validation status:** local tests, lint, documentation, fixture hashes, and an external Rust consumer are checked during Stage 1 delivery. macOS/Linux CI is configured; Linux and minimum-toolchain execution remain unverified locally.

**Stage 1 exit:** a supported continuous EEG MFF recording can be opened and read accurately through the public Rust API on macOS and Linux. A separate consumer can use the reader without graphics dependencies. This is the trigger to proceed directly to Stage 2, not a requirement to finish every MFF variant or publish a crate first.

### Reader contracts

- Keep the API synchronous and independent of an async runtime; callers choose their own worker/thread model.
- Preserve stable channel identities separately from display order, and represent sample rates per stream.
- Document sample layout, units, calibration, timestamp precision, and error behavior. Never fabricate missing recorded channels or guess unknown units.
- Keep metadata/index memory separate from a bounded sample buffer. Begin with buffered range reads; assess memory mapping only if measurements justify it.
- Open source packages read-only. Do not write caches or application state into recordings.
- Keep application settings, selections, rendering caches, and processing pipelines out of the crate.

## Stage 2 — Working waveform viewer, immediately next

`eva-viewer` is the egui/wgpu application. This stage is a near-term deliverable, not deferred work.

Start with `eframe` using its wgpu renderer: Metal on macOS and initially Vulkan on Linux. Pin a compatible egui/eframe/wgpu dependency set. These frameworks support the intended platforms; validate the actual target machines and graphics drivers during implementation. References: [egui](https://github.com/emilk/egui), [wgpu](https://wgpu.rs/).

### First visible slice

- [x] Launch on macOS and report Metal / Apple M3 Ultra.
- [ ] Verify Linux launch and graphics adapter.
- [x] Open the validated Stage 1 MFF fixture from a command-line path.
- [x] Read a bounded EEG window on a worker thread and draw stacked traces with channel labels, a time axis, and an amplitude scale.
- [x] Add horizontal navigation, time zoom, amplitude scaling, and vertical channel scrolling.

**First visible slice exit:** real MFF waveforms are on screen and navigable. Complete this before adding secondary UI features or expanding format support.

### Complete the initial viewer

- [x] Add an MFF directory picker, loading/error state, and the ability to replace or close a recording.
- [x] Show recording name, duration, sampling rate, channel count, units, polarity convention, and relevant reader warnings.
- [x] Add event markers and a simple event list with jump-to-event navigation.
- [x] Add a cursor showing time, sample index, channel, and exact sample value; do not report reduced display buckets as measurements.
- [x] Add channel hiding and reset-to-default controls.
- [x] Use min/max envelopes at coarse zoom to retain narrow extrema and raw sample traces at fine zoom. Do not connect traces across invalid or missing spans.
- [x] Bound in-memory sample/display caches and background requests. Cancel obsolete work where practical and discard stale results after navigation or recording changes.
- [ ] Verify Retina/HiDPI scaling and trackpad/mouse controls on macOS, and the intended Linux display environment.

Use egui drawing primitives over reduced waveform data first. A custom wgpu waveform pipeline is an optimization to evaluate only if profiling shows it is needed; wgpu already backs the application renderer.

**Implementation validation:** reader and viewer unit/integration tests and headless egui smoke tests cover waveform generation, navigation, exact samples, replacement/cancellation, and unsupported recordings. The native app launches with Metal. Computer Use permissions prevented native visual inspection; Retina/input testing, Linux launch, and frame-rate targets remain open.

**Stage 2 exit:** on macOS and Linux, a user can open a supported continuous MFF recording, view and navigate its EEG traces, jump to an event, inspect a sample, and close the recording without modifying the source.

## Initial validation and delivery

These checks support the first reader/viewer delivery; they must not turn into a broader analysis-platform project.

- [ ] Run the same reader fixtures on macOS and Linux; sample values and timing must agree across platforms.
- [ ] Test truncated blocks, invalid sizes/offsets, missing required metadata, unsupported encodings, empty event lists, and inconsistent stream layouts without panics or uncontrolled allocation.
- [ ] Include sub-millisecond event timing and cross-block cases from EVA's existing regression intent, especially at 1024 Hz.
- [ ] Verify the full open → navigate → event jump → inspect sample workflow on both platforms with real graphics hardware; headless CI alone does not verify rendering.
- [ ] Exercise rapid navigation and repeated open/close, ensuring stale reads cannot replace the current view and resources are released.
- [ ] Verify a recording larger than the configured sample cache can be navigated without a full-recording allocation.
- [ ] Measure cached navigation on named macOS/Linux machines. Initial target: at least 30 FPS with 128 visible channels over a 10-second window at 1 kHz, with interaction feedback under 100 ms. These are proposed targets, not existing performance claims; report hardware, viewport, and measured results.
- [ ] Document how to build/run the viewer and use the reader, supported MFF variants, known limitations, and required platform libraries. Confirm minimum OS versions and architectures against actual test machines.

Initial testing is macOS-first, with Apple silicon macOS and Linux x86_64 as proposed baseline targets. Confirm the user's actual hardware during setup. Linux validation remains part of the first delivery; additional architectures require explicit testing.

## Filtering increment

Implemented after the initial viewer: enlarged FILTER modal with Butterworth/elliptic
IIR, independent slopes, Auto crossover, Hamming/Kaiser FIR, three FIR application
modes, IIR/FIR notch, EVA-style CleanLine, package presets, precision, and EEG average
reference with exclusions. Preview/Cancel/Apply/Remove controls; fixed padded processing tiles shared by waveform and cursor; sample-rate
validation and synthetic DSP tests. Source recordings remain unchanged. See the
viewer README for algorithm definitions and the finite-context limitations.

## PNS increment

Implemented standalone PNS reads with independent rate/count, labels, units, polarity,
and GCAL. The viewer switches EEG/PNS streams at the same view time and optionally
filters PNS at its own rate; average reference remains EEG-only. Simultaneous mixed-rate
stream display and broader acquisition variants remain future work.

## MRI correction increment

First CPU implementation of Allen AAS, FASTR, FARM and Moosmann in the standalone
`eva-core` crate. MRI modal includes macOS-style algorithm help, acquisition timing,
BIDS SliceTiming import, alignment/scaling, motion censoring, OBS, ANC and channel
scope. Correction precedes display filtering and can include PNS at its own rate.
Synthetic engine and end-to-end viewer tests cover removal, preservation, donor
rules, timing, cancellation, bypass and immutable sources. Real-scanner numerical
parity, GPU acceleration, disk-backed jobs, run-grade plots, unreliable-span event
annotations and history export remain pending. See the viewer README for limits.

## Decide later

Everything here is outside the critical path for Stages 1 and 2.

- **Broader MFF coverage:** discontinuous timelines, segmented recordings, averages, richer sensor metadata, and acquisition-specific variants beyond the initial fixture set. Detect unsupported cases now; expand support later.
- **`eva-io`:** a separate library for non-MFF formats. Decide its structure and supported formats when there is a concrete next-format requirement. No egui/wgpu dependency.
- **Shared abstractions:** decide whether common cross-format types or traits are useful once another reader exists. No speculative shared core now.
- **Sumaru integration:** use `eva-mff` directly if suitable for its implementation; inspect its language and needs before choosing bindings or another interface. A standalone Rust consumer example is sufficient for the first crate boundary check.
- **Public crate release:** package naming/availability, versioning policy, publishing, and external documentation. Keep the crate reusable now without blocking the viewer on publication.
- **Standalone CLI:** a full inspection/validation command suite, JSON schema, or batch interface. Stage 1 only needs the small reader example.
- **Viewer extensions:** topographies, persistent annotations, advanced event tools, and broader EVA interface parity.
- **Analysis and writing:** additional filter designs/presets and whole-recording processing, rereferencing, artifact processing, ICA, epoching, MFF export, and other analysis features.
- **Advanced HPC work:** scheduler integration, remote visualization deployment, shared-filesystem tuning, GPU compute, and distributed processing.
- **Distribution and optimization:** installers, signing/notarization, wider architecture coverage, persistent caches, memory mapping, and custom GPU waveform rendering.

## Existing EVA references

Paths refer to the sibling `../eva` checkout. These are reference material, not instructions governing this repository.

- `../eva/EVACore/IO/MFFReader.swift`: package discovery, decoding, metadata, timing, PNS, and epochs.
- `../eva/EVACore/IO/MFFFileType.swift`: recording classification.
- `../eva/EVA/IO/MFFRecording.swift`: application integration reference.
- `../eva/EVA/Waveform/`: waveform controls, scales, channels, events, and layout behavior.
- `../eva/EVATests/IO/MFFEventPrecisionTests.swift`: timestamp precision regressions.
- `../eva/EVATests/IO/MFFPNSReaderTests.swift`: PNS stream behavior.
- `../eva/EVATests/IO/MFFFileTypeTests.swift` and `MFFAveragedRoundTripTests.swift`: classification and unsupported-variant detection references.
- `../eva/EVATests/Fixtures/`: candidate reference recordings; preserve provenance and review redistribution rights before copying.

## CLI filtering increment

- `eva filter` shares `eva-core::filter` with the viewer; no graphics runtime.
- Full filter option help and GUI Copy CLI command.
- New MFF output preserves block layout, gaps, events, calibration and PNS.
- macOS-shaped `eva.xml` history plus audit log; cross-engine replay remains
  disabled until numerical parity is validated. Existing destinations are refused.
- Other CLI processing commands and additional output formats remain future work.

## CLI MRI increment

- `eva mri` exposes Allen AAS, FASTR, FARM, and Moosmann with the viewer's timing,
  template, OBS, ANC, motion, and PNS options.
- Recording-level MRI processing now lives in eva-core and is shared with the GUI.
- New MFF exports append normalized settings, motion data and diagnostics to
  eva.xml and an audit log; input-size limits remain the same as the viewer.
