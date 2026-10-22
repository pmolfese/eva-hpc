use crate::gradient::{self, Config, Diagnostics};
use eva_mff::{Recording, SampleValues};
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};
pub struct Request {
    pub config: Config,
    pub trigger: String,
    pub pns: bool,
    pub progress: Arc<AtomicU32>,
}
pub struct Corrected {
    pub eeg: Arc<Vec<Vec<f32>>>,
    pub pns: Option<Arc<Vec<Vec<f32>>>>,
    pub reports: Vec<(String, Diagnostics)>,
    pub description: String,
    /// Immutable settings used for this result, for downstream analysis provenance.
    pub configuration: String,
}
pub fn triggers(r: &Recording, code: &str) -> Result<Vec<u64>, String> {
    let mut out = vec![];
    for event in r.metadata().events.iter().filter(|e| e.code == code) {
        let sample = event
            .sample_index(r.metadata().sample_rate)
            .map_err(|e| e.to_string())?;
        if sample >= 0 && (sample as u64) < r.metadata().sample_count {
            out.push(sample as u64);
        }
    }
    out.sort_unstable();
    out.dedup();
    if out.len() < 2 {
        return Err(format!(
            "Marker {code:?} has fewer than two usable volume triggers."
        ));
    }
    Ok(out)
}
pub fn run(
    r: &Recording,
    request: &Request,
    cancel: &dyn Fn() -> bool,
) -> Result<Corrected, String> {
    if r.metadata().kind == eva_mff::RecordingKind::SegmentedOrAveraged {
        return Err("MRI correction does not support segmented or averaged recordings.".into());
    }
    request.config.validate()?;
    if request
        .config
        .excluded_channels
        .iter()
        .chain(&request.config.template_only)
        .any(|&i| i >= r.metadata().channels.len())
    {
        return Err("EEG channel selection is outside recording".into());
    }
    let count = triggers(r, &request.trigger)?.len();
    if request.config.censored.iter().any(|&i| i >= count) {
        return Err("Censored volume index is outside trigger list".into());
    }
    let total = r
        .metadata()
        .sample_count
        .saturating_mul(r.metadata().channels.len() as u64)
        + if request.pns {
            r.pns()
                .map(|p| {
                    p.metadata()
                        .sample_count
                        .saturating_mul(p.metadata().channels.len() as u64)
                })
                .unwrap_or(0)
        } else {
            0
        };
    if total > 134_217_728 {
        return Err("EEG+PNS correction exceeds the current 512 MiB input limit. Whole-recording disk-backed correction is not implemented yet.".into());
    }
    let process = |r: &Recording,
                   config: &Config,
                   base: f32,
                   span: f32|
     -> Result<gradient::ResultData, String> {
        let markers = triggers(r, &request.trigger)?;
        let mut channels = Vec::new();
        for ch in 0..r.metadata().channels.len() {
            if cancel() {
                return Err("Cancelled".into());
            }
            let mut values = Vec::with_capacity(r.metadata().sample_count as usize);
            let mut start = 0;
            while start < r.metadata().sample_count {
                if cancel() {
                    return Err("Cancelled".into());
                }
                let end = (start + 65536).min(r.metadata().sample_count);
                values.extend(
                    r.read_window(&[ch], start..end, SampleValues::Calibrated)
                        .map_err(|e| e.to_string())?
                        .values
                        .remove(0),
                );
                start = end;
            }
            channels.push(values);
        }
        gradient::correct(
            &channels,
            &markers,
            r.metadata().sample_rate as f64,
            config,
            cancel,
            &|fraction| {
                request
                    .progress
                    .store(((base + span * fraction) * 1000.) as u32, Ordering::Relaxed);
            },
        )
    };
    let has_pns = request.pns && r.pns().is_some();
    let eeg = process(r, &request.config, 0., if has_pns { 0.8 } else { 1. })?;
    let mut reports = vec![("EEG".into(), eeg.diagnostics)];
    let pns = if has_pns {
        let mut c = request.config.clone();
        c.excluded_channels.clear();
        c.template_only.clear();
        let data = process(r.pns().unwrap(), &c, 0.8, 0.2)?;
        reports.push(("PNS".into(), data.diagnostics));
        Some(Arc::new(data.channels))
    } else {
        None
    };
    Ok(Corrected {
        configuration: format!(
            "trigger={:?}; pns={}; config={:?}",
            request.trigger, request.pns, request.config
        ),
        eeg: Arc::new(eeg.channels),
        pns,
        reports,
        description: format!(
            "{} · trigger {} · {} groups",
            request.config.method.label(),
            request.trigger,
            if request.config.offsets_seconds.is_empty() {
                request.config.groups
            } else {
                request.config.offsets_seconds.len()
            }
        ),
    })
}
