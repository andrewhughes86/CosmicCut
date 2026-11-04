//! The contextual ribbon: a tab strip over one flat body of command groups.
//!
//! Groups hold large buttons (icon over label), stacks of up to three small buttons
//! (icon beside label) and galleries of tiles. A large button with a menu is a split
//! button: the icon runs the command, the label opens the menu.

use std::sync::Arc;

use CosmicCut_engine::{ButtonSize, CommandSpec, Pane, Placement, RibbonTab, Session, Theme, cmd};
use CosmicCut_render::Projection;
use egui::{
    Align2, Color32, CornerRadius, FontId, Galley, Id, Popup, Pos2, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, pos2, vec2,
};
use serde_json::Value;

use crate::Request;
use crate::icon_svg::{self, IconCache};
use crate::theme::Tokens;
use crate::ui_kit::{Container, Menu, Panel, Surface};

/// Height of the button area in every group.
const BODY_HEIGHT: f32 = 72.0;
const CAPTION_HEIGHT: f32 = 15.0;
const TAB_HEIGHT: f32 = 24.0;
const LARGE_ICON: f32 = 32.0;
const SMALL_ICON: f32 = 16.0;
const SMALL_HEIGHT: f32 = 22.0;
const SMALLS_PER_COLUMN: usize = 3;
const LARGE_MIN_WIDTH: f32 = 46.0;
/// A large label wider than this wraps onto a second line.
const LARGE_WRAP_WIDTH: f32 = 64.0;
const TILE_WIDTH: f32 = 78.0;
const TILE_HEIGHT: f32 = 58.0;
const GALLERY_VISIBLE: usize = 4;
const LABEL_SIZE: f32 = 12.0;

/// Height of the menu bar, which doubles as the window's title bar.
const MENU_HEIGHT: f32 = 28.0;

/// The menu bar, the ribbon's tabs under it, and the ribbon. `panes` are the panes open now,
/// ticked in the View menu.
/// The menu bar, and under it (with `full`; not on the start page) the tabs and their buttons.
pub fn show(ui: &mut Ui, session: &Session, tokens: &Tokens, panes: &[Pane], full: bool, requests: &mut Vec<Request>) {
    let active = session.ui.ribbon_tab;
    Panel::top("ribbon", Surface::Ribbon).show(ui, tokens, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        menu_bar(ui, session, tokens, panes, requests);
        if !full {
            return;
        }
        tab_strip(ui, tokens, active, requests);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.add_space(4.0);
            for (panel, commands) in cmd::ribbon(active) {
                group(ui, session, tokens, panel, &commands, requests);
                separator(ui, tokens);
            }
        });
    });
}

/// The menu bar: CosmicCut, File, Edit, View and Help, the job's name, and the window's controls.
/// It doubles as the title bar: drag it to move the window.
fn menu_bar(ui: &mut Ui, session: &Session, tokens: &Tokens, panes: &[Pane], requests: &mut Vec<Request>) {
    let (strip, background) = ui.allocate_exact_size(vec2(ui.available_width(), MENU_HEIGHT), Sense::click_and_drag());
    let painter = ui.painter_at(strip);
    painter.rect_filled(strip, 0.0, tokens.app_bg);
    let right = window_controls(ui, strip, tokens, &background);

    let brand_font = FontId::proportional(14.0);
    let brand = painter.layout_no_wrap("CosmicCut".into(), brand_font, tokens.accent);
    let mut x = strip.left() + 10.0;
    painter.galley(pos2(x, strip.center().y - brand.size().y / 2.0), brand.clone(), tokens.accent);
    x += brand.size().x + 12.0;

    let font = FontId::proportional(13.0);
    for name in ["File", "Edit", "View", "Help"] {
        let galley = painter.layout_no_wrap(name.into(), font.clone(), tokens.text);
        let rect = Rect::from_min_size(pos2(x, strip.top() + 3.0), vec2(galley.size().x + 18.0, MENU_HEIGHT - 6.0));
        let response = ui.interact(rect, Id::new(("menu-bar", name)), Sense::click());
        if response.hovered() || Popup::is_id_open(ui.ctx(), Popup::default_response_id(&response)) {
            painter.rect_filled(rect, 4.0, tokens.hover_bg);
        }
        painter.text(rect.center(), Align2::CENTER_CENTER, name, font.clone(), tokens.text);
        Menu::below(&response).show(tokens, |ui| {
            ui.set_min_width(230.0);
            match name {
                "File" => file_menu(ui, session, requests),
                "Edit" => edit_menu(ui, session, requests),
                "View" => view_menu(ui, session, panes, requests),
                _ => {
                    if ui.button("About CosmicCut").clicked() {
                        requests.push(("ui.about".into(), Value::Null));
                        ui.close();
                    }
                }
            }
        });
        x = rect.right() + 2.0;
    }
    document_tabs(ui, session, tokens, Rect::from_x_y_ranges(x + 16.0..=right - 12.0, strip.y_range()), requests);
}

