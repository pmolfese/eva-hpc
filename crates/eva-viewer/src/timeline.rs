//! Three aligned event tracks with bounded, nonoverlapping label placement.
use super::*;
const ROW_HEIGHT: f32 = 30.;
pub const HEIGHT: f32 = 8. + 3. * ROW_HEIGHT + 8.;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    Events,
    Triggers,
    Artifacts,
}
impl Track {
    fn index(self) -> usize {
        match self {
            Self::Events => 0,
            Self::Triggers => 1,
            Self::Artifacts => 2,
        }
    }
}
pub fn classify(code: &str, source: &str) -> Track {
    let code = code.trim().to_ascii_uppercase();
    if code == "TREV"
        || code
            .strip_prefix("DIN")
            .is_some_and(|s| s.is_empty() || s.chars().all(|c| c.is_ascii_digit()))
    {
        Track::Triggers
    } else if matches!(code.as_str(), "EYE BLINK" | "EYE MOVEMENT" | "BLINK")
        || code.starts_with("BAD_")
        || source == "Artifact Detection"
    {
        Track::Artifacts
    } else {
        Track::Events
    }
}
pub struct Marker {
    pub track: Track,
    pub label: String,
    pub time: f64,
    pub span: Option<(f64, f64)>,
    pub color: Color32,
    pub help: String,
}
// One compact label shelf per track. Shift edge badges left; never overlap.
fn place(bounds: Rect, x: f32, width: f32, ends: &mut [f32; 1]) -> Option<Rect> {
    let width = width.min(bounds.width());
    let left = (x + 4.).clamp(bounds.left(), (bounds.right() - width).max(bounds.left()));
    for end in ends.iter_mut() {
        if left >= *end {
            *end = left + width + 4.;
            return Some(Rect::from_min_size(
                Pos2::new(left, bounds.center().y - 10.),
                Vec2::new(width, 20.),
            ));
        }
    }
    None
}
pub fn paint(
    ui: &mut egui::Ui,
    rect: Rect,
    plot: Rect,
    start: f64,
    end: f64,
    mut markers: Vec<Marker>,
) {
    markers.sort_by(|a, b| a.time.total_cmp(&b.time));
    let painter = ui.painter_at(rect);
    let x = |t: f64| plot.left() + ((t - start) / (end - start)) as f32 * plot.width();
    let panel = Rect::from_min_max(
        Pos2::new(rect.left() + 4., rect.top() + 8.),
        Pos2::new(plot.right(), rect.top() + 8. + 3. * ROW_HEIGHT),
    );
    painter.rect_filled(panel, 6., Color32::from_gray(253));
    painter.rect_stroke(
        panel,
        6.,
        Stroke::new(1., Color32::from_gray(226)),
        egui::StrokeKind::Inside,
    );
    for (index, title) in ["Events", "DINs / TREV", "Artifacts"].iter().enumerate() {
        let top = panel.top() + index as f32 * ROW_HEIGHT;
        let lane = Rect::from_min_max(
            Pos2::new(plot.left(), top),
            Pos2::new(plot.right(), top + ROW_HEIGHT),
        );
        if index > 0 {
            painter.line_segment(
                [
                    Pos2::new(panel.left() + 8., top),
                    Pos2::new(panel.right() - 8., top),
                ],
                Stroke::new(1., Color32::from_gray(237)),
            );
        }
        painter.text(
            Pos2::new(rect.left() + 8., lane.center().y),
            egui::Align2::LEFT_CENTER,
            title,
            egui::FontId::proportional(12.),
            Color32::from_gray(55),
        );
        let bounds = lane.shrink(2.);
        let p = painter.with_clip_rect(bounds);
        let mut ends = [bounds.left(); 1];
        for (id, m) in markers
            .iter()
            .enumerate()
            .filter(|(_, m)| m.track.index() == index)
        {
            let peak_visible = m.time >= start && m.time < end;
            let span = m.span.filter(|(a, b)| *a < end && *b > start);
            if !peak_visible && span.is_none() {
                continue;
            }
            let mut hit = None;
            if let Some((a, b)) = span {
                let a = x(a.max(start)).clamp(bounds.left(), bounds.right());
                let b = x(b.min(end)).clamp(bounds.left(), bounds.right());
                let band = Rect::from_min_max(
                    Pos2::new(a, bounds.bottom() - 7.),
                    Pos2::new(b, bounds.bottom()),
                );
                p.rect_filled(band, 2., m.color.gamma_multiply(0.25));
                hit = Some(Rect::from_min_max(
                    Pos2::new(a, bounds.top()),
                    Pos2::new(b, bounds.bottom()),
                ));
            }
            if peak_visible {
                let px = x(m.time).clamp(bounds.left(), bounds.right());
                p.line_segment(
                    [Pos2::new(px, bounds.top()), Pos2::new(px, bounds.bottom())],
                    Stroke::new(1.5, m.color),
                );
                let marker = Rect::from_min_max(
                    Pos2::new(px - 3., bounds.top()),
                    Pos2::new(px + 3., bounds.bottom()),
                )
                .intersect(bounds);
                hit = Some(hit.map_or(marker, |r| r.union(marker)));
            }
            let anchor = if peak_visible {
                x(m.time)
            } else {
                x(span.unwrap().0.max(start))
            };
            let text = p.layout(
                m.label.clone(),
                egui::FontId::proportional(12.),
                m.color,
                bounds.width() - 8.,
            );
            if let Some(badge) = place(bounds, anchor, text.size().x + 8., &mut ends) {
                p.rect_filled(badge, 4., Color32::WHITE);
                p.rect_stroke(
                    badge,
                    4.,
                    Stroke::new(1., m.color.gamma_multiply(0.4)),
                    egui::StrokeKind::Inside,
                );
                p.with_clip_rect(badge.intersect(bounds)).galley(
                    badge.min + Vec2::new(4., 2.),
                    text,
                    m.color,
                );
                ui.interact(
                    badge,
                    ui.id().with(("timeline_label", index, id)),
                    egui::Sense::hover(),
                )
                .on_hover_text(&m.help);
            }
            if let Some(hit) = hit {
                ui.interact(
                    hit,
                    ui.id().with(("timeline_marker", index, id)),
                    egui::Sense::hover(),
                )
                .on_hover_text(&m.help);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classification_and_edge_labels() {
        for code in ["DIN1", "din2", "DIN10", "TREV"] {
            assert_eq!(classify(code, ""), Track::Triggers);
        }
        assert_eq!(classify("DING", ""), Track::Events);
        assert_eq!(classify("Eye Movement", ""), Track::Artifacts);
        assert_eq!(classify("cue+", ""), Track::Events);
        let b = Rect::from_min_size(Pos2::new(100., 0.), Vec2::new(200., 26.));
        let mut ends = [b.left(); 1];
        let first = place(b, 299., 100., &mut ends).unwrap();
        assert!(b.contains_rect(first));
        assert!(place(b, 300., 100., &mut ends).is_none());
        assert!(b.contains_rect(place(b, -500., 500., &mut [b.left(); 1]).unwrap()));
    }
}
