//! Drawing in a sketch, over the viewport: the sketch's curves, points, constraint marks and
//! dimensions, the tool's preview with what it will snap to, and the mouse. Everything the
//! mouse does is a sketch command; the engine does the snapping and the constraints, so a click
//! here makes what `sketch.line {...}` in the console makes.

use CosmicCut_engine::Session;
use CosmicCut_engine::sketch::{
    Constraint, Geom, Kind, MIDPOINT, ORIGIN, Plane, Sketch, Snap, arc_points, closest_on_segment,
};
use CosmicCut_engine::units::DisplayUnits;
use egui::{Align2, Color32, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use glam::{DVec2, DVec3};
use serde_json::json;

use crate::Request;
use crate::theme::Tokens;

/// Pixels the pointer may be from a curve and still snap to it or pick it.
const PICK_PX: f32 = 9.0;
/// End points (and other points) reach further: a ring shows round one this near, and a click
/// takes it.
const POINT_REACH: f64 = 1.7;
/// Dragging this "item" moves the whole selection together.
const GROUP: u32 = u32::MAX - 1;

/// How far dimensions stand off what they measure, pixels.
const DIM_PX: f32 = 26.0;

/// How the viewport sees the sketch: table frame to screen, and the pointer's ray.
pub struct View<'a> {
    pub rect: Rect,
    pub project: &'a dyn Fn(DVec3) -> Option<Pos2>,
    pub ray: &'a dyn Fn(Pos2) -> Option<(DVec3, DVec3)>,
}

/// What the line tool's next segment is: a line, an arc tangent to the last segment (R), or a
/// 45° chamfer off it (C).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Segment {
    #[default]
    Line,
    Arc,
    Chamfer,
}

/// The next segment worked out from where the chain ends and the pointer.
struct Next {
    end: DVec2,
    /// For an arc, a point on it between its ends.
    through: Option<DVec2>,
    /// Its outline, for the preview.
    points: Vec<DVec2>,
    /// The way it is going at its end, for the segment after it.
    dir: DVec2,
}

#[derive(Default)]
pub struct SketchUi {
    /// What to do next, for the Instructions tab.
    pub instructions: String,
    segment: Segment,
    /// The way the chain of lines was going at its end.
    chain_dir: Option<DVec2>,
    /// The tool these clicks were made with, and the points put down so far.
    tool: &'static str,
    clicks: Vec<DVec2>,
    /// The dimension tool's picks: the first, and a second for a distance or angle.
    first: Option<u32>,
    second: Option<u32>,
    /// An item being dragged (or [`GROUP`], the selection) and where it was picked up.
    drag: Option<(u32, DVec2)>,
    /// Where a selection window was started, on screen.
    window: Option<Pos2>,
    /// A dimension being typed: its id and the text so far.
    typing: Option<(u32, String)>,
    /// A dimension's size being dragged: its id and where it is now (sketch coordinates).
    label_drag: Option<(u32, DVec2)>,
    /// The newest dimension when one was asked for, to open the new one for typing.
    awaiting_dimension: Option<u32>,
    /// Degrees of freedom left, worked out once per change.
    freedom: Option<(u64, usize)>,
}

impl SketchUi {
    /// Whether a chain of lines is being drawn (R and C then change its next segment).
    pub fn chaining(&self) -> bool {
        self.tool == "line" && !self.clicks.is_empty()
    }

