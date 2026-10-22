//! Read-only AFNI-style MFF metadata tables and event timing reports.
mod timing;
use eva_mff::{Event, Recording};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use timing::Matching;
const HELP: &str = r#"Usage: eva info [METRICS] [OPTIONS] <input.mff> [input2.mff ...]

One row per recording; columns follow metric flag order (AFNI 3dinfo style).
Options use a single dash; hyphens and underscores in option names equivalent.
No metric flags: prefix, nchannels, sfreq, nt, duration, nevents, event_types,
ngaps, gap_seconds, nsteps. No header unless -header_line is requested.

Metrics:
  -prefix             Package filename, including .mff
  -prefix_noext       Package filename without extension
  -iname              Path as supplied
  -exists             1 if readable, 0 otherwise
  -error              Load/metadata error text, or NA
  -kind               Continuous, Discontinuous, SegmentedOrAveraged
  -nchannels|-nchan    Active stream channel count (EEG by default)
  -sfreq|-srate        Sampling rate, Hz
  -nt|-nsamples       Timeline samples per channel, INCLUDING gaps
  -acquired_samples   Stored samples per channel, excluding gaps
  -data_points        Stored scalar values: acquired_samples * channels
  -duration           Timeline duration, seconds (including gaps)
  -acquired_duration  Acquired duration, seconds (excluding gaps)
  -start              Absolute recording start from info.xml
  -nevents            Number of event records (all tracks)
  -event_types        JSON object: exact event code -> count
  -event_count CODE   Count for one event code (0 if absent)
  -events             JSON array: event onsets, duration, labels, sample indices
  -out_of_range_events  Events before/after the active signal timeline
  -ngaps              Count of gaps, including leading gaps
  -gap_seconds        Total missing duration on the stream's sample grid
  -gaps               JSON gap list with half-open sample/second bounds
  -nsteps             Number of direct steps in eva.xml (0 if absent)
  -history|-processing  Ordered processing steps with ALL parameters and notes
  -labels|-names      JSON channel-name array (recorded order)
  -locations          JSON raw coordinates and unit-sphere positions
  -channels           JSON channel table: number, name, units, GCAL, position
  -nlocations         Number of scalp channels with usable unit-sphere positions
  -reference          Acquisition reference metadata
  -streams            EEG/PNS rate, sample counts, duration, gap counts
  -pns_channels       Number of PNS channels (0 if absent)
  -warnings           Reader and optional-metadata warnings
  -all                All standard metrics (large structured cells)

Timing:
  -timing A B         Expand to numeric columns: a_count, b_count, matched,
                      unmatched_a, unused_b, reused_matches, mean_ms, median_ms,
                      std_ms, min_ms, max_ms. Delays are B onset MINUS A onset.
  -timing_details A B JSON summary plus one pair record per A event
  -intervals CODE     Expand to count/mean/median/std/min/max of inter-event ms
  -match METHOD      nearest (default), next, previous, ordinal
  -max_lag MS        Maximum absolute pair delay; default 100 ms (inclusive)

Nearest/next/previous choose a B independently for each A; B may be reused.
Nearest ties choose the earlier B; equal timestamps can pair at zero delay.
Ordinal pairs A[i] with B[i] without shifting indices after rejected matches.
Duplicate event records are retained; reuse/unmatched counts expose ambiguity.
Timing uses exact event timestamps, including events outside signal coverage,
not rounded sample positions. Population standard deviation is reported.
Use -intervals for repetition timing; -timing requires distinct event codes.
All timing selections share -match/-max_lag; the flags can be repeated for pairs.

Output options:
  -header_line|-hdr   Print column names
  -format tsv|csv|json  Default TSV. JSON is an array of structured objects.
  -json              Shortcut for -format json (default: all standard metrics)
  -stream eeg|pns    Select the signal timebase; default eeg
  -NA_flag TEXT      Missing scalar in tables; default NA (JSON uses null)
  -atr_delim TEXT    Table delimiter (TSV mode only); default tab
  -h|-help    Show this help
  --                 End options (for paths beginning with '-')

