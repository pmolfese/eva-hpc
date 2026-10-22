#![forbid(unsafe_code)]
use eva_core::filter;
mod filter_help;
mod filter_ui;
mod model;
mod mri;
mod mri_ui;
mod psa_ui;
#[cfg(test)]
mod tests;
mod worker;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use eva_mff::{Recording, RecordingKind};
use model::{DisplayData, Navigation, ViewKey};
use std::{path::PathBuf, sync::Arc};
use worker::{Job, Output, Worker};

const INK: Color32 = Color32::from_rgb(0, 112, 255);
const GOLD: Color32 = Color32::from_rgb(206, 91, 13);
struct Viewer {
    worker: Worker,
    mri: mri::State,
    psa: psa_ui::State,
    active_id: u64,
    recording: Option<Arc<Recording>>,
    path: Option<PathBuf>,
    navigation: Navigation,
    hidden: Vec<bool>,
    bad: std::collections::BTreeSet<usize>,
    interpolated: Vec<usize>,
    interpolation: Vec<eva_core::interpolation::Recipe>,
    first_channel: usize,
    rows: usize,
    amplitude: f64,
    center: bool,
    wanted: Option<ViewKey>,
    display: Option<DisplayData>,
    busy: Option<&'static str>,
    error: Option<String>,
    exact: Option<(usize, u64, f32)>,
    event_filter: String,
    channel_filter: String,
    tab: usize,
    inspector_open: bool,
    backend: String,
    filters: filter::Settings,
    filter_draft: filter::Settings,
    filter_error: Option<String>,
    filter_original: Option<filter::Settings>,
    eeg_recording: Option<Arc<Recording>>,
}
impl Viewer {
    fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        let backend = cc
            .wgpu_render_state
            .as_ref()
            .map(|s| {
                let i = s.adapter.get_info();
                format!("{:?} · {}", i.backend, i.name)
            })
            .unwrap_or_else(|| "No GPU adapter".into());
        eprintln!("EVA Viewer graphics: {backend}");
        let mut app = Self {
            worker: Worker::new(cc.egui_ctx.clone()),
            mri: mri::State::default(),
            psa: psa_ui::State::default(),
            active_id: 0,
            recording: None,
            path: None,
            navigation: Navigation::new(1, 1),
            hidden: Vec::new(),
            bad: Default::default(),
            interpolated: vec![],
            interpolation: vec![],
            first_channel: 0,
            rows: 10,
            amplitude: 100.0,
            center: true,
            wanted: None,
            display: None,
            busy: None,
            error: None,
            exact: None,
            event_filter: String::new(),
            channel_filter: String::new(),
            tab: 0,
            inspector_open: false,
            backend,
            filters: filter::Settings::default(),
            filter_draft: filter::Settings::default(),
            filter_error: None,
            filter_original: None,
            eeg_recording: None,
        };
        if let Some(p) = path {
            app.open(p);
        }
        app
    }
    fn report_bad(&self) {
        if !self.bad.is_empty() {
            eprintln!(
                "Bad EEG channels for {}:",
                self.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
            println!(
                "{}",
                self.bad
                    .iter()
                    .map(|c| (c + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }
    fn rebuild_interpolation(&mut self) {
        if let Some(r) = &self.eeg_recording {
            match eva_core::interpolation::recipes(
                r,
                &self.interpolated,
                &self.bad.iter().copied().collect::<Vec<_>>(),
            ) {
                Ok(recipes) => {
                    self.interpolation = recipes;
                    self.error = None;
                }
                Err(e) => {
                    self.interpolation.clear();
                    self.error = Some(e);
                }
            }
        }
        // Keep an interpolation error visible while invalidating stale samples.
        let error = self.error.take();
        self.invalidate_signal();
        self.error = error;
    }
    fn interpolate_channel(&mut self, ch: usize) {
        let previous_targets = self.interpolated.clone();
        let previous_recipes = self.interpolation.clone();
        if !self.interpolated.contains(&ch) {
            self.interpolated.push(ch);
        }
        self.rebuild_interpolation();
        if self.error.is_some() {
            self.interpolated = previous_targets;
            self.interpolation = previous_recipes;
        }
    }
    fn mark_bad(&mut self, ch: usize) {
        if !self.bad.remove(&ch) {
            self.bad.insert(ch);
        }
        self.rebuild_interpolation();
    }
    fn close(&mut self) {
        self.report_bad();
        self.bad.clear();
        self.interpolated.clear();
        self.interpolation.clear();
        self.mri = mri::State::default();
        self.psa = psa_ui::State::default();
        self.worker.cancel();
        self.filters = filter::Settings::default();
        self.filter_draft = filter::Settings::default();
        self.filter_error = None;
        self.filter_original = None;
        self.recording = None;
        self.eeg_recording = None;
        self.path = None;
        self.display = None;
        self.wanted = None;
        self.hidden.clear();
        self.exact = None;
        self.busy = None;
        self.error = None;
    }
    fn open(&mut self, path: PathBuf) {
        self.close();
        self.path = Some(path.clone());
        self.busy = Some("Opening MFF and indexing blocks…");
        self.active_id = self.worker.submit(Job::Open(path));
    }
    fn picker(&mut self) {
        let dialog = rfd::FileDialog::new().set_title("Open an EGI MFF recording");
        // macOS presents registered MFF directories as file packages. A folder-only
        // NSOpenPanel disables them; allow both packages and ordinary directories.
        #[cfg(target_os = "macos")]
        let selected = dialog.pick_file_or_folder();
        #[cfg(not(target_os = "macos"))]
        let selected = dialog.pick_folder();
        if let Some(path) = selected {
            self.open(path);
        }
    }
    fn invalidate_signal(&mut self) {
        self.worker.cancel();
        self.wanted = None;
        self.display = None;
        self.exact = None;
        self.busy = None;
        self.error = None;
    }
    fn corrected_source(&self) -> Option<Arc<Vec<Vec<f32>>>> {
        if self.mri.bypass {
            return None;
        }
        let result = self.mri.corrected.as_ref()?;
        if self.recording.as_ref()?.is_pns() {
            result.pns.clone()
        } else {
            Some(result.eeg.clone())
        }
    }
    fn effective_filters(&self) -> filter::Settings {
        let mut s = self.filters.clone();
        if self.recording.as_ref().is_some_and(|r| r.is_pns()) {
            s.options.average_reference = false;
            s.options.excluded.clear();
            if !s.options.filter_pns {
                s.enabled = false;
            }
        }
        if !self.recording.as_ref().is_some_and(|r| r.is_pns()) {
            s.interpolation = self.interpolation.clone();
            s.options.excluded.extend(self.bad.iter().copied());
            s.options.excluded.extend(self.interpolated.iter().copied());
            s.options.excluded.sort_unstable();
            s.options.excluded.dedup();
        }
        s
    }
    fn select_stream(&mut self, pns: bool) {
        let Some(root) = self.eeg_recording.clone() else {
            return;
        };
        let next = if pns {
            let Some(p) = root.pns() else {
                return;
            };
            p.clone()
        } else {
            root
        };
        let old_rate = self
            .recording
            .as_ref()
            .map(|r| r.metadata().sample_rate)
            .unwrap_or(1) as f64;
        let time = self.navigation.start as f64 / old_rate;
        let duration = self.navigation.span as f64 / old_rate;
        self.worker.cancel();
        self.recording = Some(next.clone());
        self.hidden = vec![false; next.metadata().channels.len()];
        self.reset();
        self.navigation.start = (time * next.metadata().sample_rate as f64) as u64;
        self.navigation.span = (duration * next.metadata().sample_rate as f64) as u64;
        self.navigation.clamp();
        self.display = None;
        self.wanted = None;
        self.busy = None;
        self.error = None;
    }
    fn reset(&mut self) {
        if let Some(r) = &self.recording {
            self.navigation = Navigation::new(r.metadata().sample_count, r.metadata().sample_rate);
        }
        self.hidden.fill(false);
        self.first_channel = 0;
        self.rows = 10;
        self.amplitude = 100.;
        self.center = true;
        self.exact = None;
    }
    fn receive(&mut self) {
        let Some(reply) = self.worker.take() else {
            return;
        };
        if reply.id != self.active_id {
            return;
        }
        self.busy = None;
        if self.mri.running {
            self.mri.running = false;
            if let Err(e) = &reply.result {
                self.mri.error = Some(e.clone());
            }
        }
        match reply.result {
            Err(e) => self.error = Some(e),
            Ok(Output::Open(r)) => {
                self.hidden = vec![false; r.metadata().channels.len()];
                if r.metadata().kind == RecordingKind::SegmentedOrAveraged {
                    self.error=Some(format!("{:?} recording detected. Segmented/averaged waveforms are not yet supported.",r.metadata().kind));
                }
                self.eeg_recording = Some(r.clone());
                self.recording = Some(r);
                self.reset();
            }
            Ok(Output::Corrected(data)) => {
                self.mri.corrected = Some(data);
                self.mri.bypass = false;
                self.invalidate_signal();
            }
            Ok(Output::View(d)) => {
                if self.wanted.as_ref() == Some(&d.key) {
                    self.display = Some(*d);
                }
            }
            Ok(Output::Sample {
                channel,
                sample,
                value,
            }) => self.exact = Some((channel, sample, value)),
        }
    }
    fn fit_amplitude(&mut self) {
        if let Some(d) = &self.display {
            let center = self.center;
            self.amplitude = d
                .traces
                .iter()
                .flat_map(|t| {
                    t.bins
                        .iter()
                        .filter(|b| !b.gap && b.min.is_finite())
                        .map(move |b| {
                            let offset = if center { t.mean } else { 0. };
                            (f64::from(b.min) - offset)
                                .abs()
                                .max((f64::from(b.max) - offset).abs())
                        })
                })
                .fold(0.000001, f64::max)
                * 1.15;
        }
    }
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open MFF…    ⌘/Ctrl+O").clicked() {
                    ui.close();
                    self.picker();
                }
                if ui
                    .add_enabled(self.path.is_some(), egui::Button::new("Close recording"))
                    .clicked()
                {
                    ui.close();
                    self.close();
                }
            });
            if ui.small_button("Open MFF…").clicked() {
                self.picker();
            }
            if self.recording.is_some() && ui.small_button("Recording info").clicked() {
                self.tab = 2;
                self.inspector_open = true;
            }
            if self
                .eeg_recording
                .as_ref()
                .is_some_and(|r| r.pns().is_some())
            {
                let current = self.recording.as_ref().is_some_and(|r| r.is_pns());
                if ui.selectable_label(!current, "EEG stream").clicked() && current {
                    self.select_stream(false);
                }
                if ui.selectable_label(current, "PNS stream").clicked() && !current {
                    self.select_stream(true);
                }
            }
            if let Some(p) = &self.path {
                ui.weak(p.file_name().unwrap_or_default().to_string_lossy())
                    .on_hover_text(p.display().to_string());
            }
        });
        ui.add_space(4.);
        ui.horizontal_wrapped(|ui| {
            ui.allocate_ui(Vec2::new(275.,60.), |ui| {
                ui.add_enabled_ui(self.recording.is_some(), |ui| {
                    egui::Grid::new("scale_controls").num_columns(2).show(ui, |ui| {
                        ui.small("Scale");
                        ui.add(egui::Slider::new(&mut self.amplitude,0.001..=10000.).logarithmic(true).prefix("± ")).on_hover_text("Amplitude per row, in each channel's units. Unknown units are not assumed to be µV."); ui.end_row();
                        ui.small("Time scale");
                        let rate=self.recording.as_ref().map(|r|r.metadata().sample_rate).unwrap_or(1);
                        let base=(10.*f64::from(rate)).min(self.navigation.total as f64).max(1.);
                        let mut factor=base/self.navigation.span as f64;
                        if ui.add(egui::Slider::new(&mut factor,0.1..=1000.).logarithmic(true).suffix("×")).changed() {
                            let desired=base/factor;
                            self.navigation.zoom(desired/self.navigation.span as f64,0.5);
                        }
                        ui.end_row();
                    });
                });
            });
            for label in ["MRI", "FILTER", "ARTIFACTS", "PSA", "EEG", "EVENTS"] {
                let tab=match label {"EEG"=>1,"FILTER"=>3,_=>0};
                let implemented=label=="PSA" || label=="MRI" || label=="EEG" || label=="EVENTS" || label=="FILTER";
                let enabled=implemented && self.recording.is_some() && !self.mri.running && !self.psa.running && (label!="PSA" || !self.recording.as_ref().is_some_and(|r|r.is_pns()));
                let selected=enabled && match label {"PSA"=>self.psa.open || self.psa.show_results,"MRI"=>self.mri.open || self.corrected_source().is_some(),"FILTER"=>self.filters.active() || self.filter_original.is_some(),_=>self.inspector_open && self.tab==tab};
                let response=tool_button(ui,label,enabled,selected);
                if response.clicked() {
                    if label=="PSA" { self.psa.open=true; self.psa.bad_text=self.bad.iter().map(|v|(v+1).to_string()).collect::<Vec<_>>().join(","); } else if label=="MRI" { self.mri.open=true; } else if label=="FILTER" { self.open_filter_dialog(); } else { self.inspector_open = !(self.inspector_open && self.tab==tab); self.tab=tab; }
                }
                if !implemented { response.on_disabled_hover_text("Not implemented in this first waveform port"); }
            }
            if self.corrected_source().is_some() {ui.small("MRI corrected");}
            if let Some(r)=&self.recording {
                let m=r.metadata();
                ui.vertical(|ui| {
                    ui.small(format!("{} channels · {} Hz · {:.3} s",m.channels.len(),m.sample_rate,m.duration_seconds()));
                    let unit=m.channels.first().and_then(|c|c.unit.as_deref());
                    let units=if m.channels.iter().all(|c|c.unit.as_deref()==unit) {unit.unwrap_or("units unspecified")} else {"mixed units"};
                    ui.weak(format!("{} · {}", units, if r.is_pns() {"PNS polarity from metadata"} else {"positive up"}));
                    ui.small(self.filters.summary());
                });
            }
        });
    }
    fn navigation_bar(&mut self, ui: &mut egui::Ui) {
        let Some(r) = self.recording.clone() else {
            return;
        };
        let m = r.metadata();
        if m.kind == RecordingKind::SegmentedOrAveraged {
            return;
        }
        ui.horizontal(|ui| {
            if ui
                .small_button("◀")
                .on_hover_text("Previous half-window")
                .clicked()
            {
                self.navigation.pan(-0.5);
            }
            let mut start = self.navigation.start as f64 / f64::from(m.sample_rate);
            let max = (m.sample_count - self.navigation.span) as f64 / f64::from(m.sample_rate);
            let old_width = ui.spacing().slider_width;
            ui.spacing_mut().slider_width = (ui.available_width() - 170.).max(120.);
            if ui
                .add(egui::Slider::new(&mut start, 0.0..=max).show_value(false))
                .on_hover_text("Scroll through recording")
                .changed()
            {
                self.navigation.start = (start * f64::from(m.sample_rate)).round() as u64;
                self.navigation.clamp();
            }
            ui.spacing_mut().slider_width = old_width;
            if ui
                .small_button("▶")
                .on_hover_text("Next half-window")
                .clicked()
            {
                self.navigation.pan(0.5);
            }
            ui.small(format!(
                "{:.3}–{:.3} s",
                self.navigation.start as f64 / f64::from(m.sample_rate),
                self.navigation.range().end as f64 / f64::from(m.sample_rate)
            ));
        });
    }
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let Some(r) = self.recording.clone() else {
            return;
        };
        let m = r.metadata();
        if ui.small_button("Hide panel").clicked() {
            self.inspector_open = false;
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, 0, "Events");
            ui.selectable_value(&mut self.tab, 1, "Channels");
            ui.selectable_value(&mut self.tab, 2, "Info");
        });
        ui.separator();
        match self.tab {
            0 => {
                ui.add(egui::TextEdit::singleline(&mut self.event_filter).hint_text("Find event…"));
                let filter = self.event_filter.to_lowercase();
                let events: Vec<_> = m
                    .events
                    .iter()
                    .filter(|e| {
                        filter.is_empty()
                            || e.code.to_lowercase().contains(&filter)
                            || e.label
                                .as_deref()
                                .unwrap_or("")
                                .to_lowercase()
                                .contains(&filter)
                    })
                    .collect();
                ui.label(format!("{} events", events.len()));
                egui::ScrollArea::vertical().show_rows(ui, 42., events.len(), |ui, range| {
                    for i in range {
                        let e = events[i];
                        let sample = e
                            .sample_index(m.sample_rate)
                            .ok()
                            .filter(|s| *s >= 0 && (*s as u64) < m.sample_count);
                        let text = format!(
                            "{}   {:.6} s\n{}",
                            e.code,
                            e.onset_ns as f64 / 1e9,
                            e.label.as_deref().unwrap_or("")
                        );
                        if ui
                            .add_enabled(
                                sample.is_some() && m.kind != RecordingKind::SegmentedOrAveraged,
                                egui::Button::new(text)
                                    .min_size(Vec2::new(ui.available_width(), 38.)),
                            )
                            .on_hover_text(&e.raw_begin_time)
                            .clicked()
                        {
                            self.navigation.jump(sample.unwrap() as u64);
                        }
                    }
                });
            }
            1 => {
                ui.strong("Display");
                ui.checkbox(&mut self.center,"Center traces").on_hover_text("Subtract the visible window mean for display only. Sample readouts remain unchanged.");
                ui.horizontal(|ui| {
                    ui.label("Rows");
                    ui.add(egui::DragValue::new(&mut self.rows).range(1..=128));
                    if ui.button("Fit amplitude").clicked() {
                        self.fit_amplitude();
                    }
                    if ui.button("Reset").clicked() {
                        self.reset();
                    }
                });
                let visible = self.hidden.iter().filter(|h| !**h).count();
                ui.add(
                    egui::Slider::new(
                        &mut self.first_channel,
                        0..=visible.saturating_sub(self.rows),
                    )
                    .text("First row"),
                );
                let mut seconds = self.navigation.span as f64 / f64::from(m.sample_rate);
                if ui
                    .add(
                        egui::DragValue::new(&mut seconds)
                            .speed(0.1)
                            .range(1.0 / f64::from(m.sample_rate)..=m.duration_seconds())
                            .suffix(" s window"),
                    )
                    .changed()
                {
                    self.navigation.span = (seconds * f64::from(m.sample_rate)).round() as u64;
                    self.navigation.clamp();
                }
                ui.separator();
                ui.strong("Channels");
                ui.horizontal(|ui| {
                    if ui.button("Show all").clicked() {
                        self.hidden.fill(false);
                    }
                    if ui.button("Hide all").clicked() {
                        self.hidden.fill(true);
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.channel_filter).hint_text("Find channel…"),
                );
                let filter = self.channel_filter.to_lowercase();
                let channels: Vec<_> = m
                    .channels
                    .iter()
                    .filter(|c| c.label.to_lowercase().contains(&filter))
                    .collect();
                egui::ScrollArea::vertical().show_rows(ui, 24., channels.len(), |ui, range| {
                    for i in range {
                        let c = channels[i];
                        let mut shown = !self.hidden[c.index];
                        if ui
                            .checkbox(
                                &mut shown,
                                format!(
                                    "{}  ·  {}",
                                    c.label,
                                    c.unit.as_deref().unwrap_or("unit unspecified")
                                ),
                            )
                            .changed()
                        {
                            self.hidden[c.index] = !shown;
                        }
                    }
                });
            }
            _ => {
                egui::ScrollArea::vertical().show(ui,|ui| {
                ui.strong("Recording");ui.label(format!("{:?}",m.kind));ui.label(m.recording_start.as_deref().unwrap_or("Start time unspecified"));
                ui.separator();ui.strong("Streams");for s in &m.streams {ui.label(format!("{} · {}",s.file_name,s.kind));}
                ui.separator();ui.strong("Reader notes");for w in &m.warnings {ui.label(w);ui.add_space(6.);}
                ui.separator();ui.strong("Graphics");ui.label(&self.backend);
                ui.separator();ui.label("Drag traces to pan. Scroll to move through channels; Shift-scroll to pan. Use +/− for time zoom or pinch on a trackpad. Click a trace to inspect an exact sample. Click a channel label to hide/show its trace without removing the row. Right-click for interpolation or bad-channel marking.");
                ui.label("Centering changes display only. The ± scale is in each channel’s declared units, or unspecified stored units. Source files are read-only.");
            });
            }
        }
    }
    fn plot(&mut self, ui: &mut egui::Ui) {
        if self.mri.running {
            ui.centered_and_justified(|ui| {
                ui.label("MRI correction in progress — waveform will refresh when complete.");
            });
            return;
        }

        let Some(r) = self.recording.clone() else {
            ui.vertical_centered(|ui| {
                ui.add_space(100.);
                ui.heading("Open a recording. Stay close to the signal.");
                ui.label("EGI MFF · continuous EEG · read-only");
                ui.add_space(16.);
                if ui.button("Open MFF…").clicked() {
                    self.picker();
                }
                ui.label("You can also drop a recording directory into this window.");
            });
            return;
        };
        let m = r.metadata();
        if m.kind == RecordingKind::SegmentedOrAveraged {
            ui.label("Waveforms unavailable for this recording type. See Info for metadata.");
            return;
        }
        let (rect, response) = ui.allocate_exact_size(
            ui.available_size().max(Vec2::new(200., 120.)),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4., Color32::WHITE);
        let plot = Rect::from_min_max(
            rect.min + Vec2::new(110., 100.),
            rect.max - Vec2::new(18., 42.),
        );
        if plot.width() < 10. || plot.height() < 10. {
            return;
        }
        let visible: Vec<_> = (0..m.channels.len()).collect();
        self.first_channel = self
            .first_channel
            .min(visible.len().saturating_sub(self.rows));
        let channels: Vec<_> = visible
            .into_iter()
            .skip(self.first_channel)
            .take(self.rows)
            .collect();
        if self.filter_original.is_none()
            && !self.mri.open
            && !self.mri.running
            && response.hovered()
        {
            let (scroll, shift, zoom) =
                ui.input(|i| (i.smooth_scroll_delta, i.modifiers.shift, i.zoom_delta()));
            if scroll.x.abs() > 0.1 {
                self.navigation
                    .pan(-f64::from(scroll.x) / f64::from(plot.width()));
            }
            if shift && scroll.y.abs() > 0.1 {
                self.navigation
                    .pan(-f64::from(scroll.y) / f64::from(plot.width()));
            } else if scroll.y.abs() > 0.5 && (zoom - 1.).abs() < 0.001 {
                self.first_channel = self
                    .first_channel
                    .saturating_add_signed((-scroll.y / 12.).round() as isize);
            }
            if (zoom - 1.).abs() > 0.001 {
                let anchor = response
                    .hover_pos()
                    .map(|p| f64::from(((p.x - plot.left()) / plot.width()).clamp(0., 1.)))
                    .unwrap_or(0.5);
                self.navigation.zoom(1. / f64::from(zoom), anchor);
            }
        }
        if response.dragged() {
            let delta = ui.input(|i| i.pointer.delta().x);
            self.navigation
                .pan(-f64::from(delta) / f64::from(plot.width()));
        }
        if self.filter_original.is_none()
            && !self.mri.open
            && !self.mri.running
            && !ui.ctx().egui_wants_keyboard_input()
        {
            if ui.input(|i| i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals)) {
                self.navigation.zoom(0.5, 0.5);
            }
            if ui.input(|i| i.key_pressed(egui::Key::Minus)) {
                self.navigation.zoom(2., 0.5);
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                self.navigation.pan(-0.5);
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                self.navigation.pan(0.5);
            }
        }
        let key = ViewKey {
            range: self.navigation.range(),
            channels: channels.clone(),
            columns: (plot.width().ceil() as usize).clamp(1, 2048),
            filters: self.effective_filters(),
        };
        if channels.is_empty() {
            self.worker.cancel();
            self.display = None;
            self.wanted = None;
            self.busy = None;
            painter.text(
                plot.center(),
                egui::Align2::CENTER_CENTER,
                "All channels hidden — use Channels → Show all",
                egui::FontId::proportional(16.),
                Color32::GRAY,
            );
            return;
        }
        if self.wanted.as_ref() != Some(&key) {
            self.active_id =
                self.worker
                    .submit(Job::View(r.clone(), key.clone(), self.corrected_source()));
            self.wanted = Some(key.clone());
            self.display = None;
            self.busy = Some(if self.effective_filters().active() {
                "Filtering waveform…"
            } else {
                "Reading waveform…"
            });
            self.error = None;
        }
        let row_height = plot.height() / channels.len() as f32;
        let sample_x = |sample: f64| {
            plot.left()
                + ((sample - key.range.start as f64) / (key.range.end - key.range.start) as f64)
                    as f32
                    * plot.width()
        };
        for tick in 0..=8 {
            let x = plot.left() + plot.width() * tick as f32 / 8.;
            painter.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(1., Color32::from_gray(244)),
            );
            let t = (key.range.start as f64
                + (key.range.end - key.range.start) as f64 * tick as f64 / 8.)
                / f64::from(m.sample_rate);
            painter.text(
                Pos2::new(x, plot.bottom() + 10.),
                egui::Align2::CENTER_TOP,
                format!("{t:.3} s"),
                egui::FontId::monospace(11.),
                Color32::GRAY,
            );
        }
        let lane = Rect::from_min_max(
            Pos2::new(plot.left(), rect.top() + 8.),
            Pos2::new(plot.right(), rect.top() + 83.),
        );
        painter.rect_stroke(
            lane,
            6.,
            Stroke::new(1., Color32::from_gray(226)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            Pos2::new(rect.left() + 10., lane.top() + 4.),
            egui::Align2::LEFT_TOP,
            "Events",
            egui::FontId::proportional(13.),
            Color32::from_gray(40),
        );
        if let Some(start) = &m.recording_start {
            let short = start.get(..10).unwrap_or(start);
            painter.text(
                Pos2::new(rect.left() + 10., lane.top() + 26.),
                egui::Align2::LEFT_TOP,
                short,
                egui::FontId::proportional(10.),
                Color32::GRAY,
            );
        }
        let begin_ns = (key.range.start as f64 / f64::from(m.sample_rate) * 1e9) as i128;
        let end_ns = (key.range.end as f64 / f64::from(m.sample_rate) * 1e9) as i128;
        let first = m.events.partition_point(|e| e.onset_ns < begin_ns);
        for (i, e) in m.events[first..]
            .iter()
            .take_while(|e| e.onset_ns <= end_ns)
            .take(250)
            .enumerate()
        {
            let x = sample_x(e.onset_ns as f64 / 1e9 * f64::from(m.sample_rate));
            painter.line_segment(
                [
                    Pos2::new(x, lane.top() + 8.),
                    Pos2::new(x, lane.bottom() - 8.),
                ],
                Stroke::new(1., Color32::from_rgb(255, 149, 73)),
            );
            if i < 12 {
                let ep = painter.with_clip_rect(lane.shrink(1.));
                let text = ep.layout_no_wrap(e.code.clone(), egui::FontId::proportional(10.), GOLD);
                let badge = Rect::from_min_size(
                    Pos2::new(x + 4., lane.top() + 6. + (i % 2) as f32 * 22.),
                    text.size() + Vec2::new(10., 6.),
                );
                ep.rect_filled(badge, 6., Color32::from_rgb(255, 239, 224));
                ep.galley(badge.min + Vec2::new(5., 3.), text, GOLD);
            }
        }
        for (row, &ch) in channels.iter().enumerate() {
            let y = plot.top() + (row as f32 + 0.5) * row_height;
            let row_rect = Rect::from_min_max(
                Pos2::new(plot.left(), y - row_height / 2.),
                Pos2::new(plot.right(), y + row_height / 2.),
            );
            painter.line_segment(
                [row_rect.left_bottom(), row_rect.right_bottom()],
                Stroke::new(1., Color32::from_gray(240)),
            );
            let rp = painter.with_clip_rect(row_rect.shrink2(Vec2::new(0., 3.)));
            rp.line_segment(
                [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                Stroke::new(0.5, Color32::from_gray(226)),
            );
            let label = Rect::from_min_max(
                Pos2::new(rect.left(), y - row_height / 2.),
                Pos2::new(plot.left() - 4., y + row_height / 2.),
            );
            let hit = ui.interact(label, ui.id().with(("channel", ch)), egui::Sense::click());
            if hit.clicked() {
                self.hidden[ch] = !self.hidden[ch];
            }
            hit.context_menu(|ui| {
                if ui
                    .add_enabled(!r.is_pns(), egui::Button::new("Interpolate"))
                    .clicked()
                {
                    self.interpolate_channel(ch);
                    ui.close();
                }
                if ui
                    .add_enabled(
                        !r.is_pns(),
                        egui::Button::new(if self.bad.contains(&ch) {
                            "Unmark bad"
                        } else {
                            "Mark bad"
                        }),
                    )
                    .clicked()
                {
                    self.mark_bad(ch);
                    ui.close();
                }
                if self.interpolated.contains(&ch)
                    && !r.is_pns()
                    && ui.button("Remove interpolation").clicked()
                {
                    self.interpolated.retain(|&c| c != ch);
                    self.rebuild_interpolation();
                    ui.close();
                }
            });
            painter.text(
                Pos2::new(plot.left() - 10., y),
                egui::Align2::RIGHT_CENTER,
                format!(
                    "{}{}{}",
                    m.channels[ch].label,
                    if !r.is_pns() && self.bad.contains(&ch) {
                        " [bad]"
                    } else {
                        ""
                    },
                    if !r.is_pns() && self.interpolation.iter().any(|v| v.target == ch) {
                        " [I]"
                    } else {
                        ""
                    }
                ),
                egui::FontId::proportional(12.),
                if hit.hovered() || (!r.is_pns() && self.bad.contains(&ch)) {
                    GOLD
                } else if self.hidden[ch] {
                    Color32::GRAY
                } else {
                    INK
                },
            );
            if self.hidden[ch] {
                continue;
            }
            if let Some(d) = &self.display {
                let trace = &d.traces[row];
                let baseline = if self.center { trace.mean } else { 0. };
                let value_y = |v: f32| {
                    y - ((f64::from(v) - baseline) / self.amplitude).clamp(-10., 10.) as f32
                        * row_height
                        * 0.43
                        * if m.channels[ch].positive_up == Some(false) {
                            -1.
                        } else {
                            1.
                        }
                };
                let mut previous = None;
                for (i, b) in trace.bins.iter().enumerate() {
                    if b.gap || !b.min.is_finite() || !b.max.is_finite() {
                        previous = None;
                        continue;
                    }
                    let sample = key.range.start as f64 + i as f64 * d.samples_per_bin as f64;
                    if d.samples_per_bin == 1 {
                        let point = Pos2::new(sample_x(sample), value_y(b.min));
                        if let Some(prev) = previous {
                            rp.line_segment([prev, point], Stroke::new(1., INK));
                        }
                        if trace.bins.len() < 100 {
                            rp.circle_filled(point, 1.5, INK);
                        }
                        previous = Some(point);
                    } else {
                        let x = sample_x(sample + (d.samples_per_bin as f64 / 2.));
                        rp.line_segment(
                            [Pos2::new(x, value_y(b.min)), Pos2::new(x, value_y(b.max))],
                            Stroke::new(1., INK),
                        );
                    }
                }
            }
        }
        painter.text(
            Pos2::new(plot.left(), rect.bottom() - 4.),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "±{:.3} per row · {} · {}",
                self.amplitude,
                if self.center {
                    "window mean centered"
                } else {
                    "absolute values"
                },
                if self.display.as_ref().is_some_and(|d| d.samples_per_bin > 1) {
                    "min/max envelope"
                } else {
                    "sample traces"
                }
            ),
            egui::FontId::monospace(10.),
            Color32::GRAY,
        );
        if response.clicked() && self.display.is_some() {
            if let Some(pos) = response
                .interact_pointer_pos()
                .filter(|p| plot.contains(*p))
            {
                let row = ((pos.y - plot.top()) / row_height) as usize;
                let sample = (key.range.start as f64
                    + f64::from((pos.x - plot.left()) / plot.width())
                        * (key.range.end - key.range.start) as f64)
                    .round() as u64;
                let sample = sample.min(key.range.end - 1);
                let channel = channels[row.min(channels.len() - 1)];
                if self.hidden[channel] {
                    return;
                }
                self.active_id = self.worker.submit(Job::Sample(
                    r,
                    channel,
                    sample,
                    self.effective_filters(),
                    self.corrected_source(),
                ));
                self.busy = Some("Reading exact sample…");
            }
        }
        if let Some((_, sample, _)) = self.exact.filter(|(_, s, _)| key.range.contains(s)) {
            let x = sample_x(sample as f64);
            painter.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(1.5, Color32::from_gray(80)),
            );
        }
        if self.display.is_none() {
            painter.text(
                plot.center(),
                egui::Align2::CENTER_CENTER,
                self.error.as_deref().unwrap_or("Reading waveform…"),
                egui::FontId::proportional(15.),
                Color32::GRAY,
            );
        }
    }
}
impl Drop for Viewer {
    fn drop(&mut self) {
        self.report_bad();
    }
}
impl eframe::App for Viewer {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive();
        if self.filter_original.is_none()
            && !self.mri.open
            && !self.mri.running
            && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O))
        {
            self.picker();
        }
        let dropped = ui.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(p) = dropped
            .filter(|_| self.filter_original.is_none() && !self.mri.open && !self.mri.running)
        {
            self.open(p);
        }
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(6.);
            self.toolbar(ui);
            ui.add_space(6.);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            if let Some(e) = &self.error {
                ui.colored_label(GOLD, e);
                if ui.button("Retry").clicked() {
                    if self.recording.is_some() {
                        self.wanted = None;
                        self.error = None;
                    } else if let Some(p) = self.path.clone() {
                        self.open(p);
                    }
                }
            }
            ui.horizontal_wrapped(|ui| {
                if let Some(busy) = self.busy {
                    ui.spinner();
                    ui.label(busy);
                    if self.mri.running {
                        ui.add(
                            egui::ProgressBar::new(
                                self.mri.progress.load(std::sync::atomic::Ordering::Relaxed) as f32
                                    / 1000.,
                            )
                            .show_percentage(),
                        );
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(100));
                    }
                    if ui.button("Cancel").clicked() {
                        self.worker.cancel();
                        if self.mri.running {
                            self.mri.running = false;
                            self.mri.error =
                                Some("Cancelled; previous correction retained.".into());
                        }
                        self.busy = None;
                        self.error =
                            Some("Cancelled. Change the view or reopen to continue.".into());
                    }
                }
                if let (Some(r), Some((ch, sample, value))) = (&self.recording, self.exact) {
                    ui.monospace(format!(
                        "{} · sample {} · {:.6} s · {:.6} {}{}",
                        r.metadata().channels[ch].label,
                        sample,
                        sample as f64 / f64::from(r.metadata().sample_rate),
                        value,
                        r.metadata().channels[ch]
                            .unit
                            .as_deref()
                            .unwrap_or("(unit unspecified)"),
                        if self.effective_filters().active() {
                            " · filtered"
                        } else {
                            " · unfiltered"
                        }
                    ));
                } else if self.busy.is_none() {
                    ui.label("Click a trace for an exact sample · source read-only");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small(&self.backend);
                });
            });
        });
        if self.recording.is_some() && self.inspector_open {
            egui::Panel::right("inspector")
                .default_size(260.)
                .min_size(220.)
                .resizable(true)
                .show(ui, |ui| self.sidebar(ui));
        }
        if self.recording.is_some() {
            egui::Panel::bottom("timeline_scroll").show(ui, |ui| self.navigation_bar(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| self.plot(ui));
        self.filter_dialog(ui.ctx());
        self.mri_dialog(ui.ctx());
        self.psa_dialog(ui.ctx());
    }
}
fn tool_button(ui: &mut egui::Ui, label: &str, enabled: bool, selected: bool) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(format!("\n\n{label}")).size(10.).color(
            if selected {
                Color32::WHITE
            } else {
                Color32::from_gray(45)
            },
        ))
        .min_size(Vec2::new(70., 58.))
        .fill(if selected { INK } else { Color32::WHITE })
        .stroke(Stroke::new(1., Color32::from_gray(231))),
    );
    let color = if selected {
        Color32::WHITE
    } else if enabled {
        Color32::from_gray(40)
    } else {
        Color32::from_gray(155)
    };
    let center = Pos2::new(response.rect.center().x, response.rect.top() + 18.);
    let p = ui.painter();
    let stroke = Stroke::new(1.2, color);
    match label {
        "MRI" => {
            p.circle_stroke(center, 8., stroke);
            p.circle_stroke(center, 4., stroke);
            p.line_segment(
                [center + Vec2::new(-4., 6.), center + Vec2::new(-4., 11.)],
                stroke,
            );
        }
        "FILTER" => {
            let points = vec![
                center + Vec2::new(-9., -7.),
                center + Vec2::new(-9., 6.),
                center + Vec2::new(-3., 6.),
                center + Vec2::new(-3., -5.),
                center + Vec2::new(4., -5.),
                center + Vec2::new(4., 7.),
                center + Vec2::new(9., 7.),
                center + Vec2::new(9., -6.),
            ];
            p.add(egui::Shape::line(points, stroke));
        }
        "ARTIFACTS" => {
            p.line_segment(
                [center + Vec2::new(-9., 0.), center + Vec2::new(9., 0.)],
                stroke,
            );
            p.circle_stroke(center, 5., stroke);
            p.line_segment(
                [center + Vec2::new(0., -9.), center + Vec2::new(0., 9.)],
                stroke,
            );
        }
        "PSA" => {
            for x in [-7., 0., 7.] {
                p.line_segment(
                    [center + Vec2::new(x, -8.), center + Vec2::new(x, 8.)],
                    stroke,
                );
                p.circle_filled(center + Vec2::new(x, x / 2.), 2., color);
            }
        }
        "EEG" => {
            for y in [-5., 0., 5.] {
                let points = (0..20)
                    .map(|i| center + Vec2::new(i as f32 - 10., y + (i as f32 * 0.6).sin() * 2.))
                    .collect();
                p.add(egui::Shape::line(points, stroke));
            }
        }
        _ => {
            p.line_segment(
                [center + Vec2::new(-9., 8.), center + Vec2::new(9., 8.)],
                stroke,
            );
            p.line_segment(
                [center + Vec2::new(-3., 8.), center + Vec2::new(-3., -8.)],
                stroke,
            );
            p.add(egui::Shape::convex_polygon(
                vec![
                    center + Vec2::new(-3., -8.),
                    center + Vec2::new(6., -4.),
                    center + Vec2::new(-3., 0.),
                ],
                color,
                Stroke::NONE,
            ));
        }
    }
    response
}
fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let path = args.next();
    if path.as_deref().is_some_and(|s| s == "-help" || s == "-h") {
        println!("EVA Viewer\nUsage: eva-viewer [recording.mff]\nOpen a directory, or drag one into the window. Supports continuous EEG MFF.\nControls: drag to pan, scroll channels, Shift-scroll to pan, +/− to zoom, click to inspect.");
        return Ok(());
    }
    if path
        .as_deref()
        .is_some_and(|s| s.to_str().is_some_and(|s| s.starts_with("--")))
    {
        eprintln!("Options must use a single dash; use eva-viewer -help.");
        std::process::exit(2);
    }
    if args.next().is_some() {
        eprintln!("Usage: eva-viewer [recording.mff]");
        std::process::exit(2);
    }
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280., 820.])
            .with_min_inner_size([900., 560.]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "EVA Viewer",
        options,
        Box::new(move |cc| Ok(Box::new(Viewer::new(cc, path.map(PathBuf::from))))),
    );
    if let Err(e) = &result {
        eprintln!("Unable to start EVA Viewer: {e}\nA working graphical session and wgpu-compatible Metal/Vulkan adapter are required.");
    }
    result
}
