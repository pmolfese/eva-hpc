//! Shared command-line representation of the GUI's filter settings.
use super::*;
pub const HELP: &str = r#"Usage: eva filter [OPTIONS] <input.mff> <output.mff>

Creates a new MFF directory; existing destinations are refused. Source is read-only.
No cutoff is enabled unless requested. Uses the GUI's fixed-tile filtering engine.
Options take a value (booleans use true/false). Use -- before paths beginning with -.

  -highpass HZ|off          High-pass cutoff (default off)
  -lowpass HZ|off           Low-pass cutoff (default off)
  -preset NAME             eeglab, erplab, mne-python, netstation; applied before overrides
  -exclude-reference LIST  Comma-separated 1-based EEG channel numbers; default none
  -family VALUE
      iir (default), fir, auto
  -iir-design VALUE
      butterworth (default), elliptic
  -highpass-order VALUE
      IIR poles per pass; 2,4,6,8; default 4
  -lowpass-order VALUE
      IIR poles per pass; 2,4,6,8; default 4
  -ripple VALUE
      Elliptic passband ripple in dB; default 0.1
  -stopband VALUE
      Elliptic stopband attenuation in dB; default 60
  -fir-window VALUE
      hamming (default), kaiser
  -fir-attenuation VALUE
      Kaiser attenuation in dB; default 60
  -fir-application VALUE
      delay-compensated (default), zero-phase, forward
  -transition VALUE
      FIR transition width Hz; default 1 (use -auto-transition true for automatic)
  -auto-transition VALUE
      true/false; default false
  -fir-rule VALUE
      eva (default), eeglab-mne
  -crossover VALUE
      Auto high-pass crossover Hz; default 1
  -crossover-inclusive VALUE
      Auto IIR at crossover too: true/false; default false
  -line-noise VALUE
      off (default), notch, cleanline
  -line-frequency VALUE
      Fundamental Hz; default 60
  -harmonics VALUE
      Number of line-noise harmonics; default 1
  -notch-q VALUE
      IIR notch Q; default 30
  -fir-notch VALUE
      Use FIR notch: true/false; default false
  -cleanline-window VALUE
      Regression window seconds; default 4
  -cleanline-strength VALUE
      Regression strength 0..1; default 1
  -precision VALUE
      auto (default), float, double
  -average-reference VALUE
      EEG only; true/false; default false
  -filter-pns VALUE
      Filter PNS too: true/false; default true
  -h, -help                Show this help

IIR is zero-phase (forward/reverse); 4 poles/pass means 48 dB/oct total.
CleanLine is EVA sinusoid regression, not the EEGLAB multitaper algorithm.
Presets approximate package conventions; they do not invoke external packages.
PNS uses its own sample rate and never participates in EEG average reference.
Recording gaps are preserved. Settings must be valid for every processed stream.
The output includes eva.xml processing provenance and a log_eva_*.txt audit log.

Examples:
  eva filter -lowpass 40.0 -highpass 0.1 input.mff output.mff
  eva filter -line-noise notch -line-frequency 60 -harmonics 3 input.mff output.mff
  eva filter -preset eeglab -highpass 1 -lowpass 40 -filter-pns false input.mff output.mff