    /// Draws the sketch being drawn, and other sketches faintly; takes the mouse while drawing.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        view: &View<'_>,
        response: &Response,
        session: &Session,
        tokens: &Tokens,
        requests: &mut Vec<Request>,
    ) {
        let painter = ui.painter_at(view.rect);
        let editing = session.view.sketch.as_deref();
        // Other sketches: their outlines, faint.
        for shape in session.job.shapes.iter().filter(|s| s.sketch.is_some() && Some(s.id.as_str()) != editing) {
            let Some(plane) = shape.sketch.as_ref().map(|k| k.plane) else { continue };
            let ink = Stroke::new(1.5, tokens.text_muted.gamma_multiply(0.8));
            for path in &shape.paths {
                let mut pts: Vec<Pos2> = path.points.iter().filter_map(|p| (view.project)(plane.to_3d(*p))).collect();
                if path.closed
                    && let Some(&first) = pts.first()
                {
                    pts.push(first);
                }
                painter.add(Shape::line(pts, ink));
            }
        }
        let Some(sketch) =
            editing.and_then(|id| session.job.shapes.iter().find(|s| s.id == id)).and_then(|s| s.sketch.as_deref())
        else {
            *self = Self::default();
            return;
        };
        let plane = sketch.plane;
        let to_screen = |p: DVec2| (view.project)(plane.to_3d(p));
        let to_sketch = |pos: Pos2| -> Option<DVec2> {
            let (o, d) = (view.ray)(pos)?;
            let n = plane.normal();
            let dn = d.dot(n);
            if dn.abs() < 1.0e-9 {
                return None;
            }
            Some(plane.to_2d(o + d * (-o.dot(n) / dn)))
        };
        // Millimetres per pixel where the sketch is.
        let px_per_mm = match (to_screen(DVec2::ZERO), to_screen(DVec2::X)) {
            (Some(a), Some(b)) => f64::from(a.distance(b)).max(1.0e-6),
            _ => 1.0,
        };
        let tol = f64::from(PICK_PX) / px_per_mm;
        let units = session.prefs.units;

        let tool = session.view.sketch_tool;
        if tool != self.tool {
            self.tool = tool;
            self.clicks.clear();
            self.first = None;
        }
        if self.clicks.is_empty() {
            self.segment = Segment::Line;
            self.chain_dir = None;
        }
        // R and C, while a chain of lines goes on: an arc tangent to it, or a 45° chamfer.
        if tool == "line" && self.chain_dir.is_some() && !ui.ctx().egui_wants_keyboard_input() {
            let (r, c) = ui.input(|i| (i.key_pressed(egui::Key::R), i.key_pressed(egui::Key::C)));
            let toggle = |now: Segment, to: Segment| if now == to { Segment::Line } else { to };
            if r {
                self.segment = toggle(self.segment, Segment::Arc);
            } else if c {
                self.segment = toggle(self.segment, Segment::Chamfer);
            }
        }
        if self.freedom.is_none_or(|(rev, _)| rev != session.job.revision) {
            self.freedom = Some((session.job.revision, sketch.freedom()));
        }
        let free = self.freedom.map_or(0, |(_, f)| f);

        // A new dimension opens for typing its size.
        if let Some(before) = self.awaiting_dimension
            && let Some(c) =
                sketch.constraints.iter().filter(|c| c.kind.is_dimension() && c.id > before).max_by_key(|c| c.id)
        {
            self.typing = Some((c.id, value_text(c, units)));
            self.awaiting_dimension = None;
        }

        let pointer = response.hover_pos().or_else(|| response.interact_pointer_pos());
        let at = pointer.and_then(to_sketch);
        // Shift or Ctrl adds to the selection.
        let shift = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().egui_wants_keyboard_input();
        if escape && self.typing.is_none() {
            if tool == "select" && !session.view.sketch_selection.is_empty() {
                // Esc in Select lets go of the selection first.
                requests.push(("sketch.select".into(), json!({ "items": [] })));
            } else if self.clicks.is_empty() && self.first.is_none() {
                requests.push(("sketch.tool".into(), json!({ "tool": "select" })));
            }
            self.clicks.clear();
            self.first = None;
            self.second = None;
        }
        if response.secondary_clicked() {
            // Right-click ends a chain of lines, or lets go of what is being dimensioned.
            self.clicks.clear();
            self.first = None;
            self.second = None;
        }

        // Dragging a point or curve: a live preview, then one command when it is put down.
        let mut shown = sketch.clone();
        if tool == "move" {
            // Move drags the selection, or the whole sketch when nothing is selected.
            let items: Vec<u32> = session.view.sketch_selection.clone();
            if response.drag_started_by(egui::PointerButton::Primary)
                && let Some(p) = at
            {
                self.drag = Some((0, p));
            }
            if let (Some((_, from)), Some(to)) = (self.drag, at) {
                if response.dragged() {
                    let _ = shown.move_items(&items, to - from);
                }
                if response.drag_stopped() {
                    let names: Vec<String> = items.iter().map(|&i| sketch.item_name(i)).collect();
                    let by = to - from;
                    requests.push(("sketch.move".into(), json!({ "items": names, "by": [by.x, by.y] })));
                    self.drag = None;
                }
            }
            if !response.dragged() && !response.drag_stopped() {
                self.drag = None;
            }
        }
        if tool == "select" {
            let selection = &session.view.sketch_selection;
            if response.drag_started_by(egui::PointerButton::Primary)
                && let Some(p) = at
            {
                match pick(sketch, p, tol, false).filter(|&i| i != ORIGIN && i & MIDPOINT == 0) {
                    // On something selected, with more selected: they all move together.
                    Some(item) if selection.len() > 1 && in_selection(sketch, selection, item) => {
                        self.drag = Some((GROUP, p));
                    }
                    Some(item) => self.drag = Some((item, p)),
                    // On nothing: a selection window.
                    None => self.window = response.interact_pointer_pos(),
                }
            }
            if let (Some((item, from)), Some(to)) = (self.drag, at) {
                if response.dragged() {
                    let _ =
                        if item == GROUP { shown.move_items(selection, to - from) } else { shown.drag(item, from, to) };
                }
                if response.drag_stopped() {
                    if item == GROUP {
                        let names: Vec<String> = selection.iter().map(|&i| sketch.item_name(i)).collect();
                        let by = to - from;
                        requests.push(("sketch.move".into(), json!({ "items": names, "by": [by.x, by.y] })));
                    } else {
                        requests.push((
                            "sketch.drag".into(),
                            json!({ "item": sketch.item_name(item), "from": [from.x, from.y], "to": [to.x, to.y] }),
                        ));
                    }
                    self.drag = None;
                }
            }
            // The window: left to right takes what is wholly inside it (blue), right to left
            // whatever it touches (green).
            if let (Some(start), Some(now)) = (self.window, response.interact_pointer_pos().or(pointer)) {
                let rect = Rect::from_two_pos(start, now);
                let crossing = now.x < start.x;
                if response.dragged() {
                    let ink = if crossing { tokens.success } else { tokens.accent };
                    painter.rect(rect, 0.0, ink.gamma_multiply(0.12), Stroke::new(1.0, ink), egui::StrokeKind::Inside);
                }
                if response.drag_stopped() {
                    let items: Vec<String> = window_items(sketch, &to_screen, rect, crossing)
                        .into_iter()
                        .map(|i| sketch.item_name(i))
                        .collect();
                    requests.push(("sketch.select".into(), json!({ "items": items, "add": shift })));
                    self.window = None;
                }
            }
            if !response.dragged() && !response.drag_stopped() {
                self.drag = None;
                self.window = None;
            }
        }

        // What the pointer would snap to, and where that puts it.
        let drawing_tool = !matches!(tool, "select" | "dimension" | "move");
        let mut snapped = at.map(|p| (Snap::Free, p));
        let mut level: Option<Kind> = None;
        if drawing_tool && let Some(p) = at {
            // Points from further away than midpoints and curves.
            let (snap, mut q) = match sketch.infer(p, tol * POINT_REACH, &[]) {
                (Snap::Point(id), q) => (Snap::Point(id), q),
                _ => sketch.infer(p, tol, &[]),
            };
            if let (Snap::Free, Some(&start), "line", Segment::Line) = (snap, self.clicks.last(), tool, self.segment) {
                level = Sketch::level_or_upright(start, q, tol);
                match level {
                    Some(Kind::Horizontal) => q.y = start.y,
                    Some(Kind::Vertical) => q.x = start.x,
                    _ => {}
                }
            }
            snapped = Some((snap, q));
        }
        let hover = if drawing_tool || tool == "move" {
            None
        } else {
            // Dimensioning picks a line along its length even near its ends, for angles.
            at.and_then(|p| pick(sketch, p, tol, tool == "dimension"))
        };

        // Clicks.
        if response.clicked_by(egui::PointerButton::Primary) && self.typing.is_none() {
            let snap = json!(tol);
            match (tool, snapped) {
                ("select", _) => {
                    let items: Vec<String> = hover.map(|h| sketch.item_name(h)).into_iter().collect();
                    requests.push(("sketch.select".into(), json!({ "items": items, "add": shift })));
                }
                // The first pick shows its dimension at once; a second turns it into the distance or
                // angle between the two; a click on empty space (or on either again) puts it there.
                ("dimension", _) => match (self.first, self.second, hover) {
                    (None, _, Some(h)) => self.first = Some(h),
                    (Some(a), None, Some(h)) if h != a => self.second = Some(h),
                    (Some(a), Some(b), Some(h)) if h != a && h != b => {
                        self.first = Some(h);
                        self.second = None;
                    }
                    (Some(a), second, _) => {
                        let items: Vec<String> =
                            [Some(a), second].into_iter().flatten().map(|i| sketch.item_name(i)).collect();
                        let place = at.map(|p| json!([p.x, p.y]));
                        self.awaiting_dimension = Some(sketch.constraints.iter().map(|c| c.id).max().unwrap_or(0));
                        requests.push(("sketch.dimension".into(), json!({ "items": items, "at": place })));
                        self.first = None;
                        self.second = None;
                    }
                    (None, _, None) => {}
                },
                ("line", Some((_, p)))
                    if self.segment != Segment::Line
                        && let (Some(&start), Some(dir)) = (self.clicks.first(), self.chain_dir) =>
                {
                    if let Some(next) = next_segment(self.segment, start, dir, p) {
                        let (from, to) = (json!([start.x, start.y]), json!([next.end.x, next.end.y]));
                        match next.through {
                            Some(m) => requests.push((
                                "sketch.arc".into(),
                                json!({ "start": from, "end": to, "through": [m.x, m.y], "snap": snap, "follow": "tangent" }),
                            )),
                            None => requests.push((
                                "sketch.line".into(),
                                json!({ "from": from, "to": to, "snap": snap, "follow": "chamfer" }),
                            )),
                        }
                        self.clicks = vec![next.end];
                        self.chain_dir = Some(next.dir);
                        self.segment = Segment::Line;
                    }
                }
                (_, Some((_, p))) => {
                    if tool == "line"
                        && let Some(&start) = self.clicks.first()
                    {
                        self.chain_dir = (p - start).try_normalize();
                    }
                    self.clicks.push(p);
                    let c = &self.clicks;
                    let pt = |k: usize| c.get(k).map(|v| json!([v.x, v.y]));
                    let request = match (tool, c.len()) {
                        ("point", 1) => Some(("sketch.point", json!({ "at": pt(0), "snap": snap }))),
                        ("line", 2) => Some(("sketch.line", json!({ "from": pt(0), "to": pt(1), "snap": snap }))),
                        ("rectangle", 2) => {
                            Some(("sketch.rectangle", json!({ "from": pt(0), "to": pt(1), "snap": snap })))
                        }
                        ("rectangle_centre", 2) => {
                            Some(("sketch.rectangle.centre", json!({ "centre": pt(0), "corner": pt(1), "snap": snap })))
                        }
                        ("circle", 2) => {
                            let r = c.first().zip(c.get(1)).map_or(0.0, |(a, b)| a.distance(*b));
                            Some(("sketch.circle", json!({ "centre": pt(0), "radius": r, "snap": snap })))
                        }
                        ("polygon", 2) => {
                            Some(("sketch.polygon", json!({ "centre": pt(0), "corner": pt(1), "snap": snap })))
                        }
                        ("arc", 3) => Some((
                            "sketch.arc",
                            json!({ "start": pt(0), "end": pt(1), "through": pt(2), "snap": snap }),
                        )),
                        _ => None,
                    };
                    if let Some((id, params)) = request {
                        requests.push((id.into(), params));
                        // Lines go on from where the last one ended; the rest start again.
                        let last = self.clicks.last().copied();
                        self.clicks.clear();
                        if tool == "line"
                            && let Some(end) = last
                        {
                            self.clicks.push(end);
                        }
                    }
                }
                _ => {}
            }
        }

        // The dimension being made, shown before it is put down, its size at the pointer.
        let mut preview_dim = None;
        if tool == "dimension"
            && let Some(a) = self.first
        {
            let items: Vec<u32> = [Some(a), self.second].into_iter().flatten().collect();
            let mut trial = shown.clone();
            if let Ok(id) = trial.dimension(&items, None, None) {
                if let Some(p) = at {
                    let _ = trial.move_label(id, p);
                }
                preview_dim = Some(id);
                shown = trial;
            }
        }

        // ---- Drawing --------------------------------------------------------------------
        let colour = if free == 0 { tokens.text } else { tokens.accent };
        let selected = &session.view.sketch_selection;
        let ink = |id: u32| {
            if selected.contains(&id) || self.first == Some(id) || self.second == Some(id) {
                Stroke::new(2.5, tokens.warning)
            } else if hover == Some(id) {
                Stroke::new(3.0, tokens.accent_strong)
            } else {
                Stroke::new(2.0, colour)
            }
        };
        for e in &shown.entities {
            let stroke = ink(e.id);
            let pts: Vec<Pos2> = curve_points(&shown, e.geom).into_iter().filter_map(to_screen).collect();
            if pts.len() < 2 {
                continue;
            }
            if e.construction {
                let faint = Stroke::new(stroke.width.min(1.5), stroke.color.gamma_multiply(0.6));
                painter.extend(Shape::dashed_line(&pts, faint, 6.0, 4.0));
            } else {
                painter.add(Shape::line(pts, stroke));
            }
        }
        for p in &shown.points {
            let Some(s) = to_screen(p.at) else { continue };
            let picked = selected.contains(&p.id) || self.first == Some(p.id) || self.second == Some(p.id);
            if hover == Some(p.id) {
                painter.circle_stroke(s, 9.0, Stroke::new(1.8, tokens.warning));
            }
            if p.id == ORIGIN {
                let ink = if picked { tokens.warning } else { tokens.text_muted };
                let o = Stroke::new(1.5, ink);
                painter.line_segment([s - vec2(7.0, 0.0), s + vec2(7.0, 0.0)], o);
                painter.line_segment([s - vec2(0.0, 7.0), s + vec2(0.0, 7.0)], o);
                painter.circle_stroke(s, 3.5, o);
                continue;
            }
            let fill = if picked {
                tokens.warning
            } else if hover == Some(p.id) {
                tokens.accent_strong
            } else {
                colour
            };
            painter.rect_filled(Rect::from_center_size(s, vec2(5.0, 5.0)), 1.0, fill);
        }
        // Line midpoints that are hovered or picked: a triangle, ringed when hovered.
        let mids = [hover, self.first, self.second].into_iter().flatten().chain(selected.iter().copied());
        for m in mids.filter(|m| m & MIDPOINT != 0) {
            let Some(s) = shown.midpoint(m).and_then(to_screen) else { continue };
            let picked = selected.contains(&m) || self.first == Some(m) || self.second == Some(m);
            let ink = if picked { tokens.warning } else { tokens.accent_strong };
            painter.add(Shape::convex_polygon(
                vec![s + vec2(0.0, -5.0), s + vec2(4.5, 3.5), s + vec2(-4.5, 3.5)],
                ink,
                Stroke::NONE,
            ));
            if hover == Some(m) {
                painter.circle_stroke(s, 9.0, Stroke::new(1.8, tokens.warning));
            }
        }
        glyphs(&painter, &shown, &to_screen, tokens);
        self.dimensions(ui, &shown, &to_screen, &to_sketch, px_per_mm, units, tokens, preview_dim, requests);

        // The tool's preview and its snap.
        if let Some((snap, p)) = snapped
            && drawing_tool
        {
            let preview = Stroke::new(1.5, tokens.accent.gamma_multiply(0.85));
            let c = &self.clicks;
            let screen = |v: DVec2| to_screen(v);
            let poly = |pts: &[DVec2]| -> Vec<Pos2> { pts.iter().filter_map(|v| screen(*v)).collect() };
            match (tool, c.as_slice()) {
                ("line", [a]) => match self.chain_dir.and_then(|d| next_segment(self.segment, *a, d, p)) {
                    Some(next) if self.segment != Segment::Line => {
                        painter.add(Shape::line(poly(&next.points), preview));
                        if let Some(s) = screen(next.end) {
                            painter.circle_filled(s, 3.0, tokens.warning);
                            let text =
                                if self.segment == Segment::Arc { "Tangent arc  (R)" } else { "Chamfer 45°  (C)" };
                            badge(&painter, s + vec2(18.0, 16.0), text, tokens, tokens.warning);
                        }
                    }
                    _ => {
                        painter.add(Shape::line(poly(&[*a, p]), preview));
                    }
                },
                ("rectangle", [a]) => {
                    let pts = [*a, DVec2::new(p.x, a.y), p, DVec2::new(a.x, p.y), *a];
                    painter.add(Shape::line(poly(&pts), preview));
                }
                ("rectangle_centre", [a]) => {
                    let d = (p - *a).abs();
                    let (lo, hi) = (*a - d, *a + d);
                    let pts = [lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y), lo];
                    painter.add(Shape::line(poly(&pts), preview));
                }
                ("circle", [a]) => {
                    let r = a.distance(p);
                    let pts: Vec<DVec2> =
                        (0..=72).map(|k| *a + DVec2::from_angle(k as f64 * 5.0f64.to_radians()) * r).collect();
                    painter.add(Shape::line(poly(&pts), preview));
                }
                ("polygon", [a]) => {
                    let (r, t0) = (a.distance(p), (p - *a).to_angle());
                    let pts: Vec<DVec2> = (0..=6)
                        .map(|k| *a + DVec2::from_angle(t0 + k as f64 * std::f64::consts::TAU / 6.0) * r)
                        .collect();
                    painter.add(Shape::line(poly(&pts), preview));
                }
                ("arc", [a]) => {
                    painter.extend(Shape::dashed_line(&poly(&[*a, p]), preview, 4.0, 4.0));
                }
                ("arc", [a, b]) => {
                    if let Some(centre) = CosmicCut_engine::sketch::circumcentre(*a, p, *b) {
                        let ccw = (*b - *a).perp_dot(p - *a) < 0.0;
                        let (s, e) = if ccw { (*a, *b) } else { (*b, *a) };
                        painter.add(Shape::line(poly(&arc_points(centre, s, e)), preview));
                    }
                }
                _ => {}
            }
            if let Some(s) = screen(p) {
                let mark = Stroke::new(1.5, tokens.warning);
                match snap {
                    Snap::Point(_) => {
                        painter.circle_stroke(s, 9.0, Stroke::new(1.8, tokens.warning));
                    }
                    Snap::Midpoint(_) => {
                        painter.circle_stroke(s, 9.0, Stroke::new(1.8, tokens.warning));
                        painter.add(Shape::closed_line(
                            vec![s + vec2(0.0, -8.0), s + vec2(7.0, 5.0), s + vec2(-7.0, 5.0)],
                            mark,
                        ));
                    }
                    Snap::On(_) => {
                        painter.line_segment([s + vec2(-6.0, -6.0), s + vec2(6.0, 6.0)], mark);
                        painter.line_segment([s + vec2(-6.0, 6.0), s + vec2(6.0, -6.0)], mark);
                    }
                    Snap::Free => {
                        painter.circle_filled(s, 2.5, tokens.accent);
                    }
                    Snap::Align(level, upright) => {
                        // Dashed guides from the points it lines up with.
                        let guide = Stroke::new(1.0, tokens.warning.gamma_multiply(0.8));
                        for (with, text) in [(level, "H"), (upright, "V")] {
                            if let Some(q) = with.and_then(|q| sketch.point(q)).and_then(screen) {
                                painter.extend(Shape::dashed_line(&[q, s], guide, 5.0, 4.0));
                                let off = if text == "H" { vec2(0.0, -14.0) } else { vec2(14.0, 0.0) };
                                badge(&painter, s + off, text, tokens, tokens.warning);
                            }
                        }
                        painter.circle_filled(s, 3.0, tokens.warning);
                    }
                }
                if let Some(kind) = level {
                    let text = if kind == Kind::Horizontal { "H" } else { "V" };
                    badge(&painter, s + vec2(14.0, -14.0), text, tokens, tokens.warning);
                }
            }
        }
        self.instructions = hint_text(tool, self.clicks.len(), self.first.is_some(), free);
        // Finish, top centre.
        let button = Rect::from_center_size(pos2(view.rect.center().x, view.rect.top() + 24.0), vec2(150.0, 30.0));
        let finish = egui::Button::new(egui::RichText::new("✔  Finish sketch").strong().color(tokens.on_accent))
            .fill(tokens.accent)
            .corner_radius(4.0);
        if ui.put(button, finish).on_hover_text("Stop drawing; the sketch is a shape the cuts can use").clicked() {
            requests.push(("sketch.finish".into(), json!({})));
        }
    }

    /// The dimensions: lines with their sizes; a size clicked opens for typing a new one.
    #[allow(clippy::too_many_arguments)]
    fn dimensions(
        &mut self,
        ui: &mut Ui,
        sketch: &Sketch,
        to_screen: &dyn Fn(DVec2) -> Option<Pos2>,
        to_sketch: &dyn Fn(Pos2) -> Option<DVec2>,
        px_per_mm: f64,
        units: DisplayUnits,
        tokens: &Tokens,
        preview: Option<u32>,
        requests: &mut Vec<Request>,
    ) {
        let painter = ui.painter().clone();
        let off = f64::from(DIM_PX) / px_per_mm;
        let line = Stroke::new(1.2, tokens.text_muted);
        // The middle of the sketch, so dimensions stand off outwards.
        let (sum, n) = sketch.points.iter().fold((DVec2::ZERO, 0.0), |(s, n), p| (s + p.at, n + 1.0));
        let middle = if n > 0.0 { sum / n } else { DVec2::ZERO };
        for c in sketch.constraints.iter().filter(|c| c.kind.is_dimension() && !Kind::is_alignment(c)) {
            // Where its size goes: being dragged, dragged before, or by default.
            let placed = match self.label_drag {
                Some((id, at)) if id == c.id => Some(at),
                _ => sketch.label(c.id),
            };
            let Some((segments, label_at)) = dim_geometry(sketch, c, off, middle, placed) else { continue };
            let is_preview = preview == Some(c.id);
            let line = if is_preview { Stroke::new(1.5, tokens.warning) } else { line };
            for (a, b) in &segments {
                if let (Some(a), Some(b)) = (to_screen(*a), to_screen(*b)) {
                    painter.line_segment([a, b], line);
                }
            }
            // Arrowheads on the dimension line (the last segment), for straight dimensions.
            if matches!(c.kind, Kind::Distance | Kind::HDistance | Kind::VDistance)
                && let Some((a, b)) = segments.last()
                && let (Some(a), Some(b)) = (to_screen(*a), to_screen(*b))
            {
                arrow(&painter, b, a, line.color);
                arrow(&painter, a, b, line.color);
            }
            let Some(at) = to_screen(label_at) else { continue };
            if is_preview {
                // Not put down yet: its size where it will go, nothing to click.
                let galley = painter.layout_no_wrap(label_text(c, units), FontId::proportional(13.0), tokens.on_accent);
                let rect = Rect::from_center_size(at, galley.size() + vec2(10.0, 4.0));
                painter.rect_filled(rect, 3.0, tokens.warning);
                painter.galley(rect.center() - galley.size() * 0.5, galley, tokens.on_accent);
                continue;
            }
            if let Some((id, text)) = self.typing.as_mut()
                && *id == c.id
            {
                let rect = Rect::from_center_size(at, vec2(90.0, 22.0));
                let field = ui.put(
                    rect,
                    egui::TextEdit::singleline(text)
                        .horizontal_align(egui::Align::Center)
                        .id(ui.id().with("sketch-dim")),
                );
                if !field.has_focus() && !field.lost_focus() {
                    field.request_focus();
                    super::select_all(ui, field.id, text.chars().count());
                }
                if field.lost_focus() {
                    let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    let value = text.trim().parse::<f64>().ok().filter(|v| v.is_finite());
                    if !cancelled && let Some(v) = value {
                        let v = if c.kind == Kind::Angle { v } else { units.to_mm(v) };
                        requests
                            .push(("sketch.dimension.set".into(), json!({ "id": format!("c{}", c.id), "value": v })));
                    }
                    self.typing = None;
                }
                continue;
            }
            let text = label_text(c, units);
            let galley = painter.layout_no_wrap(text, FontId::proportional(13.0), tokens.text);
            let rect = Rect::from_center_size(at, galley.size() + vec2(10.0, 4.0));
            let response = ui
                .interact(rect, ui.id().with(("sketch-dim", c.id)), Sense::click_and_drag())
                .on_hover_text("Click to change it; drag to move it; right-click to delete it");
            // Dragging the size moves the dimension; one command when it is let go.
            if response.dragged()
                && let Some(p) = response.interact_pointer_pos().and_then(to_sketch)
            {
                self.label_drag = Some((c.id, p));
            }
            if response.drag_stopped()
                && let Some((_, p)) = self.label_drag.take()
            {
                requests
                    .push(("sketch.dimension.move".into(), json!({ "id": format!("c{}", c.id), "at": [p.x, p.y] })));
            }
            let selected = self.first.is_none() && response.hovered();
            let fill = if selected { tokens.hover_bg } else { tokens.panel_bg.gamma_multiply(0.9) };
            painter.rect(rect, 3.0, fill, Stroke::new(1.0, tokens.border), egui::StrokeKind::Inside);
            painter.galley(rect.center() - galley.size() * 0.5, galley, tokens.text);
            if response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                self.typing = Some((c.id, value_text(c, units)));
            }
            if response.secondary_clicked() {
                requests.push(("sketch.delete".into(), json!({ "items": [format!("c{}", c.id)] })));
            }
        }
    }
}