Tables escape tabs/newlines/backslashes inside cells. CSV follows normal quoting.
Structured table cells use compact JSON. Coordinates retain their original XML
values; units are unspecified unless declared. Unit-sphere coordinates are not mm.
Gaps come from indexed block timing, not NaNs within acquired sample values.
Missing eva.xml means no recorded history, not proof of unprocessed samples.
Malformed eva.xml is reported in warnings (history is null, not empty).
Unreadable input gives an error row, stderr diagnostic, and nonzero exit status;
other inputs are still inspected. No waveform samples are loaded or files edited.

Examples:
  eva info -header_line -prefix -nchan -sfreq -nt -duration *.mff
  eva info -hdr -prefix -event_count DIN1 -event_count TREV -ngaps *.mff
  eva info -hdr -prefix -timing DIN1 cue+ -max_lag 250 *.mff
  eva info -json -prefix -timing_details DIN2 TREV recording.mff > timing.json
  eva info -json recording.mff > info.json
  eva info -format csv -hdr -prefix -history -gaps *.mff > comparison.csv
"#;
const STANDARD: &[&str] = &[
    "prefix",
    "prefix_noext",
    "iname",
    "exists",
    "error",
    "kind",
    "nchannels",
    "sfreq",
    "nt",
    "acquired_samples",
    "data_points",
    "duration",
    "acquired_duration",
    "start",
    "nevents",
    "event_types",
    "events",
    "out_of_range_events",
    "ngaps",
    "gap_seconds",
    "gaps",
    "nsteps",
    "history",
    "labels",
    "locations",
    "channels",
    "nlocations",
    "reference",
    "streams",
    "pns_channels",
    "warnings",
];
const DEFAULT: &[&str] = &[
    "prefix",
    "nchannels",
    "sfreq",
    "nt",
    "duration",
    "nevents",
    "event_types",
    "ngaps",
    "gap_seconds",
    "nsteps",
];
#[derive(Debug, Clone)]
enum Column {
    Metric(String),
    Count(String),
    Timing(String, String, bool),
    Intervals(String),
}
struct Options {
    columns: Vec<Column>,
    paths: Vec<PathBuf>,
    format: String,
    header: bool,
    delimiter: String,
    na: String,
    pns: bool,
    matching: Matching,
    tolerance: f64,
}
fn parse(args: Vec<OsString>) -> Result<Options, String> {
    let mut o = Options {
        columns: vec![],
        paths: vec![],
        format: "tsv".into(),
        header: false,
        delimiter: "\t".into(),
        na: "NA".into(),
        pns: false,
        matching: Matching::Nearest,
        tolerance: 100.,
    };
    let mut args = args.into_iter();
    let mut end = false;
    while let Some(arg) = args.next() {
        if !end && arg == "--" {
            end = true;
            continue;
        }
        if end || !arg.to_str().is_some_and(|v| v.starts_with('-')) {
            o.paths.push(arg.into());
            continue;
        }
        let key = arg
            .to_str()
            .unwrap()
            .strip_prefix('-')
            .filter(|key| !key.starts_with('-'))
            .ok_or("Options must use a single dash")?
            .replace('-', "_");
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("Missing value for -{key}"))?
                .into_string()
                .map_err(|_| format!("-{key} value must be UTF-8"))
        };
        match key.as_str() {
            "header_line" | "hdr" => o.header = true,
            "format" => o.format = value()?,
            "json" => o.format = "json".into(),
            "atr_delim" => o.delimiter = value()?,
            "NA_flag" | "na_flag" => o.na = value()?,
            "stream" => {
                o.pns = match value()?.as_str() {
                    "eeg" => false,
                    "pns" => true,
                    _ => return Err("Stream must be eeg or pns".into()),
                }
            }
            "match" => {
                o.matching = match value()?.as_str() {
                    "nearest" => Matching::Nearest,
                    "next" => Matching::Next,
                    "previous" => Matching::Previous,
                    "ordinal" => Matching::Ordinal,
                    _ => return Err("Unknown event matching method".into()),
                }
            }
            "max_lag" => {
                o.tolerance = value()?.parse().map_err(|_| "Invalid max lag")?;
                if !o.tolerance.is_finite() || o.tolerance < 0. {
                    return Err("Max lag must be finite and nonnegative".into());
                }
            }
            "timing" | "timing_details" => {
                let a = value()?;
                let b = value()?;
                if a == b {
                    return Err("Use -intervals CODE for within-code timing".into());
                }
                o.columns
                    .push(Column::Timing(a, b, key == "timing_details"));
            }
            "intervals" => o.columns.push(Column::Intervals(value()?)),
            "event_count" => o.columns.push(Column::Count(value()?)),
            "all" => o
                .columns
                .extend(STANDARD.iter().map(|v| Column::Metric((*v).into()))),
            _ => {
                let key = match key.as_str() {
                    "nchan" => "nchannels",
                    "srate" => "sfreq",
                    "nsamples" => "nt",
                    "names" => "labels",
                    "processing" => "history",
                    v => v,
                };
                if !STANDARD.contains(&key) {
                    return Err(format!("Unknown metric -{key}; use eva info -help"));
                }
                o.columns.push(Column::Metric(key.into()));
            }
        }
    }
    if !["tsv", "csv", "json"].contains(&o.format.as_str()) {
        return Err("Format must be tsv, csv, or json".into());
    }
    if o.delimiter.is_empty() || o.delimiter.contains(['\n', '\r']) {
        return Err("Table delimiter must be nonempty and cannot contain newlines".into());
    }
    if o.paths.is_empty() {
        return Err("Provide one or more MFF paths; use eva info -help".into());
    }
    if o.columns.is_empty() {
        o.columns.extend(
            if o.format == "json" {
                STANDARD
            } else {
                DEFAULT
            }
            .iter()
            .map(|v| Column::Metric((*v).into())),
        );
    }
    Ok(o)
}
fn xml_text(path: &Path) -> Result<String, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut text = String::new();
    f.take(32 * 1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 32 * 1024 * 1024 {
        return Err("XML exceeds 32 MiB".into());
    }
    Ok(text)
}
fn history(path: &Path) -> Result<Value, String> {
    let file = path.join("eva.xml");
    if !file.exists() {
        return Ok(json!([]));
    }
    let text = xml_text(&file)?;
    let doc = roxmltree::Document::parse(&text).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if root.tag_name().name() != "evaProcessing" {
        return Err("Unsupported eva.xml root".into());
    }
    Ok(json!(root.children().filter(|n|n.has_tag_name("step")).map(|n|{
        let mut parameters=BTreeMap::new();let mut notes=vec![];
        for child in n.children().filter(|n|n.is_element()){
            if child.has_tag_name("param"){if let Some(key)=child.attribute("key"){parameters.insert(key.to_owned(),child.text().unwrap_or("").to_owned());}}
            if child.has_tag_name("note"){notes.push(child.text().unwrap_or("").to_owned());}
        }
        json!({"operation":n.attribute("op"),"replayable":n.attribute("replayable").map(|v|v=="true"),"applied_at":n.attribute("appliedAt"),"parameters":parameters,"notes":notes,"xml":&text[n.range()]})
    }).collect::<Vec<_>>()))
}
fn raw_positions(path: &Path) -> Result<BTreeMap<usize, Value>, String> {
    let file = path.join("coordinates.xml");
    if !file.exists() {
        return Ok(BTreeMap::new());
    }
    let text = xml_text(&file)?;
    let doc = roxmltree::Document::parse(&text).map_err(|e| e.to_string())?;
    let mut out = BTreeMap::new();
    for n in doc.descendants().filter(|n| n.has_tag_name("sensor")) {
        let get = |name: &str| {
            n.children()
                .find(|v| v.has_tag_name(name))
                .and_then(|n| n.text())
                .map(str::trim)
        };
        let Some(number) = get("number").and_then(|v| v.parse::<usize>().ok()) else {
            continue;
        };
        if !matches!(get("type"), Some("0" | "1")) {
            continue;
        }
        let xyz = ["x", "y", "z"].map(|k| {
            get(k)
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite())
        });
        let unit = get("unit").or_else(|| {
            doc.descendants()
                .find(|n| n.has_tag_name("units"))
                .and_then(|n| n.text())
        });
        out.insert(
            number,
            json!({"xyz":xyz,"units":unit,"sensor_type":get("type")}),
        );
    }
    Ok(out)
}
fn stream_summary(r: &Recording) -> Value {
    let m = r.metadata();
    json!({"type":m.streams[m.eeg_stream].kind,"file":m.streams[m.eeg_stream].file_name,"channels":m.channels.len(),"sfreq":m.sample_rate,"timeline_samples":m.sample_count,"acquired_samples":r.acquired_sample_count(),"duration_seconds":m.duration_seconds(),"gaps":r.gaps().len()})
}
fn event_json(e: &Event, r: &Recording, gaps: &[std::ops::Range<u64>]) -> Value {
    let sample = e.sample_index(r.metadata().sample_rate).ok();
    let position = e.onset_ns.checked_mul(i128::from(r.metadata().sample_rate));
    let in_gap = position.is_some_and(|s| {
        gaps.iter().any(|g| {
            s >= i128::from(g.start) * 1_000_000_000 && s < i128::from(g.end) * 1_000_000_000
        })
    });
    let outside = position
        .is_none_or(|s| s < 0 || s >= i128::from(r.metadata().sample_count) * 1_000_000_000);
    json!({"code":e.code,"label":e.label,"onset_ns":e.onset_ns.to_string(),"onset_seconds":e.onset_ns as f64/1e9,"duration_seconds":e.duration_ns.map(|v|v as f64/1e9),"sample_index":sample,"in_gap":in_gap,"outside_signal":outside,"source_file":e.source_file,"absolute_onset":e.raw_begin_time})
}
fn report(path: &Path, o: &Options) -> Result<(Value, Vec<Event>), String> {
    let root = Recording::open(path).map_err(|e| e.to_string())?;
    let r = if o.pns {
        root.pns().ok_or("Recording has no PNS stream")?.as_ref()
    } else {
        &root
    };
    let m = r.metadata();
    let gaps = r.gaps();
    let mut warnings = m.warnings.clone();
    let history = history(path).unwrap_or_else(|e| {
        warnings.push(format!("eva.xml: {e}"));
        Value::Null
    });
    let positions = if o.pns {
        BTreeMap::new()
    } else {
        raw_positions(path).unwrap_or_else(|e| {
            warnings.push(format!("coordinates.xml: {e}"));
            BTreeMap::new()
        })
    };
    let counts = m
        .events
        .iter()
        .fold(BTreeMap::<String, usize>::new(), |mut a, e| {
            *a.entry(e.code.clone()).or_default() += 1;
            a
        });
    let acquired = r.acquired_sample_count();
    let gap_samples: u64 = gaps.iter().map(|g| g.end - g.start).sum();
    let locations:Vec<_>=m.channels.iter().map(|c|json!({"channel":c.index+1,"name":c.label,"original":positions.get(&(c.index+1)),"unit_sphere":c.position})).collect();
    let channels:Vec<_>=m.channels.iter().map(|c|json!({"channel":c.index+1,"name":c.label,"unit":c.unit,"calibration":c.calibration,"positive_up":c.positive_up,"location":locations[c.index]})).collect();
    let events: Vec<_> = m.events.iter().map(|e| event_json(e, r, &gaps)).collect();
    let mut streams = vec![stream_summary(&root)];
    if let Some(p) = root.pns() {
        streams.push(stream_summary(p));
    }
    let data = json!({"prefix":path.file_name().unwrap_or_default().to_string_lossy(),"prefix_noext":path.file_stem().unwrap_or_default().to_string_lossy(),"iname":path.to_string_lossy(),"exists":1,"error":null,"kind":format!("{:?}",m.kind),"nchannels":m.channels.len(),"sfreq":m.sample_rate,"nt":m.sample_count,"acquired_samples":acquired,"data_points":acquired.checked_mul(m.channels.len() as u64),"duration":m.duration_seconds(),"acquired_duration":acquired as f64/m.sample_rate as f64,"start":m.recording_start,"nevents":m.events.len(),"event_types":counts,"out_of_range_events":events.iter().filter(|v|v["outside_signal"]==true).count(),"events":events,"ngaps":gaps.len(),"gap_seconds":gap_samples as f64/m.sample_rate as f64,"gaps":gaps.iter().map(|g|json!({"code":eva_mff::AcquisitionGap::CODE,"label":eva_mff::AcquisitionGap::LABEL,"start_sample":g.start,"end_sample":g.end,"samples":g.end-g.start,"start_seconds":g.start as f64/m.sample_rate as f64,"end_seconds":g.end as f64/m.sample_rate as f64,"duration_seconds":(g.end-g.start)as f64/m.sample_rate as f64})).collect::<Vec<_>>(),"nsteps":history.as_array().map(Vec::len),"history":history,"labels":m.channels.iter().map(|c|&c.label).collect::<Vec<_>>(),"locations":locations,"channels":channels,"nlocations":m.channels.iter().filter(|c|c.position.is_some()).count(),"reference":m.acquisition_reference.as_ref().map(|a|json!({"sensor_number":a.sensor_number,"name":a.label,"recorded_channel":a.recorded_channel.map(|c|c+1)})),"streams":streams,"pns_channels":root.pns().map(|p|p.metadata().channels.len()).unwrap_or(0),"warnings":warnings});
    Ok((data, m.events.clone()))
}
const TIMING_FIELDS: &[&str] = &[
    "a_count",
    "b_count",
    "matched",
    "unmatched_a",
    "unused_b",
    "reused_matches",
    "mean_ms",
    "median_ms",
    "std_ms",
    "min_ms",
    "max_ms",
];
const INTERVAL_FIELDS: &[&str] = &[
    "count",
    "mean_ms",
    "median_ms",
    "std_ms",
    "min_ms",
    "max_ms",
];
fn columns(o: &Options, data: &Value, events: &[Event]) -> Vec<(String, Value)> {
    let mut out = vec![];
    let valid = data["exists"] == 1;
    for c in &o.columns {
        match c {
            Column::Metric(k) => out.push((k.clone(), data[k].clone())),
            Column::Count(code) => out.push((
                format!("events[{code}]"),
                if valid {
                    data["event_types"][code]
                        .as_u64()
                        .map_or(json!(0), |v| json!(v))
                } else {
                    Value::Null
                },
            )),
            Column::Timing(a, b, details) => {
                let name = format!("timing[{a}->{b}]");
                let t = if valid {
                    timing::compare(events, a, b, o.matching, o.tolerance, *details)
                } else {
                    Value::Null
                };
                if *details {
                    out.push((name, t));
                } else {
                    for field in TIMING_FIELDS {
                        out.push((format!("{name}.{field}"), t[field].clone()));
                    }
                }
            }
            Column::Intervals(code) => {
                let t = if valid {
                    timing::intervals(events, code)
                } else {
                    Value::Null
                };
                for field in INTERVAL_FIELDS {
                    out.push((format!("intervals[{code}].{field}"), t[field].clone()));
                }
            }
        }
    }
    out
}
fn cell(v: &Value, na: &str) -> String {
    match v {
        Value::Null => na.to_owned(),
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}
fn row(values: &[String], o: &Options) -> String {
    if o.format == "csv" {
        values
            .iter()
            .map(|v| format!("\"{}\"", v.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(",")
    } else {
        values
            .iter()
            .map(|s| {
                s.replace('\\', "\\\\")
                    .replace('\t', "\\t")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
            })
            .collect::<Vec<_>>()
            .join(&o.delimiter)
    }
}
pub fn run(args: Vec<OsString>) -> Result<(), String> {
    if args.is_empty()
        || args
            .iter()
            .take_while(|a| *a != "--")
            .any(|a| matches!(a.to_str(), Some("-h" | "-help")))
    {
        println!("{HELP}");
        return Ok(());
    }
    let o = parse(args)?;
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    let mut failed = 0;
    if o.format == "json" {
        writeln!(writer, "[").map_err(|e| e.to_string())?;
    }
    for (index, path) in o.paths.iter().enumerate() {
        let (data, events) = match report(path, &o) {
            Ok(v) => v,
            Err(e) => {
                failed += 1;
                eprintln!("eva info: {}: {e}", path.display());
                (
                    json!({"prefix":path.file_name().unwrap_or_default().to_string_lossy(),"prefix_noext":path.file_stem().unwrap_or_default().to_string_lossy(),"iname":path.to_string_lossy(),"exists":0,"error":e}),
                    vec![],
                )
            }
        };
        let cols = columns(&o, &data, &events);
        if o.format == "json" {
            if index > 0 {
                writeln!(writer, ",").map_err(|e| e.to_string())?;
            }
            let obj = cols.into_iter().collect::<serde_json::Map<_, _>>();
            serde_json::to_writer_pretty(&mut writer, &obj).map_err(|e| e.to_string())?;
        } else {
            if index == 0 && o.header {
                writeln!(
                    writer,
                    "{}",
                    row(&cols.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(), &o)
                )
                .map_err(|e| e.to_string())?;
            }
            writeln!(
                writer,
                "{}",
                row(
                    &cols.iter().map(|(_, v)| cell(v, &o.na)).collect::<Vec<_>>(),
                    &o
                )
            )
            .map_err(|e| e.to_string())?;
        }
    }
    if o.format == "json" {
        writeln!(writer, "\n]").map_err(|e| e.to_string())?;
    }
    if failed > 0 {
        Err(format!("{failed} input(s) could not be inspected"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(flags: &[&str]) -> Options {
        parse(
            flags
                .iter()
                .map(OsString::from)
                .chain([OsString::from("test.mff")])
                .collect(),
        )
        .unwrap()
    }
    #[test]
    fn metadata_distinguishes_acquired_samples_gaps_and_history() {
        let d = crate::tests::fixture();
        let (v, _) = report(d.path(), &options(&[])).unwrap();
        assert_eq!(v["nt"], 4224);
        assert_eq!(v["acquired_samples"], 4096);
        assert_eq!(v["data_points"], 8192);
        assert_eq!(v["duration"], 16.5);
        assert_eq!(v["gap_seconds"], 0.5);
        assert_eq!(v["gaps"][0]["start_sample"], 2048);
        assert_eq!(v["event_types"]["TREV"], 1);
        assert_eq!(v["history"][0]["operation"], "markBad");
        assert_eq!(v["history"][0]["notes"][0], "earlier & preserved");
        let (p, _) = report(d.path(), &options(&["-stream", "pns"])).unwrap();
        assert_eq!(p["sfreq"], 128);
        assert_eq!(p["acquired_samples"], 2048);
        assert_eq!(p["gap_seconds"], 0.5);
        std::fs::write(d.path().join("eva.xml"), "broken").unwrap();
        let (v, _) = report(d.path(), &options(&[])).unwrap();
        assert!(v["history"].is_null());
        assert!(v["nsteps"].is_null());
        assert!(v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("eva.xml")));
    }
    #[test]
    fn ordered_columns_missing_events_and_csv_escaping() {
        let o = options(&["-sfreq", "-prefix", "-event_count", "absent"]);
        let cols = columns(
            &o,
            &json!({"exists":1,"prefix":"a","sfreq":256,"event_types":{}}),
            &[],
        );
        assert_eq!(
            cols.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
            ["sfreq", "prefix", "events[absent]"]
        );
        assert_eq!(cols[2].1, 0);
        let o = options(&["-format", "csv"]);
        assert_eq!(
            row(&["a,b".into(), "a\"b".into()], &o),
            "\"a,b\",\"a\"\"b\""
        );
        assert!(parse(vec!["-max_lag".into(), "NaN".into(), "x".into()]).is_err());
    }
    #[test]
    fn event_coverage_uses_exact_time_not_rounded_sample() {
        let d = crate::tests::fixture();
        let r = Recording::open(d.path()).unwrap();
        let mut e = r.metadata().events[0].clone();
        e.onset_ns = -1;
        assert_eq!(event_json(&e, &r, &r.gaps())["outside_signal"], true);
        e.onset_ns = 8_000_000_000 - 1;
        assert_eq!(event_json(&e, &r, &r.gaps())["in_gap"], false);
        e.onset_ns += 1;
        assert_eq!(event_json(&e, &r, &r.gaps())["in_gap"], true);
    }
}
