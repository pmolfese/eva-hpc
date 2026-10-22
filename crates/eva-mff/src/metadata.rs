use crate::*;
use roxmltree::{Document, Node};
use std::{fs, io::Read};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

fn named(n: Node<'_, '_>, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name
}
fn child<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.children()
        .find(|c| named(*c, name))
        .and_then(|c| c.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}
fn find<'a>(d: &'a Document<'a>, name: &str) -> Option<&'a str> {
    d.descendants()
        .find(|n| named(*n, name))
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}
fn xml(path: &Path, limits: &Limits) -> Result<String> {
    let f = File::open(path).map_err(|e| error::io(path, e))?;
    let mut text = String::new();
    f.take(limits.max_xml_bytes as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| error::io(path, e))?;
    if text.len() > limits.max_xml_bytes {
        return Err(Error::Limit(format!("XML bytes: {}", path.display())));
    }
    Ok(text)
}
fn parse(text: &str) -> Result<Document<'_>> {
    Document::parse(text).map_err(|e| Error::Invalid(format!("XML: {e}")))
}
fn number<T: std::str::FromStr>(s: &str, field: &str) -> Result<T> {
    s.parse()
        .map_err(|_| Error::Invalid(format!("invalid {field}: {s}")))
}
fn stamp(s: &str) -> Result<i128> {
    OffsetDateTime::parse(s, &Rfc3339)
        .map(|t| t.unix_timestamp_nanos())
        .map_err(|_| Error::Unsupported(format!("timestamp must be RFC3339 with timezone: {s}")))
}
pub(crate) fn open(path: &Path, limits: Limits) -> Result<Recording> {
    if !path.is_dir() {
        return Err(Error::Invalid("MFF must be an unpacked directory".into()));
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| error::io(path, e))? {
        let entry = entry.map_err(|e| error::io(path, e))?;
        if files.len() >= limits.max_package_files {
            return Err(Error::Limit("package file count".into()));
        }
        files.push(entry.path());
    }
    files.sort();
    let info_text = xml(&path.join("info.xml"), &limits)?;
    let info = parse(&info_text)?;
    let recording_start = find(&info, "recordTime").map(str::to_owned);
    let start_ns = recording_start.as_deref().map(stamp).transpose()?;
    let acquisition_version = find(&info, "acquisitionVersion").map(str::to_owned);
    let mut streams = Vec::new();
    for file in &files {
        let name = file.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if let Some(id) = name
            .strip_prefix("signal")
            .and_then(|s| s.strip_suffix(".bin"))
        {
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let text = xml(&path.join(format!("info{id}.xml")), &limits)?;
            let doc = parse(&text)?;
            let kind = doc
                .descendants()
                .find(|n| named(*n, "fileDataType"))
                .and_then(|n| n.children().find(|c| c.is_element()))
                .ok_or_else(|| Error::Invalid(format!("missing fileDataType for {name}")))?
                .tag_name()
                .name()
                .to_owned();
            streams.push(Stream {
                file_name: name.to_owned(),
                kind,
            });
        }
    }
    let eeg: Vec<_> = streams
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind.eq_ignore_ascii_case("EEG"))
        .map(|(i, _)| i)
        .collect();
    if eeg.len() != 1 {
        return Err(Error::Unsupported(
            "requires exactly one declared EEG stream".into(),
        ));
    }
    let eeg_stream = eeg[0];
    let signal_path = path.join(&streams[eeg_stream].file_name);
    let mut index = binary::index(&signal_path, &limits)?;
    let id = streams[eeg_stream]
        .file_name
        .trim_start_matches("signal")
        .trim_end_matches(".bin");
    let text = xml(&path.join(format!("info{id}.xml")), &limits)?;
    let doc = parse(&text)?;
    let mut channels: Vec<_> = (0..index.channels)
        .map(|i| Channel {
            position: None,
            index: i,
            label: format!("Ch {}", i + 1),
            unit: None,
            calibration: 1.0,
            positive_up: None,
            impedance_kohm: None,
        })
        .collect();
    let mut warnings = Vec::new();
    if streams
        .iter()
        .any(|s| !s.kind.eq_ignore_ascii_case("EEG") && !s.kind.eq_ignore_ascii_case("PNSData"))
    {
        warnings.push("Unknown additional stream types are listed only".into());
    }
    calibrate(&doc, &mut channels)?;
    let adjustment = doc
        .descendants()
        .find(|n| named(*n, "hardwareFilterAdjusted"));
    let hardware_filter_adjusted = adjustment
        .map(|n| match n.text().unwrap_or("").trim() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(Error::Invalid("hardwareFilterAdjusted boolean".into())),
        })
        .transpose()?;
    let hardware_filter_shift_microseconds = adjustment
        .and_then(|n| n.attribute("shiftMicroseconds"))
        .map(|s| number(s, "shiftMicroseconds"))
        .transpose()?;
    if adjustment.is_some() || acquisition_version.is_some() {
        warnings.push(
            "Acquisition timing metadata preserved; no additional event/sample shift applied"
                .into(),
        );
    }
    let mut acquisition_reference = None;
    let mut sensor_layout_name = None;
    let layout_path = path.join("sensorLayout.xml");
    if layout_path.exists() {
        let text = xml(&layout_path, &limits)?;
        let doc = parse(&text)?;
        sensor_layout_name = child(doc.root_element(), "name").map(str::to_owned);
        let mut seen = std::collections::HashSet::new();
        for sensor in doc.descendants().filter(|n| named(*n, "sensor")) {
            let kind = child(sensor, "type").unwrap_or("0");
            if kind != "0" && kind != "1" {
                continue;
            }
            let num: usize = number(child(sensor, "number").unwrap_or(""), "sensor number")?;
            if num == 0 || !seen.insert(num) {
                return Err(Error::Invalid("invalid/duplicate sensor number".into()));
            }
            let label = child(sensor, "name")
                .or_else(|| child(sensor, "label"))
                .map(str::to_owned)
                .unwrap_or_else(|| format!("E{num}"));
            if num <= channels.len() {
                channels[num - 1].label = label.clone();
                channels[num - 1].unit = child(sensor, "unit").map(str::to_owned);
            }
            if kind == "1" {
                if acquisition_reference.is_some() {
                    return Err(Error::Unsupported("multiple acquisition references".into()));
                }
                acquisition_reference = Some(AcquisitionReference {
                    sensor_number: num,
                    label,
                    recorded_channel: (num <= channels.len()).then_some(num - 1),
                });
            }
        }
    } else {
        warnings.push("sensorLayout.xml absent; channel labels are positional placeholders".into());
    }
    if channels.iter().any(|c| c.unit.is_none()) {
        warnings.push(
            "Missing EEG units interpreted as uV by the MFF EEG convention; sample values are unchanged".into(),
        );
    }
    for channel in &mut channels {
        if channel.unit.as_ref().is_none_or(|u| u.trim().is_empty()) {
            channel.unit = Some("uV".into());
        }
    }
    read_impedance(&doc, &mut channels);
    let coordinates = path.join("coordinates.xml");
    if coordinates.exists() {
        let text = xml(&coordinates, &limits)?;
        let doc = parse(&text)?;
        let mut seen = std::collections::HashSet::new();
        for sensor in doc.descendants().filter(|n| named(*n, "sensor")) {
            if child(sensor, "type") != Some("0") {
                continue;
            }
            let num: usize = number(
                child(sensor, "number").unwrap_or(""),
                "coordinate sensor number",
            )?;
            if num == 0 || num > channels.len() {
                continue;
            }
            if !seen.insert(num) {
                return Err(Error::Invalid("duplicate coordinate sensor".into()));
            }
            let mut v = [0.; 3];
            let mut valid = true;
            for (i, axis) in ["x", "y", "z"].iter().enumerate() {
                match child(sensor, axis).and_then(|v| v.parse::<f64>().ok()) {
                    Some(x) if x.is_finite() => v[i] = x,
                    _ => valid = false,
                }
            }
            let norm = v[0].hypot(v[1]).hypot(v[2]);
            if valid && norm.is_finite() && norm > 0. {
                channels[num - 1].position = Some(v.map(|x| x / norm));
            } else {
                warnings.push(format!("Invalid/missing coordinates for channel {num}; interpolation unavailable there."));
            }
        }
    }
    let mut kind = apply_epochs(path, &mut index, &limits, &mut warnings)?;
    let categories = path.join("categories.xml");
    if categories.exists() {
        let text = xml(&categories, &limits)?;
        let doc = parse(&text)?;
        if doc
            .descendants()
            .any(|n| named(n, "category") || named(n, "cat"))
        {
            kind = RecordingKind::SegmentedOrAveraged;
        }
    }
    let mut events = Vec::new();
    let mut event_xml_bytes = 0usize;
    for file in &files {
        let name = file.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.starts_with("Events") || !name.ends_with(".xml") {
            continue;
        }
        let text = xml(file, &limits)?;
        event_xml_bytes = event_xml_bytes
            .checked_add(text.len())
            .ok_or_else(|| Error::Limit("event XML bytes".into()))?;
        if event_xml_bytes > limits.max_xml_bytes {
            return Err(Error::Limit("aggregate event XML bytes".into()));
        }
        let doc = parse(&text)?;
        for event in doc.descendants().filter(|n| named(*n, "event")) {
            let code =
                child(event, "code").ok_or_else(|| Error::Invalid("event missing code".into()))?;
            let raw = child(event, "beginTime")
                .ok_or_else(|| Error::Invalid("event missing beginTime".into()))?;
            let onset_ns =
                stamp(raw)?
                    .checked_sub(start_ns.ok_or_else(|| {
                        Error::Invalid("absolute events require recordTime".into())
                    })?)
                    .ok_or_else(|| Error::Invalid("event onset overflow".into()))?;
            let duration_ns = child(event, "duration")
                .map(|s| {
                    let us: i128 = number(s, "event duration")?;
                    if us < 0 {
                        return Err(Error::Invalid("negative event duration".into()));
                    }
                    us.checked_mul(1000)
                        .ok_or_else(|| Error::Invalid("event duration overflow".into()))
                })
                .transpose()?;
            events.push(Event {
                code: code.into(),
                label: child(event, "label").map(str::to_owned),
                raw_begin_time: raw.into(),
                source_file: name.into(),
                onset_ns,
                duration_ns,
            });
        }
    }
    events.sort_by_key(|e| e.onset_ns);
    let metadata = Metadata {
        sensor_layout_name,
        streams,
        eeg_stream,
        channels,
        sample_rate: index.rate,
        sample_count: index.samples,
        recording_start,
        acquisition_reference,
        acquisition_version,
        hardware_filter_adjusted,
        hardware_filter_shift_microseconds,
        kind,
        events,
        warnings,
    };
    let pns = open_pns(path, &metadata, &limits)?;
    Ok(Recording {
        pns,
        metadata,
        index,
        signal_path,
        limits,
    })
}

