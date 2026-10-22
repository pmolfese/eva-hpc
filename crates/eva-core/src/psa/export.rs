//! Portable PSA result bundle. Source data are never edited.
use super::ResultData;
use serde_json::json;
use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
};
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn csv(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
pub fn write(
    result: &ResultData,
    path: &Path,
    source: &str,
    previous_xml: Option<&str>,
) -> Result<(), String> {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mff"))
    {
        return Err("PSA export is a result bundle, not MFF; use a .psa directory".into());
    }
    fs::create_dir(path)
        .map_err(|e| format!("Cannot create new output {}: {e}", path.display()))?;
    let run = || -> Result<(), Box<dyn std::error::Error>> {
        let mut epochs = BufWriter::new(fs::File::create(path.join("epochs.f32le"))?);
        for epoch in &result.epochs {
            for row in &epoch.values {
                for v in row {
                    epochs.write_all(&v.to_le_bytes())?;
                }
            }
        }
        epochs.flush()?;
        let mut averages = BufWriter::new(fs::File::create(path.join("averages.f32le"))?);
        let mut table = BufWriter::new(fs::File::create(path.join("averages.csv"))?);
        writeln!(table, "category,n,channel,name,unit,time_seconds,value")?;
        for avg in &result.averages {
            for (ch, row) in avg.values.iter().enumerate() {
                for (s, v) in row.iter().enumerate() {
                    averages.write_all(&v.to_le_bytes())?;
                    writeln!(
                        table,
                        "{},{},{},{},{},{},{}",
                        csv(&avg.category),
                        avg.count,
                        ch + 1,
                        csv(&result.channel_names[ch]),
                        csv(result.channel_units[ch].as_deref().unwrap_or("unknown")),
                        (s as f64 - result.pre_samples as f64) / f64::from(result.sample_rate),
                        v
                    )?;
                }
            }
        }
        averages.flush()?;
        table.flush()?;
        let manifest = json!({"format":"eva-psa","version":1,"source":source,"config":result.config,"config_channel_indexing":"zero-based bad_channels; report channels one-based","sample_rate":result.sample_rate,"pre_samples":result.pre_samples,"samples_per_epoch":result.samples,"channel_names":result.channel_names,"channel_units":result.channel_units,"layout":"little-endian float32, epoch/category then channel then sample","epoch_candidates":result.epochs.iter().map(|e|result.decisions[e.decision].candidate).collect::<Vec<_>>(),"averages":result.averages.iter().map(|a|json!({"category":a.category,"count":a.count})).collect::<Vec<_>>(),"decisions":result.decisions,"global_bad_channels":result.global_bad,"warnings":result.warnings,"source_processing":result.source_processing});
        fs::write(
            path.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        let mut decisions = BufWriter::new(fs::File::create(path.join("decisions.csv"))?);
        writeln!(decisions,"candidate,event_index,code,category,anchor_sample,accepted,reason,flagged_channels,repaired_channels")?;
        for d in &result.decisions {
            writeln!(
                decisions,
                "{},{},{},{},{},{},{},{},{}",
                d.candidate,
                d.event_index,
                csv(&d.code),
                csv(&d.category),
                d.anchor_sample,
                d.accepted,
                csv(&d.reason),
                csv(&serde_json::to_string(&d.flagged_channels)?),
                csv(&serde_json::to_string(&d.repaired_channels)?)
            )?;
        }
        decisions.flush()?;
        // Retain original provenance verbatim as its own file rather than guessing
        // its schema. Current PSA step is explicit about numerical replay parity.
        if let Some(xml) = previous_xml {
            fs::write(path.join("source-eva.xml"), xml)?;
        }
        let settings = escape(&serde_json::to_string(&result.config)?);
        let xml=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<evaProcessing version=\"1\" appName=\"eva-hpc\"><step op=\"segment\" replayable=\"false\"><param key=\"rustConfigJSON\">{settings}</param><param key=\"source\">{}</param><param key=\"sourceProcessing\">{}</param><note>PSA bundle; source provenance retained separately when present. Swift numerical replay parity is not claimed.</note></step></evaProcessing>\n",escape(source),escape(&result.source_processing));
        fs::write(path.join("eva.xml"), xml)?;
        fs::write(
            path.join("log_eva_psa.txt"),
            format!(
                "{} candidates; {} accepted; {} category averages\nSettings: {}\nWarnings: {:?}\n",
                result.decisions.len(),
                result.epochs.len(),
                result.averages.len(),
                serde_json::to_string(&result.config)?,
                result.warnings
            ),
        )?;
        Ok(())
    };
    if let Err(e) = run() {
        let _ = fs::remove_dir_all(path);
        return Err(e.to_string());
    }
    Ok(())
}
