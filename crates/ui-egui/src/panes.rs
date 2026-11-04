//! Explorer, inspector, cut plane, G-code and console panes. They read the session and act only
//! through commands.

use CosmicCut_engine::Session;
use CosmicCut_engine::cut::{Built, CutKind};
use CosmicCut_engine::machine::Origin;
use CosmicCut_engine::session::{MACHINE_ID, MODELS_ID, ORIGIN_ID, OutlineNode, PROGRAM_ID, SHAPES_ID, STOCK_ID};
use CosmicCut_engine::units::DisplayUnits;
use CosmicCut_engine::wire::{MoveKind, WirePath};
use egui::collapsing_header::CollapsingState;
use egui::{Pos2, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::Request;
use crate::theme::Tokens;

pub fn explorer(ui: &mut Ui, session: &Session, tokens: &Tokens, requests: &mut Vec<Request>) {
    let selection = session.view.selection.as_deref();
    let chosen = session.selected_ids();
    let outline = session.outline();
    // Every row's id, top to bottom, for Shift-click ranges.
    fn flat(nodes: &[OutlineNode], out: &mut Vec<String>) {
        for n in nodes {
            out.push(n.id.clone());
            flat(&n.children, out);
        }
    }
    let mut rows = Vec::new();
    flat(&outline, &mut rows);
    let picked = Picked { selection, chosen: &chosen, rows: &rows };
    let no_drag = egui::scroll_area::ScrollSource {
        drag: egui::scroll_area::DragScroll::Never,
        ..egui::scroll_area::ScrollSource::ALL
    };
    ScrollArea::both().scroll_source(no_drag).auto_shrink([false, false]).show(ui, |ui| {
        for node in &outline {
            outline_node(ui, node, session, tokens, &picked, requests);
        }
    });
    // Delete removes the selected cuts, models and shapes (when no text field has the keyboard).
    let typing = ui.ctx().memory(|m| m.focused().is_some());
    if !typing && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
        requests.extend(delete_request(session, &chosen));
    }
}

/// What is selected, for drawing and clicking the rows.
struct Picked<'a> {
    selection: Option<&'a str>,
    chosen: &'a [String],
    rows: &'a [String],
}

/// Asking to delete `ids` (those that can be): one item by its own command, several in one step.
pub fn delete_request(session: &Session, ids: &[String]) -> Option<Request> {
    let doomed: Vec<&String> = ids.iter().filter(|id| delete_command(id).is_some()).collect();
    match doomed.as_slice() {
        [] => None,
        [one] => {
            let label = session.selection_node(one).map(|n| n.label).unwrap_or_else(|| (*one).clone());
            Some(("ui.delete".into(), json!({ "command": delete_command(one), "id": one, "label": label })))
        }
        many => {
            let label = format!("these {} items", many.len());
            Some(("ui.delete".into(), json!({ "command": "edit.delete", "ids": many, "label": label })))
        }
    }
}

fn delete_command(id: &str) -> Option<&'static str> {
    if id.starts_with("cut.") {
        Some("cut.remove")
    } else if id.starts_with("model.") {
        Some("model.remove")
    } else if id.starts_with("shape.") {
        Some("shape.remove")
    } else {
        None
    }
}

/// An eye button: open when the item is drawn, closed (struck through) when hidden.
fn eye(ui: &mut Ui, visible: bool, tokens: &Tokens) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 16.0), egui::Sense::click());
    let painter = ui.painter_at(rect);
    let c = rect.center();
    let colour = if response.hovered() {
        tokens.accent
    } else if visible {
        tokens.text
    } else {
        tokens.text_muted.gamma_multiply(0.7)
    };
    let stroke = egui::Stroke::new(1.3, colour);
    let arc = |up: f32| -> Vec<egui::Pos2> {
        (0..=12)
            .map(|k| {
                let t = k as f32 / 12.0;
                let x = -6.5 + 13.0 * t;
                c + egui::vec2(x, up * 4.2 * (1.0 - (2.0 * t - 1.0).powi(2)))
            })
            .collect()
    };
    painter.add(egui::Shape::line(arc(-1.0), stroke));
    painter.add(egui::Shape::line(arc(1.0), stroke));
    if visible {
        painter.circle_filled(c, 2.2, colour);
    } else {
        painter.line_segment([c + egui::vec2(-6.0, 5.0), c + egui::vec2(6.0, -5.0)], stroke);
    }
    response.on_hover_text(if visible { "Shown: click to hide" } else { "Hidden: click to show" })
}

