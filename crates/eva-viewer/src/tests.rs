use super::*;
use eframe::App;
use std::{
    fs,
    sync::atomic::AtomicU64,
    time::{Duration, Instant},
};

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
    fs::write(p.join("Events_test.xml"),"<eventTrack><event><code>SPIKE</code><beginTime>2026-01-01T00:00:05Z</beginTime><duration>1000</duration></event></eventTrack>").unwrap();
    let mut bytes = Vec::new();
    for v in [1u32, 24, 40000, 1, 0, (1000 << 8) | 32] {
        bytes.extend(v.to_le_bytes());
    }
    for i in 0..10000 {
        let v: f32 = match i {
            100 => 150.,
            101 => -75.,
            500 => f32::NAN,
            _ => 0.,
        };
        bytes.extend(v.to_le_bytes());
    }
    fs::write(p.join("signal1.bin"), bytes).unwrap();
    dir
}
fn tick(app: &mut Viewer, ctx: &egui::Context, size: Vec2) {
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        },
        |ui| app.ui(ui, &mut eframe::Frame::_new_kittest()),
    );
    out.textures_delta.clear();
    assert!(!out.shapes.is_empty());
}
fn until(app: &mut Viewer, ctx: &egui::Context, predicate: impl Fn(&Viewer) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        tick(app, ctx, Vec2::new(1280., 820.));
        if predicate(app) {
            break;
        }
        assert!(Instant::now() < deadline, "timeout: {:?}", app.error);
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn envelopes_preserve_spikes_and_leave_nonfinite_buckets_blank() {
    let dir = fixture();
    let r = Recording::open(dir.path()).unwrap();
    let key = ViewKey {
        range: 0..10000,
        channels: vec![0],
        columns: 100,
        filters: filter::Settings::default(),
    };
    let d = model::reduce(&r, key, &AtomicU64::new(1), 1).unwrap();
    assert_eq!(d.traces[0].bins[1].max, 150.);
    assert_eq!(d.traces[0].bins[1].min, -75.);
    assert!(d.traces[0].bins[5].gap);
    assert!(!d.traces[0].bins[6].gap);
}
#[test]
fn headless_ui_open_navigate_inspect_hide_resize_and_close() {
    let dir = fixture();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Viewer::new(&cc, Some(dir.path().to_owned()));
    until(&mut app, &ctx, |a| a.display.is_some());
    assert_eq!(
        app.recording.as_ref().unwrap().metadata().events[0]
            .sample_index(1000)
            .unwrap(),
        5000
    );
    app.navigation.zoom(0.1, 0.5);
    app.navigation.jump(5000);
    until(&mut app, &ctx, |a| {
        a.display
            .as_ref()
            .is_some_and(|d| d.key.range == a.navigation.range())
    });
    let r = app.recording.as_ref().unwrap().clone();
    app.active_id = app
        .worker
        .submit(Job::Sample(r, 0, 100, filter::Settings::default(), None));
    until(&mut app, &ctx, |a| a.exact.is_some());
    assert_eq!(app.exact, Some((0, 100, 150.)));
    app.inspector_open = true;
    for tab in 0..3 {
        app.tab = tab;
        tick(&mut app, &ctx, Vec2::new(900., 560.));
    }
    app.hidden.fill(true);
    tick(&mut app, &ctx, Vec2::new(900., 560.));
    assert!(app.display.is_some()); // hidden traces retain their numbered rows
    app.reset();
    until(&mut app, &ctx, |a| a.display.is_some());
    app.close();
    tick(&mut app, &ctx, Vec2::new(1280., 820.));
    assert!(app.recording.is_none());
}
#[test]
fn replacing_loading_recording_never_accepts_stale_result() {
    let dir = fixture();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Viewer::new(&cc, Some(dir.path().to_owned()));
    let other =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../eva-mff/tests/fixtures/example_3.mff");
    app.open(other);
    until(&mut app, &ctx, |a| a.display.is_some());
    assert_eq!(
        app.recording.as_ref().unwrap().metadata().channels.len(),
        257
    );
    app.open(dir.path().join("nonexistent"));
    until(&mut app, &ctx, |a| a.error.is_some());
    assert!(app.recording.is_none());
    let segmented =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../eva-mff/tests/fixtures/example_4.mff");
    app.open(segmented);
    until(&mut app, &ctx, |a| a.recording.is_some());
    assert!(app.error.as_deref().unwrap().contains("Segmented/averaged"));
    assert!(app.display.is_none());
    app.open(dir.path().to_owned());
    app.close();
    tick(&mut app, &ctx, Vec2::new(1280., 820.));
    assert!(app.recording.is_none());
}

#[test]
fn filtered_cursor_matches_waveform_across_tiles_and_bypass_restores_data() {
    use eva_mff::SampleValues;
    use filter::{Family, LineMode, Plan, Settings};
    let dir = fixture();
    let p = dir.path().join("signal1.bin");
    let mut bytes = Vec::new();
    for v in [1u32, 24, 160000, 1, 0, (1000 << 8) | 32] {
        bytes.extend(v.to_le_bytes());
    }
    for i in 0..40000 {
        let t = i as f64 / 1000.;
        let v = (10. * (2. * std::f64::consts::PI * 10. * t).sin()
            + 5. * (2. * std::f64::consts::PI * 60. * t).sin()) as f32;
        bytes.extend(v.to_le_bytes());
    }
    fs::write(p, bytes).unwrap();
    let r = Recording::open(dir.path()).unwrap();
    let g = AtomicU64::new(1);
    for settings in [
        Settings {
            enabled: true,
            high_hz: 2.,
            ..Default::default()
        },
        Settings {
            enabled: true,
            family: Family::Fir,
            high_hz: 2.,
            transition_hz: 3.,
            ..Default::default()
        },
        Settings {
            enabled: true,
            high_pass: false,
            low_pass: false,
            line: LineMode::CleanLine,
            ..Default::default()
        },
    ] {
        let plan = Plan::new(&settings, 1000).unwrap();
        let whole = plan.read(&r, 0, 16380..16390, &g, 1).unwrap();
        for sample in [16383, 16384] {
            let cursor = plan.read(&r, 0, sample..sample + 1, &g, 1).unwrap();
            assert_eq!(cursor[0], whole[(sample - 16380) as usize]);
        }
        // No seam at a canonical boundary: expected low-frequency content remains smooth.
        assert!((whole[4] - whole[3]).abs() < 1.5);
    }
    let bypass = Plan::new(&Settings::default(), 1000).unwrap();
    assert_eq!(
        bypass.read(&r, 0, 100..200, &g, 1).unwrap(),
        r.read_window(&[0], 100..200, SampleValues::Calibrated)
            .unwrap()
            .values[0]
    );
}

#[test]
fn ui_filters_invalidate_waveform_cursor_and_preserve_source() {
    let dir = fixture();
    let original = fs::read(dir.path().join("signal1.bin")).unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Viewer::new(&cc, Some(dir.path().to_owned()));
    until(&mut app, &ctx, |a| a.display.is_some());
    let s = filter::Settings {
        enabled: true,
        high_pass: false,
        low_hz: 30.,
        ..Default::default()
    };
    app.exact = Some((0, 100, 150.));
    app.apply_filters(s.clone(), 1000).unwrap();
    assert!(app.exact.is_none());
    assert!(app.display.is_none());
    app.open_filter_dialog();
    until(&mut app, &ctx, |a| a.display.is_some());
    assert_eq!(app.display.as_ref().unwrap().key.filters, s);
    let recording = app.recording.as_ref().unwrap().clone();
    app.active_id = app
        .worker
        .submit(Job::Sample(recording, 0, 100, s.clone(), None));
    until(&mut app, &ctx, |a| a.exact.is_some());
    let filtered = app.exact.unwrap().2;
    assert!(filtered > 0. && filtered < 150.);

    let bad = filter::Settings {
        enabled: true,
        low_hz: 600.,
        ..Default::default()
    };
    assert!(app.apply_filters(bad, 1000).is_err());
    assert_eq!(app.filters, s);
    app.apply_filters(filter::Settings::default(), 1000)
        .unwrap();
    until(&mut app, &ctx, |a| a.display.is_some());
    assert!(!app.display.as_ref().unwrap().key.filters.active());
    assert_eq!(fs::read(dir.path().join("signal1.bin")).unwrap(), original);
}

#[test]
fn filter_modal_preview_cancel_and_recording_close() {
    let dir = fixture();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Viewer::new(&cc, Some(dir.path().to_owned()));
    until(&mut app, &ctx, |a| a.display.is_some());
    let original = app.filters.clone();
    app.open_filter_dialog();
    app.filter_draft.high_hz = 2.;
    assert_eq!(app.filters, original, "draft must not affect the trace");
    let mut preview = app.filter_draft.clone();
    preview.enabled = true;
    app.apply_filters(preview.clone(), 1000).unwrap();
    until(&mut app, &ctx, |a| a.display.is_some());
    assert_eq!(app.display.as_ref().unwrap().key.filters, preview);
    app.open_filter_dialog(); // Reopening must not replace the rollback snapshot.
    assert_eq!(app.filter_original, Some(original.clone()));
    app.cancel_filter_dialog(1000);
    until(&mut app, &ctx, |a| a.display.is_some());
    assert_eq!(app.filters, original);
    assert_eq!(app.filter_draft, original);
    assert!(app.filter_original.is_none());
    app.open_filter_dialog();
    app.close();
    assert!(app.filter_original.is_none());
}

#[test]
fn pns_view_switch_and_filter_toggle_use_own_timebase() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../eva-mff/tests/fixtures/example_3.mff");
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Viewer::new(&cc, Some(path));
    until(&mut app, &ctx, |a| a.display.is_some());
    let eeg = app.recording.clone().unwrap();
    app.select_stream(true);
    until(&mut app, &ctx, |a| a.display.is_some());
    assert!(app.recording.as_ref().unwrap().is_pns());
    assert_eq!(app.hidden.len(), 2);
    assert_eq!(
        app.recording.as_ref().unwrap().metadata().channels[0].label,
        "ECG"
    );
    let mut s = filter::Settings {
        enabled: true,
        high_pass: false,
        ..Default::default()
    };
    s.options.average_reference = true;
    s.options.filter_pns = false;
    app.apply_filters(s.clone(), eeg.metadata().sample_rate)
        .unwrap();
    assert!(!app.effective_filters().active());
    s.options.filter_pns = true;
    app.apply_filters(s, eeg.metadata().sample_rate).unwrap();
    assert!(app.effective_filters().active());
    assert!(!app.effective_filters().options.average_reference);
    app.select_stream(false);
    assert!(app.effective_filters().options.average_reference);
}

