use super::*;
fn detect_test(x: &[f32], c: &Config) -> Vec<Candidate> {
    detect(x, 250, Kind::Blink, c, &AtomicU64::new(1), 1).unwrap()
}
#[test]
fn swift_merged_peak_duration_polarity_and_kinematics() {
    let mut x = vec![0.; 1000];
    x[100..115].fill(200.);
    x[130..145].fill(350.);
    let mut c = Config::defaults(Kind::Blink);
    let r = detect_test(&x, &c);
    assert_eq!(
        (r[0].start_sample, r[0].end_sample, r[0].peak_sample),
        (100, 145, 130)
    );
    c.maximum_uv = 300.;
    assert!(detect_test(&x, &c).is_empty());
    c.maximum_uv = 0.;
    c.rise_seconds = 0.1;
    assert!(detect_test(&x, &c).is_empty());
    c.rise_seconds = 0.;
    c.polarity = Polarity::Negative;
    assert!(detect_test(&x, &c).is_empty());
    c.polarity = Polarity::Bipolar;
    c.velocity_enabled = true;
    c.velocity_uv_ms = 88.;
    assert!(detect_test(&x, &c).is_empty());
    c.velocity_uv_ms = 87.5;
    assert_eq!(detect_test(&x, &c).len(), 1);
    c.acceleration_enabled = true;
    c.acceleration_uv_ms2 = 22.;
    assert!(detect_test(&x, &c).is_empty());
    c.acceleration_uv_ms2 = 21.875;
    assert_eq!(detect_test(&x, &c).len(), 1);
    c.max_seconds = 0.1;
    assert_eq!(detect_test(&x, &c).len(), 1); // split runs; only 350 µV passes velocity
}
#[test]
fn gaps_are_not_merged_and_fragments_merge_before_duration_gate() {
    let mut c = Config::defaults(Kind::Blink);
    c.min_seconds = 0.1;
    let mut x = vec![0.; 200];
    x[20..35].fill(200.);
    x[40..55].fill(200.);
    assert_eq!(detect_test(&x, &c).len(), 1);
    x[37] = f32::NAN;
    assert!(detect_test(&x, &c).is_empty());
    c.min_seconds = 0.05;
    assert_eq!(detect_test(&x, &c).len(), 2);
    assert!(detect(&x, 250, Kind::Blink, &c, &AtomicU64::new(2), 1).is_err());
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(
        p.join("info.xml"),
        "<fileInfo><recordTime>2026-01-01T00:00:00Z</recordTime></fileInfo>",
    )
    .unwrap();
    std::fs::write(p.join("info1.xml"),"<dataInfo><fileDataType><EEG/></fileDataType><calibration><type>GCAL</type><ch n='1'>2</ch><ch n='2'>2</ch></calibration></dataInfo>").unwrap();
    std::fs::write(p.join("sensorLayout.xml"),"<sensorLayout><name>Test montage</name><sensor><number>1</number><type>0</type><unit>mV</unit></sensor><sensor><number>2</number><type>0</type><unit>mV</unit></sensor></sensorLayout>").unwrap();
    let mut bytes = vec![];
    for word in [1u32, 32, 8000, 2, 0, 4000, (250 << 8) | 32, (250 << 8) | 32] {
        bytes.extend(word.to_le_bytes());
    }
    for ch in 0..2 {
        for i in 0..1000 {
            let uv: f32 = match i {
                100..115 => 200.,
                130..145 => 350.,
                500..550 => {
                    if ch == 0 {
                        200.
                    } else {
                        -200.
                    }
                }
                _ => 0.,
            };
            bytes.extend((uv / 2000.).to_le_bytes());
        }
    }
    std::fs::write(p.join("signal1.bin"), bytes).unwrap();
    dir
}
#[test]
fn pipeline_matches_swift_reference_with_units_calibration_and_report() {
    let dir = fixture();
    let r = Recording::open(dir.path()).unwrap();
    assert_eq!(
        r.metadata().sensor_layout_name.as_deref(),
        Some("Test montage")
    );
    let mut settings = Settings::default();
    settings.blink.channels = vec![1, 2];
    settings.movement.channels = vec![1, 2];
    for topology in [Topology::Legacy, Topology::Derived] {
        settings.blink.topology = topology;
        settings.movement.topology = topology;
        let report = run(
            &r,
            &settings,
            &filter::Settings::default(),
            None,
            &AtomicU64::new(1),
            1,
            &|_| {},
        )
        .unwrap();
        let blink: Vec<_> = report
            .candidates
            .iter()
            .filter(|c| c.kind == Kind::Blink)
            .collect();
        let movement: Vec<_> = report
            .candidates
            .iter()
            .filter(|c| c.kind == Kind::Movement)
            .collect();
        assert_eq!(
            blink.len(),
            if topology == Topology::Legacy { 2 } else { 1 }
        );
        assert_eq!(
            movement.len(),
            if topology == Topology::Legacy { 2 } else { 1 }
        );
        assert_eq!(
            (
                blink[0].start_seconds,
                blink[0].end_seconds,
                blink[0].peak_seconds,
                blink[0].peak_uv
            ),
            (0.4, 0.58, 0.52, 350.)
        );
        let last = movement.last().unwrap();
        assert_eq!(
            (last.start_seconds, last.end_seconds, last.peak_seconds),
            (2., 2.2, 2.)
        );
        assert_eq!(
            last.peak_uv,
            if topology == Topology::Legacy {
                200.
            } else {
                400.
            }
        );
        let path = dir.path().join(format!("{topology:?}.json"));
        write_report(&report, &path).unwrap();
        assert!(write_report(&report, &path).is_err());
    }
    settings.blink.channels = vec![99];
    assert!(run(
        &r,
        &settings,
        &filter::Settings::default(),
        None,
        &AtomicU64::new(1),
        1,
        &|_| {}
    )
    .is_err());
}
#[test]
fn swift_montage_table_and_cli_validation() {
    for (name, n, blink, movement) in [
        ("HydroCel GSN 32", 33, vec![2, 30, 1, 29], vec![31, 32]),
        ("HydroCel GSN 64", 65, vec![10, 63, 5, 62], vec![61, 64]),
        (
            "HydroCel GSN 128",
            129,
            vec![25, 127, 21, 8, 126, 14],
            vec![125, 128],
        ),
        (
            "HydroCel GSN 256",
            257,
            vec![37, 241, 32, 18, 238, 25],
            vec![226, 252],
        ),
        (
            "GSN 200 64 v1",
            65,
            vec![14, 64, 11, 1, 63, 6],
            vec![18, 60],
        ),
        (
            "GSN 200 64 v2",
            65,
            vec![14, 64, 11, 1, 63, 6],
            vec![19, 60],
        ),
        (
            "GSN 200 128",
            129,
            vec![26, 127, 33, 8, 126, 1],
            vec![125, 128],
        ),
        (
            "GSN 200 256",
            257,
            vec![36, 242, 45, 18, 241, 10],
            vec![227, 251],
        ),
    ] {
        for (kind, expected) in [(Kind::Blink, blink), (Kind::Movement, movement)] {
            let (mut l, r, known) = auto_groups(kind, n, Some(name));
            l.extend(r);
            assert!(known);
            assert_eq!(l, expected);
        }
    }
    assert!(cli::parse(&std::collections::BTreeMap::from([(
        "blink-minimum".into(),
        "NaN".into()
    )]))
    .is_err());
    assert!(cli::parse(&std::collections::BTreeMap::from([(
        "unknown".into(),
        "1".into()
    )]))
    .is_err());
    assert_eq!(cli::channels("3,1,2").unwrap(), vec![3, 1, 2]);
    assert!(cli::channels("0").is_err());
}

#[test]
fn snapshot_cannot_turn_acquisition_gap_into_detection() {
    let dir = fixture();
    std::fs::write(dir.path().join("epochs.xml"),"<epochs><epoch><beginTime>2000000</beginTime><endTime>6000000</endTime><firstBlock>1</firstBlock><lastBlock>1</lastBlock></epoch></epochs>").unwrap();
    let r = Recording::open(dir.path()).unwrap();
    assert_eq!(r.metadata().sample_count, 1500);
    let source = Arc::new(vec![vec![0.2; 1500]; 2]); // mV; intentionally includes nonmissing values in the gap
    let mut settings = Settings::default();
    settings.movement.enabled = false;
    settings.blink.max_seconds = 0.;
    let report = run(
        &r,
        &settings,
        &filter::Settings::default(),
        Some(source),
        &AtomicU64::new(1),
        1,
        &|_| {},
    )
    .unwrap();
    assert_eq!(report.candidates.len(), 1);
    assert_eq!(
        (
            report.candidates[0].start_sample,
            report.candidates[0].end_sample
        ),
        (500, 1500)
    );
}