/// A menu item that runs command `id`, with its shortcut, greyed out when it cannot run now.
fn command_item(ui: &mut Ui, session: &Session, id: &str, label: &str, requests: &mut Vec<Request>) {
    let Some(spec) = cmd::find(id) else { return };
    let button = egui::Button::new(label).shortcut_text(spec.shortcut.unwrap_or_default());
    if ui.add_enabled(session.is_available(spec), button).on_hover_text(spec.tooltip).clicked() {
        requests.push((id.to_owned(), Value::Null));
        ui.close();
    }
}

fn file_menu(ui: &mut Ui, session: &Session, requests: &mut Vec<Request>) {
    if ui.button("Home").clicked() {
        requests.push(("ui.home".into(), Value::Null));
        ui.close();
    }
    ui.separator();
    command_item(ui, session, "job.new", "New", requests);
    command_item(ui, session, "job.open", "Open…", requests);
    command_item(ui, session, "job.save", "Save", requests);
    command_item(ui, session, "job.save.as", "Save as…", requests);
    command_item(ui, session, "job.close", "Close", requests);
    ui.separator();
    command_item(ui, session, "model.import", "Import model…", requests);
    command_item(ui, session, "shape.import", "Import shape…", requests);
    command_item(ui, session, "job.sample", "Open the sample job", requests);
    ui.separator();
    command_item(ui, session, "post.gcode.export", "Post G-code…", requests);
    ui.separator();
    command_item(ui, session, "settings.open", "Preferences…", requests);
    // A browser tab closes itself.
    if cfg!(not(target_arch = "wasm32")) {
        ui.separator();
        if ui.add(egui::Button::new("Exit").shortcut_text("Alt+F4")).clicked() {
            requests.push(("ui.quit".into(), Value::Null));
            ui.close();
        }
    }
}

fn edit_menu(ui: &mut Ui, session: &Session, requests: &mut Vec<Request>) {
    command_item(ui, session, "edit.undo", "Undo", requests);
    command_item(ui, session, "edit.redo", "Redo", requests);
    ui.separator();
    command_item(ui, session, "edit.cut", "Cut", requests);
    command_item(ui, session, "edit.copy", "Copy", requests);
    command_item(ui, session, "edit.paste", "Paste", requests);
}

fn view_menu(ui: &mut Ui, session: &Session, panes: &[Pane], requests: &mut Vec<Request>) {
    ui.label(RichText::new("Panels").small().weak());
    for pane in [Pane::Explorer, Pane::Inspector, Pane::Plane, Pane::Gcode, Pane::Console] {
        let mut shown = panes.contains(&pane);
        if ui.checkbox(&mut shown, pane.title()).clicked() {
            let id = if shown { "ui.pane.show" } else { "ui.pane.hide" };
            requests.push((id.into(), serde_json::json!({ "pane": pane.id() })));
        }
    }
    ui.separator();
    ui.label(RichText::new("Viewport").small().weak());
    for (key, label) in [
        ("grid", "Grid"),
        ("view_cube", "View cube"),
        ("orientation_triad", "Axes"),
        ("stock", "Stock"),
        ("origin", "Origin"),
        ("machine", "Turntable, towers and wire"),
        ("rulings", "Rulings across the foam"),
    ] {
        let mut on = session.prefs.display.get(key).unwrap_or(false);
        if ui.checkbox(&mut on, label).clicked() {
            requests.push(("view.display.set".into(), serde_json::json!({ key: on })));
        }
    }
    ui.separator();
    command_item(ui, session, "view.theme.toggle", "Switch light / dark theme", requests);
    command_item(ui, session, "view.layout.reset", "Reset layout", requests);
}