fn apply_epochs(
    path: &Path,
    index: &mut binary::Index,
    limits: &Limits,
    warnings: &mut Vec<String>,
) -> Result<RecordingKind> {
    let mut kind = RecordingKind::Continuous;
    let epochs_path = path.join("epochs.xml");
    if epochs_path.exists() {
        let text = xml(&epochs_path, limits)?;
        let doc = parse(&text)?;
        let epochs: Vec<_> = doc.descendants().filter(|n| named(*n, "epoch")).collect();
        let mut next_block = 1usize;
        let mut previous_end = 0i128;
        for epoch in &epochs {
            let first: usize = number(child(*epoch, "firstBlock").unwrap_or(""), "firstBlock")?;
            let last: usize = number(child(*epoch, "lastBlock").unwrap_or(""), "lastBlock")?;
            let begin: i128 = number(child(*epoch, "beginTime").unwrap_or(""), "epoch beginTime")?;
            let end: i128 = number(child(*epoch, "endTime").unwrap_or(""), "epoch endTime")?;
            if first != next_block
                || last < first
                || last > index.blocks.len()
                || begin < 0
                || end <= begin
            {
                return Err(Error::Invalid("invalid epoch block/time range".into()));
            }
            if begin != previous_end {
                kind = RecordingKind::Discontinuous;
            }
            let n: u64 = index.blocks[first - 1..last]
                .iter()
                .map(|b| b.samples)
                .sum();
            let expected = i128::from(n) * 1_000_000;
            let actual = (end - begin)
                .checked_mul(i128::from(index.rate))
                .ok_or_else(|| Error::Invalid("epoch time overflow".into()))?;
            // Allow microsecond serialization rounding, but never an entire missing sample.
            if actual.abs_diff(expected) > u128::from(index.rate) {
                return Err(Error::Unsupported(
                    "epoch duration does not match signal samples".into(),
                ));
            }
            let start = begin
                .checked_mul(i128::from(index.rate))
                .and_then(|v| v.checked_add(500_000))
                .ok_or_else(|| Error::Invalid("epoch time overflow".into()))?
                / 1_000_000;
            let start =
                u64::try_from(start).map_err(|_| Error::Invalid("epoch sample overflow".into()))?;
            let previous_sample_end = if first > 1 {
                let b = &index.blocks[first - 2];
                b.start + b.samples
            } else {
                0
            };
            if begin < previous_end || start < previous_sample_end {
                return Err(Error::Unsupported("overlapping epoch timeline".into()));
            }
            let mut position = start;
            for block in &mut index.blocks[first - 1..last] {
                block.start = position;
                position = position
                    .checked_add(block.samples)
                    .ok_or_else(|| Error::Invalid("epoch sample overflow".into()))?;
            }
            index.samples = position;
            previous_end = end;
            next_block = last + 1;
        }
        if epochs.is_empty() || next_block != index.blocks.len() + 1 {
            return Err(Error::Invalid("epochs do not cover signal blocks".into()));
        }
    } else {
        warnings.push("epochs.xml absent; continuous block timeline assumed".into());
    }
    if kind == RecordingKind::Discontinuous {
        warnings.push(
            "Recording pauses are preserved as NaN sample gaps on the acquisition timeline.".into(),
        );
    }
    Ok(kind)
}

