use super::*;
use eva_core::psa::{self, Config, ResultData};
use std::sync::{
    atomic::{AtomicU32, AtomicU64, Ordering},
    mpsc,
};
pub struct State {
    pub open: bool,
    pub config: Config,
    pub running: bool,
    pub result: Option<Arc<ResultData>>,
    pub show_results: bool,
    pub error: Option<String>,
    pub category_text: String,
    pub bad_text: String,
    pub reject_text: String,
    pub reject_epochs: String,
    pub count_mode: bool,
    pub max_count: usize,
    pub selection: usize,
    pub channel: usize,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicU64>,
    receiver: Option<mpsc::Receiver<Result<ResultData, String>>>,
    export_receiver: Option<mpsc::Receiver<Result<(), String>>>,
    source_path: Option<PathBuf>,
    previous_xml: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            config: Config::default(),
            running: false,
            result: None,
            show_results: false,
            error: None,
            category_text: String::new(),
            bad_text: String::new(),
            reject_text: String::new(),
            reject_epochs: String::new(),
            count_mode: false,
            max_count: 13,
            selection: 0,
            channel: 0,
            progress: Arc::new(AtomicU32::new(0)),
            cancel: Arc::new(AtomicU64::new(1)),
            receiver: None,
            export_receiver: None,
            source_path: None,
            previous_xml: None,
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.fetch_add(1, Ordering::Relaxed);
    }
}
fn number(ui: &mut egui::Ui, label: &str, v: &mut f64, help: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(v).speed(0.01));
        ui.small_button("?").on_hover_text(help);
    });
}
impl Viewer {
    pub(super) fn start_psa(&mut self, ctx: &egui::Context) {
        let Some(r) = self.eeg_recording.clone() else {
            return;
        };
        let parsed = (|| -> Result<(), String> {
            self.psa.config.bad_channels = psa::cli::numbers(&self.psa.bad_text)?
                .iter()
                .map(|v| v - 1)
                .collect();
            self.psa.config.reject_epochs = psa::cli::numbers(&self.psa.reject_epochs)?;
            self.psa.config.reject_codes = psa::cli::list(&self.psa.reject_text);
            self.psa.config.categories.clear();
            for item in psa::cli::list(&self.psa.category_text) {
                let (a, b) = item.split_once('=').ok_or("Use categories A=X,B=X")?;
                if a.is_empty() || b.is_empty() {
                    return Err("Empty category mapping".into());
                }
                self.psa.config.categories.insert(a.into(), b.into());
            }
            self.psa.config.max_bad_count = self.psa.count_mode.then_some(self.psa.max_count);
            self.psa.config.validate(&r)?;
            Ok(())
        })();
        if let Err(e) = parsed {
            self.psa.error = Some(e);
            return;
        }
        self.psa.source_path = self.path.clone();
        self.psa.previous_xml = None;
        if let Some(path) = &self.path {
            let path = path.join("eva.xml");
            if path.exists() {
                match std::fs::metadata(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|m| {
                        if m.len() > 32 * 1024 * 1024 {
                            Err("Source eva.xml exceeds 32 MiB".into())
                        } else {
                            std::fs::read_to_string(path).map_err(|e| e.to_string())
                        }
                    }) {
                    Ok(xml) => self.psa.previous_xml = Some(xml),
                    Err(e) => {
                        self.psa.error = Some(e);
                        return;
                    }
                }
            }
        }
        let config = self.psa.config.clone();
        let filters = self.effective_filters();
        let source = self.corrected_source();
        let mri_description = if source.is_some() {
            self.mri
                .corrected
                .as_ref()
                .map(|v| format!("{}; {}", v.description, v.configuration))
        } else {
            None
        };
        let context = ctx.clone();
        let cancel = self.psa.cancel.clone();
        let id = cancel.load(Ordering::Relaxed);
        let progress = self.psa.progress.clone();
        progress.store(0, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.psa.receiver = Some(rx);
        self.psa.running = true;
        self.psa.error = None;
        std::thread::spawn(move || {
            let mut result = psa::run(&r, &config, &filters, source, &cancel, id, &|v| {
                progress.store((v * 1000.) as u32, Ordering::Relaxed);
                context.request_repaint();
            });
            if let Ok(r) = &mut result {
                r.source_processing = format!(
                    "MRI: {}; filter command: {}",
                    mri_description.unwrap_or_else(|| "none".into()),
                    format_args!(
                        "{}; interpolation={:?}",
                        filter::cli::arguments(&filters).join(" "),
                        filters.interpolation
                    )
                );
            }
            let _ = tx.send(result);
            context.request_repaint();
        });
    }
    pub fn psa_dialog(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.psa.receiver {
            if let Ok(result) = rx.try_recv() {
                self.psa.receiver = None;
                self.psa.running = false;
                match result {
                    Ok(result) => {
                        self.psa.result = Some(Arc::new(result));
                        self.psa.show_results = true;
                        self.psa.open = false;
                        self.psa.selection = 0;
                        self.psa.channel = 0;
                    }
                    Err(e) => self.psa.error = Some(e),
                }
            }
        }
        if let Some(rx) = &self.psa.export_receiver {
            if let Ok(result) = rx.try_recv() {
                self.psa.export_receiver = None;
                self.psa.error = result.err();
            }
        }
        if self.psa.open {
            let mut apply = false;
            let mut close = false;
            egui::Modal::new(egui::Id::new("psa_config")).show(ctx,|ui|{
                ui.set_width(850.);ui.heading("PSA · Segmentation and averaging");
                ui.small("Works on the current EEG / MRI / filter snapshot. Acquisition gaps always reject an epoch.");
                if let Some(error)=&self.psa.error {ui.colored_label(GOLD,error);}
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-180.).max(250.)).show(ui,|ui|{
                ui.add_enabled_ui(!self.psa.running,|ui|{
                ui.columns(2,|cols|{
                    let ui=&mut cols[0];ui.strong("Events and categories");
                    if ui.checkbox(&mut self.psa.config.by_label,"Select by label").changed(){self.psa.config.events.clear();}
                    let codes:std::collections::BTreeSet<_>=self.eeg_recording.as_ref().map(|r|r.metadata().events.iter().map(|e|if self.psa.config.by_label {e.label.clone().unwrap_or(e.code.clone())} else {e.code.clone()}).collect()).unwrap_or_default();
                    egui::ScrollArea::vertical().id_salt("psa_events").max_height(130.).show(ui,|ui|{
                        for code in codes {let mut selected=self.psa.config.events.contains(&code);if ui.checkbox(&mut selected,&code).changed(){if selected{self.psa.config.events.push(code);}else{self.psa.config.events.retain(|v|v!=&code);}}}
                    });
                    ui.label("Category grouping (A=X,B=X)");ui.text_edit_singleline(&mut self.psa.category_text);
                    number(ui,"Pre (s)",&mut self.psa.config.pre,"Positive time before the shifted event anchor.");
                    number(ui,"Post (s)",&mut self.psa.config.post,"Time after anchor; endpoint excluded.");
                    number(ui,"Offset (s)",&mut self.psa.config.offset,"Added to original event time. Events themselves are unchanged.");
                    ui.checkbox(&mut self.psa.config.baseline,"Baseline correct");
                    number(ui,"Baseline start (s)",&mut self.psa.config.baseline_start,"Relative to anchor; must lie inside epoch.");
                    number(ui,"Baseline end (s)",&mut self.psa.config.baseline_end,"Exclusive endpoint. Default 0 excludes stimulus sample.");
                    ui.checkbox(&mut self.psa.config.average_reference,"Average reference").on_hover_text("Exclude global bad channels from reference, even after interpolation. Transient repaired channels can enter reference.");
                    ui.checkbox(&mut self.psa.config.average,"Average by category");
                    ui.label("Artifact event codes to reject");ui.text_edit_singleline(&mut self.psa.reject_text).on_hover_text("Comma-separated exact codes; any overlap rejects the epoch. No automated blink detector is run.");
                    ui.label("Exclude candidate epochs (1,8,…)");ui.text_edit_singleline(&mut self.psa.reject_epochs);
                    let ui=&mut cols[1];ui.strong("Bad channels and thresholds");
                    ui.checkbox(&mut self.psa.config.threshold,"Detect threshold artifacts");
                    number(ui,"Minimum (µV)",&mut self.psa.config.min_uv,"Amplitude after subtracting baseline mean when enabled; detection precedes average reference.");
                    number(ui,"Maximum (µV)",&mut self.psa.config.max_uv,"A sample outside bounds flags that channel for the epoch.");
                    number(ui,"Slope (µV/sample)",&mut self.psa.config.slope,"Maximum absolute adjacent-sample difference; not µV/second.");
                    number(ui,"Acceleration (µV/sample²)",&mut self.psa.config.acceleration,"Maximum absolute second difference.");
                    ui.checkbox(&mut self.psa.count_mode,"Use absolute bad-channel count");
                    if self.psa.count_mode {ui.add(egui::DragValue::new(&mut self.psa.max_count).prefix("Maximum count "));}else{number(ui,"Maximum bad (%)",&mut self.psa.config.max_bad_percent,"Reject if transient flagged count exceeds this percentage of all EEG channels.");}
                    ui.checkbox(&mut self.psa.config.interpolate_epoch,"Interpolate bad channels per epoch");
                    ui.label("Global bad channels (1,8,…)");ui.text_edit_singleline(&mut self.psa.bad_text);
                    ui.checkbox(&mut self.psa.config.interpolate_global,"Interpolate global bad channels");
                    ui.checkbox(&mut self.psa.config.escalate,"Escalate recurrent bad channels to global");
                    number(ui,"Flagged in ≥ (%)",&mut self.psa.config.escalation_percent,"Percentage of measurable candidates, before threshold rejection. Uses a second pass with all global bad channels excluded from donors.");
                    ui.small_button("? Spherical splines").on_hover_text("Perrin order 4, 40 terms, lambda 1e-5. Requires scalp coordinates and at least 3 good donors. Targets never donate to one another. Impossible interpolation rejects the epoch.");
                    if ui.button("Reset thresholds").clicked(){let d=Config::default();self.psa.config.min_uv=d.min_uv;self.psa.config.max_uv=d.max_uv;self.psa.config.slope=d.slope;self.psa.config.acceleration=d.acceleration;self.psa.config.max_bad_percent=d.max_bad_percent;}
                });});});
                ui.separator();
                if self.psa.running {ui.add(egui::ProgressBar::new(self.psa.progress.load(Ordering::Relaxed) as f32/1000.));}
                ui.horizontal(|ui|{
                    if ui.button(if self.psa.running {"Cancel processing"}else{"Cancel"}).clicked(){if self.psa.running {self.psa.cancel.fetch_add(1,Ordering::Relaxed);}else{close=true;}}
                    if ui.add_enabled(!self.psa.running,egui::Button::new("Apply PSA")).clicked(){apply=true;}
                });
            });
            if close {
                self.psa.open = false;
            }
            if apply {
                self.start_psa(ctx);
            }
        }
        if self.psa.show_results {
            self.psa_results(ctx);
        }
    }
    fn psa_results(&mut self, ctx: &egui::Context) {
        let Some(result) = self.psa.result.clone() else {
            return;
        };
        let mut open = self.psa.show_results;
        let mut export = false;
        egui::Window::new("PSA results · processing snapshot")
            .open(&mut open)
            .default_size([1000., 720.])
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} / {} accepted · {} category averages · global bad: {:?}",
                    result.epochs.len(),
                    result.decisions.len(),
                    result.averages.len(),
                    result.global_bad
                ));
                ui.collapsing("Source processing settings", |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(100.)
                        .show(ui, |ui| {
                            ui.small(&result.source_processing);
                        });
                });
                for warning in &result.warnings {
                    ui.colored_label(GOLD, warning);
                }
                if let Some(error) = &self.psa.error {
                    ui.colored_label(GOLD, error);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.psa.export_receiver.is_none(),
                            egui::Button::new("Export PSA bundle…"),
                        )
                        .clicked()
                    {
                        export = true;
                    }
                    if ui.button("Configure again").clicked() {
                        self.psa.open = true;
                    }
                });
                let count = result.averages.len() + result.epochs.len();
                if count > 0 {
                    self.psa.selection = self.psa.selection.min(count - 1);
                    let labels: Vec<_> = result
                        .averages
                        .iter()
                        .map(|a| format!("Average {} (n={})", a.category, a.count))
                        .chain(result.epochs.iter().map(|e| {
                            let d = &result.decisions[e.decision];
                            format!("Epoch {} · {}", d.candidate, d.category)
                        }))
                        .collect();
                    egui::ComboBox::from_label("Display")
                        .selected_text(&labels[self.psa.selection])
                        .show_ui(ui, |ui| {
                            for (i, label) in labels.iter().enumerate() {
                                ui.selectable_value(&mut self.psa.selection, i, label);
                            }
                        });
                    ui.add(
                        egui::Slider::new(
                            &mut self.psa.channel,
                            0..=result.channel_names.len() - 1,
                        )
                        .text("First channel (zero-based)"),
                    );
                    let data = if self.psa.selection < result.averages.len() {
                        &result.averages[self.psa.selection].values
                    } else {
                        &result.epochs[self.psa.selection - result.averages.len()].values
                    };
                    for (ch, row) in data.iter().enumerate().skip(self.psa.channel).take(6) {
                        ui.label(format!(
                            "{} · {}",
                            result.channel_names[ch],
                            result.channel_units[ch]
                                .as_deref()
                                .unwrap_or("unknown unit")
                        ));
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 48.),
                            egui::Sense::hover(),
                        );
                        let peak = row.iter().map(|v| v.abs()).fold(0f32, f32::max).max(1e-6);
                        let x0 = rect.left()
                            + rect.width() * result.pre_samples as f32 / result.samples as f32;
                        ui.painter().line_segment(
                            [Pos2::new(x0, rect.top()), Pos2::new(x0, rect.bottom())],
                            Stroke::new(1., GOLD),
                        );
                        let stride = row.len().div_ceil(1000).max(1);
                        // Min/max bins preserve transient peaks without painting every sample.
                        for (bin, chunk) in row.chunks(stride).enumerate() {
                            let x = rect.left()
                                + rect.width() * (bin * stride) as f32 / row.len() as f32;
                            let lo = chunk.iter().copied().fold(f32::INFINITY, f32::min);
                            let hi = chunk.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                            ui.painter().line_segment(
                                [
                                    Pos2::new(x, rect.center().y - lo / peak * 22.),
                                    Pos2::new(x, rect.center().y - hi / peak * 22. - 0.5),
                                ],
                                Stroke::new(1., INK),
                            );
                        }
                    }
                    ui.small(format!(
                        "Time: {:.3} to {:.3} s · orange = event anchor · each row auto-scaled",
                        -(result.pre_samples as f64) / f64::from(result.sample_rate),
                        (result.samples - result.pre_samples) as f64
                            / f64::from(result.sample_rate)
                    ));
                }
                ui.collapsing("Epoch decisions and rejection reasons", |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(180.)
                        .show(ui, |ui| {
                            for d in &result.decisions {
                                ui.label(format!(
                                    "{} · {} · {} · flagged {:?} · repaired {:?}",
                                    d.candidate,
                                    d.category,
                                    d.reason,
                                    d.flagged_channels,
                                    d.repaired_channels
                                ));
                            }
                        });
                });
            });
        self.psa.show_results = open;
        if export {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Export new PSA result directory")
                .set_file_name("results.psa")
                .save_file()
            {
                let source = self
                    .psa
                    .source_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // Never create an analysis bundle inside its source MFF package.
                let safe =
                    std::fs::canonicalize(path.parent().unwrap_or(std::path::Path::new(".")))
                        .and_then(|parent| {
                            std::fs::canonicalize(&source).map(|source| !parent.starts_with(source))
                        });
                if !matches!(safe, Ok(true)) {
                    self.psa.error = Some("Export outside the source MFF package".into());
                    return;
                }
                let xml = self.psa.previous_xml.clone();
                let context = ctx.clone();
                let (tx, rx) = mpsc::channel();
                self.psa.export_receiver = Some(rx);
                std::thread::spawn(move || {
                    let r = psa::export::write(&result, &path, &source, xml.as_deref());
                    let _ = tx.send(r);
                    context.request_repaint();
                });
            }
        }
    }
}