/// The grid spacing: 4 in, or 100 mm in metric.
pub fn grid_step(units: DisplayUnits) -> f64 {
    if units == DisplayUnits::Inches { 101.6 } else { 100.0 }
}

/// A grid on the sketch's plane over what the viewport shows: lines every grid step, fainter
/// ones at quarters, and the plane's axes through the origin.
/// Drawn under the view cube and the viewport's buttons, before them.
pub fn plane_grid(painter: &egui::Painter, view: &View<'_>, session: &Session, tokens: &Tokens) {
    let Some(plane) = session
        .view
        .sketch
        .as_deref()
        .and_then(|id| session.job.shapes.iter().find(|s| s.id == id))
        .and_then(|s| s.sketch.as_ref())
        .map(|k| k.plane)
    else {
        return;
    };
    let units = session.prefs.units;
    let to_sketch = |pos: Pos2| -> Option<DVec2> {
        let (o, d) = (view.ray)(pos)?;
        let n = plane.normal();
        let dn = d.dot(n);
        if dn.abs() < 1.0e-9 {
            return None;
        }
        Some(plane.to_2d(o + d * (-o.dot(n) / dn)))
    };
    let r = view.rect;
    let corners: Vec<DVec2> =
        [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].into_iter().filter_map(to_sketch).collect();
    if corners.len() < 4 {
        return;
    }
    let lo = corners.iter().fold(DVec2::splat(f64::INFINITY), |m, p| m.min(*p));
    let hi = corners.iter().fold(DVec2::splat(f64::NEG_INFINITY), |m, p| m.max(*p));
    let major = grid_step(units);
    // Quarters when they are far enough apart to read; otherwise only the main lines.
    let span = (hi - lo).max_element();
    let step = if span / (major / 4.0) <= 240.0 { major / 4.0 } else { major };
    if !(step > 0.0 && span / step <= 400.0) {
        return;
    }
    let to_screen = |p: DVec2| (view.project)(plane.to_3d(p));
    let faint = Stroke::new(1.0, tokens.grid_minor);
    let strong = Stroke::new(1.0, tokens.grid_major);
    let axis = Stroke::new(1.2, tokens.text_muted.gamma_multiply(0.8));
    let line = |a: DVec2, b: DVec2, stroke: Stroke| {
        if let (Some(a), Some(b)) = (to_screen(a), to_screen(b)) {
            painter.line_segment([a, b], stroke);
        }
    };
    let first = |v: f64| (v / step).floor() as i64;
    let last = |v: f64| (v / step).ceil() as i64;
    let every = (major / step).round().max(1.0) as i64;
    for k in first(lo.x)..=last(hi.x) {
        let x = k as f64 * step;
        let stroke = if k == 0 {
            axis
        } else if k % every == 0 {
            strong
        } else {
            faint
        };
        line(DVec2::new(x, lo.y), DVec2::new(x, hi.y), stroke);
    }
    for k in first(lo.y)..=last(hi.y) {
        let y = k as f64 * step;
        let stroke = if k == 0 {
            axis
        } else if k % every == 0 {
            strong
        } else {
            faint
        };
        line(DVec2::new(lo.x, y), DVec2::new(hi.x, y), stroke);
    }
}