/// The ribbon's tabs, just under the menu bar.
fn tab_strip(ui: &mut Ui, tokens: &Tokens, active: RibbonTab, requests: &mut Vec<Request>) {
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), TAB_HEIGHT), Sense::hover());
    let painter = ui.painter_at(strip);
    painter.rect_filled(strip, 0.0, tokens.raised_bg);
    painter.hline(strip.x_range(), strip.top() + 0.5, Stroke::new(1.0, tokens.border));
    painter.hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, tokens.border));

    let font = FontId::proportional(13.0);
    let mut x = strip.left() + 8.0;

    let top_only = CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 };
    for tab in RibbonTab::ALL {
        let galley = painter.layout_no_wrap(tab.label().into(), font.clone(), tokens.text);
        let rect = Rect::from_min_size(pos2(x, strip.top() + 3.0), vec2(galley.size().x + 22.0, TAB_HEIGHT - 3.0));
        let response = ui.interact(rect, Id::new(("ribbon-tab", tab.label())), Sense::click());
        let selected = active == tab;
        if selected {
            // The active tab opens into the body below it.
            let s = Stroke::new(1.0, tokens.border);
            painter.rect_filled(rect, top_only, tokens.panel_bg);
            painter.vline(rect.left(), rect.y_range(), s);
            painter.vline(rect.right(), rect.y_range(), s);
            painter.hline(rect.x_range(), rect.top(), s);
            painter.hline(rect.x_range().shrink(1.0), rect.bottom() - 0.5, Stroke::new(1.0, tokens.panel_bg));
            // The accent underlines the tab's name.
            painter.hline(rect.x_range().shrink(6.0), rect.bottom() - 2.5, Stroke::new(2.0, tokens.accent));
        } else if response.hovered() {
            painter.rect_filled(rect, top_only, tokens.hover_bg);
        }
        let color = if selected { tokens.text } else { tokens.text_muted };
        painter.text(rect.center(), Align2::CENTER_CENTER, tab.label(), font.clone(), color);
        if response.clicked() && !selected {
            requests.push(("ui.ribbon.tab.set".into(), serde_json::json!({ "tab": tab.id() })));
        }
        x = rect.right() + 2.0;
    }
}

/// Widest the job's tab grows; longer names are cut short.
const DOC_TAB_MAX: f32 = 260.0;

/// The open job's name after the ribbon tabs, with a dot while it has unsaved changes.
/// The open jobs, one tab each, and + for a new one. A tab shows its close × while the pointer
/// is over it; a dot after the name marks unsaved changes.
fn document_tabs(ui: &Ui, session: &Session, tokens: &Tokens, space: Rect, requests: &mut Vec<Request>) {
    const CLOSE: f32 = 18.0;
    const PLUS: f32 = 24.0;
    let painter = ui.painter_at(space);
    let active = session.active_tab();
    let tabs = session.tabs();
    // Tabs share the room there is, each no wider than DOC_TAB_MAX.
    let room = (space.width() - PLUS - 4.0).max(0.0);
    let widest = (room / tabs.len().max(1) as f32 - 2.0).min(DOC_TAB_MAX);
    let mut x = space.left();
    for (index, tab) in tabs.iter().enumerate() {
        let current = index == active;
        let name = if tab.modified { format!("{} \u{2022}", tab.name) } else { tab.name.clone() };
        let label = truncate(ui, &name, (widest - 20.0 - CLOSE).max(12.0));
        let rect =
            Rect::from_min_size(pos2(x, space.top() + 3.0), vec2(label.size().x + 20.0 + CLOSE, space.height() - 5.0));
        if rect.right() > space.right() - PLUS {
            break;
        }
        x = rect.right() + 2.0;
        let response = ui.interact(rect, Id::new(("document-tab", index)), Sense::click());
        let fill = if current {
            tokens.panel_bg
        } else if response.hovered() {
            tokens.hover_bg
        } else {
            tokens.app_bg
        };
        let border = if current { tokens.border } else { Color32::TRANSPARENT };
        painter.rect(rect, 4.0, fill, Stroke::new(1.0, border), StrokeKind::Inside);
        if current {
            painter.hline(rect.x_range().shrink(6.0), rect.bottom() - 1.5, Stroke::new(2.0, tokens.accent));
        }
        let ink = if current { tokens.text } else { tokens.text_muted };
        painter.galley(pos2(rect.left() + 10.0, rect.center().y - label.size().y / 2.0), label, ink);

        let close = Rect::from_center_size(pos2(rect.right() - 6.0 - CLOSE / 2.0, rect.center().y), vec2(CLOSE, CLOSE));
        let close_response = ui
            .interact(close, Id::new(("document-tab-close", index)), Sense::click())
            .on_hover_text("Close the job (Ctrl+W)");
        if response.hovered() || close_response.hovered() {
            if close_response.hovered() {
                painter.rect_filled(close, 3.0, tokens.hover_bg);
            }
            let (c, h) = (close.center(), 4.0);
            let ink = Stroke::new(1.3, if close_response.hovered() { tokens.text } else { tokens.text_muted });
            painter.line_segment([c + vec2(-h, -h), c + vec2(h, h)], ink);
            painter.line_segment([c + vec2(-h, h), c + vec2(h, -h)], ink);
        }
        if close_response.clicked() {
            // Close acts on the current job: bring this one forward first, so an unsaved one asks.
            if !current {
                requests.push(("job.tab.set".into(), serde_json::json!({ "index": index })));
            }
            requests.push(("job.close".into(), Value::Null));
        } else if response.clicked() && !current {
            requests.push(("job.tab.set".into(), serde_json::json!({ "index": index })));
        }
        if !close_response.hovered() {
            response.on_hover_text(tab.path.clone().unwrap_or_else(|| format!("{} (not saved yet)", tab.name)));
        }
    }

    // + opens a new job in a new tab.
    let plus = Rect::from_center_size(pos2(x + 4.0 + PLUS / 2.0, space.center().y), vec2(PLUS - 4.0, PLUS - 4.0));
    let response = ui.interact(plus, Id::new("document-tab-new"), Sense::click()).on_hover_text("New job (Ctrl+N)");
    if response.hovered() {
        painter.rect_filled(plus, 3.0, tokens.hover_bg);
    }
    let (c, h) = (plus.center(), 5.0);
    let ink = Stroke::new(1.4, if response.hovered() { tokens.text } else { tokens.text_muted });
    painter.line_segment([c + vec2(-h, 0.0), c + vec2(h, 0.0)], ink);
    painter.line_segment([c + vec2(0.0, -h), c + vec2(0.0, h)], ink);
    if response.clicked() {
        requests.push(("job.new".into(), Value::Null));
    }
}

