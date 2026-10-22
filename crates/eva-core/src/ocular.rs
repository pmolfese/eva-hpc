//! Threshold ocular detection ported from Swift EVA's EyeArtifactThresholdDetector.
//! Detection only: source samples are never modified.
use crate::filter;
use eva_mff::Recording;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
pub mod cli;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Blink,
    Movement,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Blink => "Eye Blink",
            Self::Movement => "Eye Movement",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Polarity {
    Positive,
    Negative,
    Bipolar,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Topology {
    Legacy,
    Derived,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
    pub minimum_uv: f32,
    pub maximum_uv: f32,
    pub rise_seconds: f64,
    pub min_seconds: f64,
    pub max_seconds: f64,
    pub merge_seconds: f64,
    pub velocity_enabled: bool,
    pub velocity_uv_ms: f32,
    pub acceleration_enabled: bool,
    pub acceleration_uv_ms2: f32,
    pub polarity: Polarity,
    pub topology: Topology,
    /// One-based channels in user order. Empty selects the Swift EGI montage table.
    pub channels: Vec<usize>,
}
impl Config {
    pub fn defaults(kind: Kind) -> Self {
        Self {
            enabled: true,
            minimum_uv: 150.,
            maximum_uv: 0.,
            rise_seconds: 0.,
            min_seconds: 0.05,
            max_seconds: 1.,
            merge_seconds: 0.25,
            velocity_enabled: false,
            velocity_uv_ms: if kind == Kind::Blink { 5. } else { 3. },
            acceleration_enabled: false,
            acceleration_uv_ms2: if kind == Kind::Blink { 2. } else { 1. },
            polarity: Polarity::Bipolar,
            topology: Topology::Legacy,
            channels: vec![],
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let numbers = [
            self.minimum_uv as f64,
            self.maximum_uv as f64,
            self.rise_seconds,
            self.min_seconds,
            self.max_seconds,
            self.merge_seconds,
            self.velocity_uv_ms as f64,
            self.acceleration_uv_ms2 as f64,
        ];
        if numbers.iter().any(|v| !v.is_finite() || *v < 0.)
            || self.minimum_uv <= 0.
            || (self.maximum_uv > 0. && self.maximum_uv < self.minimum_uv)
            || (self.max_seconds > 0. && self.max_seconds < self.min_seconds)
            || [
                self.rise_seconds,
                self.min_seconds,
                self.max_seconds,
                self.merge_seconds,
            ]
            .iter()
            .any(|v| *v > 3600.)
        {
            return Err("Invalid ocular settings: finite nonnegative values, positive minimum amplitude, maxima ≥ minima; timing ≤ 3600 s".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub blink: Config,
    pub movement: Config,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            blink: Config::defaults(Kind::Blink),
            movement: Config::defaults(Kind::Movement),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub kind: Kind,
    pub code: String,
    pub start_sample: u64,
    pub end_sample: u64,
    pub peak_sample: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub peak_seconds: f64,
    pub peak_uv: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub format: String,
    pub version: u32,
    pub settings: Settings,
    pub sample_rate: u32,
    pub source: String,
    pub source_processing: String,
    pub blink_channels: Vec<usize>,
    pub movement_channels: Vec<usize>,
    pub warnings: Vec<String>,
    pub candidates: Vec<Candidate>,
}
// Table and fallback selection intentionally match Swift EVA. Entries are one-based.
pub fn auto_groups(kind: Kind, count: usize, name: Option<&str>) -> (Vec<usize>, Vec<usize>, bool) {
    let name = name.unwrap_or("").to_lowercase().replace(['-', '_'], " ");
    let tokens: Vec<usize> = name
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect();
    let n = [32, 64, 128, 256]
        .into_iter()
        .find(|n| tokens.contains(n))
        .unwrap_or(match count {
            33 => 32,
            65 => 64,
            129 => 128,
            256.. => 256,
            _ => count,
        });
    let old = name.contains("gsn 200")
        || name.contains("gsn200")
        || name.contains("geodesic sensor net 200");
    let (left, right, movement): (Vec<usize>, Vec<usize>, Vec<usize>) = match (old, n) {
        (false, 32) => (vec![2, 30], vec![1, 29], vec![31, 32]),
        (false, 64) => (vec![10, 63], vec![5, 62], vec![61, 64]),
        (false, 128) => (vec![25, 127, 21], vec![8, 126, 14], vec![125, 128]),
        (false, 256..) => (vec![37, 241, 32], vec![18, 238, 25], vec![226, 252]),
        (true, 64) => {
            let v2 = ["v.2", "v2", "version 2", "2.0"]
                .iter()
                .any(|s| name.contains(s));
            (
                vec![14, 64, 11],
                vec![1, 63, 6],
                vec![if v2 { 19 } else { 18 }, 60],
            )
        }
        (true, 128) => (vec![26, 127, 33], vec![8, 126, 1], vec![125, 128]),
        (true, 256..) => (vec![36, 242, 45], vec![18, 241, 10], vec![227, 251]),
        _ => {
            let a: Vec<_> = (1..=count.min(4)).collect();
            let mid = (a.len() / 2).max(1).min(a.len());
            return (a[..mid].to_vec(), a[mid..].to_vec(), false);
        }
    };
    if kind == Kind::Blink {
        (left, right, true)
    } else {
        (movement[..1].to_vec(), movement[1..].to_vec(), true)
    }
}
fn resolve(r: &Recording, kind: Kind, c: &Config) -> Result<(Vec<usize>, usize, bool), String> {
    let (mut left, mut right, known) = auto_groups(
        kind,
        r.metadata().channels.len(),
        r.metadata().sensor_layout_name.as_deref(),
    );
    if !c.channels.is_empty() {
        let mid = (c.channels.len() / 2).max(1);
        left = c.channels[..mid].to_vec();
        right = c.channels[mid..].to_vec();
    }
    let mid = left.len();
    left.append(&mut right);
    if left.is_empty()
        || left
            .iter()
            .any(|i| *i == 0 || *i > r.metadata().channels.len())
    {
        return Err(format!(
            "{} ocular channels outside recording; set an explicit channel list",
            kind.label()
        ));
    }
    let mut unique = left.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != left.len() {
        return Err("Ocular channels must not repeat".into());
    }
    if c.topology == Topology::Derived && (mid == 0 || mid == left.len()) {
        return Err("Derived VEOG/HEOG requires channels on both sides (at least two)".into());
    }
    Ok((left, mid, known || !c.channels.is_empty()))
}
fn check(g: &AtomicU64, id: u64) -> Result<(), String> {
    if g.load(Ordering::Relaxed) != id {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}
fn factor(unit: Option<&str>) -> Option<f32> {
    match unit.map(str::trim) {
        Some("uV" | "µV" | "μV") => Some(1.),
        Some("mV") => Some(1000.),
        Some("V") => Some(1e6),
        _ => None,
    }
}
/// Uses the same immutable MRI/filter/interpolation snapshot as the viewer.
pub fn run(
    r: &Recording,
    settings: &Settings,
    filters: &filter::Settings,
    source: Option<Arc<Vec<Vec<f32>>>>,
    g: &AtomicU64,
    id: u64,
    progress: &dyn Fn(f32),
) -> Result<Report, String> {
    if r.is_pns() || r.metadata().kind == eva_mff::RecordingKind::SegmentedOrAveraged {
        return Err("Ocular detection requires continuous/discontinuous EEG".into());
    }
    if !settings.blink.enabled && !settings.movement.enabled {
        return Err("Enable blink or movement detection".into());
    }
    let count = usize::try_from(r.metadata().sample_count).map_err(|_| "Recording too long")?;
    if count > 64 * 1024 * 1024 {
        return Err("Ocular drive exceeds 256 MiB; select a shorter recording".into());
    }
    let plan = filter::Plan::new(filters, r.metadata().sample_rate)?
        .with_source(source)
        .with_psa_cache();
    let mut report = Report {
        format: "eva-ocular-threshold".into(),
        version: 1,
        settings: settings.clone(),
        sample_rate: r.metadata().sample_rate,
        source: String::new(),
        source_processing: format!(
            "{}; interpolation={:?}",
            filter::cli::arguments(filters).join(" "),
            filters.interpolation
        ),
        blink_channels: vec![],
        movement_channels: vec![],
        warnings: vec![],
        candidates: vec![],
    };
    for (index, (kind, c)) in [
        (Kind::Blink, &settings.blink),
        (Kind::Movement, &settings.movement),
    ]
    .into_iter()
    .enumerate()
    {
        if !c.enabled {
            continue;
        }
        c.validate()?;
        check(g, id)?;
        let (channels, mid, known) = resolve(r, kind, c)?;
        if !known {
            report.warnings.push(format!("{}: unknown montage; Swift fallback uses first {} channels. Check channel selection.",kind.label(),channels.len()));
        }
        if kind == Kind::Blink {
            report.blink_channels = channels.clone();
        } else {
            report.movement_channels = channels.clone();
        }
        let factors: Vec<_> = channels
            .iter()
            .map(|ch| {
                factor(r.metadata().channels[ch - 1].unit.as_deref())
                    .ok_or_else(|| format!("Channel {ch} requires V, mV or µV units"))
            })
            .collect::<Result<_, _>>()?;
        let mut drive = Vec::with_capacity(count);
        for start in (0..count).step_by(16384) {
            check(g, id)?;
            let end = (start + 16384).min(count);
            let rows: Vec<_> = channels
                .iter()
                .map(|ch| plan.read(r, ch - 1, start as u64..end as u64, g, id))
                .collect::<Result<_, _>>()?;
            let mut values = vec![0f32; rows.len()];
            for s in 0..end - start {
                for ((v, row), factor) in values.iter_mut().zip(&rows).zip(&factors) {
                    *v = row[s] * factor;
                }
                let v = if values.iter().any(|v| !v.is_finite()) {
                    f32::NAN
                } else if c.topology == Topology::Legacy {
                    values
                        .iter()
                        .fold(0f32, |a, &b| if b.abs() > a.abs() { b } else { a })
                } else {
                    let l = values[..mid].iter().sum::<f32>() / mid as f32;
                    let rr = values[mid..].iter().sum::<f32>() / (values.len() - mid) as f32;
                    if kind == Kind::Blink {
                        if l != 0. && rr != 0. && (l > 0.) == (rr > 0.) {
                            (l + rr) / 2.
                        } else {
                            0.
                        }
                    } else {
                        let opponent = l - rr;
                        let common = (l + rr) / 2.;
                        if opponent.abs() > common.abs() {
                            opponent
                        } else {
                            0.
                        }
                    }
                };
                drive.push(v);
            }
            progress((index as f32 + end as f32 / count.max(1) as f32) / 2.);
        }
        // Force acquisition gaps to NaN even if a supplied snapshot contains zeros.
        for gap in r.gaps() {
            drive[gap.start as usize..gap.end as usize].fill(f32::NAN);
        }
        report
            .candidates
            .extend(detect(&drive, r.metadata().sample_rate, kind, c, g, id)?);
    }
    report.candidates.sort_by_key(|v| v.peak_sample);
    progress(1.);
    Ok(report)
}
fn detect(
    drive: &[f32],
    rate: u32,
    kind: Kind,
    c: &Config,
    g: &AtomicU64,
    id: u64,
) -> Result<Vec<Candidate>, String> {
    let samples = |s: f64| ((s * rate as f64).round() as usize).max(1);
    let minimum = samples(c.min_seconds);
    let maximum = if c.max_seconds > 0. {
        samples(c.max_seconds).max(minimum)
    } else {
        usize::MAX
    };
    let merge = samples(c.merge_seconds);
    let rise = if c.rise_seconds > 0. {
        samples(c.rise_seconds)
    } else {
        usize::MAX
    };
    let mut runs: Vec<(usize, usize)> = vec![];
    let mut start = None;
    let mut barrier = 0;
    let push = |runs: &mut Vec<(usize, usize)>, a: usize, b: usize, barrier: usize| {
        if let Some(last) = runs.last_mut() {
            if last.1 >= barrier && a - (last.1 - 1) <= merge && b - last.0 <= maximum {
                last.1 = b;
                return;
            }
        }
        runs.push((a, b));
    };
    for (i, &v) in drive.iter().enumerate() {
        if i % 16384 == 0 {
            check(g, id)?;
            if runs.len() > 1_000_000 {
                return Err(
                    "Too many threshold runs; increase thresholds or shorten the recording".into(),
                );
            }
        }
        let above = v.is_finite()
            && match c.polarity {
                Polarity::Positive => v >= c.minimum_uv,
                Polarity::Negative => v <= -c.minimum_uv,
                Polarity::Bipolar => v.abs() >= c.minimum_uv,
            };
        if above {
            if start.is_none() {
                start = Some(i);
            }
        } else {
            if let Some(a) = start.take() {
                push(&mut runs, a, i, barrier);
            }
            if !v.is_finite() {
                barrier = i + 1;
            }
        }
    }
    if let Some(a) = start {
        push(&mut runs, a, drive.len(), barrier);
    }
    let mut out = vec![];
    let per_ms = rate as f32 / 1000.;
    for (a, b) in runs {
        check(g, id)?;
        if b - a < minimum || b - a > maximum {
            continue;
        }
        let mut peak = a;
        for i in a..b {
            if drive[i].abs() > drive[peak].abs() {
                peak = i;
            }
        }
        if (c.maximum_uv > 0. && drive[peak].abs() > c.maximum_uv) || peak - a > rise {
            continue;
        }
        let (mut velocity, mut acceleration) = (0f32, 0f32);
        if c.velocity_enabled || c.acceleration_enabled {
            for s in a.max(1)..b {
                if drive[s - 1].is_finite() {
                    velocity = velocity.max((drive[s] - drive[s - 1]).abs() * per_ms);
                    if s + 1 < drive.len() && drive[s + 1].is_finite() {
                        acceleration = acceleration.max(
                            (drive[s + 1] - 2. * drive[s] + drive[s - 1]).abs() * per_ms * per_ms,
                        );
                    }
                }
            }
        }
        if (c.velocity_enabled && velocity < c.velocity_uv_ms)
            || (c.acceleration_enabled && acceleration < c.acceleration_uv_ms2)
        {
            continue;
        }
        if out.len() >= 1_000_000 {
            return Err("Too many ocular candidates; increase thresholds".into());
        }
        out.push(Candidate {
            kind,
            code: kind.label().into(),
            start_sample: a as u64,
            end_sample: b as u64,
            peak_sample: peak as u64,
            start_seconds: a as f64 / rate as f64,
            end_seconds: b as f64 / rate as f64,
            peak_seconds: peak as f64 / rate as f64,
            peak_uv: drive[peak],
        });
    }
    Ok(out)
}
/// Create a new report, refusing to overwrite any existing file.
pub fn write_report(report: &Report, path: &std::path::Path) -> Result<(), String> {
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