/// Picking the plane for a new sketch: the three planes through the turntable's axis, shown
/// see-through at the size of the stock; the one under the pointer lights up and a click starts
/// the sketch on it. Returns whether to keep picking.
pub fn pick_plane(
    ui: &mut Ui,
    view: &View<'_>,
    response: &Response,
    session: &Session,
    tokens: &Tokens,
    instructions: &mut String,
    requests: &mut Vec<Request>,
) -> bool {
    let painter = ui.painter_at(view.rect);
    let b = session.job.stock.bounds();
    let half = f64::from((b.max - b.min).max_element()).max(100.0) * 0.6;
    let height = f64::from(b.max.z).max(half);
    // Each plane's corners in the sketch's own coordinates.
    let extent = |plane: Plane| -> [DVec2; 4] {
        match plane {
            Plane::Top => {
                [DVec2::new(-half, -half), DVec2::new(half, -half), DVec2::new(half, half), DVec2::new(-half, half)]
            }
            _ => [DVec2::new(-half, 0.0), DVec2::new(half, 0.0), DVec2::new(half, height), DVec2::new(-half, height)],
        }
    };
    // The plane under the pointer: the nearest whose square the pointer's ray goes through.
    let hovered = response.hover_pos().and_then(|pos| {
        let (o, d) = (view.ray)(pos)?;
        Plane::ALL
            .into_iter()
            .filter_map(|plane| {
                let n = plane.normal();
                let dn = d.dot(n);
                if dn.abs() < 1.0e-6 {
                    return None;
                }
                let t = -o.dot(n) / dn;
                let p = plane.to_2d(o + d * t);
                let [lo, _, hi, _] = extent(plane);
                (t > 0.0 && p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y).then_some((t, plane))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, plane)| plane)
    });
    for plane in Plane::ALL {
        let pts: Vec<Pos2> = extent(plane).iter().filter_map(|p| (view.project)(plane.to_3d(*p))).collect();
        if pts.len() < 4 {
            continue;
        }
        let lit = hovered == Some(plane);
        let fill = tokens.accent.gamma_multiply(if lit { 0.32 } else { 0.10 });
        let edge = Stroke::new(if lit { 2.0 } else { 1.2 }, tokens.accent.gamma_multiply(if lit { 1.0 } else { 0.6 }));
        painter.add(Shape::convex_polygon(pts.clone(), fill, edge));
        // Each name near its own plane's outer edge (the vertical planes cross in the middle).
        let label = match plane {
            Plane::Front => DVec2::new(-half * 0.62, height * 0.85),
            Plane::Right => DVec2::new(half * 0.62, height * 0.85),
            Plane::Top => DVec2::new(half * 0.62, -half * 0.62),
        };
        let Some(centre) = (view.project)(plane.to_3d(label)) else { continue };
        let name = match plane {
            Plane::Front => "Front",
            Plane::Right => "Right",
            Plane::Top => "Top",
        };
        let ink = if lit { tokens.text } else { tokens.text_muted };
        painter.text(centre, Align2::CENTER_CENTER, name, FontId::proportional(if lit { 16.0 } else { 14.0 }), ink);
    }
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    *instructions = match hovered {
        Some(plane) => format!("New sketch: click to draw on the {} plane.\nEsc cancels.", plane.label()),
        None => "New sketch: click the plane to draw on: Front, Right or Top.\nEsc cancels.".to_owned(),
    };
    if response.clicked_by(egui::PointerButton::Primary)
        && let Some(plane) = hovered
    {
        requests.push(("sketch.new".into(), json!({ "plane": plane.id() })));
        return false;
    }
    let cancel = response.secondary_clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
    !cancel
}

