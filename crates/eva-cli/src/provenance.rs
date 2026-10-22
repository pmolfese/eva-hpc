//! Matches EVAProcessingScriptXML's evaProcessing/step/param envelope.
use eva_core::filter::*;
use eva_mff::Recording;
use std::{collections::BTreeMap, fs, path::Path};
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn timestamp() -> String {
    let t = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    )
}
fn step(op: &str, p: &BTreeMap<&str, String>, note: &str, now: &str) -> String {
    // The schema is compatible, but numerical parity with Swift is not established.
    // Do not invite automatic cross-engine replay as though it were exact.
    let mut text = format!("  <step op=\"{op}\" replayable=\"false\" appliedAt=\"{now}\">\n");
    for (key, value) in p {
        text += &format!("    <param key=\"{key}\">{}</param>\n", escape(value));
    }
    text += &format!("    <note>{}</note>\n  </step>\n", escape(note));
    text
}
pub fn make(input: &Path, r: &Recording, s: &Settings) -> Result<String, String> {
    let mut history = String::new();
    let path = input.join("eva.xml");
    if path.exists() {
        if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024 {
            return Err("eva.xml exceeds 32 MiB".into());
        }
        let previous = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let doc = roxmltree::Document::parse(&previous)
            .map_err(|e| format!("Invalid existing eva.xml: {e}"))?;
        let root = doc.root_element();
        if root.tag_name().name() != "evaProcessing"
            || root.tag_name().namespace().is_some()
            || root.attribute("version").unwrap_or("1") != "1"
        {
            return Err("Unsupported existing eva.xml schema; refusing to discard history".into());
        }
        for n in root.children().filter(|n| n.is_element()) {
            history.push_str(&previous[n.range()]);
            history.push('\n');
        }
    }
    let now = timestamp();
    let mut p = BTreeMap::new();
    if s.high_pass {
        p.insert("highPassHz", s.high_hz.to_string());
    }
    if s.low_pass {
        p.insert("lowPassHz", s.low_hz.to_string());
    }
    p.insert("highPassSlope", (s.order * 6).to_string());
    p.insert("lowPassSlope", (s.low_order * 6).to_string());
    p.insert(
        "filterFamily",
        match s.family {
            Family::Iir => "iir",
            Family::Fir => "fir",
            Family::Auto => "auto",
        }
        .into(),
    );
    p.insert(
        "iirDesign",
        match s.options.design {
            IirDesign::Butterworth => "butterworth",
            IirDesign::Elliptic => "elliptic",
        }
        .into(),
    );
    p.insert("ellipticPassbandRippleDB", s.options.ripple.to_string());
    p.insert(
        "ellipticStopbandAttenuationDB",
        s.options.stop_db.to_string(),
    );
    p.insert(
        "firWindow",
        match s.options.window {
            FirWindow::Hamming => "hamming",
            FirWindow::Kaiser => "kaiser",
        }
        .into(),
    );
    p.insert("firKaiserAttenuationDB", s.options.attenuation.to_string());
    p.insert(
        "firApplication",
        match s.options.application {
            FirApplication::ZeroPhase => "zeroPhase",
            FirApplication::DelayCompensated => "delayCompensated",
            FirApplication::Forward => "forward",
        }
        .into(),
    );
    p.insert("firCrossoverHz", s.options.crossover.to_string());
    p.insert(
        "firCrossoverRule",
        if s.options.through {
            "through"
        } else {
            "below"
        }
        .into(),
    );
    if !s.options.auto_transition {
        p.insert("firTransitionHz", s.transition_hz.to_string());
    }
    p.insert(
        "firDesignRule",
        match s.options.rule {
            DesignRule::Eva => "eva",
            DesignRule::EeglabMne => "eeglabMNE",
        }
        .into(),
    );
    p.insert(
        "lineNoiseMode",
        match s.line {
            LineMode::Off => "off",
            LineMode::Notch => "notch",
            LineMode::CleanLine => "adaptiveCleanLine",
        }
        .into(),
    );
    p.insert("lineNoiseHz", s.line_hz.to_string());
    p.insert("lineNoiseHarmonics", s.harmonics.to_string());
    p.insert("notchUsesFIR", s.options.fir_notch.to_string());
    p.insert("notchQ", s.notch_q.to_string());
    p.insert("lineNoiseWindowSeconds", s.window_seconds.to_string());
    p.insert("lineNoiseStrength", s.strength.to_string());
    p.insert("filterPNS", s.options.filter_pns.to_string());
    p.insert(
        "precision",
        match s.options.precision {
            Precision::Auto => "auto",
            Precision::Float => "float",
            Precision::Double => "double",
        }
        .into(),
    );
    p.insert("engine", "eva-core fixed-tile CPU filtering".into());
    p.insert("engineVersion", env!("CARGO_PKG_VERSION").into());
    p.insert("cliArguments", cli::arguments(s).join(" "));
    p.insert("sampleRateHz", r.metadata().sample_rate.to_string());
    p.insert("timelineSamples", r.metadata().sample_count.to_string());
    p.insert(
        "sourcePackage",
        input
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    );
    let note="EVA-HPC CLI export. Parameters follow macOS EVA naming; exact Rust settings are in cliArguments. IIR slope parameters are per pass (effective zero-phase slope is doubled). Source calibration retained; gaps preserved. Cross-engine numerical parity is not established, so automatic macOS replay is disabled.";
    let mut xml=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<evaProcessing version=\"1\" appName=\"EVA-HPC\" appVersion=\"{}\" writtenAt=\"{now}\" fileType=\"continuous\">\n",env!("CARGO_PKG_VERSION"));
    xml += &history;
    if s.active() {
        xml += &step("filter", &p, note, &now);
    }
    if !s.interpolation.is_empty() {
        let channels = s
            .interpolation
            .iter()
            .map(|r| (r.target + 1).to_string())
            .collect::<Vec<_>>()
            .join(",");
        let donors = s.interpolation[0]
            .donors
            .iter()
            .map(|c| (c + 1).to_string())
            .collect::<Vec<_>>()
            .join(",");
        let excluded = s
            .options
            .excluded
            .iter()
            .map(|c| (c + 1).to_string())
            .collect::<Vec<_>>()
            .join(",");
        if !excluded.is_empty() {
            xml += &step(
                "markBad",
                &BTreeMap::from([("channels", excluded.clone()), ("scope", "ambient".into())]),
                "Additional channels excluded from interpolation donors.",
                &now,
            );
        }
        let params = BTreeMap::from([
            ("channels", channels),
            ("donorChannels", donors),
            ("excludedChannels", excluded),
            ("channelIndexBase", "1".into()),
            ("method", "sphericalSpline".into()),
            ("order", "4".into()),
            ("terms", "40".into()),
            ("lambda", "0.00001".into()),
            ("engineVersion", env!("CARGO_PKG_VERSION").into()),
        ]);
        xml += &step("interpolateChannels", &params, "Scalp EEG only. Unit vectors normalized about the MFF origin. Targets and excluded channels never donate; PNS unchanged. Source calibration and timing gaps preserved.", &now);
    }
    if s.options.average_reference {
        let reference = BTreeMap::from([
            ("method", "average".into()),
            (
                "excludedChannels",
                s.options
                    .excluded
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            ("channelIndexBase", "0".into()),
        ]);
        xml += &step(
            "reference",
            &reference,
            "EEG average reference after filtering; PNS excluded.",
            &now,
        );
    }
    xml += "</evaProcessing>\n";
    roxmltree::Document::parse(&xml)
        .map_err(|e| format!("Provenance serialization failed: {e}"))?;
    Ok(xml)
}
pub fn write_log(output: &Path, s: &Settings, xml: &str) -> Result<(), String> {
    use std::io::Write;
    let now = timestamp().replace(':', "-");
    let base = format!("log_eva_{now}");
    let command = if s.interpolation.is_empty() {
        format!("eva filter {}", cli::arguments(s).join(" "))
    } else {
        let channels = s
            .interpolation
            .iter()
            .map(|r| (r.target + 1).to_string())
            .collect::<Vec<_>>()
            .join(",");
        let excluded = s
            .options
            .excluded
            .iter()
            .map(|c| (c + 1).to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "eva interpolate {channels}{}",
            if excluded.is_empty() {
                String::new()
            } else {
                format!(" -exclude {excluded}")
            }
        )
    };
    for i in 0..1000 {
        let path = output.join(format!("{base}_{i}.txt"));
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
        {
            Ok(mut f) => {
                return writeln!(
                f,
                "EVA-HPC {} processing export\n{}\nCommand options: {}\nProcessing provenance:\n{}",
                env!("CARGO_PKG_VERSION"),
                s.summary(),
                command,
                xml
            )
                .map_err(|e| e.to_string())
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Cannot create unique audit log".into())
}

pub fn mri_record(
    mut xml: String,
    request: &eva_core::mri::Request,
    result: &eva_core::mri::Corrected,
) -> Result<String, String> {
    let mut params = BTreeMap::new();
    params.insert(
        "method",
        match request.config.method {
            eva_core::gradient::Method::AllenAas => "allenIAR",
            eva_core::gradient::Method::Fastr => "fastr",
            eva_core::gradient::Method::Farm => "farm",
            eva_core::gradient::Method::Moosmann => "moosmann",
        }
        .into(),
    );
    params.insert("trMarkerCode", request.trigger.clone());
    params.insert("appliesToPNS", request.pns.to_string());
    params.insert("engineVersion", env!("CARGO_PKG_VERSION").into());
    // Full normalized configuration includes resolved timing offsets and motion rows,
    // not just paths to sidecars that could later move or change.
    let c = &request.config;
    let configuration = serde_json::json!({
        "method": format!("{:?}",c.method),
        "groups": c.groups,
        "offsets_seconds": c.offsets_seconds,
        "trigger_position": c.trigger_position,
        "donors": c.donors,
        "upsample": c.upsample,
        "synchronization": format!("{:?}",c.synchronization),
        "align": c.align,
        "fractional": c.fractional,
        "scaling": format!("{:?}",c.scaling),
        "smoothing": c.smoothing,
        "scale_min": c.scale_min,
        "scale_max": c.scale_max,
        "squared_correlation": c.squared_correlation,
        "correlation": c.correlation,
        "minimum_donors": c.minimum_donors,
        "search": c.search,
        "section_epochs": c.section_epochs,
        "allen_gate": c.allen_gate,
        "initial_epochs": c.initial_epochs,
        "obs": format!("{:?}",c.obs),
        "components": c.components,
        "variance": c.variance,
        "max_components": c.max_components,
        "chunk_seconds": c.chunk_seconds,
        "max_basis_epochs": c.max_basis_epochs,
        "residual_floor": c.residual_floor,
        "anc": c.anc,
        "anc_slice_highpass": c.anc_slice_highpass,
        "anc_taps": c.anc_taps,
        "anc_step": c.anc_step,
        "motion": c.motion,
        "motion_metric": format!("{:?}",c.motion_metric),
        "motion_threshold": c.motion_threshold,
        "motion_radius": c.motion_radius,
        "censor_motion": c.censor_motion,
        "censored": c.censored,
        "excluded_channels": c.excluded_channels,
        "template_only": c.template_only,
    });
    params.insert("rustConfigJSON", configuration.to_string());

    params.insert(
        "motionUnits",
        "rotations radians, translations mm; AFNI/FSL/SPM normalized before processing".into(),
    );
    params.insert(
        "indexBase",
        "Config channel and volume indices are zero-based; CLI lists are one-based".into(),
    );
    params.insert("diagnostics", format!("{:?}", result.reports));
    let entry=step("mriGradientCorrection",&params,"Shared EVA-HPC CPU engine. Recording gaps preserved; epochs crossing missing samples skipped; ANC restarted on each finite segment. No extra display filtering or interpolation. Cross-engine numerical parity with macOS is not established.",&timestamp());
    let pos = xml
        .rfind("</evaProcessing>")
        .ok_or("Missing processing root")?;
    xml.insert_str(pos, &entry);
    roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
    Ok(xml)
}
pub fn write_mri_log(output: &Path, description: &str, xml: &str) -> Result<(), String> {
    use std::io::Write;
    let stamp = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    let mut f = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output.join(format!("log_eva_mri_{stamp}.txt")))
        .map_err(|e| e.to_string())?;
    writeln!(f,"EVA-HPC {} MRI correction export\n{description}\nFull normalized settings, motion data and diagnostics:\n{xml}",env!("CARGO_PKG_VERSION")).map_err(|e|e.to_string())
}