#[test]
fn average_reference_uses_all_eeg_and_exclusions_and_matches_cursor() {
    let d = fixture();
    let n = 2000u32;
    let mut bytes = Vec::new();
    for w in [
        1u32,
        32,
        n * 8,
        2,
        0,
        n * 4,
        (1000 << 8) | 32,
        (1000 << 8) | 32,
    ] {
        bytes.extend(w.to_le_bytes());
    }
    for c in 0..2 {
        for i in 0..n {
            bytes.extend((i as f32 + c as f32 * 4.).to_le_bytes());
        }
    }
    fs::write(d.path().join("signal1.bin"), bytes).unwrap();
    fs::write(d.path().join("sensorLayout.xml"),"<sensorLayout><sensor><number>1</number><type>0</type><unit>uV</unit></sensor><sensor><number>2</number><type>0</type><unit>uV</unit></sensor></sensorLayout>").unwrap();
    let r = Recording::open(d.path()).unwrap();
    let mut s = filter::Settings {
        enabled: true,
        high_pass: false,
        low_pass: false,
        ..Default::default()
    };
    s.options.average_reference = true;
    let g = AtomicU64::new(1);
    let plan = filter::Plan::new(&s, 1000).unwrap();
    assert_eq!(plan.read(&r, 0, 100..110, &g, 1).unwrap(), vec![-2.; 10]);
    assert_eq!(plan.read(&r, 1, 100..110, &g, 1).unwrap(), vec![2.; 10]);
    assert_eq!(plan.read(&r, 0, 105..106, &g, 1).unwrap(), vec![-2.]);
    s.options.excluded = vec![1];
    let plan = filter::Plan::new(&s, 1000).unwrap();
    assert_eq!(plan.read(&r, 1, 100..110, &g, 1).unwrap(), vec![4.; 10]);
    s.options.excluded = vec![0, 1];
    assert!(filter::Plan::new(&s, 1000)
        .unwrap()
        .read(&r, 0, 100..110, &g, 1)
        .is_err());
}