fn outline_node(
    ui: &mut Ui,
    node: &OutlineNode,
    session: &Session,
    tokens: &Tokens,
    picked: &Picked<'_>,
    requests: &mut Vec<Request>,
) {
    let selected = picked.selection == Some(node.id.as_str()) || picked.chosen.contains(&node.id);
    let row = |ui: &mut Ui, requests: &mut Vec<Request>| {
        match node.visible {
            Some(visible) => {
                if eye(ui, visible, tokens).clicked() {
                    match node.id.as_str() {
                        // The stock and the origin are display settings.
                        STOCK_ID | ORIGIN_ID => {
                            requests.push(("view.display.set".into(), json!({ node.id.as_str(): !visible })));
                        }
                        id => {
                            let command =
                                if id.starts_with("model.") { "model.visibility.set" } else { "cut.visibility.set" };
                            requests.push((command.into(), json!({ "id": id, "visible": !visible })));
                        }
                    }
                }
            }
            None => ui.add_space(18.0),
        }
        if let Some(icon) = node.icon {
            let size = ui.text_style_height(&egui::TextStyle::Body).round().max(12.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
            crate::ribbon::paint_named_icon(ui, rect, icon, tokens, true);
        }
        let text =
            if node.problem { RichText::new(&node.label).color(tokens.warning) } else { RichText::new(&node.label) };
        let response = ui.selectable_label(selected, text);
        if response.clicked() {
            let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            let from = picked.selection.and_then(|s| picked.rows.iter().position(|r| r == s));
            let to = picked.rows.iter().position(|r| *r == node.id);
            let params = match (shift, from, to) {
                // Shift: every row from the one selected to this one.
                (true, Some(a), Some(b)) => {
                    let range: Vec<&String> = picked.rows.get(a.min(b)..=a.max(b)).unwrap_or_default().iter().collect();
                    json!({ "id": node.id, "ids": range, "add": true })
                }
                _ if ctrl || shift => json!({ "id": node.id, "add": true }),
                _ => json!({ "id": node.id }),
            };
            requests.push(("selection.set".into(), params));
        }
        if response.double_clicked() {
            requests.extend(open(&node.id, session));
        }
        context_menu(&response, node, session, tokens, picked, requests);
        // A cut can be dragged to another place in the order, or into another operation.
        if node.id.starts_with("cut.") {
            let response = response.interact(egui::Sense::drag());
            response.dnd_set_drag_payload(CutDrag(node.id.clone()));
            if response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                // The cut's name follows the pointer.
                if let Some(at) = ui.ctx().pointer_interact_pos() {
                    let painter =
                        ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.id().with("drag")));
                    let galley =
                        painter.layout_no_wrap(node.label.clone(), egui::FontId::proportional(13.0), tokens.text);
                    let r = Rect::from_min_size(at + vec2(14.0, 6.0), galley.size()).expand(5.0);
                    painter.rect(r, 4.0, tokens.raised_bg, Stroke::new(1.0, tokens.accent), egui::StrokeKind::Inside);
                    painter.galley(r.min + vec2(5.0, 5.0), galley, tokens.text);
                }
            }
        }
        drop_target(ui, node, session, tokens, requests);
    };
    if node.children.is_empty() {
        ui.horizontal(|ui| row(ui, requests));
        return;
    }
    let id = ui.make_persistent_id(("outline", &node.id));
    CollapsingState::load_with_default_open(ui.ctx(), id, true).show_header(ui, |ui| row(ui, requests)).body(|ui| {
        for child in &node.children {
            outline_node(ui, child, session, tokens, picked, requests);
        }
    });
}

/// A cut being dragged in the explorer.
struct CutDrag(String);

/// A cut dropped on this row: on a cut, before it (top half) or after it (bottom half), in its
/// operation; on an operation, last in it. A line shows where it will go.
fn drop_target(ui: &Ui, node: &OutlineNode, session: &Session, tokens: &Tokens, requests: &mut Vec<Request>) {
    let Some(drag) = egui::DragAndDrop::payload::<CutDrag>(ui.ctx()) else { return };
    let on_cut = node.id.starts_with("cut.");
    if !(on_cut || node.id.starts_with("op.")) {
        return;
    }
    let content = ui.min_rect();
    let rect = Rect::from_x_y_ranges(ui.max_rect().x_range(), content.y_range());
    let Some(at) = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p)) else { return };
    let cuts = &session.job.cuts;
    let (op, before, line_y) = if on_cut {
        let Some(k) = cuts.iter().position(|c| c.id == node.id) else { return };
        let op = cuts.get(k).map(|c| c.op.clone()).unwrap_or_default();
        if at.y < rect.center().y {
            (op, Some(node.id.clone()), rect.top())
        } else {
            // Before the next cut of the same operation (none: last in it).
            let next = cuts.iter().skip(k + 1).find(|c| c.op == op).map(|c| c.id.clone());
            (op, next, rect.bottom())
        }
    } else {
        (node.id.clone(), None, rect.bottom())
    };
    // Dropping it where it already is changes nothing.
    if before.as_deref() == Some(drag.0.as_str()) {
        return;
    }
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.id().with("drop")));
    if on_cut {
        painter.hline(rect.x_range(), line_y, Stroke::new(2.0, tokens.accent));
    } else {
        painter.rect_stroke(rect, 3.0, Stroke::new(1.5, tokens.accent), egui::StrokeKind::Inside);
    }
    if ui.input(|i| i.pointer.any_released()) {
        let mut params = json!({ "id": drag.0, "op": op });
        if let Some(b) = before {
            params["before"] = json!(b);
        }
        requests.push(("cut.place".into(), params));
        egui::DragAndDrop::clear_payload(ui.ctx());
    }
}

