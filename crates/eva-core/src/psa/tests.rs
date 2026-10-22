use super::*;
use std::fs;
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(
        p.join("info.xml"),
        "<fileInfo><recordTime>2026-01-01T00:00:00Z</recordTime></fileInfo>",
    )
    .unwrap();
    fs::write(
        p.join("info1.xml"),
        "<dataInfo><fileDataType><EEG/></fileDataType></dataInfo>",
    )
    .unwrap();
    let mut sensors = String::new();
    let mut coords = String::new();
    for (ch, [x, y, z]) in [[1, 0, 0], [0, 1, 0], [0, 0, 1], [-1, 0, 0], [0, -1, 0]]
        .iter()
        .enumerate()
    {
        sensors += &format!(
            "<sensor><number>{}</number><type>0</type><name>E{}</name><unit>uV</unit></sensor>",
            ch + 1,
            ch + 1
        );
        coords += &format!(
            "<sensor><number>{}</number><type>0</type><x>{x}</x><y>{y}</y><z>{z}</z></sensor>",
            ch + 1
        );
    }
    fs::write(
        p.join("sensorLayout.xml"),
        format!("<sensorLayout><sensors>{sensors}</sensors></sensorLayout>"),
    )
    .unwrap();
    fs::write(
        p.join("coordinates.xml"),
        format!("<coordinates>{coords}</coordinates>"),
    )
    .unwrap();
    fs::write(p.join("Events_test.xml"),"<eventTrack><event><code>A</code><beginTime>2026-01-01T00:00:00.500Z</beginTime></event><event><code>A</code><beginTime>2026-01-01T00:00:01Z</beginTime></event><event><code>B</code><beginTime>2026-01-01T00:00:01.500Z</beginTime></event></eventTrack>").unwrap();
    write_signal(p, false, false);
    dir
}
fn write_signal(p: &std::path::Path, bad: bool, gap: bool) {
    let mut bytes = vec![];
    for block in 0..2 {
        for v in [
            1u32,
            56,
            2000,
            5,
            0,
            400,
            800,
            1200,
            1600,
            (100 << 8) | 32,
            (100 << 8) | 32,
            (100 << 8) | 32,
            (100 << 8) | 32,
            (100 << 8) | 32,
        ] {
            bytes.extend(v.to_le_bytes());
        }
        for ch in 0..5 {
            for s in 0..100 {
                let index = block * 100 + s;
                let pulse = if [50, 100, 150]
                    .iter()
                    .any(|a| index >= *a && index < *a + 10)
                {
                    10.
                } else {
                    0.
                };
                let v = if bad && ch == 0 && (45..55).contains(&index) {
                    500f32
                } else {
                    ch as f32 * 2. + pulse
                };
                bytes.extend(v.to_le_bytes());
            }
        }
    }
    fs::write(p.join("signal1.bin"), bytes).unwrap();
    if gap {
        fs::write(p.join("epochs.xml"),"<epochs><epoch><beginTime>0</beginTime><endTime>1000000</endTime><firstBlock>1</firstBlock><lastBlock>1</lastBlock></epoch><epoch><beginTime>1200000</beginTime><endTime>2200000</endTime><firstBlock>2</firstBlock><lastBlock>2</lastBlock></epoch></epochs>").unwrap();
    }
}
fn config() -> Config {
    Config {
        events: vec!["A".into(), "B".into()],
        pre: 0.1,
        post: 0.1,
        baseline_start: -0.1,
        average_reference: false,
        average: true,
        ..Config::default()
    }
}
fn run_test(r: &Recording, c: &Config) -> ResultData {
    run(
        r,
        c,
        &filter::Settings::default(),
        None,
        &AtomicU64::new(1),
        1,
        &|_| {},
    )
    .unwrap()
}
#[test]
fn baseline_categories_and_means_have_known_values() {
    let d = fixture();
    let r = Recording::open(d.path()).unwrap();
    let c = config();
    let result = run_test(&r, &c);
    assert_eq!(result.epochs.len(), 3);
    assert_eq!(result.samples, 20);
    assert_eq!(result.averages[0].count, 2);
    assert_eq!(&result.averages[0].values[2][..10], &[0.; 10]);
    assert_eq!(&result.averages[0].values[2][10..], &[10.; 10]);
    let c = Config {
        average_reference: true,
        ..c
    };
    let result = run_test(&r, &c);
    assert!(result
        .averages
        .iter()
        .flat_map(|a| a.values.iter().flatten())
        .all(|v| v.abs() < 1e-6));
}
#[test]
fn gap_rejection_is_unconditional_and_cannot_be_rescued() {
    let d = fixture();
    write_signal(d.path(), false, true);
    let r = Recording::open(d.path()).unwrap();
    let c = Config {
        threshold: false,
        escalate: false,
        baseline: false,
        ..config()
    };
    let result = run_test(&r, &c);
    assert_eq!(result.decisions[1].reason, "Acquisition gap");
    assert_eq!(result.epochs.len(), 2);
    assert!(result.decisions[1].repaired_channels.is_empty());
    let g = AtomicU64::new(2);
    assert!(
        run(&r, &c, &filter::Settings::default(), None, &g, 1, &|_| {})
            .unwrap_err()
            .contains("Cancelled")
    );
}
#[test]
fn transient_repair_rejection_and_global_escalation() {
    let d = fixture();
    write_signal(d.path(), true, false);
    let r = Recording::open(d.path()).unwrap();
    let c = Config {
        escalate: false,
        max_bad_count: Some(1),
        ..config()
    };
    let result = run_test(&r, &c);
    assert_eq!(result.decisions[0].flagged_channels, vec![1]);
    assert_eq!(result.decisions[0].repaired_channels, vec![1]);
    assert_eq!(result.epochs.len(), 3);
    // All independent donors share the same baseline-adjusted signal.
    assert!(result.epochs[0].values[0]
        .iter()
        .zip(&result.epochs[0].values[1])
        .all(|(a, b)| (a - b).abs() < 1e-4));
    let strict = Config {
        max_bad_count: Some(0),
        ..c.clone()
    };
    assert_eq!(
        run_test(&r, &strict).decisions[0].reason,
        "Too many bad channels"
    );
    let escalated = Config {
        escalate: true,
        escalation_percent: 30.,
        ..c
    };
    let result = run_test(&r, &escalated);
    assert_eq!(result.global_bad, vec![1]);
    assert!(result.decisions.iter().all(|d| d.repaired_channels == [1]));
}
#[test]
fn grouping_manual_exclusion_and_export_round_trip() {
    let d = fixture();
    let r = Recording::open(d.path()).unwrap();
    let mut c = config();
    c.categories.insert("A".into(), "combined".into());
    c.categories.insert("B".into(), "combined".into());
    c.reject_epochs = vec![2];
    let result = run_test(&r, &c);
    assert_eq!(result.averages.len(), 1);
    assert_eq!(result.averages[0].count, 2);
    assert_eq!(result.decisions[1].reason, "Manually excluded");
    let dest = tempfile::tempdir().unwrap();
    let path = dest.path().join("out.psa");
    export::write(&result, &path, "test.mff", Some("<evaProcessing/>")).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["epoch_candidates"], serde_json::json!([1, 3]));
    assert_eq!(
        fs::metadata(path.join("epochs.f32le")).unwrap().len(),
        2 * 5 * 20 * 4
    );
    assert!(export::write(&result, &path, "test.mff", None).is_err());
    assert_eq!(
        fs::read_to_string(path.join("source-eva.xml")).unwrap(),
        "<evaProcessing/>"
    );
}
#[test]
fn invalid_baseline_and_unknown_options_fail() {
    let d = fixture();
    let r = Recording::open(d.path()).unwrap();
    let c = Config {
        baseline_start: -2.,
        ..config()
    };
    assert!(c.validate(&r).is_err());
    assert!(cli::parse(&BTreeMap::from([("unknown".into(), "1".into())])).is_err());
    let c = cli::parse(&BTreeMap::from([("pre".into(), "0.4".into())])).unwrap();
    assert_eq!(c.baseline_start, -0.4);
}
