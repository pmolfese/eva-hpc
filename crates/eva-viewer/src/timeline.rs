//! Three aligned event tracks with bounded, nonoverlapping label placement.
use super::*;
pub const HEIGHT: f32 = 192.;
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
// Two label shelves per track. Shift badges left at the right edge; never overlap.
fn place(bounds: Rect, x: f32, width: f32, ends: &mut [f32; 2]) -> Option<Rect> {
    let width = width.min(bounds.width());
    let left = (x + 4.).clamp(bounds.left(), (bounds.right() - width).max(bounds.left()));
    for (row, end) in ends.iter_mut().enumerate() {
        if left >= *end {
            *end = left + width + 4.;
            return Some(Rect::from_min_size(
                Pos2::new(left, bounds.top() + 3. + row as f32 * 23.),
                Vec2::new(width, 21.),
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
    for (index, title) in ["Events", "DINs / TREV", "Artifacts"].iter().enumerate() {
        let lane = Rect::from_min_max(
            Pos2::new(plot.left(), rect.top() + 8. + index as f32 * 60.),
            Pos2::new(plot.right(), rect.top() + 64. + index as f32 * 60.),
        );
        painter.rect_stroke(
            lane,
            4.,
            Stroke::new(1., Color32::from_gray(226)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            Pos2::new(rect.left() + 8., lane.center().y),
            egui::Align2::LEFT_CENTER,
            title,
            egui::FontId::proportional(12.),
            Color32::from_gray(55),
        );
        let bounds = lane.shrink(2.);
        let p = painter.with_clip_rect(bounds);
        let mut ends = [bounds.left(); 2];
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
        let b = Rect::from_min_size(Pos2::new(100., 0.), Vec2::new(200., 52.));
        let mut ends = [b.left(); 2];
        let first = place(b, 299., 100., &mut ends).unwrap();
        let second = place(b, 300., 100., &mut ends).unwrap();
        assert!(b.contains_rect(first) && b.contains_rect(second));
        assert!(!first.intersects(second));
        assert!(place(b, 300., 100., &mut ends).is_none());
        assert!(b.contains_rect(place(b, -500., 500., &mut [b.left(); 2]).unwrap()));
    }
}