/// What double-clicking a row does.
fn open(id: &str, session: &Session) -> Vec<Request> {
    let one = |c: &str, p: Value| vec![(c.to_owned(), p)];
    if is_sketch(session, id) {
        one("sketch.edit", json!({ "id": id }))
    } else if id.starts_with("cut.") {
        one("cut.edit", json!({ "id": id }))
    } else if id.starts_with("model.") {
        one("model.lay_flat", json!({ "id": id }))
    } else if id.starts_with("shape.") {
        one("ui.cut.new", json!({ "kind": "profile", "shape": id }))
    } else if id.starts_with("op.") {
        one("op.edit", json!({ "id": id }))
    } else {
        match id {
            MACHINE_ID => one("machine.edit", Value::Null),
            ORIGIN_ID => {
                let other = if session.job.origin == Origin::Machine { Origin::Table } else { Origin::Machine };
                one("origin.set", json!({ "at": other.id() }))
            }
            STOCK_ID => one("ui.stock.edit", Value::Null),
            PROGRAM_ID => one("post.gcode.preview", Value::Null),
            _ => Vec::new(),
        }
    }
}

fn is_sketch(session: &Session, id: &str) -> bool {
    session.job.shapes.iter().any(|s| s.id == id && s.sketch.is_some())
}

fn context_menu(
    row: &egui::Response,
    node: &OutlineNode,
    session: &Session,
    tokens: &Tokens,
    picked: &Picked<'_>,
    requests: &mut Vec<Request>,
) {
    let id = node.id.as_str();
    // Delete on a row that is one of several selected deletes them all.
    let delete = || -> Vec<Request> {
        let ids: Vec<String> = if picked.chosen.len() > 1 && picked.chosen.iter().any(|c| c == id) {
            picked.chosen.to_vec()
        } else {
            vec![id.to_owned()]
        };
        delete_request(session, &ids).into_iter().collect()
    };
    crate::ui_kit::Menu::below(row).context(tokens, |ui| {
        ui.set_min_width(180.0);
        let mut item = |ui: &mut Ui, label: &str, run: Vec<Request>| {
            if ui.button(label).clicked() {
                requests.extend(run);
                ui.close();
            }
        };
        let one = |c: &str, p: Value| vec![(c.to_owned(), p)];
        if id.starts_with("cut.") {
            item(ui, "Edit…", one("cut.edit", json!({ "id": id })));
            item(ui, "Duplicate", one("cut.duplicate", json!({ "id": id })));
            item(ui, "Rename…", one("ui.rename", json!({ "command": "cut.rename", "id": id, "name": node.label })));
            // Into another operation.
            let current = session.job.cuts.iter().find(|c| c.id == id).map(|c| c.op.clone()).unwrap_or_default();
            if session.job.ops.len() > 1 {
                ui.menu_button("Move to", |ui| {
                    for (n, op) in session.job.ops.iter().enumerate().filter(|(_, o)| o.id != current) {
                        item(ui, &op.title(n), one("cut.op.set", json!({ "id": id, "op": op.id })));
                    }
                });
            }
            ui.separator();
            item(ui, "Cut earlier", one("cut.move.up", json!({ "id": id })));
            item(ui, "Cut later", one("cut.move.down", json!({ "id": id })));
            ui.separator();
            item(ui, "Delete", delete());
        } else if id.starts_with("model.") {
            item(ui, "Lay flat…", one("model.lay_flat", json!({ "id": id })));
            item(ui, "Scale…", one("ui.form", json!({ "command": "model.scale", "id": id })));
            item(ui, "Turn and lift…", one("ui.form", json!({ "command": "model.place", "id": id })));
            item(ui, "Move…", one("model.move", json!({ "id": id })));
            item(ui, "Duplicate", one("model.duplicate", json!({ "id": id })));
            ui.separator();
            item(ui, "Cut from model…", one("ui.cut.new", json!({ "kind": "model", "models": [id] })));
            for (kind, label) in [("profile", "Profile of model…"), ("twoview", "Two-view of model…")] {
                item(ui, label, one("ui.cut.new", json!({ "kind": kind, "source": "models", "models": [id] })));
            }
            ui.separator();
            item(ui, "Fit stock to model", one("stock.fit", json!({ "model": id })));
            item(ui, "Delete", delete());
        } else if id.starts_with("shape.") {
            if is_sketch(session, id) {
                item(ui, "Edit sketch", one("sketch.edit", json!({ "id": id })));
                ui.separator();
            }
            for kind in [CutKind::Profile, CutKind::Lathe] {
                item(
                    ui,
                    &format!("New {} cut", kind.label().to_lowercase()),
                    one("ui.cut.new", json!({ "kind": kind.id(), "shape": id })),
                );
            }
            item(ui, "New two-view cut", one("ui.cut.new", json!({ "kind": "twoview", "front": id })));
            ui.separator();
            item(ui, "Transform…", one("ui.form", json!({ "command": "shape.transform", "id": id })));
            item(ui, "Rename…", one("ui.rename", json!({ "command": "shape.rename", "id": id, "name": node.label })));
            item(ui, "Delete", delete());
        } else {
            match id {
                "job" => {
                    item(ui, "Save", one("job.save", Value::Null));
                    item(ui, "Save as…", one("job.save.as", Value::Null));
                    item(
                        ui,
                        "Rename…",
                        one(
                            "ui.rename",
                            json!({ "command": "job.rename", "id": "job", "name": node.label.trim_end_matches(" •") }),
                        ),
                    );
                    ui.separator();
                    item(ui, "Open the sample job", one("job.sample", Value::Null));
                }
                MACHINE_ID => {
                    item(ui, "Edit machine…", one("machine.edit", Value::Null));
                    item(ui, "Post processor…", one("post.settings", Value::Null));
                }
                ORIGIN_ID => {
                    for o in Origin::ALL {
                        let mark = if o == session.job.origin { "● " } else { "○ " };
                        item(ui, &format!("{mark}{}", o.label()), one("origin.set", json!({ "at": o.id() })));
                    }
                }
                MODELS_ID => item(ui, "Import model…", one("model.import", Value::Null)),
                STOCK_ID => item(ui, "Edit stock…", one("ui.stock.edit", Value::Null)),
                SHAPES_ID => {
                    item(ui, "Import shape…", one("shape.import", Value::Null));
                    item(ui, "NACA airfoil…", one("shape.airfoil", Value::Null));
                    item(ui, "Rectangle…", one("shape.rectangle", Value::Null));
                    item(ui, "Circle…", one("shape.circle", Value::Null));
                }
                op if op.starts_with("op.") => {
                    item(ui, "Operation settings…", one("op.edit", json!({ "id": op })));
                    item(ui, "Add cuts here", one("op.activate", json!({ "id": op })));
                    ui.separator();
                    for kind in CutKind::ALL {
                        let label = format!("New {} cut…", kind.label().to_lowercase());
                        let run = vec![
                            ("op.activate".to_owned(), json!({ "id": op })),
                            (kind.command().to_owned(), Value::Null),
                        ];
                        item(ui, &label, run);
                    }
                    ui.separator();
                    item(ui, "Add operation", one("op.add", Value::Null));
                    if session.job.ops.len() > 1 {
                        item(ui, "Delete operation", one("op.remove", json!({ "id": op })));
                    }
                }
                PROGRAM_ID => {
                    item(ui, "View G-code", one("post.gcode.preview", Value::Null));
                    item(ui, "Post G-code…", one("post.gcode.export", Value::Null));
                }
                _ => {
                    ui.label(RichText::new("No actions").color(tokens.text_muted));
                }
            }
        }
    });
}