/// Width of one window control button.
const WINDOW_BUTTON: f32 = 44.0;

/// Minimize, maximize and close at the right of the tab strip, which stands in for the title bar
/// (the window has no system decorations). Returns where the strip's free space ends.
fn window_controls(ui: &Ui, strip: Rect, tokens: &Tokens, background: &Response) -> f32 {
    use egui::ViewportCommand;
    // In a browser the page is the window: there is nothing to move, size or close.
    if cfg!(target_arch = "wasm32") {
        return strip.right();
    }
    let ctx = ui.ctx();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    if background.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    } else if background.drag_started() {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    let painter = ui.painter_at(strip);
    let ink = Stroke::new(1.2, tokens.text);
    let mut x = strip.right();
    for button in ["close", "maximize", "minimize"] {
        let rect = Rect::from_min_max(pos2(x - WINDOW_BUTTON, strip.top()), pos2(x, strip.bottom() - 1.0));
        x = rect.left();
        let response = ui.interact(rect, Id::new(("window-control", button)), Sense::click());
        if response.hovered() {
            painter.rect_filled(rect, 0.0, if button == "close" { tokens.error } else { tokens.hover_bg });
        }
        let c = rect.center();
        let h = 5.0;
        match button {
            "close" => {
                let ink = if response.hovered() { Stroke::new(1.2, Color32::WHITE) } else { ink };
                painter.line_segment([c + vec2(-h, -h), c + vec2(h, h)], ink);
                painter.line_segment([c + vec2(-h, h), c + vec2(h, -h)], ink);
            }
            "maximize" if maximized => {
                // Restore: two overlapping windows.
                painter.rect_stroke(
                    Rect::from_min_max(c + vec2(-h, -h + 2.0), c + vec2(h - 2.0, h)),
                    0.0,
                    ink,
                    egui::StrokeKind::Middle,
                );
                painter.line_segment([c + vec2(-h + 2.0, -h), c + vec2(h, -h)], ink);
                painter.line_segment([c + vec2(h, -h), c + vec2(h, h - 2.0)], ink);
            }
            "maximize" => {
                painter.rect_stroke(
                    Rect::from_center_size(c, vec2(2.0 * h, 2.0 * h)),
                    0.0,
                    ink,
                    egui::StrokeKind::Middle,
                );
            }
            _ => {
                painter.line_segment([c + vec2(-h, 0.0), c + vec2(h, 0.0)], ink);
            }
        }
        let response = response.on_hover_text(match button {
            "close" => "Close",
            "maximize" if maximized => "Restore",
            "maximize" => "Maximize",
            _ => "Minimize",
        });
        if response.clicked() {
            ctx.send_viewport_cmd(match button {
                "close" => ViewportCommand::Close,
                "maximize" => ViewportCommand::Maximized(!maximized),
                _ => ViewportCommand::Minimized(true),
            });
        }
    }
    x
}