/// The next segment of a chain of lines going `dir` at `start`, towards `to`: an arc leaving
/// tangent to it and ending at `to`, or a 45° chamfer turning towards `to` as far as it reaches.
fn next_segment(kind: Segment, start: DVec2, dir: DVec2, to: DVec2) -> Option<Next> {
    let dir = dir.try_normalize()?;
    let v = to - start;
    match kind {
        Segment::Line => {
            Some(Next { end: to, through: None, points: vec![start, to], dir: v.try_normalize().unwrap_or(dir) })
        }
        Segment::Chamfer => {
            let left = dir.perp_dot(v) >= 0.0;
            let turn = if left { 45.0f64 } else { -45.0 }.to_radians();
            let u = DVec2::from_angle(turn).rotate(dir);
            let len = v.dot(u);
            if len < 1.0e-3 {
                return None;
            }
            let end = start + u * len;
            Some(Next { end, through: None, points: vec![start, end], dir: u })
        }
        Segment::Arc => {
            // The centre is square to the line at the start, on the side the pointer is.
            let left = dir.perp_dot(v) >= 0.0;
            let n = if left { dir.perp() } else { -dir.perp() };
            let den = 2.0 * v.dot(n);
            if den.abs() < 1.0e-6 * v.length().max(1.0) {
                return None;
            }
            let r = v.length_squared() / den;
            let centre = start + n * r;
            // Counter-clockwise when it turns left.
            let (a, b) = if left { (start, to) } else { (to, start) };
            let mut points = arc_points(centre, a, b);
            if !left {
                points.reverse();
            }
            let through = *points.get(points.len() / 2)?;
            let radial = (to - centre).try_normalize()?;
            let along = if left { radial.perp() } else { -radial.perp() };
            Some(Next { end: to, through: Some(through), points, dir: along })
        }
    }
}