pub fn inspector(ui: &mut Ui, session: &Session, tokens: &Tokens) {
    let node = session.view.selection.as_deref().and_then(|id| session.selection_node(id));
    let (title, rows) = match &node {
        Some(n) => (n.label.clone(), n.properties.clone()),
        None => {
            let j = &session.job;
            (
                j.name.clone(),
                vec![
                    ("Stock".into(), j.stock.describe(session.prefs.units)),
                    ("Shapes".into(), j.shapes.len().to_string()),
                    ("Cuts".into(), j.cuts.len().to_string()),
                    ("Machine".into(), j.machine.name.clone()),
                ],
            )
        }
    };
    ui.label(RichText::new(title).strong());
    if node.is_none() {
        ui.label(RichText::new("Select an item in the explorer.").color(tokens.text_muted));
    }
    ui.separator();
    egui::Grid::new("inspector-grid").num_columns(2).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
        for (key, value) in rows {
            ui.label(RichText::new(key).color(tokens.text_muted));
            let warn = value.contains("goes ") || value.contains("no shape");
            ui.label(if warn { RichText::new(value).color(tokens.warning) } else { RichText::new(value) });
            ui.end_row();
        }
    });
}

/// The cut plane: what the wire sees, looking along it. The selected cut's path (or the one being
/// set up) over the stock's outline at its angle, the table top and the turntable's axis; while
/// simulating, the wire itself, end on.
pub fn cut_plane(ui: &mut Ui, session: &Session, tokens: &Tokens, preview: Option<&Built>) {
    let units = session.prefs.units;
    let job = &session.job;
    let sim = session.sim.as_deref();
    // What to draw: the preview, the cut the simulation is in, or the selected cut.
    let sim_cut = sim.and_then(|s| {
        let i = s.move_index();
        s.program.moves.get(i).and_then(|(c, _)| *c).and_then(|c| job.cuts.get(c))
    });
    let (title, path, release): (String, Option<&WirePath>, Option<&WirePath>) =
        match (preview, sim_cut, session.selected_cut()) {
            (Some(b), _, _) => ("Preview".into(), Some(&b.path), b.release.as_ref()),
            (None, Some(c), _) => (c.name.clone(), Some(&c.path), c.release.as_ref()),
            (None, None, Some(c)) => (c.name.clone(), Some(&c.path), c.release.as_ref()),
            _ => (String::new(), None, None),
        };
    // With no cut to show, the selected shape on its own.
    if path.is_none()
        && let Some(shape) = session.selected_shape()
    {
        shape_view(ui, shape, units, tokens);
        return;
    }
    let wire = sim.map(|s| s.wire());
    // The cut's profiles, one per table angle, in the order they are cut.
    let mut angles: Vec<f64> = Vec::new();
    for m in path.iter().flat_map(|p| &p.moves).filter(|m| m.kind == MoveKind::Cut) {
        if !angles.iter().any(|a| (a - m.to.b).abs() < 1.0e-6) {
            angles.push(m.to.b);
        }
    }
    // The profile shown in colour: where the simulated wire is, else the one stepped to.
    let step_id = egui::Id::new(("cut-plane-profile", title.clone()));
    let mut shown: usize = ui.data(|d| d.get_temp(step_id)).unwrap_or(0).min(angles.len().saturating_sub(1));
    if let Some(w) = wire
        && let Some(k) = angles.iter().position(|a| (a - w.b).abs() < 1.0e-6)
    {
        shown = k;
    }
    let b = wire.map(|w| w.b).or_else(|| angles.get(shown).copied()).unwrap_or(0.0);
    ui.horizontal(|ui| {
        if title.is_empty() {
            ui.label(
                RichText::new("Select a cut to see its path as the wire sees it, or a shape to see its outline")
                    .color(tokens.text_muted),
            );
        } else {
            ui.label(RichText::new(title.as_str()).strong());
            if angles.len() > 1 {
                let stepping = wire.is_none();
                if ui.add_enabled(stepping && shown > 0, egui::Button::new("◀").small()).clicked() {
                    shown -= 1;
                }
                ui.label(RichText::new(format!("Profile {} of {}", shown + 1, angles.len())).color(tokens.text));
                if ui.add_enabled(stepping && shown + 1 < angles.len(), egui::Button::new("▶").small()).clicked() {
                    shown += 1;
                }
            }
            ui.label(
                RichText::new(format!("B {:.1}°", angles.get(shown).copied().unwrap_or(b))).color(tokens.text_muted),
            );
        }
    });
    ui.data_mut(|d| d.insert_temp(step_id, shown));
    let current = angles.get(shown).copied().unwrap_or(b);
    let is_current = |x: f64| (x - current).abs() < 1.0e-6;
    let rect = ui.available_rect_before_wrap();
    let _ = ui.allocate_rect(rect, Sense::hover());
    if rect.width() < 20.0 || rect.height() < 20.0 {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, tokens.viewport_bottom);
    // Fit the stock (at this angle) and the path, with a margin.
    let (u0, u1) = job.stock.u_range(b);
    let (h0, h1) = (job.stock.bottom(), job.stock.top());
    let mut lo = egui::Vec2::new(u0 as f32, h0.min(0.0) as f32);
    let mut hi = egui::Vec2::new(u1 as f32, h1 as f32);
    if let Some(p) = path {
        for m in p.moves.iter().filter(|m| m.kind != MoveKind::Rapid) {
            lo = lo.min(egui::Vec2::new(m.to.u as f32, m.to.h as f32));
            hi = hi.max(egui::Vec2::new(m.to.u as f32, m.to.h as f32));
        }
    }
    if let Some(w) = wire {
        lo = lo.min(egui::Vec2::new(w.u as f32, w.h as f32));
        hi = hi.max(egui::Vec2::new(w.u as f32, w.h as f32));
    }
    let span = (hi - lo).max(egui::Vec2::splat(1.0));
    let scale = ((rect.width() - 40.0) / span.x).min((rect.height() - 40.0) / span.y).max(1.0e-6);
    let mid = (lo + hi) * 0.5;
    let to_screen =
        |u: f64, h: f64| -> Pos2 { rect.center() + vec2((u as f32 - mid.x) * scale, -(h as f32 - mid.y) * scale) };
    // Table top and the turntable's reach.
    let r = job.machine.table_radius;
    let table = Rect::from_two_pos(to_screen(-r, 0.0), to_screen(r, 0.0));
    painter.hline(rect.x_range(), table.top(), Stroke::new(1.0, tokens.grid_major));
    painter.rect_filled(
        Rect::from_min_max(pos2(table.left(), table.top()), pos2(table.right(), table.top() + 6.0)),
        1.0,
        tokens.machine,
    );
    // The turntable's axis.
    let ax = to_screen(0.0, 0.0).x;
    let mut y = rect.top();
    while y < rect.bottom() {
        painter.vline(ax, y..=(y + 6.0).min(rect.bottom()), Stroke::new(1.0, tokens.axis_z.gamma_multiply(0.6)));
        y += 10.0;
    }
    // The stock as the wire sees it.
    let stock = Rect::from_two_pos(to_screen(u0, h0), to_screen(u1, h1));
    painter.rect(
        stock,
        0.0,
        tokens.stock.gamma_multiply(0.12),
        Stroke::new(1.0, tokens.stock_edge),
        egui::StrokeKind::Middle,
    );
    // The path: the other profiles faint underneath, the current one in colour over them.
    let dim = tokens.text_muted.gamma_multiply(0.28);
    // From where the cut begins: the program reaches it from home, not from where the cut was
    // worked out from (above the foam), so that travel is left out.
    let segments = |p: &WirePath| {
        let skip = p.moves.iter().position(|m| matches!(m.kind, MoveKind::Lead | MoveKind::Cut)).unwrap_or(0);
        let mut at = skip.checked_sub(1).and_then(|k| p.moves.get(k)).map_or(p.start, |m| m.to);
        p.moves
            .iter()
            .skip(skip)
            .map(|m| {
                let seg = (at, *m);
                at = m.to;
                seg
            })
            .collect::<Vec<_>>()
    };
    if let Some(p) = path {
        let segs = segments(p);
        for pass in [false, true] {
            for (from, m) in &segs {
                let here = is_current(from.b) && is_current(m.to.b);
                if here != pass {
                    continue;
                }
                let colour = match (here, m.kind) {
                    (false, _) => dim,
                    (true, MoveKind::Cut) => tokens.accent_strong,
                    (true, MoveKind::Lead) => tokens.success,
                    (true, _) => tokens.warning.gamma_multiply(0.6),
                };
                let width = if here && m.kind == MoveKind::Cut { 2.0 } else { 1.0 };
                painter
                    .line_segment([to_screen(from.u, from.h), to_screen(m.to.u, m.to.h)], Stroke::new(width, colour));
            }
        }
    }
    // The release that frees the part at the end of the program.
    if let Some(rel) = release {
        for (from, m) in segments(rel) {
            let colour = if is_current(m.to.b) { tokens.error } else { tokens.error.gamma_multiply(0.3) };
            painter.line_segment([to_screen(from.u, from.h), to_screen(m.to.u, m.to.h)], Stroke::new(1.6, colour));
        }
    }
    // The wire, end on.
    if let Some(w) = wire {
        let c = to_screen(w.u, w.h);
        painter.circle_filled(c, 7.0, tokens.wire.gamma_multiply(0.3));
        painter.circle_filled(c, 3.5, tokens.wire);
        painter.text(
            c + vec2(9.0, -9.0),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "X {}  Y {}",
                units.plain(job.machine.program_xy(job.origin, w.u, w.h).0),
                units.plain(job.machine.program_xy(job.origin, w.u, w.h).1)
            ),
            egui::FontId::monospace(11.0),
            tokens.text,
        );
    }
    // A scale bar about 100 points long, a round length in the user's units.
    let bar_units = nice(units.from_mm(100.0 / f64::from(scale)));
    let bar = units.to_mm(bar_units) as f32 * scale;
    let base = pos2(rect.left() + 14.0, rect.bottom() - 14.0);
    painter.line_segment([base, base + vec2(bar, 0.0)], Stroke::new(2.0, tokens.text_muted));
    painter.text(
        base + vec2(bar / 2.0, -4.0),
        egui::Align2::CENTER_BOTTOM,
        format!("{} {}", trim(bar_units), units.length()),
        egui::FontId::proportional(11.0),
        tokens.text_muted,
    );
}

