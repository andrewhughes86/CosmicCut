//! The Move tool's handles: three arrows that slide the model along X, Y and Z, and three rings
//! that turn it about them. Dragging shows where the model will go and the value it comes to;
//! letting go puts that value in a field, selected, to type over or accept with Enter. The
//! values are the model's own (where it stands, how it is turned), so they are there next time.
//! Only `model.move` changes the model.

use CosmicCut_engine::model::Model;
use CosmicCut_engine::units::DisplayUnits;
use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, Ui, vec2};
use glam::DVec3;
use serde_json::json;

use crate::Request;
use crate::theme::Tokens;

/// Length of the arrows on screen, points.
const ARROW: f32 = 90.0;
/// How near the pointer must be to grab a handle, points.
const GRAB: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    /// Slide along X, Y or Z.
    Slide(usize),
    /// Turn about X, Y or Z.
    Turn(usize),
}

impl Handle {
    fn key(self) -> &'static str {
        match self {
            Self::Slide(i) => ["x", "y", "z"].get(i).copied().unwrap_or("x"),
            Self::Turn(i) => ["rx", "ry", "rz"].get(i).copied().unwrap_or("rz"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Slide(i) => ["X", "Y", "Z"].get(i).copied().unwrap_or("X"),
            Self::Turn(i) => ["Turn X", "Turn Y", "Turn Z"].get(i).copied().unwrap_or("Turn Z"),
        }
    }

    /// The model's value this handle changes: mm, or degrees.
    fn value(self, m: &Model, floor: f64) -> f64 {
        match self {
            Self::Slide(i) => m.position(floor).to_array().get(i).copied().unwrap_or(0.0),
            Self::Turn(0) => m.tilt_x,
            Self::Turn(1) => m.tilt_y,
            Self::Turn(_) => m.rotate,
        }
    }

    fn show(self, v: f64, units: DisplayUnits) -> String {
        match self {
            Self::Slide(_) => units.plain(v),
            Self::Turn(_) => {
                let s = format!("{v:.2}");
                s.trim_end_matches('0').trim_end_matches('.').to_owned()
            }
        }
    }
}

struct Drag {
    handle: Handle,
    from: Pos2,
    start: f64,
    value: f64,
    /// For turns: the pointer's last angle round the handles' middle, radians.
    angle: f32,
}

struct Edit {
    handle: Handle,
    start: f64,
    text: String,
    at: Pos2,
}

/// What the gizmo needs from the viewport this frame.
pub struct View<'a> {
    pub rect: Rect,
    pub project: &'a dyn Fn(DVec3) -> Option<Pos2>,
    /// Towards the viewer, and across the screen to the right, in the table frame.
    pub toward: DVec3,
    pub right: DVec3,
}

#[derive(Default)]
pub struct Gizmo {
    hover: Option<Handle>,
    drag: Option<Drag>,
    edit: Option<Edit>,
}

/// Where the handles are on screen: the middle, each handle's line, and the world length of an arrow.
struct Layout {
    centre: Pos2,
    origin: DVec3,
    lines: Vec<(Handle, Vec<Pos2>)>,
    axes: [DVec3; 3],
}

fn layout(view: &View<'_>, m: &Model) -> Option<Layout> {
    let b = m.bounds();
    let origin = (b.min + b.max).as_dvec3() / 2.0;
    let centre = (view.project)(origin)?;
    let unit = (view.project)(origin + view.right)?.distance(centre);
    if unit.is_nan() || unit <= 1.0e-6 {
        return None;
    }
    let length = f64::from(ARROW / unit);
    let mut lines = Vec::new();
    for (i, axis) in [DVec3::X, DVec3::Y, DVec3::Z].iter().enumerate() {
        if let Some(tip) = (view.project)(origin + *axis * length) {
            lines.push((Handle::Slide(i), vec![centre, tip]));
        }
    }
    let axes = m.turn_axes();
    for (i, axis) in axes.iter().enumerate() {
        let a = axis.normalize_or(DVec3::Z);
        let e1 = a.any_orthonormal_vector();
        let e2 = a.cross(e1);
        let r = length * 0.62;
        let ring: Vec<Pos2> = (0..=64)
            .filter_map(|k| {
                let t = std::f64::consts::TAU * k as f64 / 64.0;
                (view.project)(origin + (e1 * t.cos() + e2 * t.sin()) * r)
            })
            .collect();
        if ring.len() > 2 {
            lines.push((Handle::Turn(i), ring));
        }
    }
    Some(Layout { centre, origin, lines, axes })
}

