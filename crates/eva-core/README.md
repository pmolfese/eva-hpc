# eva-core

GUI-independent Rust processing with no egui, wgpu, Python, or platform-framework
dependency. `gradient::{Config, correct, ResultData}` implements MRI correction.
`filter::{Settings, Plan}` is the shared GUI/CLI filter engine; it uses `eva-mff`
for bounded recording reads and `rustfft` for FFT filtering. `filter::cli` defines
the command options and the GUI's Copy CLI command representation.

`correct` accepts immutable channel-major calibrated float32 samples, volume-trigger
sample indices, sampling rate, settings, a cancellation callback and a progress
callback. It returns corrected samples and diagnostics. Motion parsing normalizes
explicit AFNI/FSL/SPM six-column rows to rotation radians and translation mm.

The source reference is the user's macOS EVA `EVA/Gradient` implementation:
GradientAAS, GradientTemplateCorrector, GradientDonorSelection, GradientEpochLayout,
GradientEpochAligner, GradientSincResampler, GradientOBS, GradientANC and MotionParameters.
No third-party artifact-removal implementation was used. The source carries NIH
US-government-work provenance; retain that context when preparing a public release.

This first CPU implementation follows EVA's method choices, not bit-exact Swift
arithmetic. It uses native Lanczos interpolation, deterministic power iteration with
deflation for OBS (with convergence checks), and normalized LMS for ANC. Overlapping
artifact estimates are averaged; uncovered original samples are unchanged. OBS/ANC
are off by default. Invalid geometry, missing required motion and oversized jobs
produce errors rather than silently falling back to a different algorithm.
See the viewer README for the current parity boundaries and memory limits.

`interpolation::recipes` computes Perrin spherical-spline weights from MFF scalp
geometry (order 4, 40 terms, lambda 1e-5), following macOS EVA's SphericalSpline
and ChannelInterpolationSolver. Targets and bad donors are excluded. The shared
filter Plan evaluates these recipes after filtering/reference, using independent
original donors. The CLI can selectively export repaired channels while preserving
all other binary channel samples. Missing donor samples propagate as NaN.

`mri::{Request, run, Corrected}` provides the shared recording-level MRI pipeline
for both the GUI and CLI: calibrated loading, volume marker selection, independent
PNS timing, bounded whole-recording correction, and per-stream diagnostics.
