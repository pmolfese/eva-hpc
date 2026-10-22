//! CPU port of EVA's Allen AAS and clean-room FASTR-family mechanics.
//! This implementation is independent of graphics, files, and recording formats.
mod motion;
mod numeric;
pub use motion::{parse_motion, MotionFormat, MotionMetric};
use numeric::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    AllenAas,
    Fastr,
    Farm,
    Moosmann,
}
impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Self::AllenAas => "Allen AAS",
            Self::Fastr => "FASTR",
            Self::Farm => "FARM",
            Self::Moosmann => "Moosmann",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scaling {
    Unscaled,
    DriftTracking,
    LeastSquares,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Synchronization {
    ScannerSlaved,
    IndependentClocks,
    Custom,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obs {
    Off,
    Automatic,
    Fixed,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub method: Method,
    pub groups: usize,
    pub offsets_seconds: Vec<f64>,
    pub trigger_position: f64,
    pub donors: usize,
    pub upsample: usize,
    pub synchronization: Synchronization,
    pub align: bool,
    pub fractional: bool,
    pub scaling: Scaling,
    pub smoothing: usize,
    pub scale_min: f64,
    pub scale_max: f64,
    pub squared_correlation: bool,
    pub correlation: f64,
    pub minimum_donors: usize,
    pub search: usize,
    pub section_epochs: usize,
    pub allen_gate: f64,
    pub initial_epochs: usize,
    pub obs: Obs,
    pub components: usize,
    pub variance: f64,
    pub max_components: usize,
    pub chunk_seconds: f64,
    pub max_basis_epochs: usize,
    pub residual_floor: f64,
    pub anc: bool,
    pub anc_slice_highpass: bool,
    pub anc_taps: usize,
    pub anc_step: f64,
    pub motion: Vec<[f64; 6]>,
    pub motion_metric: MotionMetric,
    pub motion_threshold: f64,
    pub motion_radius: f64,
    pub censor_motion: bool,
    pub censored: Vec<usize>,
    pub excluded_channels: Vec<usize>,
    pub template_only: Vec<usize>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            method: Method::AllenAas,
            groups: 1,
            offsets_seconds: vec![],
            trigger_position: 0.,
            donors: 30,
            upsample: 1,
            synchronization: Synchronization::ScannerSlaved,
            align: true,
            fractional: false,
            scaling: Scaling::DriftTracking,
            smoothing: 15,
            scale_min: 0.2,
            scale_max: 5.,
            squared_correlation: false,
            correlation: 0.9,
            minimum_donors: 4,
            search: 240,
            section_epochs: 0,
            allen_gate: 0.975,
            initial_epochs: 5,
            obs: Obs::Off,
            components: 3,
            variance: 0.9,
            max_components: 5,
            chunk_seconds: 60.,
            max_basis_epochs: 256,
            residual_floor: 0.01,
            anc: false,
            anc_slice_highpass: false,
            anc_taps: 32,
            anc_step: 0.01,
            motion: vec![],
            motion_metric: MotionMetric::TranslationSum,
            motion_threshold: 0.5,
            motion_radius: 50.,
            censor_motion: false,
            censored: vec![],
            excluded_channels: vec![],
            template_only: vec![],
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=512).contains(&self.groups)
            || !(1..=16).contains(&self.upsample)
            || !(1..=500).contains(&self.donors)
            || !(1..=2000).contains(&self.search)
        {
            return Err("Groups 1–512, upsampling 1–16, donors 1–500 and correlation search 1–2000 are required.".into());
        }
        if !self.trigger_position.is_finite()
            || !(0.0..=1.).contains(&self.trigger_position)
            || !self.scale_min.is_finite()
            || !self.scale_max.is_finite()
            || self.scale_min <= 0.
            || self.scale_max < self.scale_min
            || self.smoothing == 0
        {
            return Err("Invalid trigger position or template scale range.".into());
        }
        if !self.correlation.is_finite()
            || !(-1.0..=1.).contains(&self.correlation)
            || !self.allen_gate.is_finite()
            || !(-1.0..=1.).contains(&self.allen_gate)
            || self.minimum_donors == 0
        {
            return Err("Correlation gates must be between −1 and 1.".into());
        }
        if !self.variance.is_finite()
            || !(0.0..=1.).contains(&self.variance)
            || !self.chunk_seconds.is_finite()
            || !(1.0..=3600.).contains(&self.chunk_seconds)
            || !(8..=256).contains(&self.max_basis_epochs)
            || !(1..=32).contains(&self.components)
            || !(1..=32).contains(&self.max_components)
            || !self.residual_floor.is_finite()
            || self.residual_floor < 0.
        {
            return Err("Invalid OBS settings (basis cap 8–256, components 1–32).".into());
        }
        if !(1..=1024).contains(&self.anc_taps)
            || !self.anc_step.is_finite()
            || self.anc_step <= 0.
            || self.anc_step >= 2.
        {
            return Err("ANC step must be between 0 and 2, taps 1–1024.".into());
        }
        if !self.motion_threshold.is_finite()
            || self.motion_threshold < 0.
            || !self.motion_radius.is_finite()
            || self.motion_radius <= 0.
            || self.motion.iter().flatten().any(|v| !v.is_finite())
        {
            return Err("Invalid motion parameters.".into());
        }
        if (self.method == Method::Moosmann || self.censor_motion) && self.motion.is_empty() {
            return Err("Load a motion file for Moosmann or motion censoring.".into());
        }
        if self.offsets_seconds.len() > 512
            || self
                .offsets_seconds
                .iter()
                .any(|v| !v.is_finite() || *v < 0.)
            || self.offsets_seconds.windows(2).any(|v| v[0] >= v[1])
        {
            return Err("Acquisition offsets must be sorted distinct nonnegative seconds.".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Default)]
pub struct Diagnostics {
    pub epochs: usize,
    pub corrected: usize,
    pub skipped: usize,
    pub correlation_fallbacks: usize,
    pub motion_barrier_crossings: usize,
    pub rejected_scales: usize,
    pub obs_components: usize,
    pub anc_channels: usize,
    pub high_motion: Vec<usize>,
    pub warnings: Vec<String>,
    pub reference_channel: usize,
    pub uncovered_samples: usize,
}
#[derive(Debug)]
pub struct ResultData {
    pub channels: Vec<Vec<f32>>,
    pub diagnostics: Diagnostics,
}
#[derive(Debug, Clone)]
struct Epoch {
    start: f64,
    volume: usize,
    group: usize,
}
struct Layout {
    epochs: Vec<Epoch>,
    length: usize,
    period: f64,
    volumes: usize,
}
fn layout(
    triggers: &[u64],
    n: usize,
    fs: f64,
    c: &Config,
    d: &mut Diagnostics,
) -> Result<Layout, String> {
    let mut volumes: Vec<_> = triggers.iter().copied().filter(|v| *v < n as u64).collect();
    volumes.sort_unstable();
    volumes.dedup();
    if volumes.len() < 2 {
        return Err("At least two distinct in-range volume triggers are required.".into());
    }
    if volumes.len() != triggers.len() {
        d.warnings
            .push("Duplicate or out-of-range triggers were removed.".into());
    }
    let intervals: Vec<_> = volumes.windows(2).map(|v| (v[1] - v[0]) as f64).collect();
    let tr = median(&intervals);
    if intervals
        .iter()
        .any(|v| (*v - tr).abs() > 2f64.max(tr * 0.01) && *v < 1.5 * tr)
    {
        return Err("Volume trigger spacing is irregular (>1% or two samples). Check marker selection and scanner timing.".into());
    }
    if intervals.iter().any(|v| *v > 1.5 * tr) {
        d.warnings
            .push("Missing-trigger gaps are left uncovered; no triggers were invented.".into());
    }
    let groups = if c.offsets_seconds.is_empty() {
        c.groups
    } else {
        c.offsets_seconds.len()
    };
    if volumes.len().saturating_mul(groups) > 200_000 {
        return Err("Too many acquisition epochs (limit 200,000).".into());
    }
    if c.offsets_seconds.last().is_some_and(|v| *v * fs >= tr) {
        return Err("Acquisition offsets must fall within one TR.".into());
    }
    let mut epochs = vec![];
    for (volume, &trigger) in volumes.iter().enumerate() {
        let interval = intervals
            .get(volume)
            .copied()
            .unwrap_or(*intervals.last().unwrap());
        let interval = if interval > 1.5 * tr { tr } else { interval };
        for group in 0..groups {
            let offset = if c.offsets_seconds.is_empty() {
                group as f64 * interval / groups as f64
            } else {
                c.offsets_seconds[group] * fs
            };
            let start = ((trigger as f64 + offset) * c.upsample as f64).round() / c.upsample as f64;
            if start < n as f64 {
                epochs.push(Epoch {
                    start,
                    volume,
                    group,
                });
            }
        }
    }
    let spacing: Vec<_> = epochs.windows(2).map(|v| v[1].start - v[0].start).collect();
    if spacing.iter().any(|v| *v <= 0.) {
        return Err(
            "Acquisition groups collapse on the sampling grid; increase upsampling.".into(),
        );
    }
    let period = median(&spacing);
    let length = (period * c.upsample as f64).round() as usize + 1;
    if !(3..=1_000_000).contains(&length) || length.saturating_mul(epochs.len()) > 16_000_000 {
        return Err("Epoch workspace exceeds this CPU port's limit; reduce upsampling/group count or recording size.".into());
    }
    for epoch in &mut epochs {
        epoch.start -=
            (period * c.trigger_position * c.upsample as f64).round() / c.upsample as f64;
    }
    d.epochs = epochs.len();
    Ok(Layout {
        epochs,
        length,
        period,
        volumes: volumes.len(),
    })
}
fn extract(x: &[f32], e: &Epoch, len: usize, factor: usize) -> Option<Vec<f64>> {
    let end = e.start + (len - 1) as f64 / factor as f64;
    if e.start < 0. || end > x.len() as f64 {
        return None;
    }
    let values: Vec<_> = (0..len)
        .map(|i| sample(x, e.start + i as f64 / factor as f64, 4))
        .collect();
    values.iter().all(|v| v.is_finite()).then_some(values)
}
/// Performs correction from immutable calibrated samples. Cancellation and progress
/// belong to the caller. Limits bound the temporary per-channel epoch workspace.
pub fn correct(
    channels: &[Vec<f32>],
    triggers: &[u64],
    fs: f64,
    c: &Config,
    cancel: &dyn Fn() -> bool,
    progress: &dyn Fn(f32),
) -> Result<ResultData, String> {
    c.validate()?;
    check(cancel)?;
    let n = channels.first().map(Vec::len).unwrap_or(0);
    if n == 0 || !fs.is_finite() || fs <= 0. || channels.iter().any(|v| v.len() != n) {
        return Err(
            "Nonempty equal-length channels and a positive sampling rate are required.".into(),
        );
    }
    if n.saturating_mul(channels.len()) > 134_217_728 {
        return Err("Correction input exceeds the 512 MiB sample limit.".into());
    }
    let mut d = Diagnostics::default();
    if channels
        .iter()
        .any(|channel| channel.iter().any(|v| !v.is_finite()))
    {
        d.warnings.push("Missing samples are preserved. Acquisition epochs crossing gaps are skipped; ANC restarts on each finite segment.".into());
    }
    let mut l = layout(triggers, n, fs, c, &mut d)?;
    let high = motion::high_motion(c, l.volumes, &mut d.warnings)?;
    d.high_motion = high.clone();
    if c.method == Method::Moosmann && high.is_empty() {
        d.warnings.push(
            "No volume exceeds the motion threshold; Moosmann uses temporal neighbors.".into(),
        );
    }
    let representative = (0..channels.len())
        .filter(|i| !c.excluded_channels.contains(i))
        .max_by(|&a, &b| finite_variance(&channels[a]).total_cmp(&finite_variance(&channels[b])))
        .ok_or("All channels are excluded")?;
    d.reference_channel = representative;
    if c.method != Method::AllenAas {
        align(&channels[representative], &mut l, c, cancel)?;
    }
    let mut output = Vec::with_capacity(channels.len());
    for (ch, x) in channels.iter().enumerate() {
        check(cancel)?;
        if c.excluded_channels.contains(&ch) {
            output.push(x.clone());
            continue;
        }
        let mut windows = Vec::with_capacity(l.epochs.len());
        for e in &l.epochs {
            check(cancel)?;
            windows.push(extract(x, e, l.length, c.upsample));
        }
        let eligible: Vec<_> = l
            .epochs
            .iter()
            .enumerate()
            .map(|(i, e)| {
                windows[i].is_some() && !high.contains(&e.volume) && !c.censored.contains(&e.volume)
            })
            .collect();
        let section = if c.section_epochs > 0 {
            c.section_epochs.max(2)
        } else if c.groups > 1 {
            (7.5 * fs / l.period).round().max(25.) as usize
        } else {
            25
        };
        let aas = if c.method == Method::AllenAas {
            aas_donors(&windows, &eligible, section, c)
        } else {
            vec![]
        };
        let mut templates = vec![None; windows.len()];
        let mut fits = vec![None; windows.len()];
        for i in 0..windows.len() {
            check(cancel)?;
            let Some(target) = &windows[i] else {
                d.skipped += 1;
                continue;
            };
            let donors = if c.method == Method::AllenAas {
                aas[i].clone()
            } else {
                donors(i, &l, &windows, &eligible, &high, c, &mut d)
            };
            if donors.is_empty() {
                d.skipped += 1;
                continue;
            }
            let template = average(donors.iter().filter_map(|j| windows[*j].as_ref()), l.length);
            let energy = dot(&template, &template);
            if energy <= 1e-12 {
                d.skipped += 1;
                continue;
            }
            let fit = dot(target, &template) / energy;
            fits[i] = (fit.is_finite() && fit >= c.scale_min && fit <= c.scale_max).then_some(fit);
            templates[i] = Some(template);
        }
        let mut residuals = vec![None; windows.len()];
        for i in 0..windows.len() {
            let Some(t) = templates[i].as_mut() else {
                continue;
            };
            let scale = if c.method == Method::AllenAas || c.scaling == Scaling::Unscaled {
                1.
            } else if c.scaling == Scaling::LeastSquares {
                fits[i].unwrap_or(1.)
            } else {
                let start = i.saturating_sub(c.smoothing / 2);
                let end = (i + c.smoothing / 2 + 1).min(fits.len());
                let values: Vec<_> = fits[start..end].iter().filter_map(|v| *v).collect();
                if values.is_empty() {
                    1.
                } else {
                    median(&values)
                }
            };
            if fits[i].is_none() && c.method != Method::AllenAas && c.scaling != Scaling::Unscaled {
                d.rejected_scales += 1;
            }
            for v in t.iter_mut() {
                *v *= scale;
            }
            residuals[i] = Some(
                windows[i]
                    .as_ref()
                    .unwrap()
                    .iter()
                    .zip(t.iter())
                    .map(|(x, t)| x - t)
                    .collect(),
            );
            d.corrected += 1;
        }
        if c.method != Method::AllenAas && c.obs != Obs::Off && !c.template_only.contains(&ch) {
            apply_obs(
                &mut templates,
                &residuals,
                &eligible,
                l.period,
                fs,
                c,
                cancel,
                &mut d,
            )?;
        }
        let mut artifact = vec![0f64; n];
        let mut coverage = vec![0u16; n];
        for (epoch, template) in l.epochs.iter().zip(&templates) {
            let Some(t) = template else {
                continue;
            };
            let start = epoch.start.ceil().max(0.) as usize;
            let end = (epoch.start + (l.length - 1) as f64 / c.upsample as f64)
                .floor()
                .min((n - 1) as f64) as usize;
            for i in start..=end {
                let v = sample64(t, (i as f64 - epoch.start) * c.upsample as f64, 8);
                artifact[i] += v;
                coverage[i] += 1;
            }
        }
        let mut corrected = x.clone();
        for i in 0..n {
            if coverage[i] > 0 {
                artifact[i] /= f64::from(coverage[i]);
                corrected[i] = (f64::from(x[i]) - artifact[i]) as f32;
            }
        }
        if ch == representative {
            d.uncovered_samples = coverage.iter().filter(|v| **v == 0).count();
        }
        if c.anc && !c.template_only.contains(&ch) {
            let cutoff = if c.anc_slice_highpass {
                (fs / l.period / 2.).max(2.).min(fs / 4.)
            } else {
                2.
            };
            let mut applied = false;
            let mut start = 0;
            while start < n {
                check(cancel)?;
                if !corrected[start].is_finite() {
                    start += 1;
                    continue;
                }
                let end = corrected[start..]
                    .iter()
                    .position(|v| !v.is_finite())
                    .map_or(n, |i| start + i);
                if let Some(filtered) = anc(
                    &corrected[start..end],
                    &artifact[start..end],
                    cutoff,
                    fs,
                    c.anc_taps,
                    c.anc_step,
                    cancel,
                )? {
                    // Reset high-pass and adaptive state at every missing-data boundary.
                    for i in start..end {
                        if coverage[i] > 0 {
                            corrected[i] = filtered[i - start];
                        }
                    }
                    applied = true;
                }
                start = end;
            }
            if applied {
                d.anc_channels += 1;
            } else {
                d.warnings.push(format!(
                    "Channel {}: ANC skipped (uninformative reference).",
                    ch + 1
                ));
            }
        }

        if corrected
            .iter()
            .zip(x)
            .any(|(a, b)| b.is_finite() && !a.is_finite())
        {
            return Err(format!(
                "Correction produced nonfinite output on channel {}.",
                ch + 1
            ));
        }
        output.push(corrected);
        progress((ch + 1) as f32 / channels.len() as f32);
    }
    if d.corrected == 0 {
        return Err("No epochs could be corrected. Check triggers, coverage, motion censoring and donor eligibility.".into());
    }
    if d.uncovered_samples > 0 {
        d.warnings.push(format!(
            "{} samples on the reference channel are outside corrected coverage and unchanged.",
            d.uncovered_samples
        ));
    }
    progress(1.);
    Ok(ResultData {
        channels: output,
        diagnostics: d,
    })
}
fn aas_donors(
    w: &[Option<Vec<f64>>],
    eligible: &[bool],
    section: usize,
    c: &Config,
) -> Vec<Vec<usize>> {
    let mut out = vec![vec![]; w.len()];
    for start in (0..w.len()).step_by(section) {
        let end = (start + section).min(w.len());
        let mut accepted = vec![];
        let mut running = vec![0.; w.iter().find_map(|w| w.as_ref()).map(Vec::len).unwrap_or(0)];
        for i in start..end {
            if !eligible[i] {
                continue;
            }
            let x = w[i].as_ref().unwrap();
            if accepted.len() < c.initial_epochs
                || accepted.is_empty()
                || correlation(x, &running) >= c.allen_gate
            {
                let n = accepted.len() as f64;
                for (r, v) in running.iter_mut().zip(x) {
                    *r = (*r * n + v) / (n + 1.);
                }
                accepted.push(i);
            }
        }
        for (i, row) in out.iter_mut().enumerate().take(end).skip(start) {
            *row = accepted.iter().copied().filter(|j| *j != i).collect();
        }
    }
    out
}
fn donors(
    i: usize,
    l: &Layout,
    w: &[Option<Vec<f64>>],
    eligible: &[bool],
    high: &[usize],
    c: &Config,
    d: &mut Diagnostics,
) -> Vec<usize> {
    let target = &l.epochs[i];
    let mut nearest: Vec<_> = (0..w.len())
        .filter(|&j| j != i && eligible[j] && l.epochs[j].group == target.group)
        .collect();
    nearest.sort_by_key(|&j| (l.epochs[j].volume.abs_diff(target.volume), j));
    if c.method == Method::Moosmann && !c.squared_correlation && !high.is_empty() {
        let mut inside: Vec<_> = nearest
            .iter()
            .copied()
            .filter(|&j| {
                !high.iter().any(|&h| {
                    h > target.volume.min(l.epochs[j].volume)
                        && h < target.volume.max(l.epochs[j].volume)
                })
            })
            .take(c.donors)
            .collect();
        if inside.len() < c.donors {
            for &j in &nearest {
                if inside.len() >= c.donors {
                    break;
                }
                if !inside.contains(&j) {
                    inside.push(j);
                    d.motion_barrier_crossings += 1;
                }
            }
        }
        return inside;
    }
    if c.method == Method::Farm || c.squared_correlation {
        let mut scored: Vec<_> = nearest
            .iter()
            .copied()
            .filter(|&j| l.epochs[j].volume.abs_diff(target.volume) <= c.search)
            .map(|j| {
                let r = correlation(w[i].as_ref().unwrap(), w[j].as_ref().unwrap());
                (j, if c.squared_correlation { r * r } else { r })
            })
            .filter(|(_, r)| c.squared_correlation || *r >= c.correlation)
            .collect();
        if c.squared_correlation && eligible[i] {
            scored.push((i, 1.));
        }
        if c.squared_correlation || scored.len() >= c.minimum_donors.min(c.donors) {
            scored.sort_by(|a, b| {
                b.1.total_cmp(&a.1)
                    .then_with(|| a.0.abs_diff(i).cmp(&b.0.abs_diff(i)))
                    .then_with(|| a.0.cmp(&b.0))
            });
            return scored.into_iter().take(c.donors).map(|v| v.0).collect();
        }
        d.correlation_fallbacks += 1;
    }
    nearest.truncate(c.donors);
    nearest
}
fn align(x: &[f32], l: &mut Layout, c: &Config, cancel: &dyn Fn() -> bool) -> Result<(), String> {
    let enabled = c.synchronization != Synchronization::Custom || c.align;
    if !enabled {
        return Ok(());
    }
    let mut radius = if c.synchronization == Synchronization::ScannerSlaved {
        1
    } else {
        ((l.period * c.upsample as f64 / 20.) as usize).clamp(1, 64)
    };
    let fractional = c.synchronization == Synchronization::IndependentClocks
        || c.synchronization == Synchronization::Custom && c.fractional;
    let Some(mut reference) = l
        .epochs
        .iter()
        .find_map(|e| extract(x, e, l.length, c.upsample))
    else {
        return Ok(());
    };
    let mut left_peak = false;
    for lag in 1..(2 * radius + 1).min(reference.len() / 2) {
        let r = correlation(&reference[..reference.len() - lag], &reference[lag..]);
        if r < 0.5 {
            left_peak = true;
        } else if left_peak {
            radius = radius.min((lag / 2).max(1));
            break;
        }
    }
    let original = l.epochs.clone();
    for _ in 0..2 {
        for (i, e) in l.epochs.iter_mut().enumerate() {
            check(cancel)?;
            let mut best = original[i].clone();
            let mut score = f64::NEG_INFINITY;
            let mut shifts: Vec<_> = (-(radius as i32)..=radius as i32).collect();
            shifts.sort_by_key(|v| v.abs());
            for shift in shifts {
                let mut trial = original[i].clone();
                trial.start += shift as f64 / c.upsample as f64;
                if let Some(y) = extract(x, &trial, l.length, c.upsample) {
                    let r = correlation(&y, &reference);
                    if r > score + 1e-12 {
                        score = r;
                        best = trial;
                    }
                }
            }
            if fractional {
                let center = best.start;
                for k in -16..=16 {
                    let mut trial = best.clone();
                    trial.start = center + k as f64 / (32. * c.upsample as f64);
                    if let Some(y) = extract(x, &trial, l.length, c.upsample) {
                        let r = correlation(&y, &reference);
                        if r > score + 1e-12 {
                            score = r;
                            best = trial;
                        }
                    }
                }
            }
            *e = best;
        }
        let early: Vec<_> = l
            .epochs
            .iter()
            .filter_map(|e| extract(x, e, l.length, c.upsample))
            .take(16)
            .collect();
        if !early.is_empty() {
            reference = average(early.iter(), l.length);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Vec<Vec<f32>>, Vec<u64>, Vec<f32>) {
        let n = 4000;
        let brain: Vec<_> = (0..n)
            .map(|i| (2. * std::f64::consts::PI * 7.31 * i as f64 / 100.).sin() as f32)
            .collect();
        let signal: Vec<_> = brain
            .iter()
            .enumerate()
            .map(|(i, b)| b + 100. * (-((i % 100) as f32 - 35.).powi(2) / 40.).exp())
            .collect();
        (vec![signal], (0..40).map(|i| i * 100).collect(), brain)
    }
    fn run(x: &[Vec<f32>], t: &[u64], c: &Config) -> ResultData {
        correct(x, t, 100., c, &|| false, &|_| {}).unwrap()
    }
    #[test]
    fn all_four_methods_remove_repeated_artifact_and_preserve_brain_signal() {
        let (x, t, brain) = fixture();
        for method in [
            Method::AllenAas,
            Method::Fastr,
            Method::Farm,
            Method::Moosmann,
        ] {
            let mut c = Config {
                method,
                scaling: Scaling::Unscaled,
                synchronization: Synchronization::Custom,
                align: false,
                ..Default::default()
            };
            if method == Method::Moosmann {
                c.motion = vec![[0.; 6]; 40];
                for row in &mut c.motion[20..] {
                    row[3] = 1.;
                }
            }
            let out = run(&x, &t, &c);
            let error = out.channels[0]
                .iter()
                .zip(&brain)
                .map(|(a, b)| (a - b).powi(2) as f64)
                .sum::<f64>()
                / brain.len() as f64;
            assert!(error.sqrt() < 0.2, "{method:?}: rms {}", error.sqrt());
            assert_eq!(out.channels[0].len(), brain.len());
            assert_eq!(out.diagnostics.uncovered_samples, 0);
        }
    }
    #[test]
    fn acquisition_gap_before_trigger_support_does_not_change_correction() {
        let (x, t, _) = fixture();
        let prefix = 300;
        let mut continuous = vec![0.; prefix];
        continuous.extend_from_slice(&x[0]);
        let mut paused = continuous.clone();
        paused[20..69].fill(f32::NAN);
        let triggers: Vec<_> = t.iter().map(|v| v + prefix as u64).collect();
        for method in [
            Method::AllenAas,
            Method::Fastr,
            Method::Farm,
            Method::Moosmann,
        ] {
            let c = Config {
                method,
                motion: vec![[0.; 6]; t.len()],
                ..Config::default()
            };
            let a = run(&[continuous.clone()], &triggers, &c);
            let b = run(&[paused.clone()], &triggers, &c);
            assert_eq!(
                a.diagnostics.corrected, b.diagnostics.corrected,
                "{method:?}"
            );
            assert_eq!(a.diagnostics.skipped, b.diagnostics.skipped, "{method:?}");
            assert_eq!(
                &a.channels[0][prefix..],
                &b.channels[0][prefix..],
                "{method:?}"
            );
            assert!(b.channels[0][20..69].iter().all(|v| v.is_nan()));
        }
    }
    #[test]
    fn uncovered_gaps_excluded_channels_and_cancellation_are_safe() {
        let (mut x, mut t, _) = fixture();
        x.push(x[0].clone());
        x[0][500] = f32::NAN;
        t.remove(10);
        t.remove(0);
        let c = Config {
            method: Method::Fastr,
            excluded_channels: vec![1],
            synchronization: Synchronization::Custom,
            align: false,
            ..Default::default()
        };
        let out = run(&x, &t, &c);
        assert!(out.channels[0][500].is_nan());
        assert_eq!(out.channels[1], x[1]);
        assert_eq!(&out.channels[0][..100], &x[0][..100]);
        assert_eq!(&out.channels[0][1001..1100], &x[0][1001..1100]);
        assert!(out.diagnostics.uncovered_samples > 0);
        assert!(correct(&x, &t, 100., &c, &|| true, &|_| {})
            .unwrap_err()
            .contains("Cancelled"));
    }
    #[test]
    fn timing_multiband_fractional_and_invalid_inputs() {
        let (x, t, _) = fixture();
        let mut d = Diagnostics::default();
        let c = Config {
            groups: 3,
            upsample: 4,
            ..Default::default()
        };
        let l = layout(&t, x[0].len(), 100., &c, &mut d).unwrap();
        assert_eq!(l.epochs.len(), 120);
        assert!((l.epochs[1].start - 33.25).abs() < 1e-10);
        let mut c = c;
        c.offsets_seconds = vec![0., 0.2, 0.7];
        let l = layout(&t, x[0].len(), 100., &c, &mut d).unwrap();
        assert_eq!(l.epochs[2].start, 70.);
        c.offsets_seconds = vec![0., 1.];
        assert!(layout(&t, x[0].len(), 100., &c, &mut d).is_err());
        assert!(correct(&x, &[0], 100., &Config::default(), &|| false, &|_| {}).is_err());
        let bad = Config {
            method: Method::Moosmann,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        let bad = Config {
            anc_step: f64::NAN,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
    }
    #[test]
    fn alignment_reduces_trigger_jitter_and_upsampling_preserves_uncovered_samples() {
        let mut x = vec![0f32; 4000];
        let t: Vec<u64> = (0..40).map(|i| i * 100).collect();
        for (i, v) in x.iter_mut().enumerate() {
            let jitter = match (i / 100) % 3 {
                0 => -1.,
                1 => 0.,
                _ => 1.,
            };
            *v = (100. * (-((i % 100) as f64 - 35. - jitter).powi(2) / 16.).exp()) as f32;
        }
        let mut c = Config {
            method: Method::Fastr,
            scaling: Scaling::Unscaled,
            synchronization: Synchronization::Custom,
            align: false,
            ..Default::default()
        };
        let raw = run(&[x.clone()], &t, &c);
        c.synchronization = Synchronization::IndependentClocks;
        let aligned = run(&[x.clone()], &t, &c);
        let energy = |v: &[f32]| {
            v[100..3900]
                .iter()
                .map(|v| f64::from(*v).powi(2))
                .sum::<f64>()
        };
        assert!(energy(&aligned.channels[0]) < energy(&raw.channels[0]) * 0.1);
        c.upsample = 2;
        c.synchronization = Synchronization::Custom;
        c.align = false;
        let out = run(&[x.clone()], &t[1..], &c);
        assert_eq!(&out.channels[0][..100], &x[..100]);
    }
    #[test]
    fn motion_formats_and_metrics() {
        let afni = parse_motion("# header\n0 0 0 0 0 0\n90 0 0 3 4 0", MotionFormat::Afni).unwrap();
        assert!((afni[1][0] - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        let mut c = Config {
            method: Method::Moosmann,
            motion: afni,
            motion_metric: MotionMetric::TranslationSpeed,
            motion_threshold: 5.5,
            ..Default::default()
        };
        assert!(motion::high_motion(&c, 2, &mut vec![]).unwrap().is_empty());
        c.motion_metric = MotionMetric::TranslationSum;
        assert_eq!(motion::high_motion(&c, 2, &mut vec![]).unwrap(), vec![1]);
        assert_eq!(
            parse_motion("1 2 3 0.1 0.2 0.3", MotionFormat::Spm).unwrap()[0],
            [0.1, 0.2, 0.3, 1., 2., 3.]
        );
        assert!(parse_motion("1 2 NaN 4 5 6", MotionFormat::Fsl).is_err());
    }
    #[test]
    fn donor_policies_and_scale_modes() {
        let (x, t, _) = fixture();
        let mut d = Diagnostics::default();
        let mut c = Config {
            method: Method::Farm,
            correlation: 1.,
            minimum_donors: 4,
            synchronization: Synchronization::Custom,
            align: false,
            ..Default::default()
        };
        let out = run(&x, &t, &c);
        assert!(out.diagnostics.correlation_fallbacks > 0);
        let l = layout(&t, 4000, 100., &c, &mut d).unwrap();
        let w: Vec<_> = l
            .epochs
            .iter()
            .map(|e| extract(&x[0], e, l.length, 1))
            .collect();
        let mut eligible = vec![true; 40];
        eligible[19] = false;
        c.method = Method::Moosmann;
        c.donors = 4;
        let picked = donors(20, &l, &w, &eligible, &[19], &c, &mut d);
        assert!(picked.iter().all(|i| *i > 20));
        assert!(!picked.contains(&19));
        for mode in [
            Scaling::Unscaled,
            Scaling::DriftTracking,
            Scaling::LeastSquares,
        ] {
            c.method = Method::Fastr;
            c.scaling = mode;
            assert!(run(&x, &t, &c).channels[0].iter().all(|v| v.is_finite()));
        }
    }
    #[test]
    fn obs_removes_rank_one_residual_and_respects_energy_floor() {
        let shape: Vec<_> = (0..101).map(|i| (i as f64 * 0.3).sin()).collect();
        let residuals: Vec<_> = (0..16)
            .map(|i| Some(shape.iter().map(|v| v * (i + 1) as f64).collect::<Vec<_>>()))
            .collect();
        let mut templates = vec![Some(vec![10.; 101]); 16];
        let mut d = Diagnostics::default();
        let c = Config {
            obs: Obs::Fixed,
            components: 1,
            residual_floor: 0.,
            ..Default::default()
        };
        apply_obs(
            &mut templates,
            &residuals,
            &[true; 16],
            100.,
            100.,
            &c,
            &|| false,
            &mut d,
        )
        .unwrap();
        assert_eq!(d.obs_components, 1);
        assert!(templates[0]
            .as_ref()
            .unwrap()
            .iter()
            .any(|v| (*v - 10.).abs() > 0.1));
        let before = templates.clone();
        let c = Config {
            residual_floor: 1e12,
            ..c
        };
        apply_obs(
            &mut templates,
            &residuals,
            &[true; 16],
            100.,
            100.,
            &c,
            &|| false,
            &mut d,
        )
        .unwrap();
        assert_eq!(templates, before);
    }
    #[test]
    fn anc_reduces_correlated_noise_and_skips_empty_reference() {
        let reference: Vec<_> = (0..6000).map(|i| (i as f64 * 0.8).sin() * 10.).collect();
        let x: Vec<_> = reference.iter().map(|v| (*v * 0.4) as f32).collect();
        let out = anc(&x, &reference, 2., 100., 8, 0.01, &|| false)
            .unwrap()
            .unwrap();
        let before = x[3000..].iter().map(|v| v * v).sum::<f32>();
        let after = out[3000..].iter().map(|v| v * v).sum::<f32>();
        assert!(after < before * 0.05);
        assert!(anc(&x, &vec![0.; x.len()], 2., 100., 8, 0.01, &|| false)
            .unwrap()
            .is_none());
    }
}
