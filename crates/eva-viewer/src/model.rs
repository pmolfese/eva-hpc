use crate::filter::{Plan, Settings, TILE};
use eva_mff::Recording;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Debug, PartialEq)]
pub struct ViewKey {
    pub range: Range<u64>,
    pub channels: Vec<usize>,
    pub columns: usize,
    pub filters: Settings,
}
#[derive(Clone, Copy, Debug)]
pub struct Bin {
    pub min: f32,
    pub max: f32,
    pub gap: bool,
}
impl Default for Bin {
    fn default() -> Self {
        Self {
            min: f32::INFINITY,
            max: f32::NEG_INFINITY,
            gap: false,
        }
    }
}
#[derive(Debug)]
pub struct Trace {
    pub bins: Vec<Bin>,
    pub mean: f64,
}
#[derive(Debug)]
pub struct DisplayData {
    pub key: ViewKey,
    pub traces: Vec<Trace>,
    pub samples_per_bin: u64,
}

#[cfg(test)]
pub fn reduce(r: &Recording, key: ViewKey, g: &AtomicU64, id: u64) -> Result<DisplayData, String> {
    reduce_from(r, key, g, id, None)
}
/// One bounded chunk at a time, reduced on the worker. A bucket containing any
/// nonfinite sample is deliberately blank; we never bridge a missing span.
pub fn reduce_from(
    recording: &Recording,
    key: ViewKey,
    generation: &AtomicU64,
    id: u64,
    source: Option<std::sync::Arc<Vec<Vec<f32>>>>,
) -> Result<DisplayData, String> {
    let count = key.range.end - key.range.start;
    if count == 0 || key.columns == 0 || key.columns > 4096 || key.channels.len() > 128 {
        return Err("Invalid display request".into());
    }
    let plan = Plan::new(&key.filters, recording.metadata().sample_rate)?.with_source(source);
    let step = count.div_ceil(key.columns as u64).max(1);
    let n = count.div_ceil(step) as usize;
    let mut traces: Vec<_> = key
        .channels
        .iter()
        .map(|_| Trace {
            bins: vec![Bin::default(); n],
            mean: 0.,
        })
        .collect();
    let mut finite = vec![0u64; key.channels.len()];
    let mut start = key.range.start;
    while start < key.range.end {
        let end = ((start / TILE + 1) * TILE).min(key.range.end);
        for (row, &ch) in key.channels.iter().enumerate() {
            if generation.load(Ordering::Relaxed) != id {
                return Err("Cancelled".into());
            }
            let window = plan.read(recording, ch, start..end, generation, id)?;
            for (i, &value) in window.iter().enumerate() {
                let bin =
                    &mut traces[row].bins[((start - key.range.start + i as u64) / step) as usize];
                if value.is_finite() {
                    bin.min = bin.min.min(value);
                    bin.max = bin.max.max(value);
                    traces[row].mean += f64::from(value);
                    finite[row] += 1;
                } else {
                    bin.gap = true;
                }
            }
        }
        start = end;
    }
    for (trace, count) in traces.iter_mut().zip(finite) {
        if count > 0 {
            trace.mean /= count as f64;
        }
    }
    Ok(DisplayData {
        key,
        traces,
        samples_per_bin: step,
    })
}
#[derive(Debug)]
pub struct Navigation {
    pub start: u64,
    pub span: u64,
    pub total: u64,
}
impl Navigation {
    pub fn new(total: u64, rate: u32) -> Self {
        Self {
            start: 0,
            span: (u64::from(rate) * 10).min(total).max(1),
            total,
        }
    }
    pub fn clamp(&mut self) {
        self.span = self.span.clamp(1, self.total.max(1));
        self.start = self.start.min(self.total.saturating_sub(self.span));
    }
    pub fn range(&self) -> Range<u64> {
        self.start..(self.start + self.span).min(self.total)
    }
    pub fn pan(&mut self, fraction: f64) {
        self.start =
            (self.start as f64 + fraction * self.span as f64).clamp(0.0, self.total as f64) as u64;
        self.clamp();
    }
    pub fn zoom(&mut self, factor: f64, anchor: f64) {
        let center = self.start as f64 + self.span as f64 * anchor;
        self.span = (self.span as f64 * factor)
            .round()
            .clamp(1.0, self.total.max(1) as f64) as u64;
        self.start = (center - self.span as f64 * anchor).max(0.0) as u64;
        self.clamp();
    }
    pub fn jump(&mut self, sample: u64) {
        self.start = sample.saturating_sub(self.span / 2);
        self.clamp();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use eva_mff::SampleValues;
    #[test]
    fn navigation_edges() {
        let mut n = Navigation::new(1000, 100);
        n.zoom(0.1, 0.5);
        assert_eq!(n.range(), 450..550);
        n.pan(-100.);
        assert_eq!(n.start, 0);
        n.jump(999);
        assert_eq!(n.range(), 900..1000);
        n.zoom(100., 0.5);
        assert_eq!(n.range(), 0..1000);
        n.zoom(0.00001, 0.);
        assert_eq!(n.span, 1);
    }
    #[test]
    fn real_reduction_preserves_extrema_and_cancels() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../eva-mff/tests/fixtures/example_3.mff");
        let r = Recording::open(path).unwrap();
        let key = ViewKey {
            range: 0..224,
            channels: vec![0],
            columns: 16,
            filters: Settings::default(),
        };
        let g = AtomicU64::new(1);
        let d = reduce(&r, key.clone(), &g, 1).unwrap();
        let raw = r
            .read_window(&[0], 0..224, SampleValues::Calibrated)
            .unwrap();
        for (bin, chunk) in d.traces[0].bins.iter().zip(raw.values[0].chunks(14)) {
            assert_eq!(bin.min, chunk.iter().copied().fold(f32::INFINITY, f32::min));
            assert_eq!(
                bin.max,
                chunk.iter().copied().fold(f32::NEG_INFINITY, f32::max)
            );
        }
        assert!(reduce(&r, key, &g, 2).is_err());
    }
}
