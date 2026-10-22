use crate::{filter::*, filter_help as help, Viewer};
use eframe::egui;
impl Viewer {
    pub(crate) fn apply_filters(&mut self, settings: Settings, rate: u32) -> Result<(), String> {
        settings.validate(rate)?;
        if settings.active() && settings.options.filter_pns {
            if let Some(pns) = self.eeg_recording.as_ref().and_then(|r| r.pns()) {
                let mut p = settings.clone();
                p.options.average_reference = false;
                p.validate(pns.metadata().sample_rate)
                    .map_err(|e| format!("PNS: {e}"))?;
            }
        }
        self.worker.cancel();
        self.filters = settings;
        self.wanted = None;
        self.display = None;
        self.exact = None;
        self.busy = None;
        self.error = None;
        Ok(())
    }
    pub(crate) fn open_filter_dialog(&mut self) {
        if self.filter_original.is_none() {
            self.filter_original = Some(self.filters.clone());
            self.filter_draft = self.filters.clone();
            self.filter_error = None;
        }
    }

    pub(crate) fn cancel_filter_dialog(&mut self, rate: u32) {
        if let Some(original) = self.filter_original.take() {
            if self.filters != original {
                self.filter_error = self.apply_filters(original.clone(), rate).err();
            }
            self.filter_draft = original;
        }
    }

