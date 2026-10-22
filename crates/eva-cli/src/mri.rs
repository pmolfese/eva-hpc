use eva_core::{gradient::*, mri::Request};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::Read,
    sync::{atomic::AtomicU32, Arc},
};
const HELP: &str = r#"Usage: eva mri [OPTIONS] <input.mff> <output.mff>

MRI gradient artifact correction using the viewer's shared CPU engine.
All options take values. Booleans use true/false; -key=value is accepted.
Use -- before paths beginning with -. Output must be a new directory.

  -trigger CODE          Volume marker code; default TREV
  -pns true|false         Correct PNS on its own timebase; default true
  -slices N              Slice count, alternative to -groups
  -multiband N           Multiband factor with -slices; default 1
  -offsets LIST          Comma-separated acquisition offsets in seconds
  -slice-timing FILE     BIDS JSON containing SliceTiming (seconds)
  -motion FILE           Six-column motion file (required for Moosmann)
  -motion-format FORMAT  afni (default), fsl, spm; no format guessing
  -exclude-channels LIST Comma-separated 1-based EEG channels, copied unchanged
  -template-only LIST    1-based EEG channels that skip OBS and ANC
  -censored LIST         1-based volume indices excluded as donors
  -method VALUE
      aas (default), fastr, farm, moosmann
  -groups VALUE
      Acquisition groups per volume; default 1
  -trigger-position VALUE
      Relative position within epoch, 0..1; default 0
  -donors VALUE
      Donor epoch count; default 30
  -upsample VALUE
      Upsampling factor 1..16; default 1
  -synchronization VALUE
      scanner (default), independent, custom
  -align VALUE
      Custom synchronization alignment: true/false; default true
  -fractional VALUE
      Custom fractional alignment: true/false; default false
  -scaling VALUE
      drift (default), unscaled, least-squares
  -smoothing VALUE
      Drift scale median window in epochs; default 15
  -scale-min VALUE
      Minimum accepted scale; default 0.2
  -scale-max VALUE
      Maximum accepted scale; default 5
  -squared-correlation VALUE
      Rank by squared correlation: true/false; default false
  -correlation VALUE
      FARM donor correlation threshold; default 0.9
  -minimum-donors VALUE
      Minimum acceptable donors before fallback; default 4
  -search VALUE
      Correlation candidate search count; default 240
  -section-epochs VALUE
      Allen AAS section size; 0=automatic (default)
  -allen-gate VALUE
      Allen AAS correlation gate; default 0.975
  -initial-epochs VALUE
      Allen AAS seed count; default 5
  -obs VALUE
      off (default), auto, fixed
  -components VALUE
      Fixed OBS components; default 3
  -variance VALUE
      Automatic OBS explained variance fraction; default 0.9
  -max-components VALUE
      Automatic OBS component cap; default 5
  -chunk-seconds VALUE
      OBS chunk length in seconds; default 60
  -max-basis-epochs VALUE
      OBS basis epoch cap, 8..256; default 256
  -residual-floor VALUE
      OBS minimum relative residual energy; default 0.01
  -anc VALUE
      Enable ANC: true/false; default false
  -anc-slice-highpass VALUE
      Slice-derived ANC high-pass: true/false; default false (2 Hz)
  -anc-taps VALUE
      ANC adaptive taps; default 32
  -anc-step VALUE
      ANC normalized LMS step; default 0.01
  -motion-metric VALUE
      translation-sum (default), translation-speed, all-six
  -motion-threshold VALUE
      Motion threshold; default 0.5 (L1 mm, L2 mm/volume, or six-parameter FD mm)
  -motion-radius VALUE
      Head radius for rotation displacement, mm; default 50
  -censor-motion VALUE
      Exclude high-motion donors in other methods: true/false; default false
  -h, -help              Show this help

Choose only one timing mode: groups, slices/multiband, or offsets/SliceTiming.
Duplicate SliceTiming values collapse to simultaneous acquisition groups.
AAS uses section templates; FASTR/FARM/Moosmann expose alignment/scaling/OBS.
ANC is optional for all methods. Settings have the same method-specific meaning
as in the viewer; e.g. OBS and scaling do not apply to Allen AAS.

Source files stay unchanged. Gaps stay missing; crossing epochs are skipped;
ANC restarts after each gap. PNS never uses EEG channel-exclusion lists.
Writes eva.xml settings/diagnostics plus a log_eva_*.txt audit log.
CPU only, 512 MiB combined input sample limit plus epoch workspace limits.
No display server or graphics runtime is required. Segmented/averaged data
are unsupported. This command adds no temporal filtering or interpolation.