/// A shape's outlines, fitted to the pane, with its size and its origin marked.
fn shape_view(ui: &mut Ui, shape: &CosmicCut_engine::shape::Shape, units: DisplayUnits, tokens: &Tokens) {
    let (lo, hi) = shape.bounds();
    let size = hi - lo;
    ui.horizontal(|ui| {
        ui.label(RichText::new(&shape.name).strong());
        ui.label(
            RichText::new(format!("{} × {}", units.fmt_len(size.x), units.fmt_len(size.y))).color(tokens.text_muted),
        );
    });
    let rect = ui.available_rect_before_wrap();
    let _ = ui.allocate_rect(rect, Sense::hover());
    if rect.width() < 20.0 || rect.height() < 20.0 {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, tokens.viewport_bottom);
    let span = size.max(glam::DVec2::splat(1.0e-3));
    let scale = ((f64::from(rect.width()) - 40.0) / span.x).min((f64::from(rect.height()) - 40.0) / span.y).max(1.0e-9);
    let mid = (lo + hi) * 0.5;
    let to_screen = |p: glam::DVec2| -> Pos2 {
        rect.center() + vec2(((p.x - mid.x) * scale) as f32, (-(p.y - mid.y) * scale) as f32)
    };
    // The origin, where shapes are measured from.
    let o = to_screen(glam::DVec2::ZERO);
    if rect.contains(o) {
        painter.line_segment([o - vec2(8.0, 0.0), o + vec2(8.0, 0.0)], Stroke::new(1.0, tokens.axis_x));
        painter.line_segment([o - vec2(0.0, 8.0), o + vec2(0.0, 8.0)], Stroke::new(1.0, tokens.axis_y));
    }
    for p in &shape.paths {
        let mut pts: Vec<Pos2> = p.points.iter().map(|q| to_screen(*q)).collect();
        if p.closed
            && let Some(first) = pts.first().copied()
        {
            pts.push(first);
        }
        let colour = if p.closed { tokens.accent } else { tokens.warning };
        painter.add(egui::Shape::line(pts, Stroke::new(1.6, colour)));
    }
    if shape.paths.iter().any(|p| !p.closed) {
        painter.text(
            rect.left_bottom() + vec2(10.0, -10.0),
            egui::Align2::LEFT_BOTTOM,
            "Open outlines (orange) suit lathe half profiles; profile cuts need closed ones",
            egui::FontId::proportional(11.0),
            tokens.text_muted,
        );
    }
}