/// Whether `item` is selected, or is a point of a selected curve.
fn in_selection(s: &Sketch, selection: &[u32], item: u32) -> bool {
    selection.contains(&item)
        || selection.iter().filter_map(|&i| s.entity(i)).any(|e| match e.geom {
            Geom::Line { a, b } => a == item || b == item,
            Geom::Arc { c, a, b } => [c, a, b].contains(&item),
            Geom::Circle { c, .. } => c == item,
            Geom::Point { p } => p == item,
        })
}

/// The curves (and lone points) a selection window takes: wholly inside it, or (crossing)
/// any part of them in it.
fn window_items(s: &Sketch, to_screen: &dyn Fn(DVec2) -> Option<Pos2>, rect: Rect, crossing: bool) -> Vec<u32> {
    s.entities
        .iter()
        .filter(|e| {
            let samples: Vec<DVec2> = match e.geom {
                // A line is tested along its length, so a crossing window across its middle takes it.
                Geom::Line { a, b } => match (s.point(a), s.point(b)) {
                    (Some(p), Some(q)) => (0..=24).map(|k| p.lerp(q, f64::from(k) / 24.0)).collect(),
                    _ => Vec::new(),
                },
                Geom::Point { p } => s.point(p).into_iter().collect(),
                g => curve_points(s, g),
            };
            let inside: Vec<bool> = samples.iter().map(|p| to_screen(*p).is_some_and(|q| rect.contains(q))).collect();
            !inside.is_empty() && if crossing { inside.iter().any(|x| *x) } else { inside.iter().all(|x| *x) }
        })
        .map(|e| e.id)
        .collect()
}

/// The points along a curve, in the sketch.
fn curve_points(s: &Sketch, g: Geom) -> Vec<DVec2> {
    match g {
        Geom::Line { a, b } => [s.point(a), s.point(b)].into_iter().flatten().collect(),
        Geom::Circle { c, r } => s
            .point(c)
            .map(|c| (0..=96).map(|k| c + DVec2::from_angle(std::f64::consts::TAU * k as f64 / 96.0) * r).collect())
            .unwrap_or_default(),
        Geom::Arc { c, a, b } => match (s.point(c), s.point(a), s.point(b)) {
            (Some(c), Some(a), Some(b)) => arc_points(c, a, b),
            _ => Vec::new(),
        },
        Geom::Point { .. } => Vec::new(),
    }
}

/// The item under `at`: a point within `tol` × [`POINT_REACH`] (mm), else a curve within
/// `tol`. With `curves_first` a curve wins unless the point is very near (to dimension the
/// angle between two lines that meet).
fn pick(s: &Sketch, at: DVec2, tol: f64, curves_first: bool) -> Option<u32> {
    // Points, and the middles of lines (picked as `MIDPOINT` items).
    let mids = s.entities.iter().filter_map(|e| match e.geom {
        Geom::Line { a, b } => Some(((s.point(a)? + s.point(b)?) * 0.5, e.id | MIDPOINT)),
        _ => None,
    });
    let point = s
        .points
        .iter()
        .map(|p| (p.at, p.id))
        .chain(mids)
        .map(|(p, id)| (p.distance(at), id))
        .filter(|(d, _)| *d <= tol * POINT_REACH)
        .min_by(|a, b| a.0.total_cmp(&b.0));
    let curve = nearest_curve(s, at, tol);
    match (point, curve) {
        (Some((d, p)), Some(c)) if curves_first => Some(if d <= tol * 0.4 { p } else { c }),
        (Some((_, p)), _) => Some(p),
        (None, c) => c,
    }
}

