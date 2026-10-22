use eva_mff::{Error, Limits, Recording, RecordingKind, SampleValues};
use std::{fs, path::Path};
use tempfile::TempDir;

fn u32s(out: &mut Vec<u8>, words: &[u32]) {
    for w in words {
        out.extend(w.to_le_bytes());
    }
}
fn block(out: &mut Vec<u8>, rows: &[&[f32]], rate: u32, reuse: bool) {
    if reuse {
        u32s(out, &[0]);
    } else {
        let n = rows.len() as u32;
        let bytes = rows[0].len() as u32 * 4;
        u32s(out, &[1, 16 + 8 * n, n * bytes, n]);
        for c in 0..n {
            u32s(out, &[c * bytes]);
        }
        for _ in 0..n {
            u32s(out, &[(rate << 8) | 32]);
        }
    }
    for row in rows {
        for v in *row {
            out.extend(v.to_le_bytes());
        }
    }
}
fn fixture() -> TempDir {
    let d = TempDir::new().unwrap();
    fs::write(
        d.path().join("info.xml"),
        "<fileInfo><recordTime>2020-01-01T00:00:00.000372500+00:00</recordTime></fileInfo>",
    )
    .unwrap();
    fs::write(d.path().join("info7.xml"), "<dataInfo xmlns='http://www.egi.com/info_n_mff'><generalInformation><fileDataType><EEG/></fileDataType></generalInformation><calibrations><calibration><type>GCAL</type><channels><ch n='1'>2</ch><ch n='2'>0.5</ch></channels></calibration></calibrations></dataInfo>").unwrap();
    fs::write(d.path().join("sensorLayout.xml"), "<sensorLayout><sensors><sensor><number>1</number><type>0</type><name>A</name><unit>uV</unit></sensor><sensor><number>2</number><type>0</type><name>B</name></sensor><sensor><number>3</number><type>1</type><name>VREF</name></sensor></sensors></sensorLayout>").unwrap();
    let mut b = Vec::new();
    block(&mut b, &[&[1., -2., 3.], &[10., 20., 30.]], 1024, false);
    block(&mut b, &[&[4., 5., 6.], &[40., 50., 60.]], 1024, true);
    block(&mut b, &[&[7., 8.], &[70., 80.]], 1024, false);
    fs::write(d.path().join("signal7.bin"), b).unwrap();
    d
}
fn patch(d: &TempDir, offset: usize, value: u32) {
    let p = d.path().join("signal7.bin");
    let mut b = fs::read(&p).unwrap();
    b[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    fs::write(p, b).unwrap();
}
#[test]
fn windows_calibration_order_and_reference() {
    let d = fixture();
    let r = Recording::open(d.path()).unwrap();
    assert_eq!(r.metadata().sample_count, 8);
    assert_eq!(r.metadata().sample_rate, 1024);
    assert_eq!(r.metadata().channels[0].unit.as_deref(), Some("uV"));
    assert_eq!(
        r.metadata()
            .acquisition_reference
            .as_ref()
            .unwrap()
            .recorded_channel,
        None
    );
    let w = r.read_window(&[1, 0], 2..7, SampleValues::Raw).unwrap();
    assert_eq!(
        w.values,
        vec![vec![30., 40., 50., 60., 70.], vec![3., 4., 5., 6., 7.]]
    );
    assert_eq!(
        r.read_window(&[0, 1], 0..1, SampleValues::Calibrated)
            .unwrap()
            .values,
        vec![vec![2.], vec![5.]]
    );
    assert_eq!(
        r.read_window(&[0], 7..8, SampleValues::Raw).unwrap().values[0],
        vec![8.]
    );
    assert!(r.read_window(&[0], 8..8, SampleValues::Raw).unwrap().values[0].is_empty());
    assert!(matches!(
        r.read_window(&[2], 0..1, SampleValues::Raw),
        Err(Error::InvalidSelection(_))
    ));
    assert!(matches!(
        r.read_window(&[0], 0..9, SampleValues::Raw),
        Err(Error::InvalidSelection(_))
    ));
}
#[test]
fn resource_limits_and_source_change() {
    let d = fixture();
    let r = Recording::open_with_limits(
        d.path(),
        Limits {
            max_window_bytes: 32,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        r.read_window(&[0, 1], 0..8, SampleValues::Raw),
        Err(Error::Limit(_))
    ));
    assert!(matches!(
        Recording::open_with_limits(
            d.path(),
            Limits {
                max_blocks: 1,
                ..Limits::default()
            }
        ),
        Err(Error::Limit(_))
    ));
    assert!(matches!(
        Recording::open_with_limits(
            d.path(),
            Limits {
                max_xml_bytes: 10,
                ..Limits::default()
            }
        ),
        Err(Error::Limit(_))
    ));
    assert!(matches!(
        Recording::open_with_limits(
            d.path(),
            Limits {
                max_index_bytes: 1,
                ..Limits::default()
            }
        ),
        Err(Error::Limit(_))
    ));
    fs::write(d.path().join("signal7.bin"), [0]).unwrap();
    assert!(r.read_window(&[0], 0..1, SampleValues::Raw).is_err());
}
#[test]
fn rejects_bad_headers() {
    for (offset, value) in [
        (0, 0),
        (0, 42),
        (4, 1),
        (8, u32::MAX),
        (12, u32::MAX),
        (16, 4),
        (20, 0),
        (24, (1024 << 8) | 16),
        (28, (512 << 8) | 32),
    ] {
        let d = fixture();
        patch(&d, offset, value);
        assert!(
            Recording::open(d.path()).is_err(),
            "offset {offset} value {value}"
        );
    }
    let d = fixture();
    let p = d.path().join("signal7.bin");
    let mut b = fs::read(&p).unwrap();
    b.pop();
    fs::write(p, b).unwrap();
    assert!(Recording::open(d.path()).is_err());
}
#[test]
fn changing_configuration_and_missing_metadata() {
    let d = fixture();
    patch(&d, 56 + 28 + 24, (512 << 8) | 32); // third header starts after first and reused blocks
    assert!(Recording::open(d.path()).is_err());
    let d = fixture();
    fs::remove_file(d.path().join("info7.xml")).unwrap();
    assert!(Recording::open(d.path()).is_err());
    let d = fixture();
    fs::write(d.path().join("info.xml"), "<broken>").unwrap();
    assert!(Recording::open(d.path()).is_err());
}
#[test]
fn event_nanoseconds_timezone_rounding_and_duplicates() {
    let d = fixture();
    fs::write(d.path().join("Events_test.xml"), "<eventTrack><event><code>A</code><beginTime>2019-12-31T19:00:00.001349062-05:00</beginTime><relativeBeginTime>999999</relativeBeginTime><duration>1000</duration></event><event><code>A</code><beginTime>2020-01-01T00:00:00.001349062Z</beginTime></event></eventTrack>").unwrap();
    let r = Recording::open(d.path()).unwrap();
    let e = &r.metadata().events[0];
    assert_eq!(e.onset_ns, 976562);
    assert_eq!(e.sample_index(1024).unwrap(), 1);
    assert_eq!(e.duration_ns, Some(1_000_000));
    assert_eq!(r.metadata().events.len(), 2);
    for rate in [512, 1024, 2048, 20000, 1000, 500] {
        for sample in [0, 1, 7, 1023, 3071] {
            let mut event = e.clone();
            event.onset_ns =
                (i128::from(sample) * 1_000_000_000 + i128::from(rate) / 2) / i128::from(rate);
            assert_eq!(event.sample_index(rate).unwrap(), sample);
        }
    }
    let mut e = e.clone();
    e.onset_ns = -500_000;
    assert_eq!(e.sample_index(1000).unwrap(), -1);
}
#[test]
fn discontinuous_reads_preserve_leading_gap_and_reject_categories() {
    let d = fixture();
    fs::write(d.path().join("epochs.xml"),"<epochs><epoch><beginTime>1000000</beginTime><endTime>1007813</endTime><firstBlock>1</firstBlock><lastBlock>3</lastBlock></epoch></epochs>").unwrap();
    let r = Recording::open(d.path()).unwrap();
    assert_eq!(r.metadata().kind, RecordingKind::Discontinuous);
    assert!(r
        .read_window(&[0], 0..1024, SampleValues::Raw)
        .unwrap()
        .values[0]
        .iter()
        .all(|v| v.is_nan()));
    assert_eq!(r.metadata().sample_count, 1032);
    assert_eq!(r.gap_annotations()[0].range, 0..1024);
    assert!(r.overlaps_gap(0..1).unwrap());
    assert!(r
        .read_epoch(&[0], 1023..1025, SampleValues::Raw)
        .unwrap()
        .is_none());
    assert!(r
        .read_epoch(&[0], 1024..1032, SampleValues::Raw)
        .unwrap()
        .is_some());
    assert_eq!(
        r.read_window(&[0], 1024..1032, SampleValues::Raw)
            .unwrap()
            .values[0],
        vec![1., -2., 3., 4., 5., 6., 7., 8.]
    );
    fs::write(
        d.path().join("categories.xml"),
        "<categories><category/></categories>",
    )
    .unwrap();
    assert_eq!(
        Recording::open(d.path()).unwrap().metadata().kind,
        RecordingKind::SegmentedOrAveraged
    );
}
#[test]
fn real_eva_fixtures() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let r = Recording::open(base.join("example_3.mff")).unwrap();
    assert_eq!(r.metadata().sample_rate, 250);
    assert_eq!(r.metadata().channels.len(), 257);
    assert_eq!(r.metadata().sample_count, 224);
    assert_eq!(r.metadata().streams.len(), 2);
    assert_eq!(r.metadata().kind, RecordingKind::Continuous);
    // Independently decode the source's first sample to verify the indexed read.
    let bytes = fs::read(base.join("example_3.mff/signal1.bin")).unwrap();
    let header = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let expected = f32::from_le_bytes(bytes[header..header + 4].try_into().unwrap());
    assert_eq!(
        r.read_window(&[0], 0..1, SampleValues::Raw).unwrap().values[0][0],
        expected
    );
    let r = Recording::open(base.join("example_4.mff")).unwrap();
    assert_eq!(r.metadata().kind, RecordingKind::SegmentedOrAveraged);
    assert!(matches!(
        r.read_window(&[0], 0..1, SampleValues::Raw),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(r.metadata().events.len(), 3);
    assert_eq!(r.metadata().events[0].onset_ns, 10_000_000);
}

#[test]
fn every_window_matches_known_signal() {
    let d = fixture();
    let r = Recording::open(d.path()).unwrap();
    let expected = [1., -2., 3., 4., 5., 6., 7., 8.];
    for start in 0..=8 {
        for end in start..=8 {
            let w = r.read_window(&[0], start..end, SampleValues::Raw).unwrap();
            assert_eq!(w.values[0], expected[start as usize..end as usize]);
        }
    }
}

#[test]
fn sparse_gigabyte_signal_reads_with_small_budget() {
    use std::io::Write;
    let d = fixture();
    let mut header = Vec::new();
    u32s(&mut header, &[1, 24, 1 << 30, 1, 0, (1000 << 8) | 32]);
    let mut f = fs::File::create(d.path().join("signal7.bin")).unwrap();
    f.write_all(&header).unwrap();
    f.set_len(24 + (1 << 30)).unwrap();
    fs::write(
        d.path().join("info7.xml"),
        "<dataInfo><fileDataType><EEG/></fileDataType></dataInfo>",
    )
    .unwrap();
    let r = Recording::open_with_limits(
        d.path(),
        Limits {
            max_window_bytes: 1024,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(r.metadata().sample_count, 1 << 28);
    let end = r.metadata().sample_count;
    assert_eq!(
        r.read_window(&[0], end - 2..end, SampleValues::Raw)
            .unwrap()
            .values[0],
        vec![0., 0.]
    );
}

#[test]
fn real_fixture_independent_bit_patterns() {
    let r =
        Recording::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/example_3.mff"))
            .unwrap();
    // Constants independently extracted with Python struct from the source fixture.
    for (channel, sample, bits) in [(0, 0, 0xc4e6b3d7), (1, 111, 0xc4ad9c81), (256, 223, 0)] {
        assert_eq!(
            r.read_window(&[channel], sample..sample + 1, SampleValues::Raw)
                .unwrap()
                .values[0][0]
                .to_bits(),
            bits
        );
    }
}

#[test]
fn invalid_epochs_and_event_metadata_are_not_silently_ignored() {
    let d = fixture();
    fs::write(d.path().join("epochs.xml"), "<epochs><epoch><beginTime>0</beginTime><endTime>100</endTime><firstBlock>0</firstBlock><lastBlock>3</lastBlock></epoch></epochs>").unwrap();
    assert!(Recording::open(d.path()).is_err());
    fs::remove_file(d.path().join("epochs.xml")).unwrap();
    fs::write(
        d.path().join("Events_bad.xml"),
        "<eventTrack><event><code>X</code><beginTime>bad</beginTime></event></eventTrack>",
    )
    .unwrap();
    assert!(Recording::open(d.path()).is_err());
}

#[test]
fn pns_independent_rate_metadata_calibration_and_limits() {
    let d = fixture();
    let mut bytes = Vec::new();
    block(&mut bytes, &[&[2., 4., 6.], &[10., 20., 30.]], 250, false);
    block(&mut bytes, &[&[8., 10., 12.], &[40., 50., 60.]], 250, true);
    fs::write(d.path().join("signal2.bin"), bytes).unwrap();
    fs::write(d.path().join("info2.xml"),"<dataInfo><fileDataType><PNSData/></fileDataType><calibration><type>GCAL</type><ch n='1'>0.5</ch><ch n='2'>2</ch></calibration><hardwareFilterAdjusted shiftMicroseconds='80000'>true</hardwareFilterAdjusted></dataInfo>").unwrap();
    fs::write(d.path().join("pnsSet.xml"),"<PNSSet><sensor><number>1</number><name>EMG</name><unit>mV</unit><positiveUp>true</positiveUp></sensor><sensor><number>0</number><name>ECG</name><unit>uV</unit><positiveUp>false</positiveUp></sensor></PNSSet>").unwrap();
    let r = Recording::open(d.path()).unwrap();
    let p = r.pns().unwrap();
    assert!(!r.is_pns());
    assert!(p.is_pns());
    assert!(p.pns().is_none());
    assert_eq!(r.metadata().sample_rate, 1024);
    assert_eq!(p.metadata().sample_rate, 250);
    assert_eq!(p.metadata().sample_count, 6);
    assert_eq!(p.metadata().channels[0].label, "ECG");
    assert_eq!(p.metadata().channels[0].positive_up, Some(false));
    assert_eq!(p.metadata().channels[1].unit.as_deref(), Some("mV"));
    assert_eq!(p.metadata().hardware_filter_shift_microseconds, Some(80000));
    assert_eq!(
        p.read_window(&[1, 0], 2..5, SampleValues::Calibrated)
            .unwrap()
            .values,
        vec![vec![60., 80., 100.], vec![3., 4., 5.]]
    );
    assert_eq!(
        p.read_window(&[0], 0..1, SampleValues::Raw).unwrap().values[0],
        vec![2.]
    );
    assert!(p.read_window(&[2], 0..1, SampleValues::Raw).is_err());
    assert!(p.read_window(&[0], 0..7, SampleValues::Raw).is_err());
    let limited = Recording::open_with_limits(
        d.path(),
        Limits {
            max_window_bytes: 32,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(matches!(
        limited
            .pns()
            .unwrap()
            .read_window(&[0, 1], 0..6, SampleValues::Raw),
        Err(Error::Limit(_))
    ));
    fs::write(
        d.path().join("pnsSet.xml"),
        "<PNSSet><sensor><number>0</number></sensor><sensor><number>0</number></sensor></PNSSet>",
    )
    .unwrap();
    assert!(Recording::open(d.path()).is_err());
}
#[test]
fn real_pns_fixture_matches_binary_values() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/example_3.mff");
    let r = Recording::open(&path).unwrap();
    let p = r.pns().unwrap();
    assert_eq!(p.metadata().channels.len(), 2);
    assert_eq!(p.metadata().channels[0].label, "ECG");
    assert_eq!(p.metadata().channels[1].label, "EMG");
    assert!(p
        .metadata()
        .channels
        .iter()
        .all(|c| c.positive_up == Some(false)));
    let bytes = fs::read(path.join("signal2.bin")).unwrap();
    let header = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let offset = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let w = p.read_window(&[0, 1], 0..16, SampleValues::Raw).unwrap();
    for i in 0..16 {
        for (ch, start) in [(0, header), (1, header + offset)] {
            assert_eq!(
                w.values[ch][i].to_bits(),
                u32::from_le_bytes(bytes[start + i * 4..start + i * 4 + 4].try_into().unwrap())
            );
        }
    }
    assert!((p.metadata().duration_seconds() - r.metadata().duration_seconds()).abs() < 0.05);
    assert!(Recording::open(fixture().path()).unwrap().pns().is_none());
}

#[test]
fn discontinuous_internal_gap_preserves_samples_and_event_clock() {
    let d = fixture();
    fs::write(d.path().join("epochs.xml"), "<epochs><epoch><beginTime>0</beginTime><endTime>2930</endTime><firstBlock>1</firstBlock><lastBlock>1</lastBlock></epoch><epoch><beginTime>1000000</beginTime><endTime>1004883</endTime><firstBlock>2</firstBlock><lastBlock>3</lastBlock></epoch></epochs>").unwrap();
    let r = Recording::open(d.path()).unwrap();
    assert_eq!(r.metadata().sample_count, 1029);
    let w = r.read_window(&[0], 0..1029, SampleValues::Raw).unwrap();
    assert_eq!(&w.values[0][..3], &[1., -2., 3.]);
    assert!(w.values[0][3..1024].iter().all(|v| v.is_nan()));
    assert_eq!(&w.values[0][1024..], &[4., 5., 6., 7., 8.]);
    assert_eq!(
        r.read_window(&[0], 1023..1025, SampleValues::Calibrated)
            .unwrap()
            .values[0][1],
        8.
    );
}

#[test]
fn scalp_coordinates_normalize_and_exclude_reference() {
    let d = fixture();
    fs::write(d.path().join("coordinates.xml"),"<coordinates><sensor><number>1</number><type>0</type><x>3</x><y>4</y><z>0</z></sensor><sensor><number>2</number><type>1</type><x>0</x><y>0</y><z>1</z></sensor></coordinates>").unwrap();
    let r = Recording::open(d.path()).unwrap();
    assert_eq!(r.metadata().channels[0].position, Some([0.6, 0.8, 0.]));
    assert_eq!(r.metadata().channels[1].position, None);
    fs::write(d.path().join("coordinates.xml"),"<coordinates><sensor><number>1</number><type>0</type><x>NaN</x><y>0</y><z>1</z></sensor></coordinates>").unwrap();
    let r = Recording::open(d.path()).unwrap();
    assert_eq!(r.metadata().channels[0].position, None);
    assert!(r
        .metadata()
        .warnings
        .iter()
        .any(|w| w.contains("coordinates")));
}

#[test]
fn epoch_reads_reject_gaps_with_exact_half_open_boundaries() {
    let d = fixture();
    fs::write(d.path().join("epochs.xml"), "<epochs><epoch><beginTime>0</beginTime><endTime>2930</endTime><firstBlock>1</firstBlock><lastBlock>1</lastBlock></epoch><epoch><beginTime>1000000</beginTime><endTime>1004883</endTime><firstBlock>2</firstBlock><lastBlock>3</lastBlock></epoch></epochs>").unwrap();
    let r = Recording::open(d.path()).unwrap();
    for range in [0..3, 1024..1029] {
        assert!(!r.overlaps_gap(range.clone()).unwrap());
        assert!(r
            .read_epoch(&[0, 1], range, SampleValues::Raw)
            .unwrap()
            .is_some());
    }
    for range in [2..4, 3..1024, 1023..1025, 0..1029] {
        assert!(r.overlaps_gap(range.clone()).unwrap());
        assert!(r
            .read_epoch(&[0], range, SampleValues::Raw)
            .unwrap()
            .is_none());
    }
    assert!(!r.overlaps_gap(3..3).unwrap());
    assert!(r.overlaps_gap(0..1030).is_err());
    assert!(r.read_epoch(&[], 0..1, SampleValues::Raw).is_err());
    assert!(r.read_epoch(&[0], 3..3, SampleValues::Raw).is_err());
    let gaps = r.gap_annotations();
    assert_eq!(gaps[0].range, 3..1024);
    assert_eq!(gaps[0].onset_seconds(), 3. / 1024.);
    assert_eq!(gaps[0].duration_seconds(), 1021. / 1024.);
    assert!(!r
        .metadata()
        .events
        .iter()
        .any(|e| e.code == eva_mff::AcquisitionGap::CODE));
}

#[test]
fn acquired_nan_is_not_an_acquisition_gap_but_rejects_epoch() {
    let d = fixture();
    let path = d.path().join("signal7.bin");
    let mut data = fs::read(&path).unwrap();
    let header = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    data[header..header + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    fs::write(&path, data).unwrap();
    let r = Recording::open(d.path()).unwrap();
    assert!(!r.overlaps_gap(0..8).unwrap());
    assert!(r.gap_annotations().is_empty());
    assert!(r
        .read_epoch(&[0], 0..3, SampleValues::Raw)
        .unwrap()
        .is_none());
    assert!(r
        .read_epoch(&[1], 0..3, SampleValues::Raw)
        .unwrap()
        .is_some());
    assert!(r
        .read_epoch(&[0], 1..8, SampleValues::Raw)
        .unwrap()
        .is_some());
}

#[test]
fn eeg_units_and_impedance_do_not_change_gain() {
    let d = fixture();
    let info = d.path().join("info7.xml");
    let text = fs::read_to_string(&info).unwrap().replace("</calibrations>",
        "<calibration><type>ICAL</type><beginTime>100</beginTime><ch>999</ch></calibration><calibration><type>ical</type><beginTime>0</beginTime><ch number='2'>42.5</ch><ch channel='1'>NaN</ch><ch n='99'>10</ch></calibration></calibrations>");
    fs::write(info, text).unwrap();
    let r = Recording::open(d.path()).unwrap();
    assert_eq!(r.metadata().channels[1].unit.as_deref(), Some("uV"));
    assert_eq!(r.metadata().channels[0].impedance_kohm, None);
    assert_eq!(r.metadata().channels[1].impedance_kohm, Some(42.5));
    assert_eq!(
        r.read_window(&[0, 1], 0..1, SampleValues::Calibrated)
            .unwrap()
            .values,
        vec![vec![2.], vec![5.]]
    );
    let layout = d.path().join("sensorLayout.xml");
    fs::write(
        &layout,
        fs::read_to_string(&layout)
            .unwrap()
            .replace("<unit>uV</unit>", "<unit>mV</unit>"),
    )
    .unwrap();
    assert_eq!(
        Recording::open(d.path()).unwrap().metadata().channels[0]
            .unit
            .as_deref(),
        Some("mV")
    );
}
