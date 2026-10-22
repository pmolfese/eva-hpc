# PSA performance check (2026-10-22)

Both the GUI and CLI use `eva-core::psa::run`. This update batches unfiltered
channel reads, caches canonical filtered tiles within one immutable PSA snapshot
(up to 64 MiB), and traverses contiguous channel samples during reference and
averaging. Summation order is preserved. No rejection, gap, baseline, or
interpolation settings are relaxed. Large bulk reads fall back to individual
channels when the reader's allocation limit requires it.

A local macOS smoke benchmark used NNU_04_04real_20251211_111439.mff:
257 channels, 36 Shap/Fear epochs, -0.2 to +0.8 seconds, baseline correction,
average reference, global interpolation of channel 1, and two category averages.
Thresholding was disabled to isolate read/processing costs; this does not validate
artifact rejection quality. The filtered variant enables default 1–40 Hz IIR.

| Compute path | Before (debug) | After (debug) | After (release) |
| --- | ---: | ---: | ---: |
| No temporal filter | 1.675 s | 0.573 s | 0.049 s |
| Temporal filter enabled | 55.349 s | 25.913 s | 8.441 s |

Single wall-clock observations, not a controlled benchmark distribution. Timings
exclude MFF open, output export, GUI rendering, and preceding MRI correction.
Other recordings, filter settings, storage, and bad-channel patterns will differ.
Epoch and average sample-bit checksums matched before/after and across builds:
`05e35c688d3d3a06` without filtering, `b94a792269f12849` with filtering.

Reproduce with the supplied recording (not distributed in this repository):

```sh
cargo run --release -p eva-core --example benchmark_psa -- /path/to/recording.mff
cargo run --release -p eva-core --example benchmark_psa -- /path/to/recording.mff filtered
```

The example deliberately selects Shap/Fear and interpolates channel 1; it is a
specific workload, not a general-purpose PSA command. Use `eva psa -help` for
normal processing. Use release binaries for performance measurements.