fn nearest_curve(s: &Sketch, at: DVec2, tol: f64) -> Option<u32> {
    s.entities
        .iter()
        .filter_map(|e| {
            let d = match e.geom {
                Geom::Line { a, b } => {
                    let (pa, pb) = (s.point(a)?, s.point(b)?);
                    closest_on_segment(pa, pb, at).distance(at)
                }
                Geom::Circle { c, r } => (s.point(c)?.distance(at) - r).abs(),
                Geom::Arc { c, a, b } => {
                    let (pc, pa, pb) = (s.point(c)?, s.point(a)?, s.point(b)?);
                    if !CosmicCut_engine::sketch::in_arc(pc, pa, pb, at) {
                        return None;
                    }
                    (pc.distance(at) - pc.distance(pa)).abs()
                }
                Geom::Point { .. } => return None,
            };
            (d <= tol).then_some((d, e.id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

/// A constraint's marks: a small boxed glyph beside each thing it holds.
fn glyphs(painter: &egui::Painter, s: &Sketch, to_screen: &dyn Fn(DVec2) -> Option<Pos2>, tokens: &Tokens) {
    let mut stacked: std::collections::HashMap<u32, f32> = std::collections::HashMap::new();
    // Two points lined up: a faint dashed line between them, and H or V at its middle.
    for c in s.constraints.iter().filter(|c| Kind::is_alignment(c)) {
        let (Some(p), Some(q)) = (c.refs.first().and_then(|&p| s.point(p)), c.refs.get(1).and_then(|&q| s.point(q)))
        else {
            continue;
        };
        if let (Some(a), Some(b)) = (to_screen(p), to_screen(q)) {
            painter.extend(Shape::dashed_line(
                &[a, b],
                Stroke::new(1.0, tokens.text_muted.gamma_multiply(0.7)),
                4.0,
                4.0,
            ));
            let text = if c.kind == Kind::HDistance { "V" } else { "H" };
            badge(painter, a + (b - a) * 0.5 + vec2(12.0, 0.0), text, tokens, tokens.text_muted);
        }
    }
    for c in s.constraints.iter().filter(|c| !c.kind.is_dimension()) {
        if c.kind == Kind::Fix && c.refs.first() == Some(&ORIGIN) {
            continue;
        }
        let text = match c.kind {
            Kind::Horizontal => "H",
            Kind::Vertical => "V",
            Kind::Parallel => "//",
            Kind::Perpendicular => "⊥",
            Kind::Equal => "=",
            Kind::Midpoint => "M",
            Kind::OnCurve => "o",
            Kind::Tangent => "T",
            Kind::Concentric => "◎",
            Kind::Fix => "F",
            _ => continue,
        };
        // Where: beside a line's middle, or by a point.
        let anchors: Vec<(u32, DVec2, DVec2)> = match c.kind {
            Kind::Midpoint | Kind::OnCurve | Kind::Fix => {
                c.refs.first().and_then(|&p| Some((p, s.point(p)?, DVec2::new(0.7, 0.7)))).into_iter().collect()
            }
            _ => c
                .refs
                .iter()
                .filter_map(|&id| {
                    let e = s.entity(id)?;
                    let pts = curve_points(s, e.geom);
                    let (a, b) = (*pts.first()?, *pts.last()?);
                    let mid = match e.geom {
                        Geom::Line { .. } => (a + b) * 0.5,
                        _ => *pts.get(pts.len() / 2)?,
                    };
                    let normal = (b - a).perp().try_normalize().unwrap_or(DVec2::Y);
                    Some((id, mid, normal))
                })
                .collect(),
        };
        for (key, at, normal) in anchors {
            let Some(screen) = to_screen(at) else { continue };
            let Some(tip) = to_screen(at + normal) else { continue };
            let dir = (tip - screen).normalized();
            let dir = if dir.x.is_finite() { dir } else { vec2(0.0, -1.0) };
            let k = stacked.entry(key).or_insert(0.0);
            let pos = screen + dir * 16.0 + vec2(-dir.y, dir.x) * (*k * 18.0);
            *k += 1.0;
            badge(painter, pos, text, tokens, tokens.text_muted);
        }
    }
}

/// A small boxed mark. "⊥" is drawn with lines (the interface font has no such character).
fn badge(painter: &egui::Painter, at: Pos2, text: &str, tokens: &Tokens, ink: Color32) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(10.5), ink);
    let width = if text == "⊥" { 10.0 } else { galley.size().x.max(10.0) };
    let rect = Rect::from_center_size(at, vec2(width + 6.0, 15.0));
    painter.rect(
        rect,
        3.0,
        tokens.panel_bg.gamma_multiply(0.92),
        Stroke::new(1.0, ink.gamma_multiply(0.7)),
        egui::StrokeKind::Inside,
    );
    if text == "⊥" {
        let c = rect.center();
        let line = Stroke::new(1.3, ink);
        painter.line_segment([c + vec2(-4.0, 4.0), c + vec2(4.0, 4.0)], line);
        painter.line_segment([c + vec2(0.0, 4.0), c + vec2(0.0, -4.5)], line);
    } else {
        painter.galley(rect.center() - galley.size() * 0.5, galley, ink);
    }
}

fn arrow(painter: &egui::Painter, from: Pos2, tip: Pos2, colour: Color32) {
    let d: Vec2 = (tip - from).normalized();
    if !d.x.is_finite() || d == Vec2::ZERO {
        return;
    }
    let n = vec2(-d.y, d.x);
    let back = tip - d * 8.0;
    painter.add(Shape::convex_polygon(vec![tip, back + n * 3.0, back - n * 3.0], colour, Stroke::NONE));
}

/// A dimension's lines (in the sketch) and where its size goes; dimension lines stand `off`
/// (mm) off what they measure, away from `middle`.
fn dim_geometry(
    s: &Sketch,
    c: &Constraint,
    off: f64,
    middle: DVec2,
    placed: Option<DVec2>,
) -> Option<(Vec<(DVec2, DVec2)>, DVec2)> {
    let r0 = c.refs.first().copied()?;
    let r1 = c.refs.get(1).copied();
    match c.kind {
        Kind::Distance => {
            let p = s.point(r0)?;
            if let Some((a, b)) = r1.and_then(|l| s.line(l)) {
                // A point's distance from a line (or between parallel lines, from a point on
                // the other): the dimension line square to the line, through where its size is
                // put, and the line, and the point (along the line), carried out to meet it.
                let d = (b - a).try_normalize()?;
                let foot = a + d * (p - a).dot(d);
                let across = p - foot;
                let at = placed.unwrap_or((p + foot) * 0.5);
                let slide = (at - foot).dot(d);
                let (on_line, on_point) = (foot + d * slide, p + d * slide);
                // From the nearer end of the line, if the dimension line is off its length.
                let t = |q: DVec2| (q - a).dot(d);
                let (ta, tb, ts) = (0.0f64, t(b), t(on_line));
                let line_end = if ts < ta.min(tb) {
                    Some(if ta < tb { a } else { b })
                } else if ts > ta.max(tb) {
                    Some(if ta > tb { a } else { b })
                } else {
                    None
                };
                let mut segments = Vec::new();
                if let Some(end) = line_end {
                    segments.push((end, on_line));
                }
                if slide.abs() > 1.0e-9 {
                    segments.push((p, on_point));
                }
                segments.push((on_line, on_point));
                let label = on_line + across * ((at - on_line).dot(across) / across.length_squared().max(1.0e-12));
                return Some((segments, label));
            }
            let q = s.point(r1?)?;
            let along = (q - p).try_normalize()?;
            let mut n = along.perp();
            if n.dot((p + q) * 0.5 - middle) < 0.0 {
                n = -n;
            }
            // Dragged, the line stands where the size was put, which slides along it.
            let (gap, slide) = match placed {
                Some(l) => ((l - p).dot(n), (l - p).dot(along)),
                None => (off, (q - p).dot(along) * 0.5),
            };
            let (pa, qa) = (p + n * gap, q + n * gap);
            let past = n * gap.signum() * off * 0.15;
            Some((vec![(p, pa + past), (q, qa + past), (pa, qa)], pa + along * slide))
        }
        Kind::HDistance | Kind::VDistance => {
            let (p, q) = (s.point(r0)?, s.point(r1?)?);
            if c.kind == Kind::HDistance {
                let y = placed.map_or(p.y.max(q.y) + off, |l| l.y);
                let (pa, qa) = (DVec2::new(p.x, y), DVec2::new(q.x, y));
                let x = placed.map_or((p.x + q.x) * 0.5, |l| l.x);
                Some((vec![(p, pa), (q, qa), (pa, qa)], DVec2::new(x, y)))
            } else {
                let x = placed.map_or(p.x.max(q.x) + off, |l| l.x);
                let (pa, qa) = (DVec2::new(x, p.y), DVec2::new(x, q.y));
                let y = placed.map_or((p.y + q.y) * 0.5, |l| l.y);
                Some((vec![(p, pa), (q, qa), (pa, qa)], DVec2::new(x, y)))
            }
        }
        Kind::Radius | Kind::Diameter => {
            let e = s.entity(r0)?;
            let r = s.radius(e)?;
            let centre = match e.geom {
                Geom::Circle { c, .. } | Geom::Arc { c, .. } => s.point(c)?,
                _ => return None,
            };
            let dir = placed
                .and_then(|l| (l - centre).try_normalize())
                .unwrap_or(DVec2::from_angle(std::f64::consts::FRAC_PI_4));
            let rim = centre + dir * r;
            let label = placed.unwrap_or(centre + dir * (r + off));
            let from = if c.kind == Kind::Diameter { centre - dir * r } else { centre };
            Some((vec![(from, rim), (rim, label)], label))
        }
        Kind::Angle => {
            let (a0, a1) = s.line(r0)?;
            let (b0, b1) = s.line(r1?)?;
            let (da, db) = (a1 - a0, b1 - b0);
            let den = da.perp_dot(db);
            let vertex = if den.abs() > 1.0e-12 { a0 + da * ((b0 - a0).perp_dot(db) / den) } else { (a0 + b0) * 0.5 };
            // Out from the corner along each line, as the angle is measured.
            let (fa, fb) = CosmicCut_engine::sketch::angle_flips(c.value2);
            let (ra, rb) = ((da * fa).try_normalize()?, (db * fb).try_normalize()?);
            let bis = (ra + rb).try_normalize().unwrap_or(ra.perp());
            let at = placed.unwrap_or(vertex + bis * off * 1.8);
            let radius = at.distance(vertex).max(off * 0.5);
            let (t0, mut sweep) = (ra.to_angle(), ra.angle_to(rb));
            if sweep.abs() < 1.0e-9 {
                sweep = 0.0;
            }
            let steps = 24;
            let arc: Vec<DVec2> = (0..=steps)
                .map(|k| vertex + DVec2::from_angle(t0 + sweep * k as f64 / steps as f64) * radius)
                .collect();
            let mut segments: Vec<(DVec2, DVec2)> = arc.windows(2).map(|w| (w[0], w[1])).collect();
            // Extension lines out along each line to the arc.
            segments.push((vertex, vertex + ra * radius * 1.1));
            segments.push((vertex, vertex + rb * radius * 1.1));
            Some((segments, at))
        }
        _ => None,
    }
}

fn label_text(c: &Constraint, units: DisplayUnits) -> String {
    match c.kind {
        Kind::Angle => format!("{}°", trim(c.value.abs())),
        Kind::Radius => format!("R {}", units.plain(c.value)),
        Kind::Diameter => format!("Ø {}", units.plain(c.value)),
        _ => units.plain(c.value),
    }
}

fn value_text(c: &Constraint, units: DisplayUnits) -> String {
    if c.kind == Kind::Angle { trim(c.value.abs()) } else { units.plain(c.value) }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// What to do next, at the top of the viewport.
/// What the tool wants next, and how far the sketch is constrained: for the Instructions tab.
fn hint_text(tool: &str, clicks: usize, picked: bool, free: usize) -> String {
    let what = match (tool, clicks) {
        ("line", 0) => "Line: click where it starts.",
        ("line", _) => {
            "Line: click where it ends; it carries on from there. R makes the next segment an arc tangent to it, \
             C a 45° chamfer. Esc or right-click stops."
        }
        ("rectangle", 0) => "Rectangle: click a corner.",
        ("rectangle", _) => "Rectangle: click the opposite corner.",
        ("rectangle_centre", 0) => "Centre rectangle: click the centre.",
        ("rectangle_centre", _) => "Centre rectangle: click a corner.",
        ("circle", 0) => "Circle: click the centre.",
        ("circle", _) => "Circle: click a point on the edge.",
        ("arc", 0) => "Arc: click where it starts.",
        ("arc", 1) => "Arc: click where it ends.",
        ("arc", _) => "Arc: click a point it passes through.",
        ("polygon", 0) => "Polygon: click the centre.",
        ("polygon", _) => "Polygon: click a corner.",
        ("point", _) => "Point: click to put one down.",
        ("dimension", _) if picked => {
            "Dimension: click empty space to put it there, or a second item for the distance or angle between them."
        }
        ("dimension", _) => {
            "Dimension: click a line, circle, arc or point (two lines for an angle). Click a size to change it, \
             drag it to move it."
        }
        ("move", _) => "Move: drag to move what is selected, or the whole sketch.",
        _ => {
            "Select: click items (Ctrl or Shift adds), or drag a window over them (left to right: wholly inside; \
             right to left: touching). Ctrl+A selects everything. Drag a selected item to move the selection \
             together. Constraints that do not suit the selection are greyed out."
        }
    };
    let state = if free == 0 {
        "The sketch is fully constrained.".to_owned()
    } else {
        format!("{free} degrees of freedom left.")
    };
    format!("{what}\n{state}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chains_carry_on_as_tangent_arcs_and_chamfers() {
        let start = DVec2::new(100.0, 0.0);
        let east = DVec2::X;
        // A chamfer turns 45° towards the pointer, as far as it reaches.
        let ch = next_segment(Segment::Chamfer, start, east, DVec2::new(160.0, 80.0));
        assert!(ch.as_ref().is_some_and(|n| (n.dir.to_angle().to_degrees() - 45.0).abs() < 1e-9));
        let down = next_segment(Segment::Chamfer, start, east, DVec2::new(160.0, -80.0));
        assert!(down.is_some_and(|n| (n.dir.to_angle().to_degrees() + 45.0).abs() < 1e-9));
        // An arc leaves going the way the line went, and ends at the pointer.
        let arc = next_segment(Segment::Arc, start, east, DVec2::new(150.0, 50.0));
        assert!(arc.as_ref().is_some_and(|n| n.end == DVec2::new(150.0, 50.0)));
        let pts = arc.map(|n| n.points).unwrap_or_default();
        let first = pts.get(1).copied().unwrap_or_default() - start;
        assert!(first.normalize().dot(east) > 0.99, "leaves tangent: {first:?}");
    }

    #[test]
    fn a_gap_between_parallel_lines_is_drawn_where_it_is_put() -> CosmicCut_engine::EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        let low = s.add_line(DVec2::new(0.0, 10.0), DVec2::new(100.0, 10.0), 0.5)?;
        let high = s.add_line(DVec2::new(20.0, 60.0), DVec2::new(80.0, 60.0), 0.5)?;
        let id = s.dimension(&[low, high], None, None)?;
        let c = s.constraint(id).cloned().unwrap_or(Constraint {
            id: 0,
            kind: Kind::Fix,
            refs: vec![],
            value: 0.0,
            value2: 0.0,
        });
        assert!((c.value - 50.0).abs() < 1e-6);
        // Put past the end of both lines: the dimension line is square to them there, from one
        // line to the other, and both are carried out to it.
        let (segments, label) = dim_geometry(&s, &c, 10.0, DVec2::ZERO, Some(DVec2::new(130.0, 35.0)))
            .ok_or_else(|| CosmicCut_engine::EngineError::Internal("no geometry".into()))?;
        let (from, to) = *segments.last().unwrap_or(&(DVec2::ZERO, DVec2::ZERO));
        assert!((from.x - 130.0).abs() < 1e-6 && (to.x - 130.0).abs() < 1e-6, "{segments:?}");
        assert!(((from.y - to.y).abs() - 50.0).abs() < 1e-6);
        assert_eq!(label, DVec2::new(130.0, 35.0));
        assert_eq!(segments.len(), 3, "both lines carried out: {segments:?}");
        Ok(())
    }

    #[test]
    fn windows_take_what_is_inside_and_crossings_what_they_touch() -> CosmicCut_engine::EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        let short = s.add_line(DVec2::new(10.0, 10.0), DVec2::new(40.0, 10.0), 0.5)?;
        let long = s.add_line(DVec2::new(10.0, 50.0), DVec2::new(200.0, 50.0), 0.5)?;
        // The sketch drawn 1:1 on screen, y down.
        let to_screen = |p: DVec2| Some(pos2(p.x as f32, -p.y as f32));
        let rect = Rect::from_min_max(pos2(0.0, -60.0), pos2(100.0, 0.0));
        assert_eq!(window_items(&s, &to_screen, rect, false), vec![short]);
        let mut both = window_items(&s, &to_screen, rect, true);
        both.sort_unstable();
        assert_eq!(both, vec![short, long]);
        Ok(())
    }

    #[test]
    fn picks_points_before_curves() -> CosmicCut_engine::EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        let line = s.add_line(DVec2::new(10.0, 10.0), DVec2::new(110.0, 10.0), 0.5)?;
        assert_eq!(pick(&s, DVec2::new(35.0, 10.4), 1.0, false), Some(line));
        // Near its middle, the midpoint.
        assert_eq!(pick(&s, DVec2::new(60.8, 10.6), 1.0, false), Some(line | MIDPOINT));
        let end = s.points.iter().find(|p| p.at == DVec2::new(10.0, 10.0)).map(|p| p.id);
        assert_eq!(pick(&s, DVec2::new(10.3, 10.2), 1.0, false), end);
        // An end point is taken from a little further off than a curve.
        assert_eq!(pick(&s, DVec2::new(8.6, 10.6), 1.0, false), end);
        // Dimensioning takes the line unless right on its end.
        assert_eq!(pick(&s, DVec2::new(11.0, 10.3), 1.0, true), Some(line));
        assert_eq!(pick(&s, DVec2::new(60.0, 40.0), 1.0, false), None);
        Ok(())
    }
}
