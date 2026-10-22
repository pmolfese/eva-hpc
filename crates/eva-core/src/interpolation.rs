//! Perrin spherical splines; matches EVA macOS SphericalSpline defaults.
//! Order 4, 40 Legendre terms, lambda 1e-5; unit vectors use the MFF origin.
use eva_mff::Recording;
#[derive(Debug, Clone, PartialEq)]
pub struct Recipe {
    pub target: usize,
    pub donors: Vec<usize>,
    pub weights: Vec<f64>,
}
fn g(x: f64) -> f64 {
    let x = x.clamp(-1., 1.);
    let (mut prev, mut curr) = (1., x);
    let mut sum = 0.;
    for n in 1..=40 {
        let k = n as f64;
        let p = if n == 1 {
            curr
        } else {
            let p = ((2. * k - 1.) * x * curr - (k - 1.) * prev) / k;
            prev = curr;
            curr = p;
            p
        };
        sum += (2. * k + 1.) / (k * (k + 1.)).powi(4) * p;
    }
    sum / (4. * std::f64::consts::PI)
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
pub fn recipes(r: &Recording, targets: &[usize], bad: &[usize]) -> Result<Vec<Recipe>, String> {
    if r.is_pns() {
        return Err("Spherical splines require scalp EEG electrodes, not PNS.".into());
    }
    let channels = &r.metadata().channels;
    if targets.iter().chain(bad).any(|&i| i >= channels.len()) {
        return Err("Channel number is outside the recording.".into());
    }
    let mut targets = targets.to_vec();
    targets.sort_unstable();
    targets.dedup();
    if targets.is_empty() {
        return Ok(vec![]);
    }
    for &t in &targets {
        if channels[t].position.is_none() {
            return Err(format!("No scalp coordinates for channel {}.", t + 1));
        }
    }
    let donors: Vec<_> = (0..channels.len())
        .filter(|i| !targets.contains(i) && !bad.contains(i) && channels[*i].position.is_some())
        .collect();
    if donors.len() < 3 {
        return Err(
            "Interpolation requires at least three good scalp electrodes with coordinates.".into(),
        );
    }
    if donors.len() > 512 {
        return Err("This interpolation solver supports at most 512 donor electrodes.".into());
    }
    for &t in &targets {
        if donors.iter().any(|&d| channels[d].unit != channels[t].unit) {
            return Err(
                "Interpolation requires matching channel units (or uniformly unspecified units)."
                    .into(),
            );
        }
    }
    let positions = channels.iter().map(|c| c.position).collect::<Vec<_>>();
    solve(&positions, &targets, &donors)
}
fn solve(
    positions: &[Option<[f64; 3]>],
    targets: &[usize],
    donors: &[usize],
) -> Result<Vec<Recipe>, String> {
    let n = donors.len();
    let size = n + 1;
    let width = size + targets.len();
    let mut a = vec![vec![0.; width]; size];
    for i in 0..n {
        for j in 0..n {
            let cosine = dot(positions[donors[i]].unwrap(), positions[donors[j]].unwrap());
            if i != j && cosine > 1. - 1e-12 {
                return Err("Duplicate donor positions; cannot interpolate reliably.".into());
            }
            a[i][j] = g(cosine) + if i == j { 1e-5 } else { 0. };
        }
        a[i][n] = 1.;
        a[n][i] = 1.;
    }
    for (k, &t) in targets.iter().enumerate() {
        for i in 0..n {
            a[i][size + k] = g(dot(positions[t].unwrap(), positions[donors[i]].unwrap()));
        }
        a[n][size + k] = 1.;
    }
    // Partial-pivot elimination, shared across all target right-hand sides.
    for k in 0..size {
        let pivot = (k..size)
            .max_by(|&i, &j| a[i][k].abs().total_cmp(&a[j][k].abs()))
            .unwrap();
        if a[pivot][k].abs() < 1e-14 {
            return Err("Singular spherical-spline geometry.".into());
        }
        a.swap(k, pivot);
        for i in k + 1..size {
            let f = a[i][k] / a[k][k];
            a[i][k] = 0.;
            let (before, after) = a.split_at_mut(i);
            for (value, pivot) in after[0][k + 1..width]
                .iter_mut()
                .zip(&before[k][k + 1..width])
            {
                *value -= f * pivot;
            }
        }
    }
    let mut results = vec![];
    for (k, &target) in targets.iter().enumerate() {
        let mut w = vec![0.; size];
        for i in (0..size).rev() {
            w[i] = (a[i][size + k] - (i + 1..size).map(|j| a[i][j] * w[j]).sum::<f64>()) / a[i][i];
        }
        w.pop();
        if w.iter().any(|v| !v.is_finite()) || (w.iter().sum::<f64>() - 1.).abs() > 1e-6 {
            return Err("Unstable spherical-spline weights.".into());
        }
        results.push(Recipe {
            target,
            donors: donors.to_vec(),
            weights: w,
        });
    }
    Ok(results)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn symmetric_geometry_preserves_constant_and_excludes_target() {
        let p = vec![
            Some([0., 0., 1.]),
            Some([1., 0., 0.]),
            Some([0., 1., 0.]),
            Some([-1., 0., 0.]),
            Some([0., -1., 0.]),
        ];
        let r = solve(&p, &[0], &[1, 2, 3, 4]).unwrap();
        assert!(r[0].weights.iter().all(|w| (w - 0.25).abs() < 1e-10));
        assert!((r[0].weights.iter().map(|w| w * 7.).sum::<f64>() - 7.).abs() < 1e-10);
        assert!(solve(&[Some([1., 0., 0.]); 4], &[0], &[1, 2, 3]).is_err());
    }
}
