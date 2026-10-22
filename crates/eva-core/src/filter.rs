//! Shared GUI and CLI filtering. Source recordings remain unchanged.
//! IIR/notch: cookbook biquads; FIR: Hamming-window sinc, delay compensated.
//! CleanLine: EVA's overlapping-window sinusoid regression, not EEGLAB multitaper.
use eva_mff::{Recording, SampleValues};
use rustfft::{num_complex::Complex, FftPlanner};
use std::{
    f64::consts::PI,
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
};

#[path = "elliptic.rs"]
mod elliptic;
#[path = "filter_options.rs"]
mod options;
pub use options::*;

pub const TILE: u64 = 16384;
const MAX_CONTEXT: usize = 1_048_576;
const MAX_TAPS: usize = 32767;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Iir,
    Fir,
    Auto,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineMode {
    Off,
    Notch,
    CleanLine,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub interpolation: Vec<crate::interpolation::Recipe>,
    pub enabled: bool,
    pub family: Family,
    pub high_pass: bool,
    pub high_hz: f64,
    pub low_pass: bool,
    pub low_hz: f64,
    pub order: usize,
    pub low_order: usize,
    pub options: Options,
    pub transition_hz: f64,
    pub line: LineMode,
    pub line_hz: f64,
    pub harmonics: usize,
    pub notch_q: f64,
    pub window_seconds: f64,
    pub strength: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            interpolation: vec![],
            enabled: false,
            family: Family::Iir,
            high_pass: true,
            high_hz: 1.,
            low_pass: true,
            low_hz: 40.,
            order: 4,
            low_order: 4,
            options: Options::default(),
            transition_hz: 1.,
            line: LineMode::Off,
            line_hz: 60.,
            harmonics: 1,
            notch_q: 30.,
            window_seconds: 4.,
            strength: 1.,
        }
    }
}
impl Settings {
    pub fn active(&self) -> bool {
        self.enabled
            && (self.high_pass
                || self.low_pass
                || self.line != LineMode::Off
                || self.options.average_reference)
    }
    pub fn summary(&self) -> String {
        if !self.active() {
            if !self.interpolation.is_empty() {
                return format!(
                    "Spherical-spline interpolation: {}",
                    self.interpolation
                        .iter()
                        .map(|r| (r.target + 1).to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
            return "Unfiltered · GCAL applied".into();
        }
        let mut parts = Vec::new();
        if self.high_pass || self.low_pass {
            parts.push(match self.family {
                Family::Iir => format!(
                    "IIR {:?} HP/LP {}/{} poles",
                    self.options.design, self.order, self.low_order
                ),
                Family::Fir => format!(
                    "FIR {:?} {:?}",
                    self.options.window, self.options.application
                ),
                Family::Auto => "Auto IIR/FIR".into(),
            });
            if self.high_pass {
                parts.push(format!("HP {} Hz", self.high_hz));
            }
            if self.low_pass {
                parts.push(format!("LP {} Hz", self.low_hz));
            }
        }
        if self.line != LineMode::Off {
            parts.push(format!(
                "{} {} Hz ×{}",
                if self.line == LineMode::Notch {
                    if self.options.fir_notch {
                        "FIR notch"
                    } else {
                        "IIR notch"
                    }
                } else {
                    "CleanLine (EVA)"
                },
                self.line_hz,
                self.harmonics
            ));
        }
        if self.options.average_reference {
            parts.push("average reference".into());
        }
        parts.join(" · ")
    }
    pub fn validate(&self, rate: u32) -> Result<(), String> {
        if !self.active() {
            return Ok(());
        }
        let nyq = f64::from(rate) / 2.;
        let frequency = |f: f64| f.is_finite() && f > 0. && f < nyq;
        if rate == 0
            || (self.high_pass && !frequency(self.high_hz))
            || (self.low_pass && !frequency(self.low_hz))
        {
            return Err(format!(
                "Cutoffs must be above 0 and below Nyquist ({nyq} Hz)."
            ));
        }
        if self.high_pass && self.low_pass && self.high_hz >= self.low_hz {
            return Err("High-pass cutoff must be below low-pass cutoff.".into());
        }
        if ![2, 4, 6, 8].contains(&self.order) || ![2, 4, 6, 8].contains(&self.low_order) {
            return Err("IIR order must be 2, 4, 6, or 8 poles per pass.".into());
        }
        self.validate_options(rate)?;
        if self.line != LineMode::Off {
            if !(1..=4).contains(&self.harmonics)
                || !frequency(self.line_hz)
                || !frequency(self.line_hz * self.harmonics as f64)
            {
                return Err(format!("Every selected line harmonic must be below Nyquist ({nyq} Hz); reduce harmonics if needed."));
            }
            if !self.notch_q.is_finite() || !(1.0..=1000.).contains(&self.notch_q) {
                return Err("Notch Q must be 1–1000.".into());
            }
            if !self.window_seconds.is_finite()
                || !(0.5..=10.).contains(&self.window_seconds)
                || !self.strength.is_finite()
                || !(0.1..=1.5).contains(&self.strength)
            {
                return Err("CleanLine window must be 0.5–10 s and strength 0.1–1.5.".into());
            }
        }
        if self.context(rate) > MAX_CONTEXT {
            return Err("This design needs too much boundary context. Raise the high-pass cutoff, lower the order/Q, or widen the FIR transition.".into());
        }
        Ok(())
    }
    fn sections(&self, rate: u32) -> Vec<Biquad> {
        let fs = f64::from(rate);
        let mut sections = Vec::new();
        for (active, hz, high, order) in [
            (self.high_pass, self.high_hz, true, self.order),
            (self.low_pass, self.low_hz, false, self.low_order),
        ] {
            if active && self.edge_family(high) == Family::Iir {
                if self.options.design == IirDesign::Elliptic {
                    sections.extend(elliptic::sections(
                        hz,
                        fs,
                        order,
                        high,
                        self.options.ripple,
                        self.options.stop_db,
                    ));
                } else {
                    for k in 0..order / 2 {
                        let q = 1. / (2. * (PI * (2 * k + 1) as f64 / (2 * order) as f64).cos());
                        sections.push(Biquad::new(
                            if high { Kind::High } else { Kind::Low },
                            hz,
                            fs,
                            q,
                        ));
                    }
                }
            }
        }
        if self.line == LineMode::Notch && !self.options.fir_notch {
            for h in 1..=self.harmonics {
                sections.push(Biquad::new(
                    Kind::Notch,
                    self.line_hz * h as f64,
                    fs,
                    self.notch_q,
                ));
            }
        }
        sections
    }
    fn context(&self, rate: u32) -> usize {
        if !self.active() {
            return 0;
        }
        let mut pad = self
            .sections(rate)
            .iter()
            .fold(0usize, |n, s| n.saturating_add(s.settle()));
        for (on, high) in [(self.high_pass, true), (self.low_pass, false)] {
            if on && self.edge_family(high) == Family::Fir {
                pad = pad.saturating_add(self.edge_taps(rate, high).saturating_mul(2));
            }
        }
        if self.line == LineMode::Notch && self.options.fir_notch {
            pad = pad.saturating_add(self.notch_taps(rate).saturating_mul(2));
        }
        if self.line == LineMode::CleanLine {
            pad = pad.saturating_add((2. * self.window_seconds * f64::from(rate)).ceil() as usize);
        }
        pad.max(32)
    }
}
#[derive(Clone, Copy)]
enum Kind {
    Low,
    High,
    Notch,
}
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}
impl Biquad {
    fn new(kind: Kind, hz: f64, fs: f64, q: f64) -> Self {
        let omega = 2. * PI * hz / fs;
        let c = omega.cos();
        let alpha = omega.sin() / (2. * q);
        let b = match kind {
            Kind::Low => [(1. - c) / 2., 1. - c, (1. - c) / 2.],
            Kind::High => [(1. + c) / 2., -(1. + c), (1. + c) / 2.],
            Kind::Notch => [1., -2. * c, 1.],
        };
        Self {
            b: b.map(|v| v / (1. + alpha)),
            a: [-2. * c / (1. + alpha), (1. - alpha) / (1. + alpha)],
        }
    }
    fn pass_precision(&self, x: &mut [f64], float: bool) {
        if !float {
            self.pass(x);
            return;
        }
        let b = self.b.map(|v| v as f32);
        let a = self.a.map(|v| v as f32);
        let gain = b.iter().sum::<f32>() / (1. + a.iter().sum::<f32>());
        let (mut x1, mut x2) = (x[0] as f32, x[0] as f32);
        let (mut y1, mut y2) = (x1 * gain, x1 * gain);
        for v in x {
            let input = *v as f32;
            let y = b[0] * input + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
            x2 = x1;
            x1 = input;
            y2 = y1;
            y1 = y;
            *v = f64::from(y);
        }
    }
    fn settle(&self) -> usize {
        let disc = self.a[0] * self.a[0] - 4. * self.a[1];
        let r = if disc < 0. {
            self.a[1].sqrt()
        } else {
            ((-self.a[0] + disc.sqrt()) / 2.)
                .abs()
                .max(((-self.a[0] - disc.sqrt()) / 2.).abs())
        };
        if !(0.0..1.0).contains(&r) {
            return MAX_CONTEXT + 1;
        }
        (1e-10f64.ln() / r.ln()).ceil().max(1.) as usize
    }
    fn pass(&self, x: &mut [f64]) {
        // Steady-state initialization for the endpoint; avoids a DC startup step.
        let gain = self.b.iter().sum::<f64>() / (1. + self.a.iter().sum::<f64>());
        let (mut x1, mut x2) = (x[0], x[0]);
        let (mut y1, mut y2) = (x[0] * gain, x[0] * gain);
        for v in x {
            let input = *v;
            let y = self.b[0] * input + self.b[1] * x1 + self.b[2] * x2
                - self.a[0] * y1
                - self.a[1] * y2;
            x2 = x1;
            x1 = input;
            y2 = y1;
            y1 = y;
            *v = y;
        }
    }
}
fn kernel(hz: f64, fs: f64, taps: usize, high: bool) -> Vec<f64> {
    let mid = taps / 2;
    let fc = hz / fs;
    let mut b: Vec<_> = (0..taps)
        .map(|i| {
            let k = i as f64 - mid as f64;
            let sinc = if i == mid {
                2. * fc
            } else {
                (2. * PI * fc * k).sin() / (PI * k)
            };
            sinc * (0.54 - 0.46 * (2. * PI * i as f64 / (taps - 1) as f64).cos())
        })
        .collect();
    let sum = b.iter().sum::<f64>();
    for v in &mut b {
        *v /= sum;
    }
    if high {
        for v in &mut b {
            *v = -*v;
        }
        b[mid] += 1.;
    }
    b
}
fn check(g: &AtomicU64, id: u64) -> Result<(), String> {
    if g.load(Ordering::Relaxed) != id {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}
fn reflect(i: isize, n: usize) -> usize {
    if n == 1 {
        return 0;
    }
    let period = 2 * (n - 1) as isize;
    let k = i.rem_euclid(period);
    if k < n as isize {
        k as usize
    } else {
        (period - k) as usize
    }
}
fn fir(x: &[f64], b: &[f64]) -> Vec<f64> {
    let half = b.len() / 2;
    let padded = x.len() + 2 * half;
    let n = (padded + b.len() - 1).next_power_of_two();
    let mut a = vec![Complex::default(); n];
    let mut h = vec![Complex::default(); n];
    for (i, v) in a[..padded].iter_mut().enumerate() {
        v.re = x[reflect(i as isize - half as isize, x.len())];
    }
    for (v, &c) in h.iter_mut().zip(b) {
        v.re = c;
    }
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);
    fft.process(&mut a);
    fft.process(&mut h);
    for (v, k) in a.iter_mut().zip(h) {
        *v *= k;
    }
    planner.plan_fft_inverse(n).process(&mut a);
    a[2 * half..2 * half + x.len()]
        .iter()
        .map(|v| v.re / n as f64)
        .collect()
}

fn fir_forward(x: &[f64], b: &[f64]) -> Vec<f64> {
    let n = (x.len() + b.len() - 1).next_power_of_two();
    let mut a = vec![Complex::default(); n];
    let mut h = vec![Complex::default(); n];
    for (v, &sample) in a.iter_mut().zip(x) {
        v.re = sample;
    }
    for (v, &tap) in h.iter_mut().zip(b) {
        v.re = tap;
    }
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);
    fft.process(&mut a);
    fft.process(&mut h);
    for (v, k) in a.iter_mut().zip(h) {
        *v *= k;
    }
    planner.plan_fft_inverse(n).process(&mut a);
    a[..x.len()].iter().map(|v| v.re / n as f64).collect()
}

pub struct Plan {
    source: Option<std::sync::Arc<Vec<Vec<f32>>>>,
    settings: Settings,
    rate: u32,
    sections: Vec<Biquad>,
    kernels: Vec<Vec<f64>>,
    context: usize,
    reference_cache: std::cell::RefCell<Option<(Range<u64>, Vec<f32>)>>,
}
impl Plan {
    pub fn new(settings: &Settings, rate: u32) -> Result<Self, String> {
        settings.validate(rate)?;
        let kernels = if settings.active() {
            settings.kernels(rate)
        } else {
            Vec::new()
        };
        Ok(Self {
            source: None,
            settings: settings.clone(),
            rate,
            sections: if settings.active() {
                settings.sections(rate)
            } else {
                Vec::new()
            },
            kernels,
            context: settings.context(rate),
            reference_cache: std::cell::RefCell::new(None),
        })
    }
    pub fn with_source(mut self, source: Option<std::sync::Arc<Vec<Vec<f32>>>>) -> Self {
        self.source = source;
        self
    }
    fn raw(&self, r: &Recording, ch: usize, range: Range<u64>) -> Result<Vec<f32>, String> {
        if let Some(source) = &self.source {
            source
                .get(ch)
                .and_then(|v| v.get(range.start as usize..range.end as usize))
                .map(|v| v.to_vec())
                .ok_or_else(|| "Corrected sample range is invalid".into())
        } else {
            r.read_window(&[ch], range, SampleValues::Calibrated)
                .map(|w| w.values.into_iter().next().unwrap())
                .map_err(|e| e.to_string())
        }
    }
    fn process(
        &self,
        x: &mut [f64],
        absolute_start: u64,
        g: &AtomicU64,
        id: u64,
    ) -> Result<(), String> {
        if self.sections.is_empty()
            && self.kernels.is_empty()
            && self.settings.line != LineMode::CleanLine
        {
            return Ok(());
        }
        let mut pos = 0;
        while pos < x.len() {
            check(g, id)?;
            if !x[pos].is_finite() {
                pos += 1;
                continue;
            }
            let start = pos;
            while pos < x.len() && x[pos].is_finite() {
                pos += 1;
            }
            let run = &mut x[start..pos];
            if run.len() < 3 {
                run.fill(f64::NAN);
                continue;
            }
            // Reflection at file/gap edges; interior tile boundaries are cropped away.
            let pad = self.context.min(run.len() - 1);
            let mut y: Vec<f64> = (0..run.len() + 2 * pad)
                .map(|i| run[reflect(i as isize - pad as isize, run.len())])
                .collect();
            for section in &self.sections {
                check(g, id)?;
                section.pass_precision(&mut y, self.settings.use_float(self.rate));
            }
            y.reverse();
            for section in self.sections.iter().rev() {
                check(g, id)?;
                section.pass_precision(&mut y, self.settings.use_float(self.rate));
            }
            y.reverse();
            // FIR is a single symmetric pass with delay removed, not squared filtfilt.
            for b in &self.kernels {
                check(g, id)?;
                y = match self.settings.options.application {
                    FirApplication::DelayCompensated => fir(&y, b),
                    FirApplication::ZeroPhase => fir(&fir(&y, b), b),
                    FirApplication::Forward => fir_forward(&y, b),
                };
            }
            run.copy_from_slice(&y[pad..pad + run.len()]);
            if self.settings.line == LineMode::CleanLine {
                for harmonic in 1..=self.settings.harmonics {
                    clean_line(
                        run,
                        f64::from(self.rate),
                        self.settings.line_hz * harmonic as f64,
                        self.settings.window_seconds,
                        self.settings.strength,
                        absolute_start + start as u64,
                        g,
                        id,
                    )?;
                }
            }
            if run
                .iter()
                .any(|v| !v.is_finite() || v.abs() > f64::from(f32::MAX))
            {
                return Err(
                    "Filter became numerically unstable; use less extreme settings.".into(),
                );
            }
        }
        Ok(())
    }
    /// Interpolation is evaluated after temporal filtering and reference from
    /// independent donors, never recursively from other replacements.
    pub fn read(
        &self,
        r: &Recording,
        ch: usize,
        range: Range<u64>,
        g: &AtomicU64,
        id: u64,
    ) -> Result<Vec<f32>, String> {
        if range.start > range.end
            || range.end > r.metadata().sample_count
            || range.end - range.start > 16_777_216
        {
            return Err("Invalid or oversized processing window".into());
        }
        if let Some(recipe) = self.settings.interpolation.iter().find(|v| v.target == ch) {
            if r.is_pns() || recipe.donors.len() != recipe.weights.len() || recipe.donors.len() < 3
            {
                return Err("Invalid interpolation recipe".into());
            }
            let mut sum = vec![0f64; (range.end - range.start) as usize];
            for (&donor, &weight) in recipe.donors.iter().zip(&recipe.weights) {
                check(g, id)?;
                let values = self.read_base(r, donor, range.clone(), g, id)?;
                for (s, v) in sum.iter_mut().zip(values) {
                    *s += f64::from(v) * weight;
                }
            }
            if sum
                .iter()
                .any(|v| v.is_infinite() || (v.is_finite() && v.abs() > f64::from(f32::MAX)))
            {
                return Err("Interpolated samples overflow float32".into());
            }
            return Ok(sum.into_iter().map(|v| v as f32).collect());
        }
        self.read_base(r, ch, range, g, id)
    }
    fn read_base(
        &self,
        r: &Recording,
        ch: usize,
        range: Range<u64>,
        g: &AtomicU64,
        id: u64,
    ) -> Result<Vec<f32>, String> {
        let mut out = self.read_signal(r, ch, range.clone(), g, id)?;
        if !self.settings.active()
            || !self.settings.options.average_reference
            || r.metadata().channels.len() < 2
        {
            return Ok(out);
        }
        let units: Vec<_> = r
            .metadata()
            .channels
            .iter()
            .map(|c| c.unit.as_deref())
            .collect();
        if units[0].is_none() || units.iter().any(|u| *u != units[0]) {
            return Err("Average reference requires matching, known channel units.".into());
        }
        if self
            .reference_cache
            .borrow()
            .as_ref()
            .is_none_or(|(cached, _)| *cached != range)
        {
            let mut sum = vec![0f64; out.len()];
            let mut count = 0;
            for c in 0..r.metadata().channels.len() {
                check(g, id)?;
                if self.settings.options.excluded.contains(&c) {
                    continue;
                }
                let values = self.read_signal(r, c, range.clone(), g, id)?;
                for (s, v) in sum.iter_mut().zip(values) {
                    *s += f64::from(v);
                }
                count += 1;
            }
            if count == 0 {
                return Err("Average reference needs at least one included channel.".into());
            }
            *self.reference_cache.borrow_mut() = Some((
                range,
                sum.into_iter().map(|v| (v / count as f64) as f32).collect(),
            ));
        }
        for (v, m) in out
            .iter_mut()
            .zip(&self.reference_cache.borrow().as_ref().unwrap().1)
        {
            *v -= m;
        }
        Ok(out)
    }
    /// Canonical tiles make filtered sample values independent of viewport/zoom.
    fn read_signal(
        &self,
        r: &Recording,
        ch: usize,
        range: Range<u64>,
        g: &AtomicU64,
        id: u64,
    ) -> Result<Vec<f32>, String> {
        if !self.settings.active() {
            return self.raw(r, ch, range);
        }
        let mut out = Vec::with_capacity((range.end - range.start) as usize);
        let total = r.metadata().sample_count;
        let mut tile = range.start / TILE * TILE;
        while tile < range.end {
            check(g, id)?;
            let tile_end = (tile + TILE).min(total);
            let mut read_start = tile.saturating_sub(self.context as u64);
            if self.settings.line == LineMode::CleanLine {
                let hop = ((self.settings.window_seconds * f64::from(self.rate)).round() as u64
                    / 2)
                .max(1);
                read_start = read_start / hop * hop;
            }
            let read_end = tile_end.saturating_add(self.context as u64).min(total);
            let raw = self.raw(r, ch, read_start..read_end)?;
            let mut values: Vec<_> = raw.iter().map(|&v| f64::from(v)).collect();
            self.process(&mut values, read_start, g, id)?;
            let a = range.start.max(tile);
            let b = range.end.min(tile_end);
            out.extend(
                values[(a - read_start) as usize..(b - read_start) as usize]
                    .iter()
                    .map(|&v| v as f32),
            );
            tile = tile_end;
        }
        Ok(out)
    }
}

/// EVA regression: fit a sine/cosine pair to demeaned windows, gate explained
/// energy, and Hann-blend corrections at 50% overlap. No multitaper/F test.
#[allow(clippy::too_many_arguments)]
fn clean_line(
    x: &mut [f64],
    fs: f64,
    hz: f64,
    seconds: f64,
    strength: f64,
    offset: u64,
    g: &AtomicU64,
    id: u64,
) -> Result<(), String> {
    let n = (seconds * fs).round().max(32.) as usize;
    let n = n.min(x.len());
    if n < 16 {
        return Ok(());
    }
    let hop = (n / 2).max(1);
    let omega = 2. * PI * hz / fs;
    let mut starts = vec![0];
    let first = (hop - offset as usize % hop) % hop;
    for start in (first..=x.len() - n).step_by(hop) {
        if starts.last() != Some(&start) {
            starts.push(start);
        }
    }
    if starts.last() != Some(&(x.len() - n)) {
        starts.push(x.len() - n);
    }
    let mut correction = vec![0.; x.len()];
    let mut weight = vec![0.; x.len()];
    let trig: Vec<_> = (0..n)
        .map(|i| {
            (
                (omega * i as f64).cos(),
                (omega * i as f64).sin(),
                0.5 - 0.5 * (2. * PI * i as f64 / (n - 1) as f64).cos(),
            )
        })
        .collect();
    for start in starts {
        check(g, id)?;
        let mean = x[start..start + n].iter().sum::<f64>() / n as f64;
        let (mut cc, mut ss, mut cs, mut yc, mut ys, mut energy) = (0., 0., 0., 0., 0., 0.);
        for (i, &(c, s, _)) in trig.iter().enumerate() {
            let y = x[start + i] - mean;
            cc += c * c;
            ss += s * s;
            cs += c * s;
            yc += y * c;
            ys += y * s;
            energy += y * y;
        }
        let det = cc * ss - cs * cs;
        if det <= 1e-12 || energy <= 1e-12 {
            continue;
        }
        let a = (yc * ss - ys * cs) / det;
        let b = (ys * cc - yc * cs) / det;
        let fitted = trig
            .iter()
            .map(|&(c, s, _)| (a * c + b * s).powi(2))
            .sum::<f64>();
        if fitted / energy < (0.002 / strength.max(0.1)).max(0.0002) {
            continue;
        }
        for (i, &(c, s, w)) in trig.iter().enumerate() {
            correction[start + i] += (a * c + b * s) * w;
            weight[start + i] += w;
        }
    }
    for i in 0..x.len() {
        if weight[i] > 1e-12 {
            x[i] -= strength * correction[i] / weight[i];
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tones(n: usize, fs: f64, components: &[(f64, f64)]) -> Vec<f64> {
        (0..n)
            .map(|i| {
                components
                    .iter()
                    .map(|&(hz, amp)| amp * (2. * PI * hz * i as f64 / fs).sin())
                    .sum::<f64>()
                    + 10.
            })
            .collect()
    }
    fn amplitude(x: &[f64], fs: f64, hz: f64, crop: usize) -> f64 {
        let x = &x[crop..x.len() - crop];
        let n = x.len() as f64;
        let c = x
            .iter()
            .enumerate()
            .map(|(i, v)| v * (2. * PI * hz * (i + crop) as f64 / fs).cos())
            .sum::<f64>()
            * 2.
            / n;
        let s = x
            .iter()
            .enumerate()
            .map(|(i, v)| v * (2. * PI * hz * (i + crop) as f64 / fs).sin())
            .sum::<f64>()
            * 2.
            / n;
        c.hypot(s)
    }
    fn run(settings: Settings, x: &mut [f64], fs: u32) {
        Plan::new(&settings, fs)
            .unwrap()
            .process(x, 0, &AtomicU64::new(1), 1)
            .unwrap();
    }
    #[test]
    fn iir_and_fir_keep_passband_reject_stopbands() {
        for family in [Family::Iir, Family::Fir] {
            let mut x = tones(10000, 250., &[(0.2, 1.), (10., 1.), (90., 1.)]);
            run(
                Settings {
                    enabled: true,
                    family,
                    high_hz: 2.,
                    low_hz: 40.,
                    transition_hz: 2.,
                    ..Default::default()
                },
                &mut x,
                250,
            );
            assert!(
                (amplitude(&x, 250., 10., 2000) - 1.).abs() < 0.015,
                "{family:?}"
            );
            assert!(amplitude(&x, 250., 0.2, 2000) < 0.015, "{family:?}");
            assert!(amplitude(&x, 250., 90., 2000) < 0.015, "{family:?}");
            assert!(x[2000..8000].iter().sum::<f64>().abs() / 6000. < 0.002);
        }
    }
    #[test]
    fn zero_phase_impulse_alignment() {
        for family in [Family::Iir, Family::Fir] {
            let mut x = vec![0.; 10001];
            x[5000] = 1.;
            run(
                Settings {
                    enabled: true,
                    family,
                    high_pass: false,
                    low_hz: 30.,
                    transition_hz: 3.,
                    ..Default::default()
                },
                &mut x,
                250,
            );
            let peak = x
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            assert_eq!(peak, 5000);
            for i in 1..100 {
                assert!((x[5000 - i] - x[5000 + i]).abs() < 1e-9);
            }
        }
    }
    #[test]
    fn notch_and_cleanline_remove_line_and_harmonic() {
        for line in [LineMode::Notch, LineMode::CleanLine] {
            let mut x = tones(10000, 250., &[(10., 1.), (50., 2.), (100., 0.8)]);
            run(
                Settings {
                    enabled: true,
                    high_pass: false,
                    low_pass: false,
                    line,
                    line_hz: 50.,
                    harmonics: 2,
                    ..Default::default()
                },
                &mut x,
                250,
            );
            assert!(amplitude(&x, 250., 50., 2000) < 0.01, "{line:?}");
            assert!(amplitude(&x, 250., 100., 2000) < 0.01, "{line:?}");
            assert!(
                (amplitude(&x, 250., 10., 2000) - 1.).abs() < 0.01,
                "{line:?}"
            );
        }
    }
    #[test]
    fn cleanline_does_not_remove_unrelated_tone() {
        let mut x = tones(10000, 250., &[(11., 2.)]);
        let original = x.clone();
        run(
            Settings {
                enabled: true,
                high_pass: false,
                low_pass: false,
                line: LineMode::CleanLine,
                line_hz: 50.,
                ..Default::default()
            },
            &mut x,
            250,
        );
        assert!(x.iter().zip(original).all(|(a, b)| (a - b).abs() < 1e-10));
    }
    #[test]
    fn gaps_do_not_poison_other_samples_and_cancellation_is_checked() {
        for family in [Family::Iir, Family::Fir] {
            let settings = Settings {
                enabled: true,
                family,
                transition_hz: 2.,
                high_hz: 2.,
                ..Default::default()
            };
            let plan = Plan::new(&settings, 250).unwrap();
            let mut x = tones(10000, 250., &[(10., 1.)]);
            x[5000] = f64::NAN;
            plan.process(&mut x, 0, &AtomicU64::new(1), 1).unwrap();
            assert!(x[5000].is_nan());
            assert!(x[..5000].iter().chain(&x[5001..]).all(|v| v.is_finite()));
            assert!(plan.process(&mut x, 0, &AtomicU64::new(2), 1).is_err());
        }
    }
    #[test]
    fn reject_invalid_and_excessive_designs() {
        let base = Settings {
            enabled: true,
            ..Default::default()
        };
        for bad in [
            Settings {
                high_hz: 50.,
                low_hz: 40.,
                ..base.clone()
            },
            Settings {
                low_hz: 125.,
                ..base.clone()
            },
            Settings {
                line: LineMode::Notch,
                harmonics: 3,
                ..base.clone()
            },
            Settings {
                high_hz: f64::NAN,
                ..base.clone()
            },
            Settings {
                family: Family::Fir,
                transition_hz: 0.0001,
                ..base.clone()
            },
            Settings {
                high_hz: 0.00001,
                ..base.clone()
            },
        ] {
            assert!(bad.validate(250).is_err(), "{bad:?}");
        }
        assert!(base.validate(250).is_ok());
    }
}

#[path = "filter_cli.rs"]
pub mod cli;