/// 1, 2 or 5 times a power of ten, at or below `v`.
fn nice(v: f64) -> f64 {
    if !(v.is_finite() && v > 0.0) {
        return 1.0;
    }
    let p = 10f64.powf(v.log10().floor());
    let m = v / p;
    p * if m >= 5.0 {
        5.0
    } else if m >= 2.0 {
        2.0
    } else {
        1.0
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The G-code pane's copy of the posted program, made again when the job changes.
#[derive(Default)]
pub struct GcodeState {
    revision: Option<u64>,
    text: String,
    summary: String,
    warnings: Vec<String>,
}

pub fn gcode(ui: &mut Ui, session: &Session, tokens: &Tokens, state: &mut GcodeState, requests: &mut Vec<Request>) {
    let job = &session.job;
    let key = job.revision ^ (job.edits << 32);
    if state.revision != Some(key) {
        let posted = CosmicCut_engine::post::post(&job.machine, job.origin, &job.stock, &job.cuts, &job.ops, &job.name);
        state.summary = format!(
            "{} lines  ·  about {}  ·  {}",
            posted.lines,
            CosmicCut_engine::cut::fmt_time(posted.seconds),
            job.machine.post.units.label()
        );
        state.warnings = posted.warnings;
        state.text = posted.text;
        state.revision = Some(key);
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(&state.summary).color(tokens.text_muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(!job.cuts.is_empty(), egui::Button::new("Post G-code…")).clicked() {
                requests.push(("post.gcode.export".into(), Value::Null));
            }
            if ui.button("Copy").on_hover_text("Copy the program to the clipboard").clicked() {
                ui.ctx().copy_text(state.text.clone());
            }
        });
    });
    for w in &state.warnings {
        ui.label(RichText::new(format!("⚠ {w}")).color(tokens.warning));
    }
    ui.separator();
    let lines: Vec<&str> = state.text.lines().collect();
    let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
    ScrollArea::both().auto_shrink([false, false]).show_rows(ui, row_h, lines.len(), |ui, range| {
        for (n, line) in lines.iter().enumerate().skip(range.start).take(range.len()) {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{:>5} ", n + 1)).monospace().color(tokens.text_muted.gamma_multiply(0.7)),
                );
                ui.label(gcode_job(line, tokens.dark));
            });
        }
    });
}