fn distance(p: Pos2, line: &[Pos2]) -> f32 {
    line.windows(2)
        .filter_map(|w| match w {
            [a, b] => {
                let ab = *b - *a;
                let t = if ab.length_sq() > 1.0e-6 { ((p - *a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
                Some(p.distance(*a + ab * t))
            }
            _ => None,
        })
        .fold(f32::INFINITY, f32::min)
}

impl Gizmo {
    /// Whether the pointer is on a handle (or dragging one): the view should not turn with it.
    pub fn grabs(&mut self, ui: &Ui, view: &View<'_>, m: &Model) -> bool {
        if self.drag.is_some() {
            return true;
        }
        self.hover = None;
        let Some(l) = layout(view, m) else { return false };
        if let Some(p) = ui.input(|i| i.pointer.hover_pos()).filter(|p| view.rect.contains(*p)) {
            // Arrows win over rings where they cross.
            let mut best: Option<(Handle, f32)> = None;
            for (h, line) in &l.lines {
                let d = distance(p, line) - if matches!(h, Handle::Slide(_)) { 2.0 } else { 0.0 };
                if d < GRAB && best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((*h, d));
                }
            }
            self.hover = best.map(|(h, _)| h);
        }
        self.hover.is_some()
    }

    /// Drags, draws and edits. Returns true when the tool should end (Esc with nothing open).
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        view: &View<'_>,
        response: &egui::Response,
        m: &Model,
        floor: f64,
        units: DisplayUnits,
        tokens: &Tokens,
        requests: &mut Vec<Request>,
    ) -> bool {
        let Some(l) = layout(view, m) else { return false };
        let pointer = ui.input(|i| i.pointer.interact_pos());
        // Start, follow and finish a drag.
        if self.drag.is_none()
            && response.drag_started_by(egui::PointerButton::Primary)
            && let (Some(h), Some(p)) = (self.hover, pointer)
        {
            let start = h.value(m, floor);
            self.edit = None;
            self.drag = Some(Drag { handle: h, from: p, start, value: start, angle: (p - l.centre).angle() });
        }
        if let (Some(d), Some(p)) = (self.drag.as_mut(), pointer) {
            match d.handle {
                Handle::Slide(i) => {
                    let axis = [DVec3::X, DVec3::Y, DVec3::Z].get(i).copied().unwrap_or(DVec3::X);
                    if let Some(tip) = (view.project)(l.origin + axis) {
                        let per_mm = tip - l.centre;
                        if per_mm.length_sq() > 1.0e-9 {
                            d.value = d.start + f64::from((p - d.from).dot(per_mm) / per_mm.length_sq());
                        }
                    }
                }
                Handle::Turn(i) => {
                    let a = (p - l.centre).angle();
                    let mut step = a - d.angle;
                    if step > std::f32::consts::PI {
                        step -= std::f32::consts::TAU;
                    } else if step < -std::f32::consts::PI {
                        step += std::f32::consts::TAU;
                    }
                    d.angle = a;
                    // Screen angles run clockwise (y down); a turn about an axis facing the viewer
                    // runs anticlockwise.
                    let facing = l.axes.get(i).is_some_and(|ax| ax.dot(view.toward) > 0.0);
                    let deg = f64::from(step.to_degrees());
                    d.value += if facing { -deg } else { deg };
                }
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if response.drag_stopped()
            && let Some(d) = self.drag.take()
        {
            let at = pointer.unwrap_or(l.centre) + vec2(18.0, -18.0);
            self.edit = Some(Edit { handle: d.handle, start: d.start, text: d.handle.show(d.value, units), at });
        }

        // Where the model would go: the drag, or the value being typed.
        let pending = match (&self.drag, &self.edit) {
            (Some(d), _) => Some((d.handle, d.start, d.value)),
            (None, Some(e)) => {
                let v = e.text.trim().parse::<f64>().ok().filter(|v| v.is_finite());
                v.map(|v| (e.handle, e.start, if matches!(e.handle, Handle::Slide(_)) { units.to_mm(v) } else { v }))
            }
            _ => None,
        };
        let painter = ui.painter_at(view.rect);
        if let Some((h, start, value)) = pending {
            ghost(&painter, view, m, &l, h, value - start, tokens);
        }
        let colours = [tokens.axis_x, tokens.axis_y, tokens.axis_z];
        for (h, line) in &l.lines {
            let i = match h {
                Handle::Slide(i) | Handle::Turn(i) => *i,
            };
            let lit = self.hover == Some(*h) || self.drag.as_ref().is_some_and(|d| d.handle == *h);
            let base = colours.get(i).copied().unwrap_or(tokens.accent);
            let colour = if lit { brighten(base) } else { base };
            let width = match (h, lit) {
                (Handle::Slide(_), true) => 4.5,
                (Handle::Slide(_), false) => 3.0,
                (Handle::Turn(_), true) => 3.5,
                (Handle::Turn(_), false) => 2.0,
            };
            painter.add(egui::Shape::line(line.clone(), Stroke::new(width, colour)));
            if let (Handle::Slide(_), [from, tip]) = (h, line.as_slice()) {
                arrowhead(&painter, *from, *tip, colour);
            }
        }
        painter.circle_filled(l.centre, 4.0, tokens.text);
        if let Some(d) = &self.drag {
            let unit = if matches!(d.handle, Handle::Slide(_)) { format!(" {}", units.length()) } else { "°".into() };
            let text = format!("{} {}{unit}", d.handle.name(), d.handle.show(d.value, units));
            let at = pointer.unwrap_or(l.centre) + vec2(16.0, -16.0);
            let galley = painter.layout_no_wrap(text, FontId::proportional(13.0), tokens.text);
            let r = Rect::from_min_size(at, galley.size() + vec2(12.0, 6.0));
            painter.rect(r, 4.0, tokens.panel_bg, Stroke::new(1.0, tokens.accent), egui::StrokeKind::Inside);
            painter.galley(r.min + vec2(6.0, 3.0), galley, tokens.text);
        }
        if self.hover.is_some() && self.drag.is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        self.field(ui, &m.id, units, requests);
        self.edit.is_none() && self.drag.is_none() && ui.input(|i| i.key_pressed(egui::Key::Escape))
    }

    /// The value typed after a drag: selected, so typing replaces it. Enter (or clicking away)
    /// moves the model there; Esc puts it back.
    fn field(&mut self, ui: &mut Ui, id: &str, units: DisplayUnits, requests: &mut Vec<Request>) {
        let Some(e) = self.edit.as_mut() else { return };
        let rect = Rect::from_min_size(e.at, vec2(96.0, 24.0));
        let response = ui.put(
            rect,
            egui::TextEdit::singleline(&mut e.text).horizontal_align(egui::Align::Center).id(ui.id().with("move-edit")),
        );
        if !response.has_focus() && !response.lost_focus() {
            response.request_focus();
            super::select_all(ui, response.id, e.text.chars().count());
        }
        let unit = if matches!(e.handle, Handle::Slide(_)) { units.length().to_owned() } else { "°".into() };
        ui.painter().text(
            rect.right_center() + vec2(6.0, 0.0),
            Align2::LEFT_CENTER,
            format!("{}  {unit}", e.handle.name()),
            FontId::proportional(12.0),
            ui.visuals().text_color(),
        );
        if !response.lost_focus() {
            return;
        }
        let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let value = e.text.trim().parse::<f64>().ok().filter(|v| v.is_finite());
        if !cancelled && let Some(v) = value {
            let v = if matches!(e.handle, Handle::Slide(_)) { units.to_mm(v) } else { v };
            requests.push(("model.move".into(), json!({ "id": id, e.handle.key(): v })));
        }
        self.edit = None;
    }

    /// What the tool says it wants next.
    pub fn instructions(&self) -> &'static str {
        if self.edit.is_some() {
            "Move: type the exact value and press Enter, or Enter to keep it; Esc puts the model back."
        } else {
            "Move: drag an arrow to slide the model along X, Y or Z, or a ring to turn it about that axis. Let go \
             to type the exact value. Esc ends."
        }
    }
}

fn brighten(c: Color32) -> Color32 {
    let up = |v: u8| v.saturating_add(((255 - v) as f32 * 0.45) as u8);
    Color32::from_rgb(up(c.r()), up(c.g()), up(c.b()))
}

fn arrowhead(painter: &egui::Painter, from: Pos2, tip: Pos2, colour: Color32) {
    let d = (tip - from).normalized();
    if !d.x.is_finite() || d == egui::Vec2::ZERO {
        return;
    }
    let n = vec2(-d.y, d.x);
    let back = tip - d * 13.0;
    painter.add(egui::Shape::convex_polygon(vec![tip + d * 2.0, back + n * 6.0, back - n * 6.0], colour, Stroke::NONE));
}

/// The model's box where the drag would put it.
fn ghost(painter: &egui::Painter, view: &View<'_>, m: &Model, l: &Layout, h: Handle, by: f64, tokens: &Tokens) {
    let b = m.bounds();
    let (lo, hi) = (b.min.as_dvec3(), b.max.as_dvec3());
    let corners: Vec<DVec3> = (0..8)
        .map(|k| {
            DVec3::new(
                if k & 1 == 0 { lo.x } else { hi.x },
                if k & 2 == 0 { lo.y } else { hi.y },
                if k & 4 == 0 { lo.z } else { hi.z },
            )
        })
        .map(|c| match h {
            Handle::Slide(i) => c + [DVec3::X, DVec3::Y, DVec3::Z].get(i).copied().unwrap_or(DVec3::X) * by,
            Handle::Turn(i) => {
                let axis = l.axes.get(i).copied().unwrap_or(DVec3::Z).normalize_or(DVec3::Z);
                l.origin + glam::DQuat::from_axis_angle(axis, by.to_radians()) * (c - l.origin)
            }
        })
        .collect();
    let edges = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
    let stroke = Stroke::new(1.5, tokens.accent);
    for (a, b) in edges {
        if let (Some(p), Some(q)) =
            (corners.get(a).and_then(|c| (view.project)(*c)), corners.get(b).and_then(|c| (view.project)(*c)))
        {
            painter.line_segment([p, q], stroke);
        }
    }
}
