use super::*;
use eva_core::ocular::{self, Config, Kind, Polarity, Topology};
use std::sync::{
    atomic::{AtomicU32, AtomicU64, Ordering},
    mpsc,
};
pub struct State {
    pub open: bool,
    pub running: bool,
    pub settings: ocular::Settings,
    pub result: Option<ocular::Report>,
    tab: usize,
    channels: [String; 2],
    error: Option<String>,
    cancel: Arc<AtomicU64>,
    progress: Arc<AtomicU32>,
    receiver: Option<mpsc::Receiver<Result<ocular::Report, String>>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            running: false,
            settings: Default::default(),
            result: None,
            tab: 0,
            channels: Default::default(),
            error: None,
            cancel: Arc::new(AtomicU64::new(1)),
            progress: Arc::new(AtomicU32::new(0)),
            receiver: None,
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
fn float(ui: &mut egui::Ui, label: &str, v: &mut f32, help: &str) {
    let mut value = *v as f64;
    number(ui, label, &mut value, help);
    *v = value as f32;
}
fn fields(ui: &mut egui::Ui, c: &mut Config, text: &mut String) {
    ui.checkbox(&mut c.enabled, "Enable detection");
    ui.add_enabled_ui(c.enabled,|ui|{
        ui.horizontal(|ui|{ui.strong("Signal");ui.selectable_value(&mut c.topology,Topology::Legacy,"Legacy max channel");ui.selectable_value(&mut c.topology,Topology::Derived,"Derived VEOG/HEOG");ui.small_button("?").on_hover_text("Legacy: signed largest absolute sample among ocular channels. Derived: same-sign bilateral mean for blinks; left minus right for movements when opposition exceeds common-mode activity. No baseline removal is added.");});
        ui.columns(2,|cols|{
            let ui=&mut cols[0];ui.strong("Amplitude");
            float(ui,"Minimum (µV)",&mut c.minimum_uv,"Magnitude that opens a threshold run, inclusive.");
            float(ui,"Maximum (µV)",&mut c.maximum_uv,"Reject peaks above this cap. 0 = no cap.");
            ui.horizontal(|ui|{ui.label("Polarity");for (v,s) in [(Polarity::Positive,"Positive"),(Polarity::Negative,"Negative"),(Polarity::Bipolar,"Bipolar")]{ui.selectable_value(&mut c.polarity,v,s);}});
            ui.strong("Kinematics");ui.checkbox(&mut c.velocity_enabled,"Velocity gate");
            float(ui,"Minimum (µV/ms)",&mut c.velocity_uv_ms,"Minimum maximum absolute first difference across the run, scaled by samples/ms.");
            ui.checkbox(&mut c.acceleration_enabled,"Acceleration gate");
            float(ui,"Minimum (µV/ms²)",&mut c.acceleration_uv_ms2,"Minimum maximum absolute second difference across the run. Optional sharpness gate, matching Swift EVA.");
            let ui=&mut cols[1];ui.strong("Timing");
            number(ui,"Rise window (s)",&mut c.rise_seconds,"Time from first threshold crossing to largest absolute peak. 0 = unconstrained.");
            number(ui,"Min duration (s)",&mut c.min_seconds,"Minimum merged run duration, including bridged dips.");
            number(ui,"Max duration (s)",&mut c.max_seconds,"0 = no cap. Merging cannot create a run exceeding this duration.");
            number(ui,"Merge gap (s)",&mut c.merge_seconds,"Swift rule: distance between previous last crossing and next first crossing ≤ rounded gap samples (at least one). Never bridges acquisition gaps or nonfinite data.");
        });
        ui.separator();ui.strong("Ocular channels (one-based)");ui.horizontal(|ui|{ui.text_edit_singleline(text);if ui.button("Use Auto").clicked(){text.clear();}});
        ui.small("Blank = Swift EGI montage defaults. Derived overrides: first half left, remainder right, in entered order.");
    });
}
impl Viewer {
    pub(super) fn start_ocular(&mut self, ctx: &egui::Context) {
        let Some(r) = self.eeg_recording.clone() else {
            return;
        };
        for (c, text) in [
            (&mut self.ocular.settings.blink, &self.ocular.channels[0]),
            (&mut self.ocular.settings.movement, &self.ocular.channels[1]),
        ] {
            match ocular::cli::channels(text) {
                Ok(v) => c.channels = v,
                Err(e) => {
                    self.ocular.error = Some(e);
                    return;
                }
            }
            if let Err(e) = c.validate() {
                self.ocular.error = Some(e);
                return;
            }
        }
        let settings = self.ocular.settings.clone();
        let filters = self.effective_filters();
        let source = self.corrected_source();
        let mri = if source.is_some() {
            self.mri
                .corrected
                .as_ref()
                .map(|v| format!("{}; {}", v.description, v.configuration))
                .unwrap_or_default()
        } else {
            "none".into()
        };
        let path = self.path.clone();
        let cancel = self.ocular.cancel.clone();
        let id = cancel.load(Ordering::Relaxed);
        let progress = self.ocular.progress.clone();
        progress.store(0, Ordering::Relaxed);
        let ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        self.ocular.receiver = Some(rx);
        self.ocular.running = true;
        self.ocular.error = None;
        std::thread::spawn(move || {
            let mut result = ocular::run(&r, &settings, &filters, source, &cancel, id, &|v| {
                progress.store((v * 1000.) as u32, Ordering::Relaxed);
                ctx.request_repaint();
            });
            if let Ok(report) = &mut result {
                report.source = path
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                report.source_processing = format!("MRI: {mri}; {}", report.source_processing);
            }
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    pub fn ocular_dialog(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.ocular.receiver {
            if let Ok(result) = rx.try_recv() {
                self.ocular.running = false;
                self.ocular.receiver = None;
                match result {
                    Ok(v) => self.ocular.result = Some(v),
                    Err(e) => self.ocular.error = Some(e),
                }
            }
        }
        if !self.ocular.open {
            return;
        }
        let mut apply = false;
        let mut export = false;
        let mut close = false;
        let modal=egui::Modal::new(egui::Id::new("ocular_sheet")).show(ctx,|ui|{
            ui.set_width(810.);ui.heading("Ocular Artifact Thresholds");
            ui.label("Detect eye blinks / vertical activity and horizontal eye movements independently.");
            ui.small("Uses the current EEG / MRI / filter snapshot. Detection only; EEG stays unchanged.");
            if let Some(error)=&self.ocular.error{ui.colored_label(GOLD,error);}
            egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-210.).max(200.)).show(ui,|ui|{
                ui.add_enabled_ui(!self.ocular.running,|ui|{
                    ui.horizontal(|ui|{ui.selectable_value(&mut self.ocular.tab,0,"Eye Blink / Vertical");ui.selectable_value(&mut self.ocular.tab,1,"Eye Movement / Horizontal");});
                    let kind=if self.ocular.tab==0{Kind::Blink}else{Kind::Movement};
                    let c=if self.ocular.tab==0{&mut self.ocular.settings.blink}else{&mut self.ocular.settings.movement};
                    fields(ui,c,&mut self.ocular.channels[self.ocular.tab]);
                    if let Some(r)=&self.eeg_recording {let (l,rr,known)=ocular::auto_groups(kind,r.metadata().channels.len(),r.metadata().sensor_layout_name.as_deref());ui.small(format!("Auto channels: {:?} / {:?}{}",l,rr,if known{""}else{" (unknown montage: verify fallback)"}));}
                });
                if let Some(report)=&self.ocular.result{
                    ui.separator();ui.strong(format!("{} detections from last completed snapshot",report.candidates.len()));
                    ui.small("Rerun detection after changing filters, MRI correction, interpolation, or thresholds.");
                    for warning in &report.warnings{ui.colored_label(GOLD,warning);}
                    egui::ScrollArea::vertical().id_salt("ocular_results").max_height(130.).show_rows(ui,24.,report.candidates.len(),|ui,range|{for i in range{let c=&report.candidates[i];if ui.button(format!("{} · {:.3}–{:.3} s · peak {:.3} s ({:.1} µV)",c.code,c.start_seconds,c.end_seconds,c.peak_seconds,c.peak_uv)).clicked(){self.navigation.jump(c.peak_sample);close=true;}}});
                }
            });
            ui.separator();ui.horizontal(|ui|{
                if self.ocular.running {ui.add(egui::ProgressBar::new(self.ocular.progress.load(Ordering::Relaxed) as f32/1000.).desired_width(240.));if ui.button("Cancel detection").clicked(){self.ocular.cancel.fetch_add(1,Ordering::Relaxed);}}
                else {if ui.button("Restore Defaults").clicked(){self.ocular.settings=Default::default();self.ocular.channels=Default::default();}apply=ui.button("Detect Artifacts").clicked();export=ui.add_enabled(self.ocular.result.is_some(),egui::Button::new("Export JSON…")).clicked();}
                if ui.button("Close").clicked(){close=true;}
            });
        });
        if close || modal.should_close() {
            self.ocular.open = false;
        }
        if apply {
            self.start_ocular(ctx);
        }
        if export {
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name("ocular-artifacts.json")
                .save_file()
            {
                let inside = self
                    .path
                    .as_ref()
                    .and_then(|p| std::fs::canonicalize(p).ok())
                    .is_some_and(|p| {
                        path.parent()
                            .and_then(|p| std::fs::canonicalize(p).ok())
                            .is_some_and(|parent| parent.starts_with(p))
                    });
                self.ocular.error = if inside {
                    Some("Export outside the source MFF".into())
                } else {
                    ocular::write_report(self.ocular.result.as_ref().unwrap(), &path).err()
                };
            }
        }
    }
}