fn mri_fixture() -> tempfile::TempDir {
    let d = fixture();
    let mut bytes = Vec::new();
    for w in [1u32, 24, 4000 * 4, 1, 0, (1000 << 8) | 32] {
        bytes.extend(w.to_le_bytes());
    }
    for i in 0..4000 {
        let value = if i % 100 < 15 { 100. } else { 0. };
        bytes.extend(f32::to_le_bytes(value));
    }
    fs::write(d.path().join("signal1.bin"), bytes).unwrap();
    let mut events = String::from("<eventTrack>");
    for i in 0..40 {
        events += &format!(
            "<event><code>TREV</code><beginTime>2026-01-01T00:00:{:02}.{:03}Z</beginTime></event>",
            i / 10,
            (i % 10) * 100
        );
    }
    events += "</eventTrack>";
    fs::write(d.path().join("Events_test.xml"), events).unwrap();
    // PNS has its own 500 Hz grid, with the same trigger times.
    let mut pns = Vec::new();
    for w in [1u32, 24, 2000 * 4, 1, 0, (500 << 8) | 32] {
        pns.extend(w.to_le_bytes());
    }
    for i in 0..2000 {
        pns.extend(f32::to_le_bytes(if i % 50 < 7 { 80. } else { 0. }));
    }
    fs::write(d.path().join("signal2.bin"), pns).unwrap();
    fs::write(
        d.path().join("info2.xml"),
        "<dataInfo><fileDataType><PNSData/></fileDataType></dataInfo>",
    )
    .unwrap();
    d
}
#[test]
fn mri_worker_correction_filters_cursor_bypass_and_close() {
    let d = mri_fixture();
    let original = fs::read(d.path().join("signal1.bin")).unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Viewer::new(&cc, Some(d.path().to_owned()));
    until(&mut app, &ctx, |a| a.display.is_some());
    app.invalidate_signal();
    app.mri.running = true;
    app.mri.open = true;
    app.active_id = app.worker.submit(Job::Correct(
        app.eeg_recording.clone().unwrap(),
        Box::new(mri::Request {
            config: eva_core::gradient::Config::default(),
            trigger: "TREV".into(),
            pns: true,
            progress: app.mri.progress.clone(),
        }),
    ));
    until(&mut app, &ctx, |a| {
        a.mri.corrected.is_some() && a.display.is_some()
    });
    let result = app.mri.corrected.as_ref().unwrap();
    assert_eq!(result.reports.len(), 2);
    assert_eq!(result.eeg[0].len(), 4000);
    assert_eq!(result.pns.as_ref().unwrap()[0].len(), 2000);
    assert!(result.eeg[0].iter().all(|v| v.abs() < 1e-4));
    let r = app.recording.clone().unwrap();
    app.active_id = app.worker.submit(Job::Sample(
        r,
        0,
        200,
        filter::Settings::default(),
        app.corrected_source(),
    ));
    until(&mut app, &ctx, |a| a.exact.is_some());
    assert!(app.exact.unwrap().2.abs() < 1e-4);
    app.apply_filters(
        filter::Settings {
            enabled: true,
            high_pass: false,
            ..Default::default()
        },
        1000,
    )
    .unwrap();
    until(&mut app, &ctx, |a| a.display.is_some());
    assert!(app.display.as_ref().unwrap().traces[0]
        .bins
        .iter()
        .all(|b| b.max.abs() < 1e-4));
    app.mri.bypass = true;
    app.invalidate_signal();
    app.apply_filters(filter::Settings::default(), 1000)
        .unwrap();
    until(&mut app, &ctx, |a| a.display.is_some());
    assert!(app.display.as_ref().unwrap().traces[0]
        .bins
        .iter()
        .any(|b| b.max == 100.));
    assert_eq!(fs::read(d.path().join("signal1.bin")).unwrap(), original);
    app.close();
    assert!(app.mri.corrected.is_none());
    assert!(!app.mri.open);
}
#[test]
fn mri_cancel_and_failed_input_keep_previous_result() {
    let d = mri_fixture();
    let r = Recording::open(d.path()).unwrap();
    let request = mri::Request {
        config: eva_core::gradient::Config::default(),
        trigger: "TREV".into(),
        pns: false,
        progress: Arc::new(std::sync::atomic::AtomicU32::new(0)),
    };
    assert!(mri::run(&r, &request, &|| true).is_err());
    let wrong = mri::Request {
        trigger: "missing".into(),
        ..request
    };
    assert!(mri::run(&r, &wrong, &|| false).is_err());
}