/// What a piece of a G-code line is, for its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Plain,
    /// `( … )`, `; …` and N line numbers.
    Comment,
    /// G words.
    G,
    /// M words.
    M,
    /// An axis word: the letter.
    Axis(char),
    /// F, S, P, I, J, R and the other letter words.
    Other,
}

/// The pieces of one G-code line: (start, end, kind), in byte offsets. The same rules as the
/// HotwireCNC editor's highlighter, so a program looks the same in both.
pub fn tokenize(line: &str) -> Vec<(usize, usize, Token)> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    // A value after a letter: digits, a sign, a point, #<name>, [expression] or #n.
    let value_end = |mut j: usize| -> usize {
        while b.get(j).is_some_and(|c| *c == b' ') {
            j += 1;
        }
        match b.get(j) {
            Some(b'#') if b.get(j + 1) == Some(&b'<') => {
                b.iter().skip(j).position(|c| *c == b'>').map_or(b.len(), |k| j + k + 1)
            }
            Some(b'[') => b.iter().skip(j).position(|c| *c == b']').map_or(b.len(), |k| j + k + 1),
            Some(b'#') => {
                let mut k = j + 1;
                while b.get(k).is_some_and(u8::is_ascii_digit) {
                    k += 1;
                }
                k
            }
            _ => {
                let mut k = j;
                if matches!(b.get(k), Some(b'+' | b'-')) {
                    k += 1;
                }
                while b.get(k).is_some_and(|c| c.is_ascii_digit() || *c == b'.' || *c == b' ') {
                    k += 1;
                }
                // Trailing spaces are not part of the word.
                while k > j && b.get(k - 1) == Some(&b' ') {
                    k -= 1;
                }
                k
            }
        }
    };
    while i < b.len() {
        let c = b.get(i).copied().unwrap_or(b' ').to_ascii_uppercase();
        let start = i;
        match c {
            b'(' => {
                i = b.iter().skip(i).position(|c| *c == b')').map_or(b.len(), |k| i + k + 1);
                out.push((start, i, Token::Comment));
            }
            b';' => {
                i = b.len();
                out.push((start, i, Token::Comment));
            }
            b'N' if b.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                i += 1;
                while b.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
                out.push((start, i, Token::Comment));
            }
            b'G' | b'M' => {
                let end = value_end(i + 1);
                if end > i + 1 {
                    i = end;
                    out.push((start, i, if c == b'G' { Token::G } else { Token::M }));
                } else {
                    i += 1;
                }
            }
            b'X' | b'Y' | b'Z' | b'A' | b'B' | b'C' | b'U' | b'V' | b'W' => {
                let end = value_end(i + 1);
                if end > i + 1 {
                    i = end;
                    out.push((start, i, Token::Axis(c as char)));
                } else {
                    i += 1;
                }
            }
            b'E' | b'F' | b'H' | b'I' | b'J' | b'K' | b'D' | b'Q' | b'L' | b'R' | b'P' | b'S' | b'T' | b'$' => {
                let end = value_end(i + 1);
                if end > i + 1 {
                    i = end;
                    out.push((start, i, Token::Other));
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// The colour of a token: HotwireCNC's editor colours on a dark theme, darker shades of them
/// on the light one so they read on white.
fn token_colour(t: Token, dark: bool) -> (egui::Color32, bool, bool) {
    let hex = |rgb: u32| egui::Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
    let (rgb, bold, italic) = match t {
        Token::Plain => (if dark { 0xc9d1dc } else { 0x1c2025 }, false, false),
        Token::Comment => (if dark { 0x6a7f6a } else { 0x4f6b4f }, false, true),
        Token::G => (if dark { 0x8f9bff } else { 0x3846c8 }, true, false),
        Token::M => (if dark { 0xf0a020 } else { 0xb86e00 }, true, false),
        Token::Other => (if dark { 0xf4b8ba } else { 0xb4505a }, false, false),
        Token::Axis('X') => (if dark { 0xff7a7a } else { 0xc62828 }, false, false),
        Token::Axis('Y') => (if dark { 0x6ee7a0 } else { 0x1b8a4c }, false, false),
        Token::Axis('Z') => (if dark { 0x4fc3f7 } else { 0x0277bd }, false, false),
        Token::Axis('A') => (if dark { 0xffb86b } else { 0xc0661a }, false, false),
        Token::Axis('B') => (if dark { 0xd19bff } else { 0x7b3fc4 }, false, false),
        Token::Axis('C') => (if dark { 0x4dd0c8 } else { 0x00897b }, false, false),
        Token::Axis(_) => (if dark { 0xffd166 } else { 0x9a7400 }, false, false),
    };
    (hex(rgb), bold, italic)
}

/// One G-code line, coloured.
fn gcode_job(line: &str, dark: bool) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let font = egui::FontId::monospace(13.0);
    let format = |t: Token| {
        // The monospace font has no bold face, so G and M words show by their colour alone.
        let (color, _bold, italic) = token_colour(t, dark);
        TextFormat { font_id: font.clone(), color, italics: italic, ..Default::default() }
    };
    let mut job = LayoutJob::default();
    let mut at = 0;
    for (start, end, t) in tokenize(line) {
        if let Some(plain) = line.get(at..start).filter(|p| !p.is_empty()) {
            job.append(plain, 0.0, format(Token::Plain));
        }
        if let Some(word) = line.get(start..end) {
            job.append(word, 0.0, format(t));
        }
        at = end;
    }
    if let Some(rest) = line.get(at..).filter(|p| !p.is_empty()) {
        job.append(rest, 0.0, format(Token::Plain));
    }
    job
}

#[derive(Default)]
pub struct ConsoleState {
    input: String,
    parse_error: Option<String>,
}

/// Parses `command.id {json params}`.
pub fn parse_console_line(line: &str) -> Result<Option<Request>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let (id, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    let params = if rest.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(rest).map_err(|e| format!("parameters are not valid JSON: {e}"))?
    };
    Ok(Some((id.to_owned(), params)))
}

pub fn console(ui: &mut Ui, session: &Session, tokens: &Tokens, state: &mut ConsoleState, requests: &mut Vec<Request>) {
    let input_height = 30.0;
    let log_height = (ui.available_height() - input_height).max(0.0);
    ScrollArea::vertical().max_height(log_height).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        for entry in session.log() {
            ui.horizontal_wrapped(|ui| {
                let params = if entry.params.is_null() { String::new() } else { entry.params.to_string() };
                ui.label(RichText::new(format!("> {} {params}", entry.command)).monospace());
                match &entry.outcome {
                    Ok(Value::Null) => {
                        ui.label(RichText::new("ok").monospace().color(tokens.success));
                    }
                    Ok(v) => {
                        let mut text = v.to_string();
                        if text.len() > 400 {
                            text = format!("{}…", text.chars().take(400).collect::<String>());
                        }
                        ui.label(RichText::new(text).monospace().color(tokens.success));
                    }
                    Err(e) => {
                        ui.label(RichText::new(e.to_string()).monospace().color(tokens.error));
                    }
                }
            });
        }
        if let Some(err) = &state.parse_error {
            ui.label(RichText::new(err).monospace().color(tokens.error));
        }
    });
    ui.horizontal(|ui| {
        let edit = ui.add(
            TextEdit::singleline(&mut state.input)
                .hint_text("command.id {\"param\": value}   e.g. cut.slab {\"thickness\": 50.8}")
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );
        if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            match parse_console_line(&state.input) {
                Ok(Some(request)) => {
                    requests.push(request);
                    state.input.clear();
                    state.parse_error = None;
                }
                Ok(None) => {}
                Err(e) => state.parse_error = Some(e),
            }
            edit.request_focus();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_lines_parse() {
        assert_eq!(parse_console_line("  "), Ok(None));
        assert_eq!(parse_console_line("view.fit"), Ok(Some(("view.fit".into(), Value::Null))));
        let got = parse_console_line("cut.slab {\"thickness\": 50}");
        assert_eq!(got, Ok(Some(("cut.slab".into(), json!({ "thickness": 50 })))));
        assert!(parse_console_line("x {nope").is_err());
        assert_eq!(nice(37.0), 20.0);
        let kinds: Vec<Token> = tokenize("N10 G1 X1.5 Y-2 B90 F40 (cut) ; end").into_iter().map(|t| t.2).collect();
        assert_eq!(
            kinds,
            vec![
                Token::Comment,
                Token::G,
                Token::Axis('X'),
                Token::Axis('Y'),
                Token::Axis('B'),
                Token::Other,
                Token::Comment,
                Token::Comment
            ]
        );
        assert_eq!(tokenize("M3").first().map(|t| t.2), Some(Token::M));
        assert!(tokenize("(MSG, Turn it)").iter().all(|t| t.2 == Token::Comment));
        assert_eq!(nice(0.7), 0.5);
    }
}