Examples:
  eva mri -method aas -trigger TREV input.mff cleaned.mff
  eva mri -method fastr -slices 60 -multiband 2 -obs auto input.mff cleaned.mff
  eva mri -method moosmann -motion motion.par -motion-format fsl input.mff cleaned.mff
"#;
fn text_file(path: &str) -> Result<String, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let mut s = String::new();
    f.take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut s)
        .map_err(|e| e.to_string())?;
    if s.len() > 16 * 1024 * 1024 {
        return Err("Sidecar exceeds 16 MiB".into());
    }
    Ok(s)
}
fn parse(pairs: &BTreeMap<String, String>) -> Result<Request, String> {
    let mut c = Config::default();
    let mut trigger = "TREV".to_owned();
    let mut pns = true;
    for (key, value) in pairs {
        let v = value.as_str();
        let invalid = || format!("Invalid -{key} value: {value}");
        match key.as_str() {
            "trigger" => trigger = value.clone(),
            "pns" => pns = v.parse().map_err(|_| invalid())?,
            "exclude-channels" => c.excluded_channels = super::channel_list(v)?,
            "template-only" => c.template_only = super::channel_list(v)?,
            "censored" => c.censored = super::channel_list(v)?,
            "slices" | "multiband" | "offsets" | "slice-timing" | "motion" | "motion-format" => {}
            "method" => {
                c.method = match v {
                    "aas" => Method::AllenAas,
                    "fastr" => Method::Fastr,
                    "farm" => Method::Farm,
                    "moosmann" => Method::Moosmann,
                    _ => return Err(invalid()),
                }
            }
            "groups" => c.groups = v.parse().map_err(|_| invalid())?,
            "trigger-position" => c.trigger_position = v.parse().map_err(|_| invalid())?,
            "donors" => c.donors = v.parse().map_err(|_| invalid())?,
            "upsample" => c.upsample = v.parse().map_err(|_| invalid())?,
            "synchronization" => {
                c.synchronization = match v {
                    "scanner" => Synchronization::ScannerSlaved,
                    "independent" => Synchronization::IndependentClocks,
                    "custom" => Synchronization::Custom,
                    _ => return Err(invalid()),
                }
            }
            "align" => c.align = v.parse().map_err(|_| invalid())?,
            "fractional" => c.fractional = v.parse().map_err(|_| invalid())?,
            "scaling" => {
                c.scaling = match v {
                    "drift" => Scaling::DriftTracking,
                    "unscaled" => Scaling::Unscaled,
                    "least-squares" => Scaling::LeastSquares,
                    _ => return Err(invalid()),
                }
            }
            "smoothing" => c.smoothing = v.parse().map_err(|_| invalid())?,
            "scale-min" => c.scale_min = v.parse().map_err(|_| invalid())?,
            "scale-max" => c.scale_max = v.parse().map_err(|_| invalid())?,
            "squared-correlation" => c.squared_correlation = v.parse().map_err(|_| invalid())?,
            "correlation" => c.correlation = v.parse().map_err(|_| invalid())?,
            "minimum-donors" => c.minimum_donors = v.parse().map_err(|_| invalid())?,
            "search" => c.search = v.parse().map_err(|_| invalid())?,
            "section-epochs" => c.section_epochs = v.parse().map_err(|_| invalid())?,
            "allen-gate" => c.allen_gate = v.parse().map_err(|_| invalid())?,
            "initial-epochs" => c.initial_epochs = v.parse().map_err(|_| invalid())?,
            "obs" => {
                c.obs = match v {
                    "off" => Obs::Off,
                    "auto" => Obs::Automatic,
                    "fixed" => Obs::Fixed,
                    _ => return Err(invalid()),
                }
            }
            "components" => c.components = v.parse().map_err(|_| invalid())?,
            "variance" => c.variance = v.parse().map_err(|_| invalid())?,
            "max-components" => c.max_components = v.parse().map_err(|_| invalid())?,
            "chunk-seconds" => c.chunk_seconds = v.parse().map_err(|_| invalid())?,
            "max-basis-epochs" => c.max_basis_epochs = v.parse().map_err(|_| invalid())?,
            "residual-floor" => c.residual_floor = v.parse().map_err(|_| invalid())?,
            "anc" => c.anc = v.parse().map_err(|_| invalid())?,
            "anc-slice-highpass" => c.anc_slice_highpass = v.parse().map_err(|_| invalid())?,
            "anc-taps" => c.anc_taps = v.parse().map_err(|_| invalid())?,
            "anc-step" => c.anc_step = v.parse().map_err(|_| invalid())?,
            "motion-metric" => {
                c.motion_metric = match v {
                    "translation-sum" => MotionMetric::TranslationSum,
                    "translation-speed" => MotionMetric::TranslationSpeed,
                    "all-six" => MotionMetric::AllSix,
                    _ => return Err(invalid()),
                }
            }
            "motion-threshold" => c.motion_threshold = v.parse().map_err(|_| invalid())?,
            "motion-radius" => c.motion_radius = v.parse().map_err(|_| invalid())?,
            "censor-motion" => c.censor_motion = v.parse().map_err(|_| invalid())?,
            _ => return Err(format!("Unknown option -{key}; use eva mri -help")),
        }
    }
    let modes = ["groups", "slices", "offsets", "slice-timing"]
        .iter()
        .filter(|k| pairs.contains_key(**k))
        .count();
    if modes > 1 {
        return Err("Choose only one acquisition timing mode".into());
    }
    if pairs.contains_key("multiband") && !pairs.contains_key("slices") {
        return Err("-multiband requires -slices".into());
    }
    if let Some(slices) = pairs.get("slices") {
        let n = slices.parse::<usize>().map_err(|_| "Invalid slice count")?;
        let mb = pairs
            .get("multiband")
            .map(|v| v.parse::<usize>())
            .transpose()
            .map_err(|_| "Invalid multiband factor")?
            .unwrap_or(1);
        if mb == 0 || n == 0 || !n.is_multiple_of(mb) {
            return Err("Slice count must be positive and divisible by multiband factor".into());
        }
        c.groups = n / mb;
    }
    if let Some(offsets) = pairs.get("offsets") {
        c.offsets_seconds = offsets
            .split(',')
            .map(|v| {
                v.trim()
                    .parse::<f64>()
                    .map_err(|_| "Invalid acquisition offset".to_owned())
            })
            .collect::<Result<_, _>>()?;
    }
    if let Some(file) = pairs.get("slice-timing") {
        let value: serde_json::Value = serde_json::from_str(&text_file(file)?)
            .map_err(|e| format!("Invalid timing JSON: {e}"))?;
        c.offsets_seconds = value
            .get("SliceTiming")
            .and_then(|v| v.as_array())
            .ok_or("JSON requires a SliceTiming array")?
            .iter()
            .map(|v| {
                v.as_f64()
                    .ok_or("SliceTiming entries must be numbers".to_owned())
            })
            .collect::<Result<_, _>>()?;
    }
    if pairs.contains_key("offsets") || pairs.contains_key("slice-timing") {
        if c.offsets_seconds.is_empty() {
            return Err("Acquisition offsets cannot be empty".into());
        }
        c.offsets_seconds.sort_by(f64::total_cmp);
        c.offsets_seconds.dedup();
        c.groups = c.offsets_seconds.len();
    }
    let format = match pairs
        .get("motion-format")
        .map(String::as_str)
        .unwrap_or("afni")
    {
        "afni" => MotionFormat::Afni,
        "fsl" => MotionFormat::Fsl,
        "spm" => MotionFormat::Spm,
        _ => return Err("Motion format must be afni, fsl, or spm".into()),
    };
    if pairs.contains_key("motion-format") && !pairs.contains_key("motion") {
        return Err("-motion-format requires -motion".into());
    }
    if let Some(file) = pairs.get("motion") {
        c.motion = parse_motion(&text_file(file)?, format)?;
    }
    if trigger.is_empty() {
        return Err("Trigger code cannot be empty".into());
    }
    c.validate()?;
    Ok(Request {
        config: c,
        trigger,
        pns,
        progress: Arc::new(AtomicU32::new(0)),
    })
}
pub fn run(args: Vec<OsString>) -> Result<(), String> {
    if args.is_empty()
        || args
            .iter()
            .take_while(|a| *a != "--")
            .any(|a| a == "-h" || a == "-help")
    {
        println!("{HELP}\n{}", super::batch::HELP);
        return Ok(());
    }
    let arguments = super::batch::parse(args)?;
    let request = parse(&arguments.options)?;
    let mut settings = super::Settings::default();
    settings.options.filter_pns = request.pns;
    super::batch::execute(&arguments.jobs, |input, output| {
        super::export_with_mri(input, output, &settings, Some(&request))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(items: &[(&str, &str)]) -> BTreeMap<String, String> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
    #[test]
    fn timing_motion_and_option_validation() {
        let r = parse(&options(&[
            ("slices", "60"),
            ("multiband", "2"),
            ("method", "fastr"),
            ("obs", "auto"),
            ("exclude-channels", "1,8"),
        ]))
        .unwrap();
        assert_eq!(r.config.groups, 30);
        assert_eq!(r.config.excluded_channels, vec![0, 7]);
        assert_eq!(r.config.obs, Obs::Automatic);
        for opts in [
            vec![("slices", "60"), ("groups", "30")],
            vec![("multiband", "2")],
            vec![("slices", "61"), ("multiband", "2")],
            vec![("method", "moosmann")],
            vec![("anc-step", "NaN")],
            vec![("wat", "1")],
            vec![("exclude-channels", "0")],
            vec![("offsets", "")],
        ] {
            assert!(parse(&options(&opts)).is_err());
        }
        let d = tempfile::tempdir().unwrap();
        let timing = d.path().join("timing.json");
        std::fs::write(&timing, r#"{"SliceTiming":[0.4,0,0.4,0]}"#).unwrap();
        let r = parse(&options(&[("slice-timing", timing.to_str().unwrap())])).unwrap();
        assert_eq!(r.config.offsets_seconds, vec![0., 0.4]);
        assert_eq!(r.config.groups, 2);
        let motion = d.path().join("motion.txt");
        std::fs::write(&motion, "90 0 0 1 2 3\n0 0 0 0 0 0\n").unwrap();
        let r = parse(&options(&[
            ("method", "moosmann"),
            ("motion", motion.to_str().unwrap()),
            ("motion-format", "afni"),
        ]))
        .unwrap();
        assert!((r.config.motion[0][0] - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    }
    #[test]
    fn all_methods_export_same_samples_as_viewer_with_gaps_and_pns() {
        let input = crate::tests::fixture();
        let output = tempfile::tempdir().unwrap();
        let mut events = String::from("<eventTrack>");
        for i in 0..16 {
            let ms = i * 1000 + if i >= 8 { 500 } else { 0 };
            events+=&format!("<event><code>TREV</code><beginTime>2026-01-01T00:00:{:02}.{:03}Z</beginTime></event>",ms/1000,ms%1000);
        }
        events += "</eventTrack>";
        std::fs::write(input.path().join("Events_test.xml"), &events).unwrap();
        let motion = output.path().join("motion.txt");
        std::fs::write(&motion, "0 0 0 0 0 0\n".repeat(16)).unwrap();
        let source = eva_mff::Recording::open(input.path()).unwrap();
        let original = std::fs::read(input.path().join("signal1.bin")).unwrap();
        for (i, method) in ["aas", "fastr", "farm", "moosmann"].iter().enumerate() {
            let pns = if i % 2 == 0 { "true" } else { "false" };
            let options = options(&[
                ("method", method),
                ("pns", pns),
                ("motion", motion.to_str().unwrap()),
                ("exclude-channels", "2"),
                ("anc", "true"),
            ]);
            let request = parse(&options).unwrap();
            let expected = eva_core::mri::run(&source, &request, &|| false).unwrap();
            let dest = output.path().join(format!("{method}.mff"));
            let mut args: Vec<OsString> = vec![];
            for (k, v) in &options {
                args.push(format!("-{k}").into());
                args.push(v.into());
            }
            args.push(input.path().into());
            args.push(dest.clone().into());
            run(args).unwrap();
            let r = eva_mff::Recording::open(&dest).unwrap();
            for (ch, values) in expected.eeg.iter().enumerate() {
                let actual = r
                    .read_window(
                        &[ch],
                        0..r.metadata().sample_count,
                        eva_mff::SampleValues::Calibrated,
                    )
                    .unwrap();
                assert!(actual.values[0]
                    .iter()
                    .zip(values)
                    .all(|(a, b)| (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-5));
            }
            if let Some(expected) = &expected.pns {
                let p = r.pns().unwrap();
                let actual = p
                    .read_window(
                        &[0],
                        0..p.metadata().sample_count,
                        eva_mff::SampleValues::Calibrated,
                    )
                    .unwrap();
                assert!(actual.values[0]
                    .iter()
                    .zip(&expected[0])
                    .all(|(a, b)| (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-5));
            } else {
                assert_eq!(
                    std::fs::read(input.path().join("signal2.bin")).unwrap(),
                    std::fs::read(dest.join("signal2.bin")).unwrap()
                );
            }
            let xml = std::fs::read_to_string(dest.join("eva.xml")).unwrap();
            assert!(xml.contains("mriGradientCorrection"));
            assert!(xml.contains("diagnostics"));
            assert!(xml.contains("markBad"));
            assert!(!xml.contains("op=\"filter\""));
            assert_eq!(
                std::fs::read(input.path().join("signal1.bin")).unwrap(),
                original
            );
            assert!(crate::export_with_mri(
                input.path(),
                &dest,
                &crate::Settings::default(),
                Some(&request)
            )
            .is_err());
        }
        let bad = output.path().join("failed.mff");
        let request = parse(&options(&[("trigger", "missing")])).unwrap();
        assert!(crate::export_with_mri(
            input.path(),
            &bad,
            &crate::Settings::default(),
            Some(&request)
        )
        .is_err());
        assert!(!bad.exists());
    }
}