/// Lets the window, which has no system frame, be resized from its edges and corners.
pub fn resize_edges(ctx: &egui::Context) {
    use egui::{CursorIcon, ResizeDirection, ViewportCommand};
    const GRIP: f32 = 5.0;
    let (maximized, pointer, pressed, screen) = ctx.input(|i| {
        (
            i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false),
            i.pointer.hover_pos(),
            i.pointer.primary_pressed(),
            i.content_rect(),
        )
    });
    let Some(p) = pointer.filter(|_| !maximized) else {
        return;
    };
    let (w, e) = (p.x - screen.left() < GRIP, screen.right() - p.x < GRIP);
    let (n, s) = (p.y - screen.top() < GRIP, screen.bottom() - p.y < GRIP);
    let (direction, cursor) = match (n, s, w, e) {
        (true, _, true, _) => (ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest),
        (true, _, _, true) => (ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast),
        (_, true, true, _) => (ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest),
        (_, true, _, true) => (ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast),
        (true, ..) => (ResizeDirection::North, CursorIcon::ResizeNorth),
        (_, true, ..) => (ResizeDirection::South, CursorIcon::ResizeSouth),
        (_, _, true, _) => (ResizeDirection::West, CursorIcon::ResizeWest),
        (_, _, _, true) => (ResizeDirection::East, CursorIcon::ResizeEast),
        _ => return,
    };
    ctx.set_cursor_icon(cursor);
    if pressed {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}

fn separator(ui: &mut Ui, tokens: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, BODY_HEIGHT + CAPTION_HEIGHT + 4.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range().shrink(6.0), Stroke::new(1.0, tokens.border));
}

fn group(
    ui: &mut Ui,
    session: &Session,
    tokens: &Tokens,
    name: &str,
    commands: &[(&'static CommandSpec, Placement)],
    requests: &mut Vec<Request>,
) {
    Container::new(Surface::RibbonGroup).show(ui, tokens, |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
            let row = ui.horizontal(|ui| {
                ui.set_height(BODY_HEIGHT);
                let mut rest = commands;
                while let Some((_, first)) = rest.first() {
                    let kind = first.size;
                    let run_len = rest.iter().take_while(|(_, p)| p.size == kind).count();
                    let (run, tail) = rest.split_at(run_len);
                    rest = tail;
                    match kind {
                        ButtonSize::Large => {
                            for (spec, p) in run {
                                large_button(ui, session, tokens, spec, p.menu, requests);
                            }
                        }
                        ButtonSize::Small => {
                            for column in run.chunks(SMALLS_PER_COLUMN) {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                                    for (spec, _) in column {
                                        small_button(ui, session, tokens, spec, requests);
                                    }
                                });
                            }
                        }
                        ButtonSize::Gallery => {
                            let tiles: Vec<&'static CommandSpec> = run.iter().map(|(spec, _)| *spec).collect();
                            gallery(ui, session, tokens, name, &tiles, requests);
                        }
                    }
                }
            });
            let width = row.response.rect.width();
            let (caption, _) = ui.allocate_exact_size(vec2(width, CAPTION_HEIGHT), Sense::hover());
            ui.painter().text(
                caption.center_bottom(),
                Align2::CENTER_BOTTOM,
                name,
                FontId::proportional(11.0),
                tokens.text_muted,
            );
        });
    });
}

/// Colours for a button in its current state.
struct Look {
    fill: Option<Color32>,
    stroke: Option<Stroke>,
    text: Color32,
}

fn look(tokens: &Tokens, enabled: bool, checked: bool, hovered: bool, pressed: bool) -> Look {
    if !enabled {
        return Look { fill: None, stroke: None, text: tokens.text_muted.gamma_multiply(0.6) };
    }
    let accent_fill = tokens.accent.gamma_multiply(if tokens.dark { 0.22 } else { 0.16 });
    if checked {
        return Look { fill: Some(accent_fill), stroke: Some(Stroke::new(1.0, tokens.accent)), text: tokens.text };
    }
    if hovered || pressed {
        let fill = if pressed { accent_fill } else { tokens.hover_bg };
        return Look { fill: Some(fill), stroke: Some(Stroke::new(1.0, tokens.border)), text: tokens.text };
    }
    Look { fill: None, stroke: None, text: tokens.text }
}

