use super::Config;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionMetric {
    TranslationSum,
    TranslationSpeed,
    AllSix,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionFormat {
    Afni,
    Fsl,
    Spm,
}
/// Six columns. Normalized storage: rotation radians XYZ, translation mm XYZ.
/// AFNI is roll/pitch/yaw degrees, dS/dL/dP mm; FSL rotations then translations;
/// SPM translations then rotations. AFNI's axis permutation does not change norms.
pub fn parse_motion(text: &str, format: MotionFormat) -> Result<Vec<[f64; 6]>, String> {
    let mut out = vec![];
    for (line, row) in text.lines().enumerate() {
        let row = row.split('#').next().unwrap_or("").trim();
        if row.is_empty() {
            continue;
        }
        let values: Vec<f64> = row
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .map_err(|_| format!("Motion line {} is not numeric.", line + 1))?;
        if values.len() != 6 || values.iter().any(|v| !v.is_finite()) {
            return Err(format!(
                "Motion line {} must contain six finite values; select a six-column motion export.",
                line + 1
            ));
        }
        let mut v = [0.; 6];
        v.copy_from_slice(&values);
        match format {
            MotionFormat::Afni => {
                for a in &mut v[..3] {
                    *a = a.to_radians();
                }
            }
            MotionFormat::Fsl => {}
            MotionFormat::Spm => {
                v = [
                    values[3], values[4], values[5], values[0], values[1], values[2],
                ];
            }
        }
        out.push(v);
        if out.len() > 200_000 {
            return Err("Motion file exceeds 200,000 rows.".into());
        }
    }
    if out.is_empty() {
        Err("Motion file has no rows.".into())
    } else {
        Ok(out)
    }
}
pub(super) fn high_motion(
    c: &Config,
    volumes: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<usize>, String> {
    if c.motion.is_empty() || !(c.censor_motion || c.method == super::Method::Moosmann) {
        return Ok(vec![]);
    }
    // Match EVA's explicit front-padding/trailing-truncation rule, always reported.
    let mut motion = vec![[0.; 6]; volumes];
    if c.motion.len() < volumes {
        let missing = volumes - c.motion.len();
        motion[missing..].copy_from_slice(&c.motion);
        warnings.push(format!(
            "Motion file front-padded by {missing} zero rows (assumed omitted dummy scans)."
        ));
    } else {
        motion.copy_from_slice(&c.motion[..volumes]);
        if c.motion.len() > volumes {
            warnings.push(format!(
                "Ignored {} trailing motion rows.",
                c.motion.len() - volumes
            ));
        }
    }
    let mut high = vec![];
    for i in 1..volumes {
        let delta: Vec<_> = motion[i]
            .iter()
            .zip(motion[i - 1])
            .map(|(a, b)| (a - b).abs())
            .collect();
        let value = match c.motion_metric {
            MotionMetric::TranslationSum => delta[3..].iter().sum(),
            MotionMetric::TranslationSpeed => delta[3..].iter().map(|v| v * v).sum::<f64>().sqrt(),
            MotionMetric::AllSix => {
                delta[3..].iter().sum::<f64>() + c.motion_radius * delta[..3].iter().sum::<f64>()
            }
        };
        if value > c.motion_threshold {
            high.push(i);
        }
    }
    Ok(high)
}
