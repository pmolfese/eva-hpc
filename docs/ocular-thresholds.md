# Ocular threshold port

Source behavior: Swift EVA `EVA/Epoching/EyeArtifactThresholdConfiguration.swift`,
`EyeArtifactThresholdDetector.swift`, and `EyeArtifactThresholdSheet.swift`,
inspected 2026-10-22. Rust implements threshold detection only. No templates,
Define Artifact, classification, or cleaning algorithms are enabled.

Defaults and the HydroCel / GSN 200 ocular tables are ported directly. Legacy
mode takes the signed largest absolute channel sample (first wins ties).
Derived blinks require nonzero same-sign left/right means and average them.
Derived movements use left-minus-right only where its magnitude exceeds the
common-mode mean. Threshold crossings are inclusive; nearest-sample rounding,
merge-before-gating, maximum-duration-safe merge chains, first strongest peak,
and derivative scaling match Swift. Merge distance is measured from the previous
last crossing to the next first crossing, not the count of intervening samples.
A zero merge setting is clamped to a one-sample distance, as in Swift.
The rise gate measures first crossing to peak, despite Swift's baseline-to-peak
caption. Output exposes exact [start,end) bounds separately from peak time.

Rust-specific safeguards:

- GCAL-calibrated V/mV/µV are converted to µV before detection; unknown units fail.
- Acquisition gaps and any nonfinite selected-channel sample break runs and
  prevent merging, including when a supplied MRI snapshot contains gap zeros.
- Invalid, duplicate, or unavailable override channels fail explicitly. Derived
  mode requires two nonempty groups instead of silently reverting to legacy.
- Unknown montage selection retains Swift's first-four-channel fallback, with a
  visible warning. Verify these electrodes; automatic selection is not anatomical
  inference from coordinates.
- The drive has a 256 MiB limit; threshold-run and candidate counts are bounded.
  Processing is cancellable and reads selected channels in bounded chunks.
- The viewer retains the last completed detection snapshot; changing processing
  does not silently recompute it. Rerun detection to update marks.

## Verification

Executed the original Swift detector/configuration files in a standalone Swift
harness, stubbing only the MFF event value type. At 250 Hz, a two-channel input
with bilateral 200 µV samples 100–114, bilateral 350 µV samples 130–144, and
opposing ±200 µV samples 500–549 produced:

| Mode | Kind | Start/end seconds | Peak seconds | Peak µV |
| --- | --- | --- | --- | --- |
| Legacy | Blink and movement | 0.4–0.58; 2.0–2.2 | 0.52; 2.0 | 350; 200 |
| Derived | Blink | 0.4–0.58 | 0.52 | 350 |
| Derived | Movement | 2.0–2.2 | 2.0 | 400 |

Rust regression tests reproduce these outputs through the MFF reader with mV
units and nonidentity GCAL. Other tests cover amplitude caps, polarity, rise and
derivative gates, split/merged durations, nonfinite/gap barriers, snapshot gap
masking, all eight net table variants, malformed options, no-overwrite reports,
source immutability, and the viewer sheet/worker/close lifecycle.

An unfiltered NNU recording was also processed successfully as a CLI smoke test.
This verifies execution and reporting, not physiological accuracy of candidates
in raw MRI-contaminated data. Preprocess before interpreting detections.
