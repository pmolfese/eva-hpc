//! Run with: cargo run --release -p eva-core --example benchmark_psa -- recording.mff [filtered]
//! Reports compute time (excluding export) and a deterministic sample checksum.
use eva_core::{filter, psa};
use std::{sync::atomic::AtomicU64, time::Instant};
fn main() {
    let mut args = std::env::args().skip(1);
    let recording = eva_mff::Recording::open(args.next().expect("recording.mff")).unwrap();
    let filters = filter::Settings {
        enabled: args.next().as_deref() == Some("filtered"),
        ..Default::default()
    };
    let config = psa::Config {
        events: vec!["Shap".into(), "Fear".into()],
        threshold: false,
        average: true,
        bad_channels: vec![0],
        ..Default::default()
    };
    let start = Instant::now();
    let result = psa::run(
        &recording,
        &config,
        &filters,
        None,
        &AtomicU64::new(1),
        1,
        &|_| {},
    )
    .unwrap();
    let seconds = start.elapsed().as_secs_f64();
    let hash = result
        .epochs
        .iter()
        .flat_map(|e| e.values.iter().flatten())
        .chain(
            result
                .averages
                .iter()
                .flat_map(|e| e.values.iter().flatten()),
        )
        .fold(0xcbf29ce484222325u64, |h, v| {
            (h ^ u64::from(v.to_bits())).wrapping_mul(0x100000001b3)
        });
    println!(
        "{seconds:.3}s, {} epochs, {} averages, sample checksum {hash:016x}",
        result.epochs.len(),
        result.averages.len()
    );
}