fn paint_frame(ui: &Ui, rect: Rect, look: &Look) {
    if let Some(fill) = look.fill {
        ui.painter().rect_filled(rect, 3.0, fill);
    }
    if let Some(stroke) = look.stroke {
        ui.painter().rect_stroke(rect, 3.0, stroke, StrokeKind::Inside);
    }
}

/// Draws the command's icon; disabled icons are drawn faded. A missing or broken master
/// shows an empty slot of the same size.
pub(crate) fn paint_icon(ui: &Ui, rect: Rect, spec: &CommandSpec, tokens: &Tokens, enabled: bool) {
    paint_named_icon(ui, rect, &icon_svg::name_for(spec), tokens, enabled);
}

/// Draws the icon master `name` (from `assets/icons/ui/design/svg/`) into `rect`.
pub(crate) fn paint_named_icon(ui: &Ui, rect: Rect, name: &str, tokens: &Tokens, enabled: bool) {
    let ctx = ui.ctx();
    let px = (rect.width() * ctx.pixels_per_point()).round() as u32;
    let cache_id = Id::new("ribbon-icon-cache");
    let mut cache = ctx.data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<IconCache>(cache_id)));
    let texture = cache.get(ctx, name, tokens, px);
    ctx.data_mut(|d| d.insert_temp(cache_id, cache));
    match texture {
        Some(id) => {
            let tint = if enabled { Color32::WHITE } else { Color32::WHITE.gamma_multiply(0.38) };
            ui.painter().image(id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        }
        None => {
            let alpha = if enabled { 0.55 } else { 0.25 };
            let slot = rect.shrink(rect.width() * 0.1);
            let stroke = Stroke::new(1.0, tokens.text_muted.gamma_multiply(alpha));
            ui.painter().rect_stroke(slot, 3.0, stroke, StrokeKind::Inside);
        }
    }
}

/// Lays out a label in the placeholder colour, so painting picks the state's colour.
fn layout(ui: &Ui, text: &str) -> Arc<Galley> {
    ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(LABEL_SIZE), Color32::PLACEHOLDER)
}

/// One line, or two when the label is long and has a space to break at.
fn label_lines(ui: &Ui, text: &str) -> Vec<Arc<Galley>> {
    let one = layout(ui, text);
    if one.size().x <= LARGE_WRAP_WIDTH || !text.contains(' ') {
        return vec![one];
    }
    let mid = text.len() / 2;
    let split = text.match_indices(' ').map(|(i, _)| i).min_by_key(|i| i.abs_diff(mid)).unwrap_or(mid);
    let (a, b) = text.split_at(split);
    vec![layout(ui, a), layout(ui, b.trim_start())]
}

fn large_button(
    ui: &mut Ui,
    session: &Session,
    tokens: &Tokens,
    spec: &'static CommandSpec,
    menu: &'static [&'static str],
    requests: &mut Vec<Request>,
) {
    let enabled = session.is_available(spec);
    let split = !menu.is_empty();
    let arrow_w = if split { 12.0 } else { 0.0 };
    let lines = label_lines(ui, spec.label);
    let text_w = lines.iter().map(|g| g.size().x).fold(0.0, f32::max) + arrow_w;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2((text_w + 12.0).max(LARGE_MIN_WIDTH), BODY_HEIGHT), sense);
    let icon_rect = Rect::from_center_size(
        pos2(rect.center().x, rect.top() + 4.0 + LARGE_ICON / 2.0),
        vec2(LARGE_ICON, LARGE_ICON),
    );
    let divider_y = icon_rect.bottom() + 3.0;

    // A split button runs the command from its top part and opens the menu from its label.
    let (run, open) = if split {
        let top = Rect::from_min_max(rect.min, pos2(rect.right(), divider_y));
        let bottom = Rect::from_min_max(pos2(rect.left(), divider_y), rect.max);
        (ui.interact(top, response.id.with("run"), sense), Some(ui.interact(bottom, response.id.with("menu"), sense)))
    } else {
        (response.clone(), None)
    };
    let hovered = response.hovered() || run.hovered() || open.as_ref().is_some_and(Response::hovered);
    let lk = look(tokens, enabled, is_checked(spec.id, session), hovered, run.is_pointer_button_down_on());
    paint_frame(ui, rect, &lk);
    if split && hovered && enabled {
        ui.painter().hline(rect.x_range().shrink(3.0), divider_y, Stroke::new(1.0, tokens.border));
    }
    paint_icon(ui, icon_rect, spec, tokens, enabled);

    let mut y = icon_rect.bottom() + 5.0;
    let last = lines.len().saturating_sub(1);
    for (n, galley) in lines.into_iter().enumerate() {
        let extra = if n == last { arrow_w } else { 0.0 };
        let pos = pos2(rect.center().x - (galley.size().x + extra) / 2.0, y);
        let (w, h) = (galley.size().x, galley.size().y);
        ui.painter().galley(pos, galley, lk.text);
        if split && n == last {
            arrow_down(ui, pos2(pos.x + w + 6.0, y + h / 2.0), lk.text);
        }
        y += h - 1.0;
    }

    let run = with_tooltip(run, spec, enabled);
    if run.clicked() {
        requests.push((spec.id.to_owned(), Value::Null));
    }
    if let Some(open) = open {
        let open = with_tooltip(open, spec, enabled);
        Menu::below(&open).show(tokens, |ui| {
            for item in menu.iter().filter_map(|id| cmd::find(id)) {
                let on = session.is_available(item);
                if ui.add_enabled(on, egui::Button::new(item.label)).on_hover_text(item.tooltip).clicked() {
                    requests.push((item.id.to_owned(), Value::Null));
                }
            }
        });
    }
}

