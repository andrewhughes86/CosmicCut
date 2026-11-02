//! The stock's dimensions, drawn over the viewport while the Stock tab is open: width, depth
//! and height for a block, diameter and height for a round billet. Each value is a button;
//! clicking it opens a field there, and Enter sets the stock through its command.

use CosmicCut_engine::stock::{Stock, StockShape};
use CosmicCut_engine::units::DisplayUnits;
use CosmicCut_geom::Vec3;
use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2, vec2};
use serde_json::json;

use crate::Request;
use crate::theme::Tokens;

/// One dimension: the parameter it sets, the edge it measures, which way it stands off the
/// stock, and its length in millimetres.
struct Dim {
    key: &'static str,
    a: Vec3,
    b: Vec3,
    off: Vec3,
    mm: f64,
}

/// A value being typed in the viewport: its parameter and the text so far.
pub type DimEdit = Option<(&'static str, String)>;

/// The dimensions to draw, standing off the sides that face the viewer (`toward` points from
/// the stock to the camera; `right` is the screen's right in the table frame).
fn dims(stock: &Stock, toward: Vec3, right: Vec3) -> Vec<Dim> {
    let c = Vec3::new(stock.centre.x as f32, stock.centre.y as f32, stock.base as f32);
    let up = Vec3::Z;
    match stock.shape {
        StockShape::Block { size } => {
            let (w, d, h) = (size.x as f32, size.y as f32, size.z as f32);
            let gap = w.max(d).max(h) * 0.12;
            let (s, co) = (stock.rotation as f32).to_radians().sin_cos();
            let ux = Vec3::new(co, s, 0.0);
            let uy = Vec3::new(-s, co, 0.0);
            let sx = if ux.dot(toward) >= 0.0 { 1.0 } else { -1.0 };
            let sy = if uy.dot(toward) >= 0.0 { 1.0 } else { -1.0 };
            let front = c + uy * (sy * d / 2.0);
            let side = c + ux * (sx * w / 2.0);
            let corner = c + ux * (-sx * w / 2.0) + uy * (sy * d / 2.0);
            vec![
                Dim {
                    key: "width",
                    a: front - ux * (w / 2.0),
                    b: front + ux * (w / 2.0),
                    off: uy * (sy * gap),
                    mm: size.x,
                },
                Dim {
                    key: "depth",
                    a: side - uy * (d / 2.0),
                    b: side + uy * (d / 2.0),
                    off: ux * (sx * gap),
                    mm: size.y,
                },
                Dim { key: "height", a: corner, b: corner + up * h, off: ux * (-sx * gap), mm: size.z },
            ]
        }
        StockShape::Cylinder { radius, height } => {
            let (r, h) = (radius as f32, height as f32);
            let gap = (2.0 * r).max(h) * 0.12;
            let across = Vec3::new(right.x, right.y, 0.0).try_normalize().unwrap_or(Vec3::X);
            let top = c + up * h;
            let side = c + across * r;
            vec![
                Dim { key: "diameter", a: top - across * r, b: top + across * r, off: up * gap, mm: radius * 2.0 },
                Dim { key: "height", a: side, b: side + up * h, off: across * gap, mm: height },
            ]
        }
    }
}

/// How the viewport sees the table: its rectangle, the direction to the camera and the
/// screen's right (both in the table frame), and table frame to screen.
pub struct View<P: Fn(Vec3) -> Option<Pos2>> {
    pub rect: Rect,
    pub toward: Vec3,
    pub right: Vec3,
    pub project: P,
}

/// Draws the dimensions and their fields. Clicking a value starts editing it in `edit`; Enter
/// (or clicking away) sets it.
pub fn show(
    ui: &mut Ui,
    view: View<impl Fn(Vec3) -> Option<Pos2>>,
    stock: &Stock,
    units: DisplayUnits,
    tokens: &Tokens,
    edit: &mut DimEdit,
    requests: &mut Vec<Request>,
) {
    let View { rect, toward, right, project } = view;
    let painter = ui.painter_at(rect);
    let line = Stroke::new(1.4, tokens.accent);
    let ext = Stroke::new(1.0, tokens.accent.gamma_multiply(0.6));
    for dim in dims(stock, toward, right) {
        let (Some(a), Some(b), Some(a2), Some(b2)) =
            (project(dim.a), project(dim.b), project(dim.a + dim.off), project(dim.b + dim.off))
        else {
            continue;
        };
        // Extension lines from the stock a little past the dimension line, then the line itself
        // with an arrowhead at each end.
        let past = |from: Pos2, to: Pos2| to + (to - from).normalized() * 6.0;
        painter.line_segment([a, past(a, a2)], ext);
        painter.line_segment([b, past(b, b2)], ext);
        painter.line_segment([a2, b2], line);
        arrow(&painter, b2, a2, tokens);
        arrow(&painter, a2, b2, tokens);

        let mid = a2 + (b2 - a2) * 0.5;
        let editing = edit.as_ref().is_some_and(|(k, _)| *k == dim.key);
        if editing {
            field(ui, mid, stock, units, edit, requests);
            continue;
        }
        let text = format!("{} {}", units.plain(dim.mm), units.length());
        let galley = painter.layout_no_wrap(text, FontId::proportional(13.0), tokens.text);
        let label = Rect::from_center_size(mid, galley.size() + vec2(14.0, 6.0));
        let response = ui
            .interact(label, ui.id().with(("stock-dim", dim.key)), Sense::click())
            .on_hover_text(format!("{}: click to change it", crate::stock_panel::label(dim.key)));
        let fill = if response.hovered() { tokens.hover_bg } else { tokens.panel_bg };
        painter.rect(label, 4.0, fill, Stroke::new(1.0, tokens.accent), egui::StrokeKind::Inside);
        painter.galley(label.center() - galley.size() * 0.5, galley, tokens.text);
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            *edit = Some((dim.key, units.plain(dim.mm)));
        }
    }
}