    pub(crate) fn filter_dialog(&mut self, ctx: &egui::Context) {
        if self.filter_original.is_none() {
            return;
        }
        let Some(recording) = self
            .eeg_recording
            .clone()
            .or_else(|| self.recording.clone())
        else {
            return;
        };
        let rate = recording.metadata().sample_rate;
        let continuous = recording.metadata().kind != eva_mff::RecordingKind::SegmentedOrAveraged;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("filter_dialog"))
            .frame(
                egui::Frame::popup(&ctx.style_of(egui::Theme::Light))
                    .fill(egui::Color32::from_rgb(245, 245, 247))
                    .corner_radius(18)
                    .inner_margin(22.),
            )
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 76.).clamp(250., 900.));
                ui.spacing_mut().item_spacing = egui::vec2(10., 9.);
                ui.horizontal(|ui| {
                    ui.heading("Filter");
                    help::button(ui, "Preview and Apply");
                });
                // Full-size windows fit both columns even with all options expanded.
                // A small-window fallback keeps controls reachable instead of clipping.
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 180.).max(140.))
                    .show(ui, |ui| {
                        if ui.available_width() >= 740. {
                            ui.columns(2, |cols| {
                                passband(&mut cols[0], &mut self.filter_draft, rate);
                                other_options(&mut cols[1], &mut self.filter_draft, &recording);
                            });
                        } else {
                            passband(ui, &mut self.filter_draft, rate);
                            ui.separator();
                            other_options(ui, &mut self.filter_draft, &recording);
                        }
                    });
                ui.separator();
                let mut candidate = self.filter_draft.clone();
                candidate.enabled = true;
                let mut validation = candidate.validate(rate);
                if candidate.options.filter_pns {
                    if let Some(pns) = recording.pns() {
                        let mut p = candidate.clone();
                        p.options.average_reference = false;
                        if let Err(e) = p.validate(pns.metadata().sample_rate) {
                            validation = Err(format!("PNS: {e}"));
                        }
                    }
                }
                if let Err(e) = &validation {
                    ui.colored_label(crate::GOLD, e);
                }
                if let Some(e) = &self.filter_error {
                    ui.colored_label(crate::GOLD, e);
                }
                if self.filter_original.as_ref() != Some(&self.filters) {
                    ui.small("Preview active · Cancel restores previous filters");
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Reset 0.1–30 Hz").clicked() {
                        self.filter_draft.high_pass = true;
                        self.filter_draft.high_hz = 0.1;
                        self.filter_draft.low_pass = true;
                        self.filter_draft.low_hz = 30.;
                        self.filter_draft.line = LineMode::Off;
                        self.filter_draft.options.average_reference = false;
                    }
                    if ui
                        .add_enabled(
                            continuous && validation.is_ok(),
                            egui::Button::new("Preview"),
                        )
                        .clicked()
                    {
                        self.filter_error = self.apply_filters(candidate.clone(), rate).err();
                    }
                    if ui.button("Remove filters").clicked() {
                        self.filter_draft = Settings::default();
                        self.filter_error = self.apply_filters(Settings::default(), rate).err();
                        if self.filter_error.is_none() {
                            self.filter_original = None;
                        }
                    }
                    if ui
                        .button("Copy CLI command")
                        .on_hover_text(
                            "Copy these settings for eva filter; replace the input/output paths.",
                        )
                        .clicked()
                    {
                        ctx.copy_text(format!(
                            "eva filter {} 'input.mff' 'output.mff'",
                            crate::filter::cli::arguments(&candidate).join(" ")
                        ));
                    }
                    cancel = ui.button("Cancel").clicked();
                    if ui
                        .add_enabled(
                            continuous && validation.is_ok(),
                            egui::Button::new(
                                egui::RichText::new("Apply Filter").color(egui::Color32::WHITE),
                            )
                            .fill(crate::INK),
                        )
                        .clicked()
                    {
                        self.filter_error = self.apply_filters(candidate, rate).err();
                        if self.filter_error.is_none() {
                            self.filter_original = None;
                        }
                    }
                });
            });
        if cancel || (!modal.any_popup_open && ctx.input(|i| i.key_pressed(egui::Key::Escape))) {
            self.cancel_filter_dialog(rate);
        }
    }
}
fn select<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    choices: &[(T, &str)],
) {
    let selected = choices
        .iter()
        .find(|(v, _)| v == value)
        .map(|(_, s)| *s)
        .unwrap_or("");
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_label(label)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for &(v, s) in choices {
                    ui.selectable_value(value, v, s);
                }
            });
        help::button(ui, label);
    });
}
fn number(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    speed: f64,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).range(range).speed(speed));
        help::button(ui, label);
    });
}
fn passband(ui: &mut egui::Ui, s: &mut Settings, rate: u32) {
    help::heading(ui, "Filter Type");
    ui.horizontal(|ui| {
        for (v, label) in [
            (Family::Iir, "IIR"),
            (Family::Fir, "FIR"),
            (Family::Auto, "Auto"),
        ] {
            ui.selectable_value(&mut s.family, v, label);
        }
    });
    for (high, label) in [
        (true, "High-Pass Cutoff (Hz)"),
        (false, "Low-Pass Cutoff (Hz)"),
    ] {
        help::heading(ui, label);
        let is_iir = s.edge_family(high) == Family::Iir;
        ui.push_id(label, |ui| {
            ui.horizontal_wrapped(|ui| {
                let (on, hz, order) = if high {
                    (&mut s.high_pass, &mut s.high_hz, &mut s.order)
                } else {
                    (&mut s.low_pass, &mut s.low_hz, &mut s.low_order)
                };
                ui.checkbox(on, "Enabled");
                ui.add_enabled(
                    *on,
                    egui::DragValue::new(hz)
                        .speed(if high { 0.1 } else { 0.5 })
                        .range(0.001..=100000.)
                        .min_decimals(1),
                );
                if is_iir {
                    let choices = [
                        (2, "24 dB/oct"),
                        (4, "48 dB/oct"),
                        (6, "72 dB/oct"),
                        (8, "96 dB/oct"),
                    ];
                    select(ui, "Slope", order, &choices);
                } else {
                    ui.label("FIR");
                }
            })
        });
    }
    if s.family != Family::Fir {
        select(
            ui,
            "IIR Design",
            &mut s.options.design,
            &[
                (IirDesign::Butterworth, "Butterworth"),
                (IirDesign::Elliptic, "Elliptic"),
            ],
        );
        if s.options.design == IirDesign::Elliptic {
            number(
                ui,
                "Passband ripple (dB)",
                &mut s.options.ripple,
                0.001..=10.,
                0.01,
            );
            number(
                ui,
                "Stopband attenuation (dB)",
                &mut s.options.stop_db,
                1.0..=120.,
                1.,
            );
        }
        ui.small("IIR slopes include both zero-phase passes.");
    }
    if s.family == Family::Auto {
        number(
            ui,
            "IIR high-pass crossover (Hz)",
            &mut s.options.crossover,
            0.001..=100.,
            0.1,
        );
        select(
            ui,
            "Crossover rule",
            &mut s.options.through,
            &[(false, "Below"), (true, "At or below")],
        );
    }
    if s.family != Family::Iir || s.options.fir_notch {
        ui.separator();
        select(
            ui,
            "FIR Window",
            &mut s.options.window,
            &[
                (FirWindow::Hamming, "Hamming"),
                (FirWindow::Kaiser, "Kaiser"),
            ],
        );
        if s.options.window == FirWindow::Kaiser {
            number(
                ui,
                "Kaiser attenuation (dB)",
                &mut s.options.attenuation,
                21.0..=120.,
                1.,
            );
        }
        select(
            ui,
            "Application",
            &mut s.options.application,
            &[
                (FirApplication::ZeroPhase, "Zero phase (two pass)"),
                (
                    FirApplication::DelayCompensated,
                    "Single pass (delay compensated)",
                ),
                (FirApplication::Forward, "Forward only (causal)"),
            ],
        );
        select(
            ui,
            "Design rule",
            &mut s.options.rule,
            &[
                (DesignRule::Eva, "EVA"),
                (DesignRule::EeglabMne, "EEGLAB / MNE"),
            ],
        );
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.options.auto_transition, "Automatic transition width");
            help::button(ui, "Automatic transition width");
        });
        if !s.options.auto_transition {
            number(
                ui,
                "Transition width (Hz)",
                &mut s.transition_hz,
                0.001..=10000.,
                0.1,
            );
        }
        for (on, high, name) in [(s.high_pass, true, "HP"), (s.low_pass, false, "LP")] {
            if on && s.edge_family(high) == Family::Fir {
                ui.small(format!("{name}: {} taps", s.edge_taps(rate, high)));
            }
        }
    }
}
fn other_options(ui: &mut egui::Ui, s: &mut Settings, r: &eva_mff::Recording) {
    help::heading(ui, "Line Noise");
    ui.horizontal_wrapped(|ui| {
        for (v, label) in [
            (LineMode::Off, "Off"),
            (LineMode::Notch, "Notch"),
            (LineMode::CleanLine, "CleanLine"),
        ] {
            ui.selectable_value(&mut s.line, v, label);
        }
    });
    if s.line != LineMode::Off {
        egui::CollapsingHeader::new("Line Noise Options")
            .default_open(true)
            .show(ui, |ui| {
                number(ui, "Frequency (Hz)", &mut s.line_hz, 1.0..=10000., 0.5);
                ui.horizontal(|ui| {
                    ui.label("Harmonics");
                    ui.add(egui::DragValue::new(&mut s.harmonics).range(1..=4));
                    help::button(ui, "Harmonics");
                });
                if s.line == LineMode::Notch {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut s.options.fir_notch, "Use FIR notch");
                        help::button(ui, "Use FIR notch");
                    });
                    if s.options.fir_notch {
                        ui.small("1 Hz stopband per harmonic · FIR settings at left");
                    } else {
                        number(ui, "Notch Q", &mut s.notch_q, 1.0..=1000., 1.);
                    }
                } else {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Slider::new(&mut s.window_seconds, 0.5..=10.)
                                .suffix(" s")
                                .text("Window"),
                        );
                        help::button(ui, "Window");
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(&mut s.strength, 0.1..=1.5).text("Strength"));
                        help::button(ui, "Strength");
                    });
                    ui.small("EVA adaptive sinusoid regression; not EEGLAB multitaper.");
                }
            });
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.checkbox(
            &mut s.options.average_reference,
            "Average reference (EEG only)",
        );
        help::button(ui, "Average reference (EEG only)");
    });
    if s.options.average_reference {
        ui.small(
            "All EEG channels contribute unless excluded here. Hidden traces still contribute.",
        );
        egui::ComboBox::from_label("Reference exclusions")
            .selected_text(format!("{} excluded", s.options.excluded.len()))
            .show_ui(ui, |ui| {
                for ch in &r.metadata().channels {
                    let mut excluded = s.options.excluded.contains(&ch.index);
                    if ui.checkbox(&mut excluded, &ch.label).changed() {
                        s.options.excluded.retain(|v| *v != ch.index);
                        if excluded {
                            s.options.excluded.push(ch.index);
                        }
                    }
                }
            });
    }
    if let Some(pns) = r.pns() {
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.options.filter_pns, "Filter PNS");
            help::button(ui, "Filter PNS");
        });
        ui.small(format!(
            "PNS: {} channels · {} Hz",
            pns.metadata().channels.len(),
            pns.metadata().sample_rate
        ));
    }
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_label("Presets")
            .selected_text("Apply package approximation…")
            .show_ui(ui, |ui| {
                for p in [
                    Preset::Eeglab,
                    Preset::Erplab,
                    Preset::MnePython,
                    Preset::NetStation,
                ] {
                    if ui.button(p.label()).clicked() {
                        s.preset(p);
                        ui.close();
                    }
                }
            });
        help::button(ui, "Presets");
    });
    ui.small("Presets set design mechanics; cutoffs, line noise and reference stay unchanged.");
    select(
        ui,
        "Precision",
        &mut s.options.precision,
        &[
            (Precision::Auto, "Auto"),
            (Precision::Float, "Float"),
            (Precision::Double, "Double"),
        ],
    );
    ui.small("Auto uses Double for stable recursive filtering. FIR uses Double in all modes.");
    ui.small(format!(
        "EEG: {} Hz · source samples unchanged",
        r.metadata().sample_rate
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_expanded_options_fit_default_window_without_scrolling() {
        let r = eva_mff::Recording::open(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../eva-mff/tests/fixtures/example_3.mff"),
        )
        .unwrap();
        let ctx = egui::Context::default();
        for line in [LineMode::Notch, LineMode::CleanLine] {
            let mut s = Settings {
                family: Family::Auto,
                line,
                ..Default::default()
            };
            s.options.design = IirDesign::Elliptic;
            s.options.window = FirWindow::Kaiser;
            s.options.fir_notch = true;
            s.options.average_reference = true;
            for _ in 0..3 {
                let mut height = 0.;
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280., 820.),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        ui.set_width(900.);
                        ui.spacing_mut().item_spacing = egui::vec2(10., 9.);
                        let top = ui.cursor().top();
                        ui.columns(2, |cols| {
                            passband(&mut cols[0], &mut s, r.metadata().sample_rate);
                            other_options(&mut cols[1], &mut s, &r);
                        });
                        height = ui.cursor().top() - top;
                    },
                );
                out.textures_delta.clear();
                assert!(height <= 640., "expanded filter content height {height}");
            }
        }
    }
}