#[test]
fn discontinuous_recording_opens_and_displays_on_acquisition_clock() {
    let dir = fixture();
    fs::write(dir.path().join("epochs.xml"), "<epochs><epoch><beginTime>1000000</beginTime><endTime>11000000</endTime><firstBlock>1</firstBlock><lastBlock>1</lastBlock></epoch></epochs>").unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Viewer::new(&cc, Some(dir.path().to_owned()));
    until(&mut app, &ctx, |a| a.display.is_some());
    assert!(app.error.is_none());
    let r = app.recording.as_ref().unwrap().clone();
    assert_eq!(r.metadata().kind, RecordingKind::Discontinuous);
    assert_eq!(r.metadata().sample_count, 11000);
    app.active_id = app
        .worker
        .submit(Job::Sample(r, 0, 1100, filter::Settings::default(), None));
    until(&mut app, &ctx, |a| a.exact.is_some());
    assert_eq!(app.exact, Some((0, 1100, 150.)));
}

#[test]
fn mri_all_methods_accept_recording_pauses_and_preserve_eeg_pns_clocks() {
    use eva_core::gradient::{Config, Method, Synchronization};
    let d = mri_fixture();
    // Two two-second acquisitions separated by a half-second recording pause.
    for (file, half) in [("signal1.bin", 2000usize), ("signal2.bin", 1000)] {
        let original = fs::read(d.path().join(file)).unwrap();
        let mut header = original[..24].to_vec();
        header[8..12].copy_from_slice(&((half * 4) as u32).to_le_bytes());
        let mut bytes = header.clone();
        bytes.extend_from_slice(&original[24..24 + half * 4]);
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&original[24 + half * 4..]);
        fs::write(d.path().join(file), bytes).unwrap();
    }
    fs::write(d.path().join("epochs.xml"), "<epochs><epoch><beginTime>0</beginTime><endTime>2000000</endTime><firstBlock>1</firstBlock><lastBlock>1</lastBlock></epoch><epoch><beginTime>2500000</beginTime><endTime>4500000</endTime><firstBlock>2</firstBlock><lastBlock>2</lastBlock></epoch></epochs>").unwrap();
    let mut events = String::from("<eventTrack>");
    for i in 0..40 {
        let ms = i * 100 + if i >= 20 { 500 } else { 0 };
        events += &format!(
            "<event><code>TREV</code><beginTime>2026-01-01T00:00:{:02}.{:03}Z</beginTime></event>",
            ms / 1000,
            ms % 1000
        );
    }
    events += "</eventTrack>";
    fs::write(d.path().join("Events_test.xml"), events).unwrap();
    let r = Recording::open(d.path()).unwrap();
    let original = fs::read(d.path().join("signal1.bin")).unwrap();
    assert_eq!(r.metadata().kind, RecordingKind::Discontinuous);
    assert_eq!(mri::triggers(&r, "TREV").unwrap()[20], 2500);
    for method in [
        Method::AllenAas,
        Method::Fastr,
        Method::Farm,
        Method::Moosmann,
    ] {
        let request = mri::Request {
            config: Config {
                method,
                anc: true,
                motion: vec![[0.; 6]; 40],
                synchronization: Synchronization::Custom,
                align: false,
                ..Default::default()
            },
            trigger: "TREV".into(),
            pns: true,
            progress: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        };
        let result = mri::run(&r, &request, &|| false).unwrap();
        for (data, start, end, len) in [
            (&result.eeg, 2000, 2500, 4500),
            (result.pns.as_ref().unwrap(), 1000, 1250, 2250),
        ] {
            assert_eq!(data[0].len(), len);
            assert!(data[0][start..end].iter().all(|v| v.is_nan()));
            assert!(data[0][..start].iter().all(|v| v.is_finite()));
            assert!(data[0][end..].iter().all(|v| v.is_finite()));
            // ANC may spread boundary residuals within a finite segment; require
            // >90% interior energy removal, not sample-for-sample zero output.
            let energy =
                |v: &[f32]| v.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / v.len() as f64;
            assert!(
                energy(&data[0][100..start - 100]) < 80.,
                "{method:?}: {}",
                energy(&data[0][100..start - 100])
            );
            assert!(
                energy(&data[0][end + 100..len - 100]) < 80.,
                "{method:?}: {}",
                energy(&data[0][end + 100..len - 100])
            );
        }
        assert!(result
            .reports
            .iter()
            .all(|(_, d)| d.corrected > 0 && d.anc_channels == 1));
    }
    assert_eq!(fs::read(d.path().join("signal1.bin")).unwrap(), original);
}

