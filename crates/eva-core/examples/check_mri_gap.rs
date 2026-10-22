//! Read-only diagnostic: export one calibrated channel and exercise MRI engines.
//! Usage: cargo run -p eva-core --example check_mri_gap -- INPUT.mff OUTPUT_DIRECTORY [full]
use eva_core::gradient::{self, Config, Method};
use eva_mff::{Recording, SampleValues};
use std::{fs, io::Write, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let r = Recording::open(args.next().ok_or("Expected input MFF")?)?;
    let output = PathBuf::from(args.next().ok_or("Expected output directory")?);
    let full = args.next().is_some_and(|a| a == "full");
    fs::create_dir_all(&output)?;
    let mut x = vec![];
    let n = r.metadata().sample_count;
    for start in (0..n).step_by(65536) {
        x.extend(
            r.read_window(
                &[0],
                start..(start + 65536).min(n),
                SampleValues::Calibrated,
            )?
            .values
            .remove(0),
        );
    }
    let mut f = fs::File::create(output.join("eeg-channel-1.f32le"))?;
    for v in &x {
        f.write_all(&v.to_le_bytes())?;
    }
    let markers = eva_core::mri::triggers(&r, "TREV")?;
    println!(
        "samples={n}, rate={}, gaps={:?}, triggers={} ({}..{})",
        r.metadata().sample_rate,
        r.gaps(),
        markers.len(),
        markers[0],
        markers[markers.len() - 1]
    );
    let mut summary = fs::File::create(output.join("rust-correction.txt"))?;
    for (mode, groups, offsets) in [
        ("volume", 1, vec![]),
        ("uniform22", 22, vec![]),
        ("offset90ms", 22, (0..22).map(|i| i as f64 * 0.09).collect()),
    ] {
        for method in [Method::AllenAas, Method::Fastr, Method::Farm] {
            let config = Config {
                method,
                groups,
                offsets_seconds: offsets.clone(),
                ..Config::default()
            };
            let result = gradient::correct(
                std::slice::from_ref(&x),
                &markers,
                r.metadata().sample_rate as f64,
                &config,
                &|| false,
                &|_| {},
            );
            match result {
                Ok(result) => {
                    assert!(x
                        .iter()
                        .zip(&result.channels[0])
                        .all(|(a, b)| a.is_finite() == b.is_finite()));
                    let mut f = fs::File::create(output.join(format!("{method:?}-{mode}.f32le")))?;
                    for v in &result.channels[0] {
                        f.write_all(&v.to_le_bytes())?;
                    }
                    let changed = x
                        .iter()
                        .zip(&result.channels[0])
                        .filter(|(a, b)| a.is_finite() && a != b)
                        .count();
                    let line = format!(
                        "{} ({mode}): changed={changed}, {:?}",
                        method.label(),
                        result.diagnostics
                    );
                    println!("{line}");
                    writeln!(summary, "{line}")?;
                }
                Err(e) => {
                    println!("{}: ERROR {e}", method.label());
                    writeln!(summary, "{}: ERROR {e}", method.label())?;
                }
            }
        }
    }
    if !full {
        return Ok(());
    }
    let request = eva_core::mri::Request {
        config: Config::default(),
        trigger: "TREV".into(),
        pns: true,
        progress: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
    };
    println!("Running full-recording shared MRI loader + Allen AAS…");
    match eva_core::mri::run(&r, &request, &|| false) {
        Ok(result) => {
            let line = format!(
                "Full-recording AAS: {} EEG channels, reports={:?}",
                result.eeg.len(),
                result.reports
            );
            println!("{line}");
            writeln!(summary, "{line}")?;
        }
        Err(e) => {
            println!("Full-recording AAS ERROR: {e}");
            writeln!(summary, "Full-recording AAS ERROR: {e}")?;
        }
    }
    Ok(())
}
