use crate::{mri, worker::Job, Viewer};
use eframe::egui;
use eva_core::gradient::*;
use std::{
    io::Read,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
};
fn help(ui: &mut egui::Ui, title: &str, text: &str) {
    ui.push_id(("mri_help", title), |ui| {
        let response = ui.small_button("?").on_hover_text(text);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Help: {title}"))
        });
        egui::Popup::from_toggle_button_response(&response).show(|ui| {
            ui.set_max_width(360.);
            ui.strong(title);
            ui.label(text);
        });
    });
}
fn heading(ui: &mut egui::Ui, title: &str, text: &str) {
    ui.horizontal(|ui| {
        ui.strong(title);
        help(ui, title, text);
    });
}
fn choice<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    choices: &[(T, &str)],
) {
    egui::ComboBox::from_label(label)
        .selected_text(
            choices
                .iter()
                .find(|(v, _)| v == value)
                .map(|(_, t)| *t)
                .unwrap_or(""),
        )
        .show_ui(ui, |ui| {
            for &(v, t) in choices {
                ui.selectable_value(value, v, t);
            }
        });
}
fn integer(ui: &mut egui::Ui, label: &str, v: &mut usize, range: std::ops::RangeInclusive<usize>) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(v).range(range));
    });
}
fn number(
    ui: &mut egui::Ui,
    label: &str,
    v: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    speed: f64,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(v).range(range).speed(speed));
    });
}
fn load_text(path: &std::path::Path) -> Result<String, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut text = String::new();
    f.take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 16 * 1024 * 1024 {
        Err("File exceeds 16 MiB.".into())
    } else {
        Ok(text)
    }
}
fn algorithm_help(method: Method) -> &'static str {
    match method {
    Method::AllenAas=>"Allen AAS (Imaging Artifact Reduction) averages artifacts within fixed sections. The first accepted epochs seed a running template; later donors must pass a correlation gate. Each target is left out of its own template. This is the current macOS AAS choice, not retired Fast AAS. Optional ANC models remaining correlated artifact.",
    Method::Fastr=>"FASTR aligns repeated acquisition epochs, averages nearby donor epochs, scales and subtracts the template, and optionally removes residuals with OBS and ANC. Supply scanner volume triggers and the number or timing of acquisition groups. This ports EVA's clean-room implementation, not the external FMRIB toolbox.",
    Method::Farm=>"FARM uses the FASTR pipeline but ranks same-position acquisition epochs by waveform correlation. Donors must meet the threshold. If too few qualify, temporal neighbors are used and the fallback is reported. Similar artifacts can be selected from farther away in time.",
    Method::Moosmann=>"Moosmann uses motion-informed neighborhoods. High-motion volumes never donate, and motion events act as barriers. If too few neighbors remain, barriers may be crossed and reported. A motion file is required. With no above-threshold motion this reduces to temporal averaging. Targets with motion are still corrected.",
}
}
impl Viewer {
    pub(crate) fn mri_dialog(&mut self, ctx: &egui::Context) {
        if !self.mri.open {
            return;
        }
        let Some(r) = self.eeg_recording.clone() else {
            return;
        };
        let mut close = false;
        let mut apply = false;
        let mut clear = false;
        let mut bypass = false;
        let modal=egui::Modal::new(egui::Id::new("mri_dialog")).frame(egui::Frame::popup(&ctx.style_of(egui::Theme::Light)).inner_margin(22.).corner_radius(18)).show(ctx,|ui|{
            ui.set_width((ctx.content_rect().width()-76.).clamp(260.,1000.));
            heading(ui,"MRI gradient artifact removal","Runs a whole-recording CPU correction using calibrated source samples. Temporal filters and EEG average reference run afterward. Applying again starts from the source, never from an already corrected result. Cancel keeps the previous completed result; removing correction returns to the source. Recording pauses stay blank. Acquisition epochs that cross missing samples are skipped; ANC restarts after each gap. No files are modified.");
            ui.add_enabled_ui(!self.mri.running,|ui|{
                ui.horizontal_wrapped(|ui|{
                    choice(ui,"Method",&mut self.mri.config.method,&[(Method::AllenAas,"Allen AAS"),(Method::Fastr,"FASTR"),(Method::Farm,"FARM"),(Method::Moosmann,"Moosmann")]);
                    help(ui,self.mri.config.method.label(),algorithm_help(self.mri.config.method));
                    for method in [Method::AllenAas,Method::Fastr,Method::Farm,Method::Moosmann] {ui.push_id(method.label(),|ui|{ui.label(method.label());help(ui,method.label(),algorithm_help(method));});}
                });
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-220.).max(160.)).show(ui,|ui|{
                    if ui.available_width()>800. {ui.columns(2,|cols|{timing(&mut cols[0],&mut self.mri,&r);advanced(&mut cols[1],&mut self.mri,&r);});}
                    else {timing(ui,&mut self.mri,&r);advanced(ui,&mut self.mri,&r);}
                    if let Some(result)=&self.mri.corrected {
                        ui.separator();ui.strong(&result.description);
                        for (name,d) in &result.reports {
                            ui.label(format!("{name}: {} epochs · {} channel-epochs corrected · {} skipped · {} high-motion volumes",d.epochs,d.corrected,d.skipped,d.high_motion.len()));
                            ui.small(format!("{} correlation fallbacks · {} barrier crossings · {} rejected scale fits · {} OBS components · {} ANC channels",d.correlation_fallbacks,d.motion_barrier_crossings,d.rejected_scales,d.obs_components,d.anc_channels));
                            for warning in d.warnings.iter().take(20){ui.colored_label(crate::GOLD,warning);}
                        }
                    }
                });
            });
            if let Some(e)=&self.mri.error{ui.colored_label(crate::GOLD,e);}
            if self.mri.running {ui.add(egui::ProgressBar::new(self.mri.progress.load(Ordering::Relaxed) as f32/1000.).show_percentage());ctx.request_repaint_after(std::time::Duration::from_millis(100));if ui.button("Cancel correction").clicked(){self.worker.cancel();self.mri.running=false;self.mri.error=Some("Cancelled; previous completed correction retained.".into());self.busy=None;}}
            ui.horizontal_wrapped(|ui|{
                close=ui.add_enabled(!self.mri.running,egui::Button::new("Close")).clicked();
                bypass=ui.add_enabled(!self.mri.running && self.mri.corrected.is_some(),egui::Button::new(if self.mri.bypass{"Use corrected"}else{"Bypass correction"})).clicked();
                clear=ui.add_enabled(!self.mri.running && self.mri.corrected.is_some(),egui::Button::new("Remove correction")).clicked();
                apply=ui.add_enabled(!self.mri.running && r.metadata().kind!=eva_mff::RecordingKind::SegmentedOrAveraged,egui::Button::new("Apply MRI Correction")).clicked();
                ui.small("CPU · source read-only");
            });
        });
        if apply {
            let prepared = (|| -> Result<(), String> {
                if self.mri.timing_mode == 0 {
                    if !self.mri.slices.is_multiple_of(self.mri.multiband) {
                        return Err("Slices must be divisible by multiband factor.".into());
                    }
                    self.mri.config.groups = self.mri.slices / self.mri.multiband;
                    self.mri.config.offsets_seconds.clear();
                } else if self.mri.timing_mode == 1 {
                    self.mri.config.offsets_seconds.clear();
                } else {
                    let mut values = self
                        .mri
                        .offsets_text
                        .split(|c: char| c == ',' || c.is_whitespace())
                        .filter(|s| !s.is_empty())
                        .map(str::parse::<f64>)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|_| "Invalid acquisition offsets")?;
                    values.sort_by(f64::total_cmp);
                    values.dedup();
                    if values.is_empty() {
                        return Err("Enter or import acquisition offsets.".into());
                    }
                    self.mri.config.offsets_seconds = values;
                }
                self.mri.config.validate()?;
                mri::triggers(&r, &self.mri.trigger)?;
                Ok(())
            })();
            match prepared {
                Err(e) => self.mri.error = Some(e),
                Ok(()) => {
                    self.invalidate_signal();
                    self.mri.progress = Arc::new(AtomicU32::new(0));
                    self.mri.error = None;
                    self.mri.running = true;
                    self.active_id = self.worker.submit(Job::Correct(
                        r,
                        Box::new(mri::Request {
                            config: self.mri.config.clone(),
                            trigger: self.mri.trigger.clone(),
                            pns: self.mri.pns,
                            progress: self.mri.progress.clone(),
                        }),
                    ));
                    self.busy = Some("Correcting MRI gradient artifact…");
                }
            }
        }
        if bypass {
            self.mri.bypass = !self.mri.bypass;
            self.invalidate_signal();
        }
        if clear {
            self.mri.corrected = None;
            self.mri.bypass = false;
            self.invalidate_signal();
        }
        if close
            || (!self.mri.running
                && !modal.any_popup_open
                && ctx.input(|i| i.key_pressed(egui::Key::Escape)))
        {
            self.mri.open = false;
        }
    }
}
fn timing(ui: &mut egui::Ui, state: &mut mri::State, r: &eva_mff::Recording) {
    heading(ui,"Scanner timing","Select volume triggers, not individual slice markers. Repeated multiband slice times count as one acquisition group. Missing-trigger gaps are not filled with invented markers. Irregular TRs must be resolved before correction.");
    let mut codes: Vec<_> = r
        .metadata()
        .events
        .iter()
        .map(|e| e.code.as_str())
        .collect();
    codes.sort_unstable();
    codes.dedup();
    egui::ComboBox::from_label("Volume marker")
        .selected_text(&state.trigger)
        .show_ui(ui, |ui| {
            for code in codes {
                ui.selectable_value(&mut state.trigger, code.to_owned(), code);
            }
        });
    match mri::triggers(r, &state.trigger) {
        Ok(t) => {
            let mut intervals: Vec<_> = t.windows(2).map(|v| v[1] - v[0]).collect();
            intervals.sort_unstable();
            ui.small(format!(
                "{} volumes · median TR {:.4} s",
                t.len(),
                intervals[intervals.len() / 2] as f64 / r.metadata().sample_rate as f64
            ));
        }
        Err(e) => {
            ui.colored_label(crate::GOLD, e);
        }
    }
    choice(
        ui,
        "Acquisition timing",
        &mut state.timing_mode,
        &[
            (0, "Slices / multiband"),
            (1, "Group count"),
            (2, "Explicit offsets / JSON"),
        ],
    );
    if state.timing_mode == 0 {
        integer(ui, "Slices per volume", &mut state.slices, 1..=512);
        integer(ui, "Multiband factor", &mut state.multiband, 1..=512);
    } else if state.timing_mode == 1 {
        integer(ui, "Groups per volume", &mut state.config.groups, 1..=512);
    } else {
        ui.label("Offsets in seconds (comma separated)");
        ui.text_edit_multiline(&mut state.offsets_text);
        if ui.button("Import BIDS SliceTiming JSON…").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("JSON", &["json"])
                .pick_file()
            {
                let result = load_text(&path).and_then(|text| {
                    let doc: serde_json::Value =
                        serde_json::from_str(&text).map_err(|e| e.to_string())?;
                    let values = doc
                        .get("SliceTiming")
                        .and_then(|v| v.as_array())
                        .ok_or("JSON needs a SliceTiming array")?;
                    let mut times = values
                        .iter()
                        .map(|v| v.as_f64().ok_or("SliceTiming values must be numeric"))
                        .collect::<Result<Vec<_>, _>>()?;
                    if times.iter().any(|v| !v.is_finite() || *v < 0.) {
                        return Err("Invalid SliceTiming values".into());
                    }
                    times.sort_by(f64::total_cmp);
                    times.dedup();
                    Ok(times)
                });
                match result {
                    Ok(times) => {
                        state.offsets_text = times
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ");
                        state.timing_name = Some(path.display().to_string());
                        state.error = None;
                    }
                    Err(e) => state.error = Some(e),
                }
            }
        }
        if let Some(name) = &state.timing_name {
            ui.small(name);
        }
    }
    number(
        ui,
        "Trigger position in epoch",
        &mut state.config.trigger_position,
        0.0..=1.,
        0.01,
    );
    integer(ui, "Upsample factor", &mut state.config.upsample, 1..=16);
    ui.separator();
    heading(ui,"Template","Templates are built from source epochs, never already corrected data. Allen AAS uses sections and a running correlation gate. FASTR uses nearby donors; FARM ranks similarity; Moosmann respects motion boundaries. Drift-tracking smooths scale fits to reduce absorption of physiological activity. Per-epoch fitting and squared-correlation self-donation are more aggressive.");
    if state.config.method == Method::AllenAas {
        integer(
            ui,
            "Section epochs (0 = automatic)",
            &mut state.config.section_epochs,
            0..=10000,
        );
        number(
            ui,
            "Correlation gate",
            &mut state.config.allen_gate,
            -1.0..=1.,
            0.005,
        );
        integer(
            ui,
            "Initial accepted epochs",
            &mut state.config.initial_epochs,
            0..=100,
        );
    } else {
        integer(ui, "Donor volumes", &mut state.config.donors, 1..=500);
        choice(
            ui,
            "Synchronization",
            &mut state.config.synchronization,
            &[
                (Synchronization::ScannerSlaved, "Scanner slaved"),
                (Synchronization::IndependentClocks, "Independent clocks"),
                (Synchronization::Custom, "Custom"),
            ],
        );
        if state.config.synchronization == Synchronization::Custom {
            ui.checkbox(&mut state.config.align, "Align epochs");
            ui.checkbox(&mut state.config.fractional, "Sub-sample alignment");
        }
        choice(
            ui,
            "Template scaling",
            &mut state.config.scaling,
            &[
                (Scaling::Unscaled, "Unscaled average"),
                (Scaling::DriftTracking, "Drift-tracking (median fit)"),
                (Scaling::LeastSquares, "Per-epoch least squares"),
            ],
        );
        if state.config.scaling != Scaling::Unscaled {
            integer(
                ui,
                "Smoothing epochs",
                &mut state.config.smoothing,
                1..=1001,
            );
            number(
                ui,
                "Minimum scale",
                &mut state.config.scale_min,
                0.001..=100.,
                0.1,
            );
            number(
                ui,
                "Maximum scale",
                &mut state.config.scale_max,
                0.001..=100.,
                0.1,
            );
        }
        ui.checkbox(
            &mut state.config.squared_correlation,
            "Squared-correlation ranking (allows self donation)",
        );
        if state.config.method == Method::Farm || state.config.squared_correlation {
            number(
                ui,
                "Correlation threshold",
                &mut state.config.correlation,
                -1.0..=1.,
                0.01,
            );
            integer(
                ui,
                "Minimum correlated donors",
                &mut state.config.minimum_donors,
                1..=500,
            );
            integer(
                ui,
                "Search window (volumes/side)",
                &mut state.config.search,
                1..=2000,
            );
        }
    }
    if r.pns().is_some() {
        ui.checkbox(&mut state.pns, "Apply to PNS (at its own sample rate)");
    }
}
fn advanced(ui: &mut egui::Ui, state: &mut mri::State, r: &eva_mff::Recording) {
    heading(ui,"Motion","AFNI: rotations in degrees followed by translations in mm. FSL: rotations in radians, then translations. SPM: translations, then rotations in radians. Select the format explicitly. Translation sum uses L1 changes; Translation speed uses Euclidean changes; All six adds rotational displacement at the chosen radius. Short files are front-padded, long files truncated, with a run warning.");
    choice(
        ui,
        "File format",
        &mut state.motion_format,
        &[
            (MotionFormat::Afni, "AFNI 6 columns"),
            (MotionFormat::Fsl, "FSL MCFLIRT"),
            (MotionFormat::Spm, "SPM realignment"),
        ],
    );
    if ui.button("Load motion file…").clicked() {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            match load_text(&path).and_then(|text| parse_motion(&text, state.motion_format)) {
                Ok(rows) => {
                    state.config.motion = rows;
                    state.motion_name =
                        Some(format!("{} ({:?})", path.display(), state.motion_format));
                    state.error = None;
                }
                Err(e) => state.error = Some(e),
            }
        }
    }
    if let Some(name) = &state.motion_name {
        ui.small(format!("{} rows · {name}", state.config.motion.len()));
        if ui.small_button("Clear motion").clicked() {
            state.config.motion.clear();
            state.motion_name = None;
        }
    }
    if state.config.method != Method::Moosmann {
        ui.checkbox(
            &mut state.config.censor_motion,
            "Exclude high-motion donors",
        );
    }
    if state.config.method == Method::Moosmann || state.config.censor_motion {
        choice(
            ui,
            "Metric",
            &mut state.config.motion_metric,
            &[
                (MotionMetric::TranslationSum, "Translation (sum)"),
                (MotionMetric::TranslationSpeed, "Translation speed"),
                (MotionMetric::AllSix, "All six (FD)"),
            ],
        );
        number(
            ui,
            "Threshold (mm)",
            &mut state.config.motion_threshold,
            0.0..=100.,
            0.05,
        );
        if state.config.motion_metric == MotionMetric::AllSix {
            number(
                ui,
                "Head radius (mm)",
                &mut state.config.motion_radius,
                1.0..=200.,
                1.,
            );
        }
    }
    if state.config.method != Method::AllenAas {
        ui.separator();
        heading(ui,"OBS residual removal","Optimal basis sets use principal components of detrended residual epochs. Automatic retains enough components to meet the variance target, up to the maximum. Bases are estimated in chunks, with a deterministic epoch cap. The energy floor skips OBS when template subtraction already explains almost all artifact. OBS can also remove neural signal; it defaults off.");
        choice(
            ui,
            "OBS",
            &mut state.config.obs,
            &[
                (Obs::Off, "Off"),
                (Obs::Automatic, "Automatic"),
                (Obs::Fixed, "Fixed components"),
            ],
        );
        if state.config.obs != Obs::Off {
            if state.config.obs == Obs::Fixed {
                integer(ui, "Components", &mut state.config.components, 1..=32);
            } else {
                number(
                    ui,
                    "Variance fraction",
                    &mut state.config.variance,
                    0.0..=1.,
                    0.01,
                );
                integer(
                    ui,
                    "Maximum components",
                    &mut state.config.max_components,
                    1..=32,
                );
            }
            number(
                ui,
                "Chunk duration (s)",
                &mut state.config.chunk_seconds,
                1.0..=3600.,
                1.,
            );
            integer(
                ui,
                "Maximum basis epochs",
                &mut state.config.max_basis_epochs,
                8..=256,
            );
            number(
                ui,
                "Residual energy floor",
                &mut state.config.residual_floor,
                0.0..=1.,
                0.005,
            );
        }
    }
    ui.separator();
    heading(ui,"ANC","Adaptive noise cancellation uses the removed artifact as a reference and a normalized LMS filter to estimate residual contamination. Both signals are high-pass filtered before fitting. A larger step adapts faster but may remove physiological activity. Uninformative or nonfinite references are skipped and reported. Defaults off.");
    ui.checkbox(&mut state.config.anc, "Enable ANC");
    if state.config.anc {
        ui.checkbox(
            &mut state.config.anc_slice_highpass,
            "Slice-rate high-pass (otherwise 2 Hz)",
        );
        integer(ui, "Filter taps", &mut state.config.anc_taps, 1..=1024);
        number(
            ui,
            "Step size",
            &mut state.config.anc_step,
            0.0001..=1.99,
            0.005,
        );
    }
    ui.separator();
    heading(ui,"Channel scope","Excluded channels pass through unchanged. Template-only channels receive template subtraction but skip OBS and ANC. These lists refer to EEG channel numbers; PNS is processed separately when selected.");
    egui::ComboBox::from_label("EEG channel scope")
        .selected_text(format!(
            "{} excluded · {} template only",
            state.config.excluded_channels.len(),
            state.config.template_only.len()
        ))
        .show_ui(ui, |ui| {
            for ch in &r.metadata().channels {
                ui.push_id(ch.index, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(&ch.label);
                        let mut excluded = state.config.excluded_channels.contains(&ch.index);
                        if ui.checkbox(&mut excluded, "Exclude").changed() {
                            state.config.excluded_channels.retain(|v| *v != ch.index);
                            if excluded {
                                state.config.excluded_channels.push(ch.index);
                            }
                        }
                        let mut template = state.config.template_only.contains(&ch.index);
                        if ui.checkbox(&mut template, "Template only").changed() {
                            state.config.template_only.retain(|v| *v != ch.index);
                            if template {
                                state.config.template_only.push(ch.index);
                            }
                        }
                    });
                });
            }
        });
}