fn arrow(painter: &egui::Painter, from: Pos2, tip: Pos2, tokens: &Tokens) {
    let d: Vec2 = (tip - from).normalized();
    if !d.x.is_finite() || d == Vec2::ZERO {
        return;
    }
    let n = vec2(-d.y, d.x);
    let back = tip - d * 9.0;
    painter.add(egui::Shape::convex_polygon(vec![tip, back + n * 3.5, back - n * 3.5], tokens.accent, Stroke::NONE));
}

/// The field a value is typed into, where its label was.
fn field(ui: &mut Ui, at: Pos2, stock: &Stock, units: DisplayUnits, edit: &mut DimEdit, requests: &mut Vec<Request>) {
    let Some((key, text)) = edit.as_mut() else { return };
    let key = *key;
    let rect = Rect::from_center_size(at, vec2(110.0, 24.0));
    let response = ui.put(
        rect,
        egui::TextEdit::singleline(text).horizontal_align(egui::Align::Center).id(ui.id().with("stock-dim-edit")),
    );
    if !response.has_focus() && !response.lost_focus() {
        response.request_focus();
        super::select_all(ui, response.id, text.chars().count());
    }
    ui.painter().text(
        rect.right_center() + vec2(6.0, 0.0),
        Align2::LEFT_CENTER,
        units.length(),
        FontId::proportional(12.0),
        ui.visuals().text_color(),
    );
    if !response.lost_focus() {
        return;
    }
    let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let value = text.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    if !cancelled && let Some(v) = value {
        requests.push(request(stock, key, units.to_mm(v)));
    }
    *edit = None;
}

/// The stock's command with one value changed, the others as they are.
fn request(stock: &Stock, key: &str, mm: f64) -> Request {
    let mut params = CosmicCut_engine::cmd::stock::prefill(stock);
    if let Some(o) = params.as_object_mut() {
        o.insert(key.into(), json!(mm));
    }
    let command = match stock.shape {
        StockShape::Block { .. } => "stock.block",
        StockShape::Cylinder { .. } => "stock.cylinder",
    };
    (command.into(), params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{DVec2, DVec3};

    #[test]
    fn a_block_has_three_dimensions_and_a_billet_two() {
        let block = Stock::block(DVec3::new(300.0, 200.0, 100.0), DVec2::ZERO, 0.0, 0.0).unwrap_or_default();
        let keys: Vec<&str> = dims(&block, Vec3::new(1.0, -1.0, 1.0), Vec3::X).iter().map(|d| d.key).collect();
        assert_eq!(keys, ["width", "depth", "height"]);
        let width = &dims(&block, Vec3::new(1.0, -1.0, 1.0), Vec3::X)[0];
        assert!((width.a.distance(width.b) - 300.0).abs() < 1e-3);
        let round = Stock::cylinder(50.0, 120.0, DVec2::ZERO, 0.0).unwrap_or_default();
        let keys: Vec<&str> = dims(&round, Vec3::Z, Vec3::X).iter().map(|d| d.key).collect();
        assert_eq!(keys, ["diameter", "height"]);
    }

    #[test]
    fn a_value_typed_in_the_viewport_keeps_the_others() {
        let block = Stock::block(DVec3::new(300.0, 200.0, 100.0), DVec2::new(5.0, 0.0), 10.0, 0.0).unwrap_or_default();
        let (id, p) = request(&block, "height", 150.0);
        assert_eq!(id, "stock.block");
        assert_eq!(p["height"], 150.0);
        assert_eq!(p["width"], 300.0);
        assert_eq!(p["base"], 10.0);
        assert_eq!(p["x"], 5.0);
    }
}