"#;
pub fn parse(pairs: &[(String, String)]) -> Result<Settings, String> {
    let mut s = Settings {
        enabled: true,
        high_pass: false,
        low_pass: false,
        ..Default::default()
    };
    if let Some((_, v)) = pairs.iter().find(|(k, _)| k == "preset") {
        s.preset(match v.as_str() {
            "eeglab" => Preset::Eeglab,
            "erplab" => Preset::Erplab,
            "mne-python" => Preset::MnePython,
            "netstation" => Preset::NetStation,
            _ => return Err(format!("Unknown preset {v}")),
        });
    }
    for (key, value) in pairs {
        let v = value.as_str();
        let invalid = || format!("Invalid -{key} value: {value}");
        match key.as_str() {
            "preset" => {}
            "highpass" | "lowpass" => {
                let (on, hz) = if key == "highpass" {
                    (&mut s.high_pass, &mut s.high_hz)
                } else {
                    (&mut s.low_pass, &mut s.low_hz)
                };
                *on = v != "off";
                if *on {
                    *hz = v.parse().map_err(|_| invalid())?;
                }
            }
            "exclude-reference" => {
                s.options.excluded = if v == "none" {
                    vec![]
                } else {
                    v.split(',')
                        .map(|n| {
                            n.parse::<usize>()
                                .ok()
                                .and_then(|v| v.checked_sub(1))
                                .ok_or_else(invalid)
                        })
                        .collect::<Result<_, _>>()?
                };
            }
            "family" => {
                s.family = match v {
                    "iir" => Family::Iir,
                    "fir" => Family::Fir,
                    "auto" => Family::Auto,
                    _ => return Err(invalid()),
                }
            }
            "iir-design" => {
                s.options.design = match v {
                    "butterworth" => IirDesign::Butterworth,
                    "elliptic" => IirDesign::Elliptic,
                    _ => return Err(invalid()),
                }
            }
            "highpass-order" => s.order = v.parse().map_err(|_| invalid())?,
            "lowpass-order" => s.low_order = v.parse().map_err(|_| invalid())?,
            "ripple" => s.options.ripple = v.parse().map_err(|_| invalid())?,
            "stopband" => s.options.stop_db = v.parse().map_err(|_| invalid())?,
            "fir-window" => {
                s.options.window = match v {
                    "hamming" => FirWindow::Hamming,
                    "kaiser" => FirWindow::Kaiser,
                    _ => return Err(invalid()),
                }
            }
            "fir-attenuation" => s.options.attenuation = v.parse().map_err(|_| invalid())?,
            "fir-application" => {
                s.options.application = match v {
                    "zero-phase" => FirApplication::ZeroPhase,
                    "delay-compensated" => FirApplication::DelayCompensated,
                    "forward" => FirApplication::Forward,
                    _ => return Err(invalid()),
                }
            }
            "transition" => s.transition_hz = v.parse().map_err(|_| invalid())?,
            "auto-transition" => s.options.auto_transition = v.parse().map_err(|_| invalid())?,
            "fir-rule" => {
                s.options.rule = match v {
                    "eva" => DesignRule::Eva,
                    "eeglab-mne" => DesignRule::EeglabMne,
                    _ => return Err(invalid()),
                }
            }
            "crossover" => s.options.crossover = v.parse().map_err(|_| invalid())?,
            "crossover-inclusive" => s.options.through = v.parse().map_err(|_| invalid())?,
            "line-noise" => {
                s.line = match v {
                    "off" => LineMode::Off,
                    "notch" => LineMode::Notch,
                    "cleanline" => LineMode::CleanLine,
                    _ => return Err(invalid()),
                }
            }
            "line-frequency" => s.line_hz = v.parse().map_err(|_| invalid())?,
            "harmonics" => s.harmonics = v.parse().map_err(|_| invalid())?,
            "notch-q" => s.notch_q = v.parse().map_err(|_| invalid())?,
            "fir-notch" => s.options.fir_notch = v.parse().map_err(|_| invalid())?,
            "cleanline-window" => s.window_seconds = v.parse().map_err(|_| invalid())?,
            "cleanline-strength" => s.strength = v.parse().map_err(|_| invalid())?,
            "precision" => {
                s.options.precision = match v {
                    "auto" => Precision::Auto,
                    "float" => Precision::Float,
                    "double" => Precision::Double,
                    _ => return Err(invalid()),
                }
            }
            "average-reference" => {
                s.options.average_reference = v.parse().map_err(|_| invalid())?
            }
            "filter-pns" => s.options.filter_pns = v.parse().map_err(|_| invalid())?,
            _ => return Err(format!("Unknown option -{key}. Use eva filter -help.")),
        }
    }
    Ok(s)
}
pub fn arguments(s: &Settings) -> Vec<String> {
    let mut out = vec![];
    let mut add = |k: &str, v: String| {
        out.push(format!("-{k}"));
        out.push(v);
    };
    add(
        "highpass",
        if s.high_pass {
            s.high_hz.to_string()
        } else {
            "off".into()
        },
    );
    add(
        "lowpass",
        if s.low_pass {
            s.low_hz.to_string()
        } else {
            "off".into()
        },
    );
    add(
        "exclude-reference",
        if s.options.excluded.is_empty() {
            "none".into()
        } else {
            s.options
                .excluded
                .iter()
                .map(|v| (v + 1).to_string())
                .collect::<Vec<_>>()
                .join(",")
        },
    );
    add(
        "family",
        match s.family {
            Family::Iir => "iir",
            Family::Fir => "fir",
            Family::Auto => "auto",
        }
        .into(),
    );
    add(
        "iir-design",
        match s.options.design {
            IirDesign::Butterworth => "butterworth",
            IirDesign::Elliptic => "elliptic",
        }
        .into(),
    );
    add("highpass-order", s.order.to_string());
    add("lowpass-order", s.low_order.to_string());
    add("ripple", s.options.ripple.to_string());
    add("stopband", s.options.stop_db.to_string());
    add(
        "fir-window",
        match s.options.window {
            FirWindow::Hamming => "hamming",
            FirWindow::Kaiser => "kaiser",
        }
        .into(),
    );
    add("fir-attenuation", s.options.attenuation.to_string());
    add(
        "fir-application",
        match s.options.application {
            FirApplication::ZeroPhase => "zero-phase",
            FirApplication::DelayCompensated => "delay-compensated",
            FirApplication::Forward => "forward",
        }
        .into(),
    );
    add("transition", s.transition_hz.to_string());
    add("auto-transition", s.options.auto_transition.to_string());
    add(
        "fir-rule",
        match s.options.rule {
            DesignRule::Eva => "eva",
            DesignRule::EeglabMne => "eeglab-mne",
        }
        .into(),
    );
    add("crossover", s.options.crossover.to_string());
    add("crossover-inclusive", s.options.through.to_string());
    add(
        "line-noise",
        match s.line {
            LineMode::Off => "off",
            LineMode::Notch => "notch",
            LineMode::CleanLine => "cleanline",
        }
        .into(),
    );
    add("line-frequency", s.line_hz.to_string());
    add("harmonics", s.harmonics.to_string());
    add("notch-q", s.notch_q.to_string());
    add("fir-notch", s.options.fir_notch.to_string());
    add("cleanline-window", s.window_seconds.to_string());
    add("cleanline-strength", s.strength.to_string());
    add(
        "precision",
        match s.options.precision {
            Precision::Auto => "auto",
            Precision::Float => "float",
            Precision::Double => "double",
        }
        .into(),
    );
    add("average-reference", s.options.average_reference.to_string());
    add("filter-pns", s.options.filter_pns.to_string());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gui_settings_round_trip_through_cli() {
        for preset in [
            Preset::Eeglab,
            Preset::Erplab,
            Preset::MnePython,
            Preset::NetStation,
        ] {
            let mut s = Settings {
                enabled: true,
                ..Default::default()
            };
            s.preset(preset);
            s.options.average_reference = true;
            s.options.excluded = vec![0, 7];
            s.line = LineMode::CleanLine;
            s.line_hz = 50.;
            s.harmonics = 2;
            s.options.precision = Precision::Double;
            let args = arguments(&s);
            let pairs = args
                .as_chunks::<2>()
                .0
                .iter()
                .map(|v| (v[0].trim_start_matches('-').to_owned(), v[1].clone()))
                .collect::<Vec<_>>();
            assert_eq!(parse(&pairs).unwrap(), s);
        }
        assert!(parse(&[("unknown".into(), "1".into())]).is_err());
        assert!(parse(&[("exclude-reference".into(), "0".into())]).is_err());
    }
}