fn calibrate(doc: &Document<'_>, channels: &mut [Channel]) -> Result<()> {
    let mut gcal_seen = false;
    for cal in doc.descendants().filter(|n| {
        named(*n, "calibration")
            && child(*n, "type").is_some_and(|s| s.eq_ignore_ascii_case("GCAL"))
    }) {
        if gcal_seen {
            return Err(Error::Unsupported("multiple GCAL calibrations".into()));
        }
        gcal_seen = true;
        if child(cal, "beginTime").is_some_and(|s| s != "0")
            || child(cal, "endTime").is_some_and(|s| s != "0")
        {
            return Err(Error::Unsupported("time-varying GCAL".into()));
        }
        let mut seen = std::collections::HashSet::new();
        for (seq, n) in cal.descendants().filter(|n| named(*n, "ch")).enumerate() {
            let num: usize = match n
                .attribute("n")
                .or(n.attribute("number"))
                .or(n.attribute("channel"))
            {
                Some(v) => number(v, "calibration channel")?,
                None => seq + 1,
            };
            if num == 0 || num > channels.len() || !seen.insert(num) {
                return Err(Error::Invalid("invalid/duplicate GCAL channel".into()));
            }
            let factor: f32 = number(n.text().unwrap_or("").trim(), "GCAL")?;
            if !factor.is_finite() {
                return Err(Error::Invalid("nonfinite GCAL".into()));
            }
            channels[num - 1].calibration = factor;
        }
    }
    Ok(())
}