#[test]
#[ignore = "requires EVA_MRI_TEST_MFF pointing to a local recording"]
fn mri_external_recording_smoke() {
    let r = Recording::open(std::env::var_os("EVA_MRI_TEST_MFF").unwrap()).unwrap();
    let m = r.metadata();
    let channels = [0, m.channels.len() / 2, m.channels.len() - 1];
    let source = r
        .read_window(
            &channels,
            0..m.sample_count,
            eva_mff::SampleValues::Calibrated,
        )
        .unwrap();
    let result = eva_core::gradient::correct(
        &source.values,
        &mri::triggers(&r, "TREV").unwrap(),
        m.sample_rate as f64,
        &eva_core::gradient::Config::default(),
        &|| false,
        &|_| {},
    )
    .unwrap();
    assert!(result.diagnostics.corrected > 0);
    for (a, b) in source.values.iter().zip(&result.channels) {
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(b).all(|(a, b)| a.is_finite() == b.is_finite()));
    }
    // A reference channel can legitimately remain flat and unchanged.
    assert!(source
        .values
        .iter()
        .flatten()
        .zip(result.channels.iter().flatten())
        .any(|(a, b)| a.is_finite() && (a - b).abs() > 0.01));
}

#[test]
fn channel_interpolation_updates_with_filters_and_bad_donors_and_keeps_rows() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../eva-mff/tests/fixtures/example_3.mff");
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Viewer::new(&cc, Some(path));
    until(&mut app, &ctx, |a| a.display.is_some());
    app.mark_bad(2);
    app.interpolate_channel(0);
    assert_eq!(app.interpolation.len(), 1);
    assert!(!app.interpolation[0].donors.contains(&0));
    assert!(!app.interpolation[0].donors.contains(&2));
    app.interpolate_channel(1);
    assert_eq!(app.interpolation.len(), 2);
    assert!(app.interpolation.iter().all(|r| !r.donors.contains(&1)));
    let r = app.recording.clone().unwrap();
    app.filters = filter::Settings {
        enabled: true,
        high_pass: false,
        low_hz: 30.,
        ..Default::default()
    };
    let effective = app.effective_filters();
    let plan = filter::Plan::new(&effective, 250).unwrap();
    let values = plan.read(&r, 0, 0..32, &AtomicU64::new(1), 1).unwrap();
    let recipe = &app.interpolation[0];
    let mut expected = vec![0f64; 32];
    let mut base = effective.clone();
    base.interpolation.clear();
    let base = filter::Plan::new(&base, 250).unwrap();
    for (&ch, &w) in recipe.donors.iter().zip(&recipe.weights) {
        for (a, v) in expected
            .iter_mut()
            .zip(base.read(&r, ch, 0..32, &AtomicU64::new(1), 1).unwrap())
        {
            *a += w * f64::from(v);
        }
    }
    assert!(values
        .iter()
        .zip(expected)
        .all(|(a, b)| (f64::from(*a) - b).abs() < 0.001));
    app.hidden[0] = true;
    until(&mut app, &ctx, |a| a.display.is_some());
    assert!(app.display.as_ref().unwrap().key.channels.contains(&0));
    app.mark_bad(3);
    assert!(app.interpolation.iter().all(|r| !r.donors.contains(&3)));
    app.select_stream(true);
    assert!(app.effective_filters().interpolation.is_empty());
    app.select_stream(false);
    assert_eq!(app.effective_filters().interpolation.len(), 2);
    app.close();
    assert!(app.bad.is_empty());
    assert!(app.interpolation.is_empty());
}

