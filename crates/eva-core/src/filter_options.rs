//! Settings and FIR mechanics ported from EVA's EEGSignalFilter/FilterViewModel.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IirDesign {
    Butterworth,
    Elliptic,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirWindow {
    Hamming,
    Kaiser,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirApplication {
    ZeroPhase,
    DelayCompensated,
    Forward,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    Auto,
    Float,
    Double,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesignRule {
    Eva,
    EeglabMne,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Eeglab,
    Erplab,
    MnePython,
    NetStation,
}
impl Preset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Eeglab => "EEGLAB",
            Self::Erplab => "ERPLAB",
            Self::MnePython => "MNE-Python",
            Self::NetStation => "EGI Net Station",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub design: IirDesign,
    pub ripple: f64,
    pub stop_db: f64,
    pub window: FirWindow,
    pub attenuation: f64,
    pub application: FirApplication,
    pub crossover: f64,
    pub through: bool,
    pub auto_transition: bool,
    pub rule: DesignRule,
    pub fir_notch: bool,
    pub precision: Precision,
    pub average_reference: bool,
    pub excluded: Vec<usize>,
    pub filter_pns: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            design: IirDesign::Butterworth,
            ripple: 0.1,
            stop_db: 60.,
            window: FirWindow::Hamming,
            attenuation: 60.,
            application: FirApplication::DelayCompensated,
            crossover: 1.,
            through: false,
            auto_transition: false,
            rule: DesignRule::Eva,
            fir_notch: false,
            precision: Precision::Auto,
            average_reference: false,
            excluded: vec![],
            filter_pns: true,
        }
    }
}
impl Settings {
    pub fn preset(&mut self, p: Preset) {
        match p {
            Preset::Eeglab | Preset::MnePython => {
                self.family = Family::Fir;
                self.options.window = FirWindow::Hamming;
                self.options.application = FirApplication::DelayCompensated;
                self.options.auto_transition = true;
                self.options.rule = DesignRule::EeglabMne;
            }
            Preset::Erplab => {
                self.family = Family::Iir;
                self.options.design = IirDesign::Butterworth;
                self.order = 4;
                self.low_order = 4;
                self.options.rule = DesignRule::Eva;
            }
            Preset::NetStation => {
                self.family = Family::Auto;
                self.options.design = IirDesign::Elliptic;
                self.options.ripple = 0.1;
                self.options.stop_db = 60.;
                self.options.window = FirWindow::Kaiser;
                self.options.attenuation = 60.;
                self.options.application = FirApplication::Forward;
                self.options.crossover = 1.;
                self.options.through = true;
                self.options.auto_transition = true;
                self.options.rule = DesignRule::Eva;
            }
        }
    }
    pub fn edge_family(&self, high: bool) -> Family {
        match self.family {
            Family::Auto => {
                if high
                    && (self.high_hz < self.options.crossover
                        || self.options.through && self.high_hz == self.options.crossover)
                {
                    Family::Iir
                } else {
                    Family::Fir
                }
            }
            v => v,
        }
    }
    fn transition(&self, rate: u32, high: bool) -> f64 {
        if !self.options.auto_transition {
            return self.transition_hz;
        }
        let hz = if high { self.high_hz } else { self.low_hz };
        (hz * 0.25)
            .max(2.)
            .min(if high { hz } else { rate as f64 / 2. - hz })
            .max(rate as f64 / 10000.)
    }
    fn tap_count(&self, rate: u32, transition: f64) -> usize {
        let estimate = match self.options.window {
            FirWindow::Hamming => 3.3 * rate as f64 / transition,
            FirWindow::Kaiser => {
                (self.options.attenuation.max(21.) - 8.)
                    / (2.285 * 2. * PI * transition / rate as f64)
                    + 1.
            }
        };
        (estimate.ceil() as usize).max(3) | 1
    }
    pub fn edge_taps(&self, rate: u32, high: bool) -> usize {
        let mut t = self.transition(rate, high);
        if self.options.rule == DesignRule::EeglabMne
            && self.high_pass
            && self.low_pass
            && self.edge_family(true) == Family::Fir
            && self.edge_family(false) == Family::Fir
        {
            t = t.min(self.transition(rate, !high));
        }
        self.tap_count(rate, t)
    }
    pub fn notch_taps(&self, rate: u32) -> usize {
        self.tap_count(
            rate,
            if self.options.auto_transition {
                1.
            } else {
                self.transition_hz
            },
        )
    }
    pub(super) fn validate_options(&self, rate: u32) -> Result<(), String> {
        let o = &self.options;
        if !o.crossover.is_finite()
            || o.crossover <= 0.
            || !o.ripple.is_finite()
            || !(0.001..=10.).contains(&o.ripple)
            || !o.stop_db.is_finite()
            || o.stop_db <= o.ripple
            || o.stop_db > 120.
            || !o.attenuation.is_finite()
            || !(21.0..=120.).contains(&o.attenuation)
        {
            return Err("Check crossover, elliptic ripple (0.001–10 dB), stopband (above ripple, ≤120 dB), and Kaiser attenuation (21–120 dB).".into());
        }
        if !o.auto_transition && (!self.transition_hz.is_finite() || self.transition_hz <= 0.) {
            return Err("FIR transition must be positive.".into());
        }
        for (on, high, hz) in [
            (self.high_pass, true, self.high_hz),
            (self.low_pass, false, self.low_hz),
        ] {
            if !on {
                continue;
            }
            if self.edge_family(high) == Family::Fir {
                let t = self.transition(rate, high);
                let nyq = rate as f64 / 2.;
                let room = if o.rule == DesignRule::EeglabMne {
                    if high {
                        hz
                    } else {
                        nyq - hz
                    }
                } else {
                    2. * hz.min(nyq - hz)
                };
                if t > room {
                    return Err("FIR transition extends beyond DC or Nyquist.".into());
                }
                if self.edge_taps(rate, high) > MAX_TAPS {
                    return Err(format!(
                        "FIR needs {} taps (limit {MAX_TAPS}); widen transition or use Auto/IIR.",
                        self.edge_taps(rate, high)
                    ));
                }
            } else if o.design == IirDesign::Elliptic
                && elliptic::sections(
                    hz,
                    rate as f64,
                    if high { self.order } else { self.low_order },
                    high,
                    o.ripple,
                    o.stop_db,
                )
                .is_empty()
            {
                return Err("Elliptic parameters could not produce a stable design.".into());
            }
        }
        if self.line == LineMode::Notch && o.fir_notch && self.notch_taps(rate) > MAX_TAPS {
            return Err("FIR notch exceeds the tap limit; widen transition.".into());
        }
        if o.precision == Precision::Float
            && self.sections(rate).iter().any(|s| {
                let a = s.a.map(|v| v as f32);
                (1. + a[0] + a[1]).abs() < 1e-7 || a[1].abs() >= 1.
            })
        {
            return Err("Float precision is unsafe for this design; choose Auto or Double.".into());
        }
        Ok(())
    }
    pub(super) fn use_float(&self, _rate: u32) -> bool {
        // Auto uses double for recursive sections: predictable stability across tiled reads.
        self.options.precision == Precision::Float
    }
    pub(super) fn kernels(&self, rate: u32) -> Vec<Vec<f64>> {
        let mut out = Vec::new();
        for (on, high, mut hz) in [
            (self.high_pass, true, self.high_hz),
            (self.low_pass, false, self.low_hz),
        ] {
            if on && self.edge_family(high) == Family::Fir {
                if self.options.rule == DesignRule::EeglabMne {
                    hz += if high { -0.5 } else { 0.5 } * self.transition(rate, high);
                }
                out.push(window_kernel(
                    hz,
                    rate as f64,
                    self.edge_taps(rate, high),
                    high,
                    self.options.window,
                    self.options.attenuation,
                ));
            }
        }
        if self.line == LineMode::Notch && self.options.fir_notch {
            let taps = self.notch_taps(rate);
            let mut b = vec![0.; taps];
            b[taps / 2] = 1.;
            for h in 1..=self.harmonics {
                let c = self.line_hz * h as f64;
                let nyq = rate as f64 / 2.;
                let upper = window_kernel(
                    (c + 0.5).min(nyq * (1. - 1e-4)),
                    rate as f64,
                    taps,
                    false,
                    self.options.window,
                    self.options.attenuation,
                );
                let lower = window_kernel(
                    (c - 0.5).max(nyq * 1e-4),
                    rate as f64,
                    taps,
                    false,
                    self.options.window,
                    self.options.attenuation,
                );
                for i in 0..taps {
                    b[i] -= upper[i] - lower[i];
                }
            }
            out.push(b);
        }
        out
    }
}
fn i0(x: f64) -> f64 {
    let mut sum = 1.;
    let mut term = 1.;
    for j in 1..100 {
        term *= (x / (2. * j as f64)).powi(2);
        sum += term;
        if term < sum * 1e-15 {
            break;
        }
    }
    sum
}
fn window_kernel(
    hz: f64,
    fs: f64,
    taps: usize,
    high: bool,
    window: FirWindow,
    attenuation: f64,
) -> Vec<f64> {
    if window == FirWindow::Hamming {
        return kernel(hz, fs, taps, high);
    }
    let beta = if attenuation > 50. {
        0.1102 * (attenuation - 8.7)
    } else {
        0.5842 * (attenuation - 21.).powf(0.4) + 0.07886 * (attenuation - 21.)
    };
    let mid = taps / 2;
    let fc = hz / fs;
    let mut b: Vec<f64> = (0..taps)
        .map(|i| {
            let d = i as f64 - mid as f64;
            let sinc = if i == mid {
                2. * fc
            } else {
                (2. * PI * fc * d).sin() / (PI * d)
            };
            let r = 2. * i as f64 / (taps - 1) as f64 - 1.;
            sinc * i0(beta * (1. - r * r).max(0.).sqrt()) / i0(beta)
        })
        .collect();
    let sum = b.iter().sum::<f64>();
    for v in &mut b {
        *v /= sum;
        if high {
            *v = -*v;
        }
    }
    if high {
        b[mid] += 1.;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_boundary_and_presets_keep_signal_choices() {
        let mut s = Settings {
            enabled: true,
            high_hz: 1.,
            low_hz: 35.,
            line: LineMode::CleanLine,
            ..Default::default()
        };
        s.options.average_reference = true;
        s.preset(Preset::NetStation);
        assert_eq!(s.edge_family(true), Family::Iir);
        assert_eq!(s.edge_family(false), Family::Fir);
        s.options.through = false;
        assert_eq!(s.edge_family(true), Family::Fir);
        for p in [
            Preset::Eeglab,
            Preset::MnePython,
            Preset::Erplab,
            Preset::NetStation,
        ] {
            s.preset(p);
            s.validate(250).unwrap();
            assert_eq!((s.high_hz, s.low_hz), (1., 35.));
            assert_eq!(s.line, LineMode::CleanLine);
            assert!(s.options.average_reference);
        }
    }
    #[test]
    fn independent_iir_orders_and_fir_application_delay() {
        let s = Settings {
            enabled: true,
            order: 2,
            low_order: 8,
            ..Default::default()
        };
        assert_eq!(s.sections(250).len(), 5);
        let mut s = Settings {
            enabled: true,
            family: Family::Fir,
            high_pass: false,
            low_hz: 30.,
            transition_hz: 5.,
            ..Default::default()
        };
        s.options.window = FirWindow::Kaiser;
        let taps = s.edge_taps(250, false);
        for (mode, expected) in [
            (FirApplication::Forward, 3000 + taps / 2),
            (FirApplication::DelayCompensated, 3000),
            (FirApplication::ZeroPhase, 3000),
        ] {
            s.options.application = mode;
            let p = Plan::new(&s, 250).unwrap();
            let mut x = vec![0.; 6001];
            x[3000] = 1.;
            p.process(&mut x, 0, &AtomicU64::new(1), 1).unwrap();
            let peak = x
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .unwrap()
                .0;
            assert_eq!(peak, expected, "{mode:?}");
        }
    }
    #[test]
    fn elliptic_responses_stable_and_fir_notch_suppresses_line() {
        for order in [2, 4, 6, 8] {
            for high in [true, false] {
                let sos =
                    elliptic::sections(if high { 1. } else { 40. }, 250., order, high, 0.1, 60.);
                assert_eq!(sos.len(), order / 2);
                assert!(sos.iter().all(|s| s.a[1] > 0. && s.a[1] < 1.));
            }
        }
        let response = |sos: &[Biquad], hz: f64| {
            let z = Complex::from_polar(1., -2. * PI * hz / 250.);
            sos.iter()
                .map(|s| {
                    ((s.b[0] + s.b[1] * z + s.b[2] * z * z) / (1. + s.a[0] * z + s.a[1] * z * z))
                        .norm_sqr()
                })
                .product::<f64>()
        };
        let sos = elliptic::sections(40., 250., 4, false, 0.1, 60.);
        assert!(response(&sos, 10.) > 0.95);
        assert!(response(&sos, 90.) < 0.01);
        let mut s = Settings {
            enabled: true,
            high_pass: false,
            low_pass: false,
            line: LineMode::Notch,
            line_hz: 50.,
            harmonics: 2,
            transition_hz: 0.5,
            ..Default::default()
        };
        s.options.fir_notch = true;
        s.options.window = FirWindow::Kaiser;
        s.options.application = FirApplication::ZeroPhase;
        let kernel = s.kernels(250).remove(0);
        let gain = |hz: f64| {
            kernel
                .iter()
                .enumerate()
                .map(|(i, b)| Complex::from_polar(*b, -2. * PI * hz * i as f64 / 250.))
                .sum::<Complex<f64>>()
                .norm_sqr()
        };
        assert!(gain(50.) < 0.01);
        assert!(gain(100.) < 0.01);
        assert!(gain(10.) > 0.95);
    }
    #[test]
    fn fir_rules_and_precision_are_effective() {
        let mut s = Settings {
            enabled: true,
            family: Family::Fir,
            high_pass: false,
            low_hz: 30.,
            ..Default::default()
        };
        s.options.auto_transition = true;
        let eva = s.kernels(250);
        s.options.rule = DesignRule::EeglabMne;
        let package = s.kernels(250);
        assert_ne!(eva, package);
        let gain = |b: &Vec<f64>| {
            b.iter()
                .enumerate()
                .map(|(i, v)| Complex::from_polar(*v, -2. * PI * 30. * i as f64 / 250.))
                .sum::<Complex<f64>>()
                .norm()
        };
        assert!((gain(&eva[0]) - 0.5).abs() < 0.02);
        assert!(gain(&package[0]) > 0.98);
        let mut s = Settings {
            enabled: true,
            high_pass: false,
            ..Default::default()
        };
        let mut a: Vec<f64> = (0..2000).map(|i| (i as f64 * 0.13).sin()).collect();
        let mut b = a.clone();
        Plan::new(&s, 250)
            .unwrap()
            .process(&mut a, 0, &AtomicU64::new(1), 1)
            .unwrap();
        s.options.precision = Precision::Float;
        Plan::new(&s, 250)
            .unwrap()
            .process(&mut b, 0, &AtomicU64::new(1), 1)
            .unwrap();
        assert_ne!(a, b);
        assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4));
    }
}
