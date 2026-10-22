use eva_core::filter::{self, Settings};
use eva_mff::{Recording, RecordingKind};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicU64,
};
mod artifactdetect;
mod batch;
mod info;
mod mri;
mod provenance;
mod psa;
fn main() {
    if let Err(e) = run(std::env::args_os().skip(1).collect()) {
        eprintln!("eva: {e}");
        std::process::exit(1);
    }
}
fn run(mut args: Vec<OsString>) -> Result<(), String> {
    if args.is_empty() || matches!(args[0].to_str(), Some("-help" | "-h")) {
        println!("EVA command-line tools\n\nUsage: eva filter [OPTIONS] <input.mff> <output.mff>\n\nUsage: eva interpolate <channels> [-exclude <channels>] <input.mff> <output.mff>\n\nUsage: eva mri [OPTIONS] <input.mff> <output.mff>\n\nUsage: eva info [METRICS] <input.mff> [input2.mff ...]\n\nUsage: eva psa [OPTIONS] <input.mff> <output-directory>\n\nUsage: eva artifactdetect [OPTIONS] <input.mff> <output.json>\n\nUse eva <command> -help for options.");
        return Ok(());
    }
    if args[0] == "-version" {
        println!("eva {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let command = args.remove(0);
    if command == "artifactdetect" {
        return artifactdetect::run(args);
    }
    if command == "psa" {
        return psa::run(args);
    }
    if command == "info" {
        return info::run(args);
    }
    if command == "mri" {
        return mri::run(args);
    }
    if command == "interpolate" {
        return interpolate(args);
    }
    if command != "filter" {
        return Err("Unknown command. Use eva -help.".into());
    }
    if args
        .iter()
        .take_while(|a| *a != "--")
        .any(|a| a == "-help" || a == "-h")
    {
        println!("{}\n{}", filter::cli::HELP, batch::HELP);
        return Ok(());
    }
    let arguments = batch::parse(args)?;
    let pairs: Vec<_> = arguments.options.into_iter().collect();
    let settings = filter::cli::parse(&pairs)?;
    if !settings.active() {
        return Err(
            "No processing selected. Specify a cutoff, line-noise mode, or average reference."
                .into(),
        );
    }
    batch::execute(&arguments.jobs, |input, output| {
        export(input, output, &settings)
    })
}
fn channel_list(s: &str) -> Result<Vec<usize>, String> {
    let mut result = Vec::new();
    for part in s.split(',') {
        let value = part
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|v| v.checked_sub(1))
            .ok_or("Use comma-separated 1-based channel numbers, e.g. 1,8,12")?;
        result.push(value);
    }
    result.sort_unstable();
    result.dedup();
    Ok(result)
}
fn interpolate(args: Vec<OsString>) -> Result<(), String> {
    if args.is_empty()
        || args
            .iter()
            .take_while(|a| *a != "--")
            .any(|a| a == "-help" || a == "-h")
    {
        println!("Usage: eva interpolate <channels> [-exclude <channels>] <input.mff> <output.mff>\n\nChannels are comma-separated, 1-based numbers (e.g. 1,8,12).\n-exclude lists additional bad donors; targets never donate to one another.\nUses coordinates.xml scalp positions, Perrin order 4, 40 terms, lambda 1e-5.\nRequires >=3 good positioned electrodes with compatible units.\nEEG only; PNS copied unchanged. No temporal filtering is added.\nWrites a new MFF with eva.xml and an audit log; refuses existing outputs.\nUse -- before paths beginning with -. Missing donor samples produce NaN.\nExample: eva interpolate 1,8 -exclude 12 input.mff repaired.mff");
        return Ok(());
    }
    let targets = channel_list(args[0].to_str().ok_or("Channel list must be UTF-8")?)?;
    let mut excluded = Vec::new();
    let mut paths = Vec::new();
    let mut end = false;
    let mut iter = args.into_iter().skip(1);
    while let Some(a) = iter.next() {
        if !end && a == "--" {
            end = true;
            continue;
        }
        if !end && a == "-exclude" {
            if !excluded.is_empty() {
                return Err("Duplicate -exclude".into());
            }
            excluded = channel_list(
                iter.next()
                    .ok_or("Missing -exclude list")?
                    .to_str()
                    .ok_or("Invalid channel list")?,
            )?;
        } else if !end && a.to_str().is_some_and(|v| v.starts_with('-')) {
            return Err(format!("Unknown option {}", a.to_string_lossy()));
        } else {
            paths.push(PathBuf::from(a));
        }
    }
    if paths.len() != 2 {
        return Err("Expected input and output MFF paths; use eva interpolate -help".into());
    }
    let r = Recording::open(&paths[0]).map_err(|e| e.to_string())?;
    let mut settings = Settings::default();
    settings.options.filter_pns = false;
    settings.options.excluded = excluded.clone();
    settings.interpolation = eva_core::interpolation::recipes(&r, &targets, &excluded)?;
    export(&paths[0], &paths[1], &settings)
}
fn export(input: &Path, output: &Path, s: &Settings) -> Result<(), String> {
    export_with_mri(input, output, s, None)
}
fn export_with_mri(
    input: &Path,
    output: &Path,
    s: &Settings,
    request: Option<&eva_core::mri::Request>,
) -> Result<(), String> {
    let input = fs::canonicalize(input).map_err(|e| e.to_string())?;
    let parent = fs::canonicalize(
        output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )
    .map_err(|e| e.to_string())?;
    let output = parent.join(
        output
            .file_name()
            .ok_or("Output requires a directory name")?,
    );
    if output.starts_with(&input) {
        return Err("Output must be outside the input package.".into());
    }
    let r = Recording::open(&input).map_err(|e| e.to_string())?;
    if r.metadata().kind == RecordingKind::SegmentedOrAveraged {
        return Err("Segmented/averaged export is not supported.".into());
    }
    s.validate(r.metadata().sample_rate)?;
    if s.options
        .excluded
        .iter()
        .any(|&i| i >= r.metadata().channels.len())
    {
        return Err("Excluded reference channel is outside EEG channel range".into());
    }
    let mut pns_settings = s.clone();
    pns_settings.interpolation.clear();
    pns_settings.options.average_reference = false;
    pns_settings.options.excluded.clear();
    pns_settings.enabled = s.enabled && s.options.filter_pns;
    if let Some(p) = r.pns() {
        pns_settings.validate(p.metadata().sample_rate)?;
    }
    // Validate source package and existing provenance before creating output.
    let entries = fs::read_dir(&input)
        .map_err(|e| e.to_string())?
        .map(|e| e.map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    for e in &entries {
        if !e.file_type().map_err(|e| e.to_string())?.is_file() {
            return Err(
                "Only flat MFF packages with regular files are supported (no symlinks).".into(),
            );
        }
    }
    let mut xml = provenance::make(&input, &r, s)?;
    if output.exists() {
        return Err(format!("Output already exists: {}", output.display()));
    }
    let corrected = request
        .map(|request| {
            eprintln!(
                "Running {} with trigger {}…",
                request.config.method.label(),
                request.trigger
            );
            eva_core::mri::run(&r, request, &|| false)
        })
        .transpose()?;
    if let (Some(request), Some(result)) = (request, &corrected) {
        xml = provenance::mri_record(xml, request, result)?;
        for (stream, d) in &result.reports {
            eprintln!(
                "{stream}: {} channel-epochs corrected, {} skipped",
                d.corrected, d.skipped
            );
            for w in &d.warnings {
                eprintln!("{stream}: {w}");
            }
        }
    }
    fs::create_dir(&output)
        .map_err(|e| format!("Cannot create new output {}: {e}", output.display()))?;
    let result: Result<(), String> = (|| {
        let eeg_name = &r.metadata().streams[r.metadata().eeg_stream].file_name;
        let pns_name = r.pns().map(|p| {
            p.metadata().streams[p.metadata().eeg_stream]
                .file_name
                .clone()
        });
        for e in entries {
            if e.file_name() == eeg_name.as_str()
                || pns_name
                    .as_ref()
                    .is_some_and(|p| e.file_name() == p.as_str())
                || e.file_name() == "eva.xml"
            {
                continue;
            }
            fs::copy(e.path(), output.join(e.file_name())).map_err(|e| e.to_string())?;
        }
        let generation = AtomicU64::new(1);
        let process = |recording: &Recording, settings: &Settings| -> Result<(), String> {
            let name = &recording.metadata().streams[recording.metadata().eeg_stream].file_name;
            eprintln!(
                "Processing {name}: {}",
                corrected
                    .as_ref()
                    .map(|c| c.description.clone())
                    .unwrap_or_else(|| settings.summary())
            );
            let source = corrected.as_ref().and_then(|c| {
                if recording.is_pns() {
                    c.pns.clone()
                } else {
                    Some(c.eeg.clone())
                }
            });
            let plan =
                filter::Plan::new(settings, recording.metadata().sample_rate)?.with_source(source);
            let selected = if let Some(request) = request.filter(|_| !recording.is_pns()) {
                (0..recording.metadata().channels.len())
                    .filter(|i| !request.config.excluded_channels.contains(i))
                    .collect::<Vec<_>>()
            } else if settings.interpolation.is_empty() {
                (0..recording.metadata().channels.len()).collect::<Vec<_>>()
            } else {
                settings.interpolation.iter().map(|r| r.target).collect()
            };
            recording
                .export_signal_channels(output.join(name), &selected, |ch, range| {
                    plan.read(recording, ch, range, &generation, 1)
                })
                .map_err(|e| e.to_string())
        };
        process(&r, s)?;
        if let Some(p) = r.pns() {
            if s.options.filter_pns {
                process(p, &pns_settings)?;
            } else {
                fs::copy(
                    input.join(pns_name.as_ref().unwrap()),
                    output.join(pns_name.as_ref().unwrap()),
                )
                .map_err(|e| e.to_string())?;
            }
        }
        fs::write(output.join("eva.xml"), &xml).map_err(|e| e.to_string())?;
        if let Some(result) = &corrected {
            provenance::write_mri_log(&output, &result.description, &xml)?;
        } else {
            provenance::write_log(&output, s, &xml)?;
        }
        Recording::open(&output).map_err(|e| format!("Output verification: {e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&output);
    }
    result?;
    eprintln!("Wrote {}", output.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use eva_core::filter::{Family, LineMode};
    use eva_mff::SampleValues;
    pub(super) fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("info.xml"),
            "<fileInfo><recordTime>2026-01-01T00:00:00Z</recordTime></fileInfo>",
        )
        .unwrap();
        fs::write(d.path().join("info1.xml"),"<dataInfo><fileDataType><EEG/></fileDataType><calibrations><calibration><type>GCAL</type><channels><ch n='1'>2</ch><ch n='2'>0.5</ch></channels></calibration></calibrations></dataInfo>").unwrap();
        fs::write(
            d.path().join("info2.xml"),
            "<dataInfo><fileDataType><PNSData/></fileDataType></dataInfo>",
        )
        .unwrap();
        fs::write(d.path().join("sensorLayout.xml"),"<sensorLayout><sensors><sensor><number>1</number><type>0</type><name>A</name><unit>uV</unit></sensor><sensor><number>2</number><type>0</type><name>B</name><unit>uV</unit></sensor></sensors></sensorLayout>").unwrap();
        fs::write(d.path().join("epochs.xml"),"<epochs><epoch><beginTime>0</beginTime><endTime>8000000</endTime><firstBlock>1</firstBlock><lastBlock>1</lastBlock></epoch><epoch><beginTime>8500000</beginTime><endTime>16500000</endTime><firstBlock>2</firstBlock><lastBlock>2</lastBlock></epoch></epochs>").unwrap();
        fs::write(d.path().join("Events_test.xml"),"<eventTrack><event><code>TREV</code><beginTime>2026-01-01T00:00:09Z</beginTime></event></eventTrack>").unwrap();
        fs::write(d.path().join("eva.xml"),"<evaProcessing version='1' appName='EVA'><step op='markBad' replayable='false'><note>earlier &amp; preserved</note></step></evaProcessing>").unwrap();
        for (file, rate, n) in [("signal1.bin", 256u32, 2u32), ("signal2.bin", 128, 1)] {
            let count = rate * 8;
            let mut bytes = vec![];
            for block in 0..2 {
                for v in [1, 16 + 8 * n, count * 4 * n, n] {
                    bytes.extend(v.to_le_bytes());
                }
                for ch in 0..n {
                    bytes.extend((ch * count * 4).to_le_bytes());
                }
                for _ in 0..n {
                    bytes.extend(((rate << 8) | 32).to_le_bytes());
                }
                for ch in 0..n {
                    for i in 0..count {
                        let t = (i + block * count) as f64 / rate as f64;
                        let v = ((t * 10. * std::f64::consts::TAU).sin()
                            + 0.4 * (t * 60. * std::f64::consts::TAU).sin())
                            as f32
                            * (ch + 1) as f32;
                        bytes.extend(v.to_le_bytes());
                    }
                }
            }
            fs::write(d.path().join(file), bytes).unwrap();
        }
        d
    }
    #[test]
    fn exported_mff_matches_gui_engine_and_preserves_gaps_calibration_history_pns() {
        let input = fixture();
        let out = tempfile::tempdir().unwrap();
        let original = fs::read(input.path().join("signal1.bin")).unwrap();
        let r = Recording::open(input.path()).unwrap();
        for (i, (family, line)) in [
            (Family::Iir, LineMode::Off),
            (Family::Fir, LineMode::Notch),
            (Family::Auto, LineMode::CleanLine),
        ]
        .into_iter()
        .enumerate()
        {
            let s = Settings {
                enabled: true,
                family,
                line,
                high_hz: 4.,
                low_hz: 40.,
                transition_hz: 4.,
                ..Default::default()
            };
            let dest = out.path().join(format!("filtered-{i}.mff"));
            export(input.path(), &dest, &s).unwrap();
            let result = Recording::open(&dest).unwrap();
            assert_eq!(result.metadata().sample_count, r.metadata().sample_count);
            assert_eq!(result.metadata().events[0].onset_ns, 9_000_000_000);
            for (src, dst) in [
                (&r, &result),
                (r.pns().unwrap().as_ref(), result.pns().unwrap().as_ref()),
            ] {
                let plan = filter::Plan::new(&s, src.metadata().sample_rate).unwrap();
                let n = src.metadata().sample_count;
                for ch in 0..src.metadata().channels.len() {
                    let expected = plan.read(src, ch, 0..n, &AtomicU64::new(1), 1).unwrap();
                    let actual = dst
                        .read_window(&[ch], 0..n, SampleValues::Calibrated)
                        .unwrap();
                    assert!(expected
                        .iter()
                        .zip(&actual.values[0])
                        .all(|(a, b)| (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-5));
                }
            }
            let xml = fs::read_to_string(dest.join("eva.xml")).unwrap();
            let doc = roxmltree::Document::parse(&xml).unwrap();
            let steps = doc
                .root_element()
                .children()
                .filter(|n| n.has_tag_name("step"))
                .collect::<Vec<_>>();
            assert_eq!(steps.len(), 2);
            assert_eq!(steps[0].attribute("op"), Some("markBad"));
            assert_eq!(steps[1].attribute("op"), Some("filter"));
            assert!(xml.contains("cliArguments"));
            assert!(fs::read_dir(&dest).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("log_eva_")));
            assert!(export(input.path(), &dest, &s).is_err());
        }
        assert_eq!(
            fs::read(input.path().join("signal1.bin")).unwrap(),
            original
        );
    }
    #[test]
    fn cli_validation_pns_bypass_reference_and_failure_cleanup() {
        let input = fixture();
        let out = tempfile::tempdir().unwrap();
        let dest = out.path().join("result.mff");
        let mut s = Settings {
            enabled: true,
            ..Default::default()
        };
        s.options.filter_pns = false;
        s.options.average_reference = true;
        s.options.excluded = vec![1];
        export(input.path(), &dest, &s).unwrap();
        assert_eq!(
            fs::read(dest.join("signal2.bin")).unwrap(),
            fs::read(input.path().join("signal2.bin")).unwrap()
        );
        let r = Recording::open(&dest).unwrap();
        assert!(r
            .read_window(&[0], 0..100, SampleValues::Calibrated)
            .unwrap()
            .values[0]
            .iter()
            .all(|v| v.abs() < 1e-6));
        assert!(fs::read_to_string(dest.join("eva.xml"))
            .unwrap()
            .contains("op=\"reference\""));
        assert!(run(vec![
            "filter".into(),
            "-wat".into(),
            "1".into(),
            input.path().into(),
            out.path().join("bad.mff").into()
        ])
        .is_err());
        assert!(export(input.path(), &input.path().join("nested.mff"), &s).is_err());
        let layout = input.path().join("sensorLayout.xml");
        fs::write(
            &layout,
            fs::read_to_string(&layout)
                .unwrap()
                .replacen("<unit>uV</unit>", "<unit>mV</unit>", 1),
        )
        .unwrap();
        let fail = out.path().join("failed.mff");
        assert!(export(input.path(), &fail, &s).is_err());
        assert!(!fail.exists());
        fs::write(input.path().join("eva.xml"), "not xml").unwrap();
        assert!(export(
            input.path(),
            &fail,
            &Settings {
                enabled: true,
                ..Default::default()
            }
        )
        .is_err());
        assert!(!fail.exists());
    }
}

#[cfg(test)]
mod interpolation_tests {
    use super::*;
    #[test]
    fn cli_interpolation_reopens_with_history_and_unchanged_other_channels() {
        let input =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../eva-mff/tests/fixtures/example_3.mff");
        let out = tempfile::tempdir().unwrap();
        let dest = out.path().join("repaired.mff");
        run(vec![
            "interpolate".into(),
            "1,8".into(),
            "-exclude".into(),
            "12".into(),
            input.clone().into(),
            dest.clone().into(),
        ])
        .unwrap();
        let r = Recording::open(&input).unwrap();
        let o = Recording::open(&dest).unwrap();
        let s = Settings {
            interpolation: eva_core::interpolation::recipes(&r, &[0, 7], &[11]).unwrap(),
            ..Default::default()
        };
        assert!(s
            .interpolation
            .iter()
            .all(|r| r.donors.iter().all(|c| ![0, 7, 11].contains(c))));
        let p = filter::Plan::new(&s, r.metadata().sample_rate).unwrap();
        let n = r.metadata().sample_count;
        for c in 0..r.metadata().channels.len() {
            let actual = o
                .read_window(&[c], 0..n, eva_mff::SampleValues::Calibrated)
                .unwrap()
                .values
                .remove(0);
            if [0, 7].contains(&c) {
                let expected = p.read(&r, c, 0..n, &AtomicU64::new(1), 1).unwrap();
                assert!(actual
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() < 1e-3));
            } else {
                let a = r
                    .read_window(&[c], 0..n, eva_mff::SampleValues::Raw)
                    .unwrap();
                let b = o
                    .read_window(&[c], 0..n, eva_mff::SampleValues::Raw)
                    .unwrap();
                assert!(a.values[0]
                    .iter()
                    .zip(&b.values[0])
                    .all(|(a, b)| a.to_bits() == b.to_bits()));
            }
        }
        assert_eq!(
            fs::read(input.join("signal2.bin")).unwrap(),
            fs::read(dest.join("signal2.bin")).unwrap()
        );
        let xml = fs::read_to_string(dest.join("eva.xml")).unwrap();
        assert!(xml.contains("interpolateChannels"));
        assert!(xml.contains("donorChannels"));
        assert!(!xml.contains("op=\"filter\""));
        assert!(run(vec![
            "interpolate".into(),
            "0".into(),
            input.into(),
            out.path().join("bad.mff").into()
        ])
        .is_err());
    }
}
