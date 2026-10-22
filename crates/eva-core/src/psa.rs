//! Event-locked EEG epochs, rejection, spherical-spline repair and category means.
pub mod cli;
pub mod export;
use crate::{filter, interpolation};
use eva_mff::Recording;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub events: Vec<String>,
    pub by_label: bool,
    pub categories: BTreeMap<String, String>,
    pub pre: f64,
    pub post: f64,
    pub offset: f64,
    pub baseline: bool,
    pub baseline_start: f64,
    pub baseline_end: f64,
    pub average_reference: bool,
    pub average: bool,
    pub threshold: bool,
    pub min_uv: f64,
    pub max_uv: f64,
    pub slope: f64,
    pub acceleration: f64,
    pub max_bad_percent: f64,
    pub max_bad_count: Option<usize>,
    pub interpolate_epoch: bool,
    pub interpolate_global: bool,
    /// Zero-based internally; CLI/GUI display one-based channels.
    pub bad_channels: Vec<usize>,
    pub escalate: bool,
    pub escalation_percent: f64,
    pub reject_codes: Vec<String>,
    /// One-based candidate numbers in recording event order.
    pub reject_epochs: Vec<usize>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            events: vec![],
            by_label: false,
            categories: BTreeMap::new(),
            pre: 0.2,
            post: 0.8,
            offset: 0.,
            baseline: true,
            baseline_start: -0.2,
            baseline_end: 0.,
            average_reference: true,
            average: false,
            threshold: true,
            min_uv: -150.,
            max_uv: 150.,
            slope: 25.,
            acceleration: 15.,
            max_bad_percent: 10.,
            max_bad_count: None,
            interpolate_epoch: true,
            interpolate_global: true,
            bad_channels: vec![],
            escalate: true,
            escalation_percent: 50.,
            reject_codes: vec![],
            reject_epochs: vec![],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub candidate: usize,
    pub event_index: usize,
    pub code: String,
    pub category: String,
    pub anchor_sample: i64,
    pub accepted: bool,
    pub reason: String,
    /// One-based channel numbers for all public result metadata.
    pub flagged_channels: Vec<usize>,
    pub repaired_channels: Vec<usize>,
}
#[derive(Debug)]
pub struct Epoch {
    pub decision: usize,
    pub values: Vec<Vec<f32>>,
}
#[derive(Debug)]
pub struct Average {
    pub category: String,
    pub count: usize,
    pub values: Vec<Vec<f32>>,
}
#[derive(Debug)]
pub struct ResultData {
    pub config: Config,
    pub sample_rate: u32,
    pub pre_samples: usize,
    pub samples: usize,
    pub channel_names: Vec<String>,
    pub channel_units: Vec<Option<String>>,
    pub decisions: Vec<Decision>,
    pub epochs: Vec<Epoch>,
    pub averages: Vec<Average>,
    pub global_bad: Vec<usize>,
    pub warnings: Vec<String>,
    pub source_processing: String,
}
fn check(g: &AtomicU64, id: u64) -> Result<(), String> {
    if g.load(Ordering::Relaxed) != id {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}
fn microvolts(unit: Option<&str>) -> Option<f64> {
    match unit.map(str::trim) {
        Some("uV" | "µV" | "μV") => Some(1.),
        Some("mV") => Some(1000.),
        Some("V") => Some(1e6),
        _ => None,
    }
}
impl Config {
    pub fn validate(&self, r: &Recording) -> Result<(usize, usize, usize, usize), String> {
        if r.is_pns() || r.metadata().kind == eva_mff::RecordingKind::SegmentedOrAveraged {
            return Err("PSA requires continuous/discontinuous scalp EEG".into());
        }
        if self.events.is_empty() || self.events.iter().any(|s| s.is_empty()) {
            return Err("Select at least one event code/label".into());
        }
        for v in [
            self.pre,
            self.post,
            self.offset,
            self.baseline_start,
            self.baseline_end,
            self.min_uv,
            self.max_uv,
            self.slope,
            self.acceleration,
            self.max_bad_percent,
            self.escalation_percent,
        ] {
            if !v.is_finite() {
                return Err("PSA settings must be finite".into());
            }
        }
        if self.pre < 0.
            || self.post <= 0.
            || self.pre + self.post > 60.
            || self.offset.abs() > 86400.
        {
            return Err(
                "Epoch window must be positive, at most 60 seconds; offset at most one day".into(),
            );
        }
        if self.min_uv >= self.max_uv
            || self.slope <= 0.
            || self.acceleration <= 0.
            || !(0. ..=100.).contains(&self.max_bad_percent)
            || !(0. ..=100.).contains(&self.escalation_percent)
            || self.escalation_percent == 0.
        {
            return Err("Invalid threshold or percentage".into());
        }
        let rate = f64::from(r.metadata().sample_rate);
        let pre = (self.pre * rate).round() as usize;
        let len = pre + (self.post * rate).round() as usize;
        if !(2..=16_777_216).contains(&len) {
            return Err("Invalid epoch sample count".into());
        }
        let bs = (pre as f64 + self.baseline_start * rate).round();
        let be = (pre as f64 + self.baseline_end * rate).round();
        if self.baseline && (bs < 0. || be > len as f64 || bs >= be) {
            return Err("Baseline must be a nonempty interval inside the epoch".into());
        }
        if self
            .bad_channels
            .iter()
            .any(|c| *c >= r.metadata().channels.len())
        {
            return Err("Bad channel outside recording".into());
        }
        if self.threshold
            && r.metadata()
                .channels
                .iter()
                .any(|c| microvolts(c.unit.as_deref()).is_none())
        {
            return Err("Thresholding requires declared V, mV or µV units on every EEG channel; disable thresholds for unknown units".into());
        }
        if self.average_reference
            && r.metadata()
                .channels
                .iter()
                .any(|c| c.unit != r.metadata().channels[0].unit)
        {
            return Err("Average reference requires matching channel units".into());
        }
        Ok((pre, len, bs.max(0.) as usize, be.max(0.) as usize))
    }
}
fn flagged(x: &[f32], factor: f64, c: &Config, baseline: std::ops::Range<usize>) -> bool {
    let center = if c.baseline {
        x[baseline.clone()]
            .iter()
            .map(|v| f64::from(*v))
            .sum::<f64>()
            / baseline.len() as f64
    } else {
        0.
    };
    x.iter().any(|v| {
        let v = (f64::from(*v) - center) * factor;
        v < c.min_uv || v > c.max_uv
    }) || x
        .windows(2)
        .any(|v| (f64::from(v[1]) - f64::from(v[0])).abs() * factor > c.slope)
        || x.windows(3).any(|v| {
            (f64::from(v[2]) - 2. * f64::from(v[1]) + f64::from(v[0])).abs() * factor
                > c.acceleration
        })
}
fn repair(values: &mut [Vec<f32>], recipes: &[interpolation::Recipe]) -> Result<(), String> {
    // Evaluate all targets from independent original donors before committing.
    let mut replacements = vec![];
    for recipe in recipes {
        let mut out = vec![0f64; values[0].len()];
        for (&donor, &weight) in recipe.donors.iter().zip(&recipe.weights) {
            for (sum, v) in out.iter_mut().zip(&values[donor]) {
                *sum += weight * f64::from(*v);
            }
        }
        if out
            .iter()
            .any(|v| !v.is_finite() || v.abs() > f64::from(f32::MAX))
        {
            return Err("Interpolation produced nonfinite samples".into());
        }
        replacements.push((recipe.target, out.into_iter().map(|v| v as f32).collect()));
    }
    for (ch, v) in replacements {
        values[ch] = v;
    }
    Ok(())
}
/// Snapshot of the same immutable MRI -> filter source used by the viewer.
/// Epoch gaps are checked on the original acquisition grid before any processing.
pub fn run(
    r: &Recording,
    c: &Config,
    filters: &filter::Settings,
    source: Option<Arc<Vec<Vec<f32>>>>,
    g: &AtomicU64,
    id: u64,
    progress: &dyn Fn(f32),
) -> Result<ResultData, String> {
    let (pre, len, bs, be) = c.validate(r)?;
    check(g, id)?;
    let plan = filter::Plan::new(filters, r.metadata().sample_rate)?.with_source(source);
    let channels = r.metadata().channels.len();
    let candidates: Vec<_> = r
        .metadata()
        .events
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            let key = if c.by_label {
                e.label.as_deref().unwrap_or(&e.code)
            } else {
                &e.code
            };
            c.events.iter().any(|s| s == key)
        })
        .collect();
    if candidates.is_empty() {
        return Err("No matching events".into());
    }
    if candidates.len() > 100_000 {
        return Err("PSA candidate limit is 100,000".into());
    }
    if c.reject_epochs
        .iter()
        .any(|i| *i == 0 || *i > candidates.len())
    {
        return Err("Rejected epoch number outside candidate list".into());
    }
    let budget = candidates
        .len()
        .checked_mul(channels)
        .and_then(|n| n.checked_mul(len))
        .and_then(|n| n.checked_mul(4))
        .ok_or("PSA size overflow")?;
    // Two copies can be resident during averaging/export; cap the epoch payload.
    if budget > 256 * 1024 * 1024 {
        return Err(
            "PSA epoch payload exceeds 256 MiB; select fewer conditions or a shorter window".into(),
        );
    }
    let mut result = ResultData {
        config: c.clone(),
        sample_rate: r.metadata().sample_rate,
        pre_samples: pre,
        samples: len,
        channel_names: r
            .metadata()
            .channels
            .iter()
            .map(|c| c.label.clone())
            .collect(),
        channel_units: r
            .metadata()
            .channels
            .iter()
            .map(|c| c.unit.clone())
            .collect(),
        decisions: vec![],
        epochs: vec![],
        averages: vec![],
        global_bad: vec![],
        warnings: vec![],
        source_processing: filters.summary(),
    };
    let global: BTreeSet<_> = c.bad_channels.iter().copied().collect();
    let mut evidence = vec![0usize; channels];
    let mut measured = 0usize;
    for (i, (event_index, event)) in candidates.iter().enumerate() {
        check(g, id)?;
        let mut shifted = (*event).clone();
        shifted.onset_ns = shifted
            .onset_ns
            .checked_add((c.offset * 1e9).round() as i128)
            .ok_or("Anchor overflow")?;
        let anchor = shifted
            .sample_index(r.metadata().sample_rate)
            .map_err(|e| e.to_string())?;
        let key = if c.by_label {
            event.label.as_deref().unwrap_or(&event.code)
        } else {
            &event.code
        };
        let category = c
            .categories
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_owned());
        let mut d = Decision {
            candidate: i + 1,
            event_index: event_index + 1,
            code: event.code.clone(),
            category,
            anchor_sample: anchor,
            accepted: false,
            reason: String::new(),
            flagged_channels: vec![],
            repaired_channels: vec![],
        };
        let start = i128::from(anchor) - pre as i128;
        let end = start + len as i128;
        let mut values = None;
        if start < 0 || end > i128::from(r.metadata().sample_count) {
            d.reason = "Out of bounds".into();
        } else if r
            .overlaps_gap(start as u64..end as u64)
            .map_err(|e| e.to_string())?
        {
            d.reason = "Acquisition gap".into();
        } else if c.reject_epochs.contains(&(i + 1)) {
            d.reason = "Manually excluded".into();
        } else if r.metadata().events.iter().any(|e| {
            if !c.reject_codes.contains(&e.code) {
                return false;
            }
            let a = e.onset_ns as f64 / 1e9 * f64::from(r.metadata().sample_rate);
            let b = a + e.duration_ns.unwrap_or(0).max(0) as f64 / 1e9
                * f64::from(r.metadata().sample_rate);
            if b == a {
                a >= start as f64 && a < end as f64
            } else {
                a < end as f64 && b > start as f64
            }
        }) {
            d.reason = "Artifact event overlap".into();
        } else {
            let mut x = Vec::with_capacity(channels);
            for ch in 0..channels {
                check(g, id)?;
                x.push(plan.read(r, ch, start as u64..end as u64, g, id)?);
            }
            if x.iter().flatten().any(|v| !v.is_finite()) {
                d.reason = "Nonfinite samples".into();
            } else {
                measured += 1;
                if c.threshold {
                    for ch in 0..channels {
                        if !global.contains(&ch)
                            && flagged(
                                &x[ch],
                                microvolts(r.metadata().channels[ch].unit.as_deref()).unwrap(),
                                c,
                                bs..be,
                            )
                        {
                            d.flagged_channels.push(ch + 1);
                            evidence[ch] += 1;
                        }
                    }
                }
                values = Some(x);
            }
        }
        result.decisions.push(d);
        if let Some(values) = values {
            result.epochs.push(Epoch {
                decision: i,
                values,
            });
        }
        progress(0.5 * (i + 1) as f32 / candidates.len() as f32);
    }
    let mut global = global;
    if c.threshold && c.escalate && measured > 0 {
        for (ch, count) in evidence.iter().enumerate() {
            if *count as f64 * 100. / measured as f64 >= c.escalation_percent {
                global.insert(ch);
            }
        }
    }
    result.global_bad = global.iter().map(|ch| ch + 1).collect();
    let global: Vec<_> = global.into_iter().collect();
    let reference: Vec<_> = (0..channels).filter(|ch| !global.contains(ch)).collect();
    if c.average_reference && reference.is_empty() {
        return Err("Average reference has no good channels".into());
    }
    if !global.is_empty() && !c.interpolate_global {
        result.warnings.push("Global bad channels remain unrepaired and are excluded from reference and transient detection".into());
    }
    let mut cache: BTreeMap<Vec<usize>, Vec<interpolation::Recipe>> = BTreeMap::new();
    let total = result.epochs.len();
    let mut kept = vec![];
    for (i, mut epoch) in std::mem::take(&mut result.epochs).into_iter().enumerate() {
        check(g, id)?;
        let d = &mut result.decisions[epoch.decision];
        let local: Vec<_> = d
            .flagged_channels
            .iter()
            .map(|v| v - 1)
            .filter(|ch| !global.contains(ch))
            .collect();
        let limit = c
            .max_bad_count
            .map(|n| n as f64)
            .unwrap_or(channels as f64 * c.max_bad_percent / 100.);
        if local.len() as f64 > limit {
            d.reason = "Too many bad channels".into();
            continue;
        }
        if !local.is_empty() && !c.interpolate_epoch {
            d.reason = "Uncorrected threshold artifact".into();
            continue;
        }
        let mut targets = local;
        if c.interpolate_global {
            targets.extend(&global);
        }
        targets.sort_unstable();
        targets.dedup();
        if !targets.is_empty() {
            if !cache.contains_key(&targets) {
                match interpolation::recipes(r, &targets, &global) {
                    Ok(v) => {
                        if cache.len() > 128 {
                            cache.clear();
                        }
                        cache.insert(targets.clone(), v);
                    }
                    Err(e) => {
                        d.reason = format!("Interpolation unavailable: {e}");
                        continue;
                    }
                }
            }
            repair(&mut epoch.values, &cache[&targets])?;
            d.repaired_channels = targets.iter().map(|ch| ch + 1).collect();
        }
        if c.average_reference {
            for s in 0..len {
                let mean = reference
                    .iter()
                    .map(|ch| f64::from(epoch.values[*ch][s]))
                    .sum::<f64>()
                    / reference.len() as f64;
                for row in &mut epoch.values {
                    row[s] = (f64::from(row[s]) - mean) as f32;
                }
            }
        }
        if c.baseline {
            for row in &mut epoch.values {
                let mean =
                    row[bs..be].iter().map(|v| f64::from(*v)).sum::<f64>() / (be - bs) as f64;
                for v in row {
                    *v = (f64::from(*v) - mean) as f32;
                }
            }
        }
        if epoch.values.iter().flatten().any(|v| !v.is_finite()) {
            return Err("Postprocessing produced nonfinite values".into());
        }
        d.accepted = true;
        d.reason = "Accepted".into();
        kept.push(epoch);
        progress(0.5 + 0.4 * (i + 1) as f32 / total.max(1) as f32);
    }
    result.epochs = kept;
    if c.average {
        let names: BTreeSet<_> = result
            .epochs
            .iter()
            .map(|e| result.decisions[e.decision].category.clone())
            .collect();
        for category in names {
            check(g, id)?;
            let members: Vec<_> = result
                .epochs
                .iter()
                .filter(|e| result.decisions[e.decision].category == category)
                .collect();
            let mut values = vec![vec![0f32; len]; channels];
            for (ch, row) in values.iter_mut().enumerate() {
                for (s, v) in row.iter_mut().enumerate() {
                    *v = (members
                        .iter()
                        .map(|e| f64::from(e.values[ch][s]))
                        .sum::<f64>()
                        / members.len() as f64) as f32;
                }
            }
            result.averages.push(Average {
                category,
                count: members.len(),
                values,
            });
        }
    }
    if result.epochs.is_empty() {
        result
            .warnings
            .push("All candidate epochs were rejected; no average was produced".into());
    }
    progress(1.);
    Ok(result)
}

#[cfg(test)]
mod tests;