fn arrow_down(ui: &Ui, center: Pos2, color: Color32) {
    let points = vec![center + vec2(-3.0, -1.5), center + vec2(3.0, -1.5), center + vec2(0.0, 2.0)];
    ui.painter().add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

fn small_button(
    ui: &mut Ui,
    session: &Session,
    tokens: &Tokens,
    spec: &'static CommandSpec,
    requests: &mut Vec<Request>,
) {
    let enabled = session.is_available(spec);
    let galley = layout(ui, spec.label);
    let width = 4.0 + SMALL_ICON + 5.0 + galley.size().x + 8.0;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, SMALL_HEIGHT), sense);
    let lk =
        look(tokens, enabled, is_checked(spec.id, session), response.hovered(), response.is_pointer_button_down_on());
    paint_frame(ui, rect, &lk);
    let icon =
        Rect::from_min_size(pos2(rect.left() + 4.0, rect.center().y - SMALL_ICON / 2.0), vec2(SMALL_ICON, SMALL_ICON));
    paint_icon(ui, icon, spec, tokens, enabled);
    let pos = pos2(icon.right() + 5.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, lk.text);
    if with_tooltip(response, spec, enabled).clicked() {
        requests.push((spec.id.to_owned(), Value::Null));
    }
}

/// A bordered strip of tiles with paging arrows on the right.
fn gallery(
    ui: &mut Ui,
    session: &Session,
    tokens: &Tokens,
    name: &str,
    tiles: &[&'static CommandSpec],
    requests: &mut Vec<Request>,
) {
    let id = Id::new(("ribbon-gallery", name));
    let pages = tiles.len().div_ceil(GALLERY_VISIBLE).max(1);
    let mut page = ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0).min(pages - 1);
    let visible = tiles.len().clamp(1, GALLERY_VISIBLE);
    let arrows_w = 16.0;
    let (rect, _) =
        ui.allocate_exact_size(vec2(visible as f32 * TILE_WIDTH + arrows_w + 4.0, BODY_HEIGHT - 2.0), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, tokens.raised_bg);
    ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, tokens.border), StrokeKind::Inside);

    for (n, spec) in tiles.iter().skip(page * GALLERY_VISIBLE).take(GALLERY_VISIBLE).enumerate() {
        let tile = Rect::from_min_size(
            pos2(rect.left() + 2.0 + n as f32 * TILE_WIDTH, rect.center().y - TILE_HEIGHT / 2.0),
            vec2(TILE_WIDTH, TILE_HEIGHT),
        );
        let enabled = session.is_available(spec);
        let response = ui.interact(tile, id.with(spec.id), if enabled { Sense::click() } else { Sense::hover() });
        let lk = look(tokens, enabled, false, response.hovered(), response.is_pointer_button_down_on());
        paint_frame(ui, tile.shrink(1.0), &lk);
        let icon = Rect::from_center_size(
            pos2(tile.center().x, tile.top() + 4.0 + LARGE_ICON / 2.0),
            vec2(LARGE_ICON, LARGE_ICON),
        );
        paint_icon(ui, icon, spec, tokens, enabled);
        let label = truncate(ui, spec.label, TILE_WIDTH - 6.0);
        ui.painter().galley(pos2(tile.center().x - label.size().x / 2.0, icon.bottom() + 4.0), label, lk.text);
        if with_tooltip(response, spec, enabled).clicked() {
            requests.push((spec.id.to_owned(), Value::Null));
        }
    }

    let column = Rect::from_min_max(
        pos2(rect.right() - arrows_w - 1.0, rect.top() + 1.0),
        pos2(rect.right() - 1.0, rect.bottom() - 1.0),
    );
    ui.painter().vline(column.left(), column.y_range(), Stroke::new(1.0, tokens.border));
    let third = column.height() / 3.0;
    // Up, down and "show all" (a menu of every tile).
    for (n, active) in [page > 0, page + 1 < pages, pages > 1].into_iter().enumerate() {
        let cell =
            Rect::from_min_size(pos2(column.left(), column.top() + n as f32 * third), vec2(column.width(), third));
        let response = ui.interact(cell, id.with(n), if active { Sense::click() } else { Sense::hover() });
        if active && response.hovered() {
            ui.painter().rect_filled(cell.shrink(1.0), 2.0, tokens.hover_bg);
        }
        let color = if active { tokens.text } else { tokens.text_muted.gamma_multiply(0.5) };
        let c = cell.center();
        let points = match n {
            0 => vec![c + vec2(-3.0, 1.5), c + vec2(3.0, 1.5), c + vec2(0.0, -2.0)],
            _ => vec![c + vec2(-3.0, -1.5), c + vec2(3.0, -1.5), c + vec2(0.0, 2.0)],
        };
        if n == 2 {
            ui.painter().hline((c.x - 3.0)..=(c.x + 3.0), c.y - 3.5, Stroke::new(1.0, color));
        }
        ui.painter().add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
        if n == 2 {
            if active {
                Menu::below(&response).show(tokens, |ui| {
                    for spec in tiles {
                        let on = session.is_available(spec);
                        let clicked = ui
                            .horizontal(|ui| {
                                let (icon, _) = ui.allocate_exact_size(vec2(SMALL_ICON, SMALL_ICON), Sense::hover());
                                paint_icon(ui, icon, spec, tokens, on);
                                ui.add_enabled(on, egui::Button::new(spec.label).frame(false))
                                    .on_hover_text(spec.tooltip)
                                    .clicked()
                            })
                            .inner;
                        if clicked {
                            requests.push((spec.id.to_owned(), Value::Null));
                        }
                    }
                });
            }
        } else if response.clicked() {
            page = if n == 0 { page.saturating_sub(1) } else { (page + 1).min(pages - 1) };
        }
    }
    ui.data_mut(|d| d.insert_temp(id, page));
}

