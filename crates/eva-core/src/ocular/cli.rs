use super::*;
use std::collections::BTreeMap;
pub const HELP: &str = "Usage: eva artifactdetect [OPTIONS] <input.mff> <output.json>

Threshold detection only; does not clean EEG or modify the input MFF.
Detects Eye Blink (vertical/common-mode) and Eye Movement (horizontal/opponent).
Options use a single dash. Each setting has a blink- or movement- prefix:
  -blink true|false             Enable blinks (default true)
  -movement true|false          Enable movements (default true)
  -blink-minimum 150            Minimum amplitude in µV
  -blink-maximum 0              Maximum peak µV; 0 means no cap
  -blink-polarity bipolar       positive|negative|bipolar
  -blink-topology legacy        legacy|derived (VEOG/HEOG)
  -blink-channels 37,241,32,18,238,25  One-based channels; auto by default
  -blink-rise 0                 Crossing-to-peak limit (s); 0 unconstrained
  -blink-min-duration 0.05      Minimum merged duration (s)
  -blink-max-duration 1         Maximum duration (s); 0 means no cap
  -blink-merge-gap 0.25         Merge neighboring threshold runs (s)
  -blink-velocity-enabled false
  -blink-velocity 5             Minimum peak first difference (µV/ms)
  -blink-acceleration-enabled false
  -blink-acceleration 2         Minimum peak second difference (µV/ms²)
All blink-* flags also accept movement-*; movement velocity/acceleration
thresholds default to 3 and 1. Other defaults match. Channels 'auto' uses the
Swift EVA EGI table, falling back to the first four channels for unknown nets.
Derived overrides split in user order: first floor(n/2) left, remainder right.
Legacy uses the signed sample with largest absolute magnitude across channels.
Derived blinks use same-sign bilateral means; movements use left minus right
when opponent magnitude exceeds common-mode magnitude. No DC removal is added.
Gaps/nonfinite data are barriers and are never bridged. JSON reports exact
[start,end) intervals and peak times separately, settings, and resolved channels.
CLI scans calibrated samples on disk; preprocess with eva filter/mri first to
match a GUI processing snapshot. Output must be new and outside the input MFF.
Use -- before paths beginning with '-'. No template/Define Artifact features.

Example: eva artifactdetect -blink-minimum 100 -movement-minimum 125 input.mff eyes.json";
fn boolean(s: &str) -> Result<bool, String> {
    match s {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("Use true or false".into()),
    }
}
pub fn channels(s: &str) -> Result<Vec<usize>, String> {
    if s.trim().is_empty() || s == "auto" {
        return Ok(vec![]);
    }
    s.split(',')
        .map(|p| {
            p.trim()
                .parse::<usize>()
                .ok()
                .filter(|v| *v > 0)
                .ok_or("Use comma-separated positive channel numbers".into())
        })
        .collect()
}
pub fn parse(options: &BTreeMap<String, String>) -> Result<Settings, String> {
    let mut settings = Settings::default();
    for (key, value) in options {
        if key == "blink" {
            settings.blink.enabled = boolean(value)?;
            continue;
        }
        if key == "movement" {
            settings.movement.enabled = boolean(value)?;
            continue;
        }
        let (prefix, field) = key
            .split_once('-')
            .ok_or_else(|| format!("Unknown option -{key}"))?;
        let c = match prefix {
            "blink" => &mut settings.blink,
            "movement" => &mut settings.movement,
            _ => return Err(format!("Unknown option -{key}")),
        };
        let num = || {
            value
                .parse::<f64>()
                .map_err(|_| format!("Invalid number for -{key}"))
        };
        match field {
            "minimum" => c.minimum_uv = num()? as f32,
            "maximum" => c.maximum_uv = num()? as f32,
            "rise" => c.rise_seconds = num()?,
            "min-duration" => c.min_seconds = num()?,
            "max-duration" => c.max_seconds = num()?,
            "merge-gap" => c.merge_seconds = num()?,
            "velocity" => c.velocity_uv_ms = num()? as f32,
            "acceleration" => c.acceleration_uv_ms2 = num()? as f32,
            "velocity-enabled" => c.velocity_enabled = boolean(value)?,
            "acceleration-enabled" => c.acceleration_enabled = boolean(value)?,
            "channels" => c.channels = channels(value)?,
            "polarity" => {
                c.polarity = match value.as_str() {
                    "positive" => Polarity::Positive,
                    "negative" => Polarity::Negative,
                    "bipolar" => Polarity::Bipolar,
                    _ => return Err("Use positive, negative or bipolar".into()),
                }
            }
            "topology" => {
                c.topology = match value.as_str() {
                    "legacy" => Topology::Legacy,
                    "derived" => Topology::Derived,
                    _ => return Err("Use legacy or derived".into()),
                }
            }
            _ => return Err(format!("Unknown option -{key}")),
        }
    }
    settings.blink.validate()?;
    settings.movement.validate()?;
    Ok(settings)
}