// Match Swift's first usable acquisition-start ICAL measurement. ICAL is
// contact impedance, not gain. Absent/nonfinite entries stay unknown.
fn read_impedance(doc: &Document<'_>, channels: &mut [Channel]) {
    for cal in doc.descendants().filter(|n| {
        named(*n, "calibration")
            && child(*n, "type").is_some_and(|s| s.eq_ignore_ascii_case("ICAL"))
    }) {
        if child(cal, "beginTime").is_some_and(|s| {
            s.parse::<f64>()
                .map_or(true, |v| !v.is_finite() || v.abs() > 0.000001)
        }) {
            continue;
        }
        let mut any = false;
        for (seq, node) in cal.descendants().filter(|n| named(*n, "ch")).enumerate() {
            let number = node
                .attribute("n")
                .or(node.attribute("number"))
                .or(node.attribute("channel"))
                .map_or(Some(seq + 1), |s| s.trim().parse::<usize>().ok());
            let value = node.text().unwrap_or("").trim().parse::<f32>().ok();
            if let (Some(n), Some(v)) = (number, value) {
                if n == 0 || n > channels.len() || !v.is_finite() || v < 0.0 {
                    continue;
                }
                channels[n - 1].impedance_kohm = Some(v);
                any = true;
            }
        }
        if any {
            break;
        }
    }
}