fn truncate(ui: &Ui, text: &str, max: f32) -> Arc<Galley> {
    let full = layout(ui, text);
    if full.size().x <= max {
        return full;
    }
    let mut chars: Vec<char> = text.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let g = layout(ui, &format!("{}…", chars.iter().collect::<String>()));
        if g.size().x <= max {
            return g;
        }
    }
    full
}

pub(crate) fn with_tooltip(response: Response, spec: &CommandSpec, enabled: bool) -> Response {
    if enabled {
        response.on_hover_ui(|ui| tooltip(ui, spec))
    } else {
        response.on_disabled_hover_ui(|ui| tooltip(ui, spec))
    }
}

fn tooltip(ui: &mut Ui, spec: &CommandSpec) {
    ui.label(RichText::new(spec.label).strong());
    ui.label(spec.tooltip);
    if let Some(shortcut) = spec.shortcut {
        ui.label(RichText::new(format!("Shortcut: {shortcut}")).small());
    }
    ui.label(RichText::new(spec.id).small().monospace());
}

/// Toggle commands show as pressed while their state is on.
pub(crate) fn is_checked(id: &str, session: &Session) -> bool {
    match id {
        "view.grid.toggle" => session.prefs.display.grid,
        "view.projection.toggle" => session.view.camera.projection == Projection::Perspective,
        "view.theme.toggle" => session.prefs.theme == Theme::Light,
        "sim.play" => session.sim.as_ref().is_some_and(|s| s.playing),
        "model.centre" => session.selected_model().or(session.job.models.first()).is_some_and(|m| m.centred),
        _ => false,
    }
}