#[test]
fn psa_modal_snapshot_results_and_close_cancel() {
    let dir = fixture();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Viewer::new(&cc, Some(dir.path().to_owned()));
    until(&mut app, &ctx, |a| a.display.is_some());
    app.psa.open = true;
    app.psa.config.events = vec!["SPIKE".into()];
    app.psa.config.threshold = false;
    app.psa.config.average_reference = false;
    app.psa.config.average = true;
    tick(&mut app, &ctx, Vec2::new(1280., 900.));
    app.start_psa(&ctx);
    until(&mut app, &ctx, |a| !a.psa.running);
    let result = app.psa.result.as_ref().unwrap();
    assert_eq!(result.epochs.len(), 1);
    assert_eq!(result.averages.len(), 1);
    assert!(app.psa.show_results);
    tick(&mut app, &ctx, Vec2::new(1000., 700.));
    app.psa.open = true;
    app.start_psa(&ctx);
    app.close();
    tick(&mut app, &ctx, Vec2::new(1000., 700.));
    assert!(app.psa.result.is_none());
    assert!(!app.psa.running);
}

#[test]
fn ocular_sheet_detect_and_reset_on_close() {
    let d = fixture();
    let ctx = egui::Context::default();
    let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), None);
    app.open(d.path().to_owned());
    until(&mut app, &ctx, |a| a.recording.is_some());
    app.ocular.open = true;
    tick(&mut app, &ctx, Vec2::new(1280., 900.));
    app.start_ocular(&ctx);
    until(&mut app, &ctx, |a| !a.ocular.running);
    assert!(app.ocular.result.is_some());
    app.close();
    assert!(app.ocular.result.is_none());
    assert!(!app.ocular.open);
}
