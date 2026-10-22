use super::Config;
use std::collections::BTreeMap;
pub const HELP: &str = r#"Usage: eva psa [OPTIONS] <input.mff> <output-directory>

Event-locked EEG segmentation and category averaging. Options use one dash.
All booleans take true/false. Writes a new PSA bundle, NOT a segmented MFF:
manifest.json, epochs.f32le, averages.f32le, averages.csv, decisions.csv, eva.xml.
Source MFF is unchanged. No output overwrite. All rejected -> report, no average.

-events CODE,CODE        Required exact event selection
-by-label true|false     Match labels instead of codes (default false)
-categories A=X,B=X      Pool selected codes/labels into named categories
-pre SECONDS             Before anchor (default 0.2)
-post SECONDS            After anchor, exclusive endpoint (default 0.8)
-offset SECONDS          Shift anchors (default 0)
-baseline true|false     Baseline correction (default true)
-baseline-start SECONDS  Relative to anchor (default -pre)
-baseline-end SECONDS    Relative to anchor, exclusive (default 0)
-average-reference BOOL  Good-channel average reference (default true)
-average BOOL            Average accepted trials by category (default false)
-threshold BOOL          Enable channel threshold detection (default true)
-min-uv VALUE            Minimum amplitude, baseline-adjusted if enabled (-150)
-max-uv VALUE            Maximum amplitude (150)
-slope VALUE             Maximum absolute difference, µV/sample (25)
-acceleration VALUE      Maximum absolute second difference, µV/sample² (15)
-max-bad-percent VALUE   Reject if more channels bad than this percentage (10)
-max-bad-count N         Alternative absolute count; exclusive with percentage
-interpolate-epoch BOOL  Spherical-spline repair of transient bad channels (true)
-interpolate-global BOOL Repair all global bad channels in every epoch (true)
-bad-channels 1,8        Known global bad channels, one-based
-escalate BOOL           Promote recurrent bad channels to global (true)
-escalation-percent N    Flagged in >= N percent of measurable candidates (50)
-reject-codes CODE,CODE  Reject epochs overlapping these artifact event codes
-reject-epochs 1,8       Reject one-based candidate numbers in event order
-h, -help               Show help

Bounds, acquisition gaps and nonfinite data always reject; they cannot be repaired
or manually kept. Thresholds require declared V/mV/µV units. Global bad channels
never donate to interpolation or reference; repaired transient channels can enter
reference. Missing geometry rejects affected epochs with a recorded reason.
Order: source -> threshold evidence (baseline-adjusted amplitude) -> escalation
-> rejection/repair -> average reference -> baseline -> category means.
GUI runs on its current MRI/filter snapshot. CLI reads its input samples; run
mri/filter first to reproduce that processing. No Decision Matrix or eye detector.
Examples:
  eva psa -events Shap,Fear -average true input.mff results.psa
  eva psa -events DIN1 -pre 0.1 -post 0.5 -bad-channels 1,8 input.mff results.psa
"#;
pub fn list(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect()
}
pub fn numbers(s: &str) -> Result<Vec<usize>, String> {
    list(s)
        .iter()
        .map(|v| {
            v.parse::<usize>()
                .ok()
                .filter(|v| *v > 0)
                .ok_or("Use comma-separated one-based numbers".into())
        })
        .collect()
}
pub fn parse(p: &BTreeMap<String, String>) -> Result<Config, String> {
    let mut c = Config::default();
    for (k, v) in p {
        let invalid = || format!("Invalid -{k}: {v}");
        match k.as_str() {
            "events" => c.events = list(v),
            "by-label" => c.by_label = v.parse().map_err(|_| invalid())?,
            "categories" => {
                for pair in list(v) {
                    let (a, b) = pair.split_once('=').ok_or("Categories use A=X,B=X")?;
                    if a.is_empty() || b.is_empty() {
                        return Err(invalid());
                    }
                    c.categories.insert(a.into(), b.into());
                }
            }
            "pre" => c.pre = v.parse().map_err(|_| invalid())?,
            "post" => c.post = v.parse().map_err(|_| invalid())?,
            "offset" => c.offset = v.parse().map_err(|_| invalid())?,
            "baseline" => c.baseline = v.parse().map_err(|_| invalid())?,
            "baseline-start" => c.baseline_start = v.parse().map_err(|_| invalid())?,
            "baseline-end" => c.baseline_end = v.parse().map_err(|_| invalid())?,
            "average-reference" => c.average_reference = v.parse().map_err(|_| invalid())?,
            "average" => c.average = v.parse().map_err(|_| invalid())?,
            "threshold" => c.threshold = v.parse().map_err(|_| invalid())?,
            "min-uv" => c.min_uv = v.parse().map_err(|_| invalid())?,
            "max-uv" => c.max_uv = v.parse().map_err(|_| invalid())?,
            "slope" => c.slope = v.parse().map_err(|_| invalid())?,
            "acceleration" => c.acceleration = v.parse().map_err(|_| invalid())?,
            "max-bad-percent" => c.max_bad_percent = v.parse().map_err(|_| invalid())?,
            "max-bad-count" => c.max_bad_count = Some(v.parse().map_err(|_| invalid())?),
            "interpolate-epoch" => c.interpolate_epoch = v.parse().map_err(|_| invalid())?,
            "interpolate-global" => c.interpolate_global = v.parse().map_err(|_| invalid())?,
            "bad-channels" => c.bad_channels = numbers(v)?.iter().map(|v| v - 1).collect(),
            "escalate" => c.escalate = v.parse().map_err(|_| invalid())?,
            "escalation-percent" => c.escalation_percent = v.parse().map_err(|_| invalid())?,
            "reject-codes" => c.reject_codes = list(v),
            "reject-epochs" => c.reject_epochs = numbers(v)?,
            _ => return Err(format!("Unknown PSA option -{k}; use eva psa -help")),
        }
    }
    if p.contains_key("max-bad-count") && p.contains_key("max-bad-percent") {
        return Err("Choose bad-channel count OR percent".into());
    }
    if !p.contains_key("baseline-start") {
        c.baseline_start = -c.pre;
    }
    Ok(c)
}