fn open_pns(
    path: &Path,
    primary: &Metadata,
    limits: &Limits,
) -> Result<Option<std::sync::Arc<Recording>>> {
    let streams: Vec<_> = primary
        .streams
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind.eq_ignore_ascii_case("PNSData"))
        .collect();
    if streams.is_empty() {
        return Ok(None);
    }
    if streams.len() != 1 {
        return Err(Error::Unsupported("multiple PNS streams".into()));
    }
    let (stream, descriptor) = streams[0];
    let signal_path = path.join(&descriptor.file_name);
    let mut index = binary::index(&signal_path, limits)?;
    if primary.kind == RecordingKind::Discontinuous {
        apply_epochs(path, &mut index, limits, &mut Vec::new())?;
    }
    let mut channels: Vec<_> = (0..index.channels)
        .map(|i| Channel {
            position: None,
            index: i,
            label: format!("PNS {}", i + 1),
            unit: None,
            calibration: 1.,
            positive_up: None,
            impedance_kohm: None,
        })
        .collect();
    let id = descriptor
        .file_name
        .trim_start_matches("signal")
        .trim_end_matches(".bin");
    let text = xml(&path.join(format!("info{id}.xml")), limits)?;
    let doc = parse(&text)?;
    calibrate(&doc, &mut channels)?;
    let mut warnings = vec![];
    let pns_path = path.join("pnsSet.xml");
    if pns_path.exists() {
        let text = xml(&pns_path, limits)?;
        let doc = parse(&text)?;
        let mut seen = std::collections::HashSet::new();
        for sensor in doc.descendants().filter(|n| named(*n, "sensor")) {
            let i: usize = number(
                child(sensor, "number")
                    .ok_or_else(|| Error::Invalid("PNS sensor number missing".into()))?,
                "PNS sensor number",
            )?;
            if i >= channels.len() || !seen.insert(i) {
                return Err(Error::Invalid("invalid/duplicate PNS sensor number".into()));
            }
            if let Some(label) = child(sensor, "name") {
                channels[i].label = label.to_owned();
            }
            channels[i].unit = child(sensor, "unit").map(str::to_owned);
            channels[i].positive_up = child(sensor, "positiveUp")
                .map(|v| match v {
                    "true" | "1" => Ok(true),
                    "false" | "0" => Ok(false),
                    _ => Err(Error::Invalid("PNS positiveUp boolean".into())),
                })
                .transpose()?;
        }
    } else {
        warnings.push("No pnsSet.xml: channel labels and units unspecified".into());
    }
    let adjustment = doc
        .descendants()
        .find(|n| named(*n, "hardwareFilterAdjusted"));
    let hardware_filter_adjusted = adjustment
        .map(|n| match n.text().unwrap_or("").trim() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(Error::Invalid("PNS hardwareFilterAdjusted boolean".into())),
        })
        .transpose()?;
    let hardware_filter_shift_microseconds = adjustment
        .and_then(|n| n.attribute("shiftMicroseconds"))
        .map(|v| number(v, "PNS shiftMicroseconds"))
        .transpose()?;
    if adjustment.is_some() {
        warnings
            .push("PNS acquisition timing metadata preserved; no additional shift applied".into());
    }
    let metadata = Metadata {
        sensor_layout_name: primary.sensor_layout_name.clone(),
        streams: primary.streams.clone(),
        eeg_stream: stream,
        channels,
        sample_rate: index.rate,
        sample_count: index.samples,
        recording_start: primary.recording_start.clone(),
        acquisition_reference: None,
        acquisition_version: primary.acquisition_version.clone(),
        hardware_filter_adjusted,
        hardware_filter_shift_microseconds,
        kind: primary.kind,
        events: primary.events.clone(),
        warnings,
    };
    Ok(Some(std::sync::Arc::new(Recording {
        metadata,
        index,
        signal_path,
        limits: limits.clone(),
        pns: None,
    })))
}
