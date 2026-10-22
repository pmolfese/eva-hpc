use super::{Config, Diagnostics, Obs};
pub(super) fn check(cancel: &dyn Fn() -> bool) -> Result<(), String> {
    if cancel() {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}
pub(super) fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}
pub(super) fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
pub(super) fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let am = a.iter().sum::<f64>() / a.len() as f64;
    let bm = b.iter().sum::<f64>() / b.len() as f64;
    let (mut ab, mut aa, mut bb) = (0., 0., 0.);
    for (a, b) in a.iter().zip(b) {
        let a = a - am;
        let b = b - bm;
        ab += a * b;
        aa += a * a;
        bb += b * b;
    }
    if aa * bb <= 1e-24 {
        0.
    } else {
        (ab / (aa * bb).sqrt()).clamp(-1., 1.)
    }
}
pub(super) fn finite_variance(x: &[f32]) -> f64 {
    let mut n = 0.;
    let mut mean = 0.;
    let mut m2 = 0.;
    for &v in x.iter().filter(|v| v.is_finite()) {
        n += 1.;
        let d = f64::from(v) - mean;
        mean += d / n;
        m2 += d * (f64::from(v) - mean);
    }
    if n > 0. {
        m2 / n
    } else {
        0.
    }
}
pub(super) fn average<'a>(v: impl Iterator<Item = &'a Vec<f64>>, len: usize) -> Vec<f64> {
    let mut out = vec![0.; len];
    let mut n = 0.;
    for row in v {
        n += 1.;
        for (a, b) in out.iter_mut().zip(row) {
            *a += b;
        }
    }
    if n > 0. {
        for v in &mut out {
            *v /= n;
        }
    }
    out
}
fn interpolate(len: usize, get: impl Fn(usize) -> f64, t: f64, lobes: i32) -> f64 {
    if (t - t.round()).abs() < 1e-10 {
        return get((t.round() as isize).clamp(0, len as isize - 1) as usize);
    }
    let base = t.floor() as isize;
    let mut sum = 0.;
    let mut weight = 0.;
    for j in (-lobes + 1)..=lobes {
        let i = base + j as isize;
        let d = t - i as f64;
        let pi = std::f64::consts::PI * d;
        let k = if d.abs() >= lobes as f64 {
            0.
        } else {
            pi.sin() / pi * (pi / lobes as f64).sin() / (pi / lobes as f64)
        };
        sum += k * get(i.clamp(0, len as isize - 1) as usize);
        weight += k;
    }
    sum / weight
}
pub(super) fn sample(x: &[f32], t: f64, lobes: i32) -> f64 {
    interpolate(x.len(), |i| f64::from(x[i]), t, lobes)
}
pub(super) fn sample64(x: &[f64], t: f64, lobes: i32) -> f64 {
    interpolate(x.len(), |i| x[i], t, lobes)
}
fn detrend(x: &[f64]) -> Vec<f64> {
    let center = (x.len() - 1) as f64 / 2.;
    let mean = x.iter().sum::<f64>() / x.len() as f64;
    let denom = (0..x.len())
        .map(|i| (i as f64 - center).powi(2))
        .sum::<f64>();
    let slope = x
        .iter()
        .enumerate()
        .map(|(i, v)| (i as f64 - center) * (v - mean))
        .sum::<f64>()
        / denom.max(1.);
    x.iter()
        .enumerate()
        .map(|(i, v)| v - mean - slope * (i as f64 - center))
        .collect()
}
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_obs(
    templates: &mut [Option<Vec<f64>>],
    residuals: &[Option<Vec<f64>>],
    eligible: &[bool],
    period: f64,
    fs: f64,
    c: &Config,
    cancel: &dyn Fn() -> bool,
    d: &mut Diagnostics,
) -> Result<(), String> {
    let step = (c.chunk_seconds * fs / period).round().max(8.) as usize;
    let mut start = 0;
    while start < residuals.len() {
        check(cancel)?;
        let mut end = (start + step).min(residuals.len());
        if residuals.len() - end < 8 {
            end = residuals.len();
        }
        let candidates: Vec<_> = (start..end)
            .filter(|&i| eligible[i] && residuals[i].is_some())
            .collect();
        if candidates.len() >= 8 {
            let count = candidates.len().min(c.max_basis_epochs);
            let selected: Vec<_> = (0..count)
                .map(|k| {
                    candidates[(k as f64 * (candidates.len() - 1) as f64 / (count - 1) as f64)
                        .round() as usize]
                })
                .collect();
            let rows: Vec<_> = selected
                .iter()
                .map(|&i| detrend(residuals[i].as_ref().unwrap()))
                .collect();
            let removed = selected
                .iter()
                .filter_map(|&i| templates[i].as_ref())
                .map(|v| dot(v, v))
                .sum::<f64>();
            let energy = rows.iter().map(|v| dot(v, v)).sum::<f64>();
            if energy > 1e-20 && energy >= removed * c.residual_floor {
                let n = rows.len();
                let len = rows[0].len();
                let mut gram = vec![vec![0.; n]; n];
                for i in 0..n {
                    check(cancel)?;
                    for j in i..n {
                        let v = dot(&rows[i], &rows[j]);
                        gram[i][j] = v;
                        gram[j][i] = v;
                    }
                }
                let wanted = if c.obs == Obs::Fixed {
                    c.components
                } else {
                    c.max_components
                }
                .min(n)
                .min(len);
                let mut basis: Vec<Vec<f64>> = vec![];
                let mut explained = 0.;
                for component in 0..wanted {
                    check(cancel)?;
                    // Deterministic power iteration with deflation and a residual convergence check.
                    let mut v: Vec<_> = (0..n)
                        .map(|i| ((i + 1) * (component + 3)) as f64)
                        .map(f64::sin)
                        .collect();
                    let mut converged = false;
                    for _ in 0..500 {
                        check(cancel)?;
                        let mut next: Vec<_> = gram.iter().map(|r| dot(r, &v)).collect();
                        let norm = dot(&next, &next).sqrt();
                        if norm <= 1e-20 {
                            break;
                        }
                        for x in &mut next {
                            *x /= norm;
                        }
                        let av: Vec<_> = gram.iter().map(|r| dot(r, &next)).collect();
                        let lambda = dot(&next, &av);
                        let err = av
                            .iter()
                            .zip(&next)
                            .map(|(a, b)| (a - lambda * b).powi(2))
                            .sum::<f64>()
                            .sqrt();
                        v = next;
                        if err < lambda.abs() * 1e-8 + 1e-12 {
                            converged = true;
                            break;
                        }
                    }
                    if !converged {
                        d.warnings.push(
                            "OBS eigenvector did not converge; remaining components skipped."
                                .into(),
                        );
                        break;
                    }
                    let lambda = dot(&v, &gram.iter().map(|r| dot(r, &v)).collect::<Vec<_>>());
                    if lambda <= energy * 1e-12 {
                        break;
                    }
                    let mut b = vec![0.; len];
                    for (row, weight) in rows.iter().zip(&v) {
                        for (a, x) in b.iter_mut().zip(row) {
                            *a += weight * x;
                        }
                    }
                    for old in &basis {
                        let proj = dot(&b, old);
                        for (a, v) in b.iter_mut().zip(old) {
                            *a -= proj * v;
                        }
                    }
                    let norm = dot(&b, &b).sqrt();
                    if norm < 1e-12 {
                        break;
                    }
                    for a in &mut b {
                        *a /= norm;
                    }
                    basis.push(b);
                    explained += lambda;
                    for i in 0..n {
                        for j in 0..n {
                            gram[i][j] -= lambda * v[i] * v[j];
                        }
                    }
                    if c.obs == Obs::Automatic && explained / energy >= c.variance {
                        break;
                    }
                }
                d.obs_components += basis.len();
                for i in start..end {
                    if let (Some(t), Some(r)) = (&mut templates[i], &residuals[i]) {
                        let detrended = detrend(r);
                        for b in &basis {
                            let a = dot(&detrended, b);
                            for (t, b) in t.iter_mut().zip(b) {
                                *t += a * b;
                            }
                        }
                    }
                }
            }
        }
        start = end;
    }
    Ok(())
}
fn highpass(x: &[f64], hz: f64, fs: f64) -> Vec<f64> {
    if x.len() < 9 || hz >= fs / 2. {
        return x.to_vec();
    }
    let n = x.len();
    let pad = (3. * fs / hz).max(16.).min((n - 1) as f64) as usize;
    let mut y = Vec::with_capacity(n + 2 * pad);
    for i in (1..=pad).rev() {
        y.push(2. * x[0] - x[i]);
    }
    y.extend_from_slice(x);
    for i in 1..=pad {
        y.push(2. * x[n - 1] - x[n - 1 - i]);
    }
    let w = (std::f64::consts::PI * hz / fs).tan();
    let norm = 1. / (1. + 2f64.sqrt() * w + w * w);
    let a1 = 2. * (w * w - 1.) * norm;
    let a2 = (1. - 2f64.sqrt() * w + w * w) * norm;
    for _ in 0..2 {
        let (mut x1, mut x2, mut y1, mut y2) = (0., 0., 0., 0.);
        for v in &mut y {
            let input = *v;
            let out = norm * (input - 2. * x1 + x2) - a1 * y1 - a2 * y2;
            x2 = x1;
            x1 = input;
            y2 = y1;
            y1 = out;
            *v = out;
        }
        y.reverse();
    }
    y[pad..pad + n].to_vec()
}
pub(super) fn anc(
    x: &[f32],
    reference: &[f64],
    hz: f64,
    fs: f64,
    taps: usize,
    step: f64,
    cancel: &dyn Fn() -> bool,
) -> Result<Option<Vec<f32>>, String> {
    if x.iter().any(|v| !v.is_finite()) {
        return Ok(None);
    }
    let desired = highpass(&x.iter().map(|&v| f64::from(v)).collect::<Vec<_>>(), hz, fs);
    let source = highpass(reference, hz, fs);
    let variance = |v: &[f64]| {
        let m = v.iter().sum::<f64>() / v.len() as f64;
        v.iter().map(|v| (v - m).powi(2)).sum::<f64>() / v.len() as f64
    };
    let power = variance(&source);
    if power <= 1e-12 || power <= 1e-4 * variance(&desired) {
        return Ok(None);
    }
    let regularizer = (power * 1e-6).max(f64::MIN_POSITIVE);
    let mut weights = vec![0.; taps];
    let mut out = x.to_vec();
    for i in 0..x.len() {
        if i % 4096 == 0 {
            check(cancel)?;
        }
        let count = taps.min(i + 1);
        let mut predicted = 0.;
        let mut energy = 0.;
        for (j, w) in weights.iter().take(count).enumerate() {
            predicted += w * source[i - j];
            energy += source[i - j].powi(2);
        }
        let gain = step * (desired[i] - predicted) / (energy + regularizer);
        for (j, w) in weights.iter_mut().take(count).enumerate() {
            *w += gain * source[i - j];
        }
        out[i] = (f64::from(x[i]) - predicted) as f32;
        if !out[i].is_finite() {
            return Ok(None);
        }
    }
    Ok(Some(out))
}
