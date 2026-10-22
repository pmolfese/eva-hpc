use eva_mff::{Recording, RecordingKind, SampleValues};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: cargo run -p eva-mff --example inspect -- PATH.mff")?;
    let r = Recording::open(path)?;
    let m = r.metadata();
    println!(
        "{:?}: {} channels, {} Hz, {} samples ({:.6} s), {} events",
        m.kind,
        m.channels.len(),
        m.sample_rate,
        m.sample_count,
        m.duration_seconds(),
        m.events.len()
    );
    for stream in &m.streams {
        println!("stream: {} ({})", stream.file_name, stream.kind);
    }
    for warning in &m.warnings {
        eprintln!("warning: {warning}");
    }
    if m.kind != RecordingKind::SegmentedOrAveraged {
        let w = r.read_window(&[0], 0..m.sample_count.min(8), SampleValues::Raw)?;
        println!("{} raw samples: {:?}", m.channels[0].label, w.values[0]);
    }
    Ok(())
}
