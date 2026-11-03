//! The desktop application shell: the ribbon, the dockable workspace, the cut panel, dialogs
//! and the status bar, laid out as in CosmicDMS.

use std::path::PathBuf;
use std::sync::Arc;

use CosmicCut_engine::cut::CutKind;
use CosmicCut_engine::{Pane, Session, Theme, UiRequest, cmd};
use egui::{Event, Key, KeyboardShortcut, Modifiers, RichText, Ui, UserData, ViewportCommand, vec2};
use egui_dock::{DockArea, DockState, Style};
use egui_wgpu::RenderState;
use serde_json::{Value, json};

use crate::Request;
use crate::automation::{self, Automation};
use crate::cut_panel::CutPanel;
use crate::dock::{self, Tab};
use crate::home;
use crate::panes::{ConsoleState, GcodeState};
use crate::ribbon;
use crate::theme::{self, Tokens};
use crate::tools::{self, Tools};
use crate::ui_kit::{Panel, Surface};
use crate::viewport::Viewport;

/// How the app was launched.
#[derive(Default)]
pub struct LaunchOptions {
    pub automation: Automation,
    /// Keep nothing between runs: scripted and screenshot runs leave the operator's settings,
    /// machines and post processors as they were.
    pub ephemeral: bool,
    /// Open on the start page.
    pub home: bool,
}

pub struct CosmicApp {
    session: Session,
    dock: DockState<Tab>,
    layout_generation: u64,
    viewport: Viewport,
    console: ConsoleState,
    gcode: GcodeState,
    /// The cut editor, while one is open.
    cut_panel: Option<CutPanel>,
    /// The stock editor, while it is open.
    stock_panel: Option<crate::stock_panel::StockPanel>,
    /// The operation settings dialog, while it is open.
    op_dialog: Option<crate::op_dialog::OpDialog>,
    /// Nothing is saved between runs (a scripted or screenshot run).
    ephemeral: bool,
    /// The machine or post processor library, while it is open.
    library_dialog: Option<crate::library_dialog::LibraryDialog>,
    /// Library revision last written to storage.
    saved_library: u64,
    /// The post-processor dialog, while it is open.
    post_panel: Option<crate::post_panel::PostPanel>,
    /// The machine editor, while it is open.
    machine_panel: Option<crate::machine_panel::MachinePanel>,
    /// The console or the instructions, whichever was showing last (kept between runs).
    bottom: Pane,
    settings_open: bool,
    /// Help → About is open.
    about_open: bool,
    /// The start page is showing instead of the workspace.
    home: bool,
    home_page: home::Home,
    /// Jobs opened or saved lately, newest first, and the list as last written to storage.
    recent: Vec<String>,
    saved_recent: Vec<String>,
    /// File → Close is waiting for Save, Close without saving or Cancel.
    confirm_close: bool,
    /// File → Exit was chosen: the window closes at the end of the frame.
    quit: bool,
    /// Preferences revision last written to storage.
    saved_prefs: u64,
    tools: Tools,
    render_state: Option<RenderState>,
    applied_theme: Option<Theme>,
    /// Last command failure, shown in the status bar.
    status: Option<String>,
    /// A success worth telling the user about, shown when there is no failure.
    notice: Option<String>,
    /// An item being renamed from the explorer: (command, id, name typed so far).
    rename: Option<(String, String, String)>,
    /// A delete waiting to be confirmed: the command, its id and what it deletes.
    confirm_delete: Option<(String, String, String)>,
    shortcuts: Vec<(KeyboardShortcut, &'static str)>,
    automation: Automation,
    /// A window capture waiting: where, and whether to quit after.
    capture: Option<PathBuf>,
    capture_requested: bool,
    capture_wait: u32,
}

/// Storage key of the preferences.
const SETTINGS_KEY: &str = "cosmiccut.settings";
/// Storage key of the machine, so new jobs start with the user's machine.
const MACHINE_KEY: &str = "cosmiccut.machine";
/// Storage key of the machine and post processor libraries.
const LIBRARY_KEY: &str = "cosmiccut.library";

impl CosmicApp {
    pub fn new(cc: &eframe::CreationContext<'_>, mut session: Session, options: LaunchOptions) -> Self {
        let LaunchOptions { automation, ephemeral, home } = options;
        let mut bottom = Pane::Console;
        let mut recent: Vec<String> = Vec::new();
        if let Some(saved) =
            cc.storage.and_then(|s| s.get_string(SETTINGS_KEY)).and_then(|t| serde_json::from_str::<Value>(&t).ok())
        {
            if let Some(units) =
                saved.get("units").and_then(Value::as_str).and_then(CosmicCut_engine::units::DisplayUnits::from_id)
            {
                session.prefs.units = units;
            }
            if let Some(theme) = saved.get("theme").and_then(Value::as_str).and_then(Theme::from_id) {
                session.prefs.theme = theme;
            }
            if let Some(pane) = saved.get("bottom").and_then(Value::as_str).and_then(Pane::from_id) {
                bottom = pane;
            }
            recent = saved
                .get("recent")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).take(home::MAX_RECENT).map(str::to_owned).collect())
                .unwrap_or_default();
        }
        // The machine last used, unless the session already has a job of its own.
        if session.job.cuts.is_empty()
            && let Some(m) = cc
                .storage
                .and_then(|s| s.get_string(MACHINE_KEY))
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| CosmicCut_engine::machine::Machine::from_json(&v).ok())
        {
            session.job.machine = m;
            session.job.rebuild();
        }
        // The libraries of machines and post processors.
        let mut startup_status = None;
        if let Some(text) = cc.storage.and_then(|s| s.get_string(LIBRARY_KEY)) {
            match serde_json::from_str::<Value>(&text)
                .map_err(|e| e.to_string())
                .and_then(|v| CosmicCut_engine::library::Library::from_json(&v).map_err(|e| e.to_string()))
            {
                Ok((library, dropped)) => {
                    if !dropped.is_empty() {
                        startup_status =
                            Some(format!("Some library entries could not be read: {}", dropped.join("; ")));
                    }
                    session.library = library;
                }
                Err(e) => startup_status = Some(format!("The saved library could not be read ({e})")),
            }
        }
        // The job's machine belongs in the library (a machine saved before the library existed).
        let added = session.library.machine(&session.job.machine.name).is_none();
        if added {
            session.library.machines.push(session.job.machine.clone());
            session.library.touch();
        }
        let saved_prefs = session.prefs.revision;
        // Unsaved when the job's machine was just added, so the library is written.
        let saved_library = if added { session.library.revision.wrapping_sub(1) } else { session.library.revision };
        if let Some(tab) = &automation.ribbon_tab {
            let _ = session.execute("ui.ribbon.tab.set", &json!({ "tab": tab.to_lowercase() }));
        }
        Self {
            layout_generation: session.view.layout_generation,
            session,
            // The console or the instructions in front, as last time.
            dock: {
                let mut layout = dock::default_layout();
                if bottom == Pane::Instructions {
                    dock::focus(&mut layout, Pane::Instructions);
                }
                layout
            },
            viewport: Viewport::new(),
            console: ConsoleState::default(),
            gcode: GcodeState::default(),
            cut_panel: None,
            stock_panel: None,
            post_panel: None,
            machine_panel: None,
            bottom,
            library_dialog: None,
            ephemeral,
            saved_library,
            op_dialog: None,
            settings_open: false,
            about_open: false,
            home,
            home_page: home::Home::default(),
            saved_recent: recent.clone(),
            recent,
            confirm_close: false,
            quit: false,
            saved_prefs,
            tools: Tools::default(),
            render_state: cc.wgpu_render_state.clone(),
            applied_theme: None,
            status: startup_status,
            notice: None,
            rename: None,
            confirm_delete: None,
            shortcuts: cmd::all().filter_map(|c| Some((parse_shortcut(c.shortcut?)?, c.id))).collect(),
            automation,
            capture: None,
            capture_requested: false,
            capture_wait: 0,
        }
    }

    /// Runs a request from the ribbon, a pane, a shortcut or the console. Clicks without
    /// parameters open the command's panel, form or file dialog first.
    pub fn run(&mut self, id: &str, params: &Value) {
        // A cut tile opens the cut panel; Edit opens it on the selected cut.
        if let Some(kind) = CutKind::from_command(id).filter(|_| params.is_null()) {
            self.open_cut_panel(CutPanel::new(&self.session, kind, None, &Value::Null));
            self.status = None;
            return;
        }
        if id == "ui.cut.new" {
            let kind =
                params.get("kind").and_then(Value::as_str).and_then(CutKind::from_id).unwrap_or(CutKind::Profile);
            let mut given = params.clone();
            if let Some(o) = given.as_object_mut() {
                o.remove("kind");
            }
            self.open_cut_panel(CutPanel::new(&self.session, kind, None, &given));
            self.status = None;
            return;
        }
        if id == "cut.edit" {
            let target = params
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| self.session.selected_cut().map(|c| c.id.clone()));
            match target.as_deref().and_then(|t| self.session.job.cut(t)) {
                Some(c) => {
                    let (kind, cid) = (c.kind, c.id.clone());
                    self.open_cut_panel(CutPanel::new(&self.session, kind, Some(&cid), &Value::Null));
                    self.status = None;
                }
                None => self.status = Some("select a cut to edit".into()),
            }
            return;
        }
        // Stock opens the Stock tab, beside the cut plane and the G-code.
        if id == "ui.stock.edit" || (params.is_null() && (id == "stock.block" || id == "stock.cylinder")) {
            self.stock_panel = Some(crate::stock_panel::StockPanel::new(&self.session));
            dock::focus(&mut self.dock, Pane::Stock);
            self.status = None;
            return;
        }
        if id == "ui.form" {
            let command = params.get("command").and_then(Value::as_str).unwrap_or_default().to_owned();
            let mut given = params.clone();
            if let Some(o) = given.as_object_mut() {
                o.remove("command");
            }
            self.status = self.tools.start(&command, &self.session, &given).err();
            return;
        }
        // Operation settings open their dialog; a new operation opens it straight away.
        if id == "op.edit" && params.as_object().is_none_or(|p| p.keys().all(|k| k == "id")) {
            let op = params
                .get("id")
                .and_then(Value::as_str)
                .map_or_else(|| self.session.job.active_op.clone(), str::to_owned);
            self.op_dialog = crate::op_dialog::OpDialog::new(&self.session, &op);
            return;
        }
        if id == "op.add" && params.is_null() {
            match self.session.execute(id, &json!({})) {
                Ok(op) => {
                    let op = op["id"].as_str().unwrap_or_default().to_owned();
                    self.op_dialog = crate::op_dialog::OpDialog::new(&self.session, &op);
                    self.status = None;
                }
                Err(e) => self.status = Some(e.to_string()),
            }
            return;
        }
        // The libraries open their windows; Edit in the post library opens that post.
        if params.is_null() && (id == "machine.library" || id == "post.library") {
            use crate::library_dialog::{Kind, LibraryDialog};
            let kind = if id == "machine.library" { Kind::Machines } else { Kind::Posts };
            self.library_dialog = Some(LibraryDialog::new(&self.session, kind));
            return;
        }
        // The machine and a post processor open in their tabs, beside the cut plane.
        if id == "ui.post.edit" || (params.is_null() && id == "post.settings") {
            let name = params.get("name").and_then(Value::as_str);
            self.post_panel = Some(crate::post_panel::PostPanel::new(&self.session, name));
            dock::focus(&mut self.dock, Pane::Post);
            self.status = None;
            return;
        }
        if params.is_null() && id == "machine.edit" {
            self.machine_panel = crate::machine_panel::MachinePanel::new(&self.session);
            dock::focus(&mut self.dock, Pane::Machine);
            self.status = None;
            return;
        }
        // Move without values puts the handles on the model in the viewport.
        if id == "model.move" && params.as_object().is_none_or(|p| p.keys().all(|k| k == "id")) {
            let model = params
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| self.session.selected_model().map(|m| m.id.clone()))
                .or_else(|| {
                    self.session.job.models.first().filter(|_| self.session.job.models.len() == 1).map(|m| m.id.clone())
                });
            match model {
                Some(m) => {
                    self.session.sim = None;
                    self.viewport.lay_flat = None;
                    self.viewport.moving = Some(m);
                    self.status = None;
                }
                None => self.status = Some("select a model to move".into()),
            }
            return;
        }
        // Lay flat without a face picks one in the viewport.
        if id == "model.lay_flat" && params.as_object().is_none_or(|p| p.keys().all(|k| k == "id")) {
            let model = params
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| self.session.selected_model().map(|m| m.id.clone()))
                .or_else(|| (self.session.job.models.len() == 1).then(|| self.session.job.models[0].id.clone()));
            match model {
                Some(m) => {
                    self.session.sim = None;
                    self.viewport.lay_flat = Some(m);
                    self.status = None;
                }
                None => self.status = Some("select a model to lay flat".into()),
            }
            return;
        }
        if id == "ui.about" {
            self.about_open = true;
            return;
        }
        // The start page: open it, leave it, or start a new job from it (in place of an empty
        // job, rather than beside it).
        match id {
            "ui.home" => {
                self.home = true;
                return;
            }
            "ui.home.close" => {
                self.home = false;
                return;
            }
            "ui.home.new" => {
                if !self.job_is_blank() {
                    self.run("job.new", &Value::Null);
                }
                self.home = false;
                return;
            }
            _ => {}
        }
        if id == "ui.quit" {
            self.quit = true;
            return;
        }
        // Closing a job with unsaved changes asks first.
        if id == "job.close" && params.is_null() && self.session.job.is_modified() {
            self.confirm_close = true;
            return;
        }
        // New sketch from the ribbon: the Sketch tab, and the plane picked in the viewport, seen
        // from an angle.
        if id == "sketch.new" && params.is_null() {
            self.viewport.picking_plane = true;
            let _ = self.session.execute("ui.ribbon.tab.set", &json!({ "tab": "sketch" }));
            if self.session.view.sketch.is_some() {
                let _ = self.session.execute("sketch.finish", &json!({}));
            }
            let _ = self.session.execute("view.camera.iso", &json!({}));
            self.status = None;
            return;
        }
        if params.is_null() && id == "settings.open" {
            self.settings_open = true;
            return;
        }
        // Space (or Simulate) while the simulation plays pauses it.
        if id == "sim.play" && params.is_null() && self.session.sim.as_ref().is_some_and(|s| s.playing) {
            self.run("sim.pause", &Value::Null);
            return;
        }
        // The explorer's Rename… and Delete open a small dialog first.
        if id == "ui.rename" || id == "ui.delete" {
            let field = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
            if id == "ui.rename" {
                self.rename = Some((field("command"), field("id"), field("name")));
            } else {
                self.confirm_delete = Some((field("command"), field("id"), field("label")));
            }
            return;
        }
        // Shape edits from the ribbon act on the selected shape.
        if id == "shape.transform" && params.is_null() {
            let given = self.session.selected_shape().map_or(Value::Null, |s| json!({ "id": s.id }));
            self.status = self.tools.start(id, &self.session, &given).err();
            return;
        }
        if id == "shape.remove" && params.is_null() {
            match self.session.selected_shape() {
                Some(s) => self.confirm_delete = Some((id.into(), s.id.clone(), s.name.clone())),
                None => self.status = Some("select a shape first".into()),
            }
            return;
        }
        if id == "cut.remove" && params.is_null() {
            match self.session.selected_cut() {
                Some(c) => self.confirm_delete = Some((id.into(), c.id.clone(), c.name.clone())),
                None => self.status = Some("select a cut first".into()),
            }
            return;
        }
        // Save asks where only the first time; after that it saves over the job's file.
        let saves_in_place = id == "job.save" && self.session.job.path.is_some();
        if params.is_null() && tools::has_start(id) && !saves_in_place {
            self.status = self.tools.start(id, &self.session, &Value::Null).err();
            return;
        }
        let params = if params.is_null() && saves_in_place { json!({}) } else { params.clone() };
        match self.session.execute(id, &params) {
            Ok(result) => {
                self.status = None;
                self.notice = notice(id, &result).or_else(|| self.notice.take().filter(|_| id == "selection.set"));
                if id.starts_with("job.") && id != "job.save" && id != "job.save.as" {
                    self.cut_panel = None;
                }
                // A job opened or a model brought in from the start page goes to the workspace.
                if LEAVES_HOME.contains(&id) {
                    self.home = false;
                }
            }
            Err(e) => {
                self.status = Some(e.to_string());
                self.notice = None;
            }
        }
    }

    /// Opens the cut editor in the Cut tab, beside the cut plane and the G-code.
    fn open_cut_panel(&mut self, panel: CutPanel) {
        self.cut_panel = Some(panel);
        dock::focus(&mut self.dock, Pane::Cut);
        self.status = None;
    }

    /// Puts the cut editor away and shows the cut plane in its place.
    fn close_cut_panel(&mut self) {
        self.cut_panel = None;
        dock::hide(&mut self.dock, Pane::Cut);
        dock::focus(&mut self.dock, Pane::Plane);
    }

    fn apply_ui_requests(&mut self) {
        for request in std::mem::take(&mut self.session.ui.requests) {
            match request {
                UiRequest::ShowPane(p) => dock::show(&mut self.dock, p),
                UiRequest::HidePane(p) => dock::hide(&mut self.dock, p),
                UiRequest::FocusPane(p) => dock::focus(&mut self.dock, p),
            }
        }
    }

    /// Requests a capture while one is pending and saves it when the image arrives.
    fn service_capture(&mut self, ctx: &egui::Context) {
        let Some(path) = self.capture.clone() else { return };
        ctx.request_repaint();
        if !self.capture_requested {
            self.capture_wait += 1;
            if (self.viewport.is_animating() || self.capture_wait < 3) && self.capture_wait < 120 {
                return;
            }
            self.capture_requested = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::default()));
            return;
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                _ => None,
            })
        });
        let Some(image) = image else { return };
        match automation::save_png(&image, &path) {
            Ok(()) => println!("screenshot saved to {}", path.display()),
            Err(e) => eprintln!("could not save screenshot {}: {e}", path.display()),
        }
        self.capture = None;
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) -> Vec<Request> {
        if ctx.egui_wants_keyboard_input() {
            return Vec::new();
        }
        let mut fired: Vec<Request> = Vec::new();
        ctx.input_mut(|i| {
            for (shortcut, id) in &self.shortcuts {
                // Only what can run now takes its key (L draws a line only in a sketch), and not
                // keys the viewport takes itself (R and C mid-line).
                if !cmd::find(id).is_some_and(|spec| self.session.is_enabled(spec))
                    || self.viewport.takes_key(shortcut.logical_key)
                {
                    continue;
                }
                if i.consume_shortcut(shortcut) {
                    fired.push(((*id).to_owned(), Value::Null));
                }
            }
            if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)) {
                fired.push(("edit.redo".into(), Value::Null));
            }
            // The window turns Ctrl+X, C and V into clipboard events rather than key presses.
            for event in &i.events {
                let id = match event {
                    Event::Cut => "edit.cut",
                    Event::Copy => "edit.copy",
                    Event::Paste(_) => "edit.paste",
                    _ => continue,
                };
                if !fired.iter().any(|(fired, _)| fired == id) {
                    fired.push((id.into(), Value::Null));
                }
            }
        });
        fired
    }

    fn preferences(&mut self, ctx: &egui::Context, tokens: &Tokens, requests: &mut Vec<Request>) {
        let mut open = true;
        let units = self.session.prefs.units;
        crate::ui_kit::Window::dialog("Preferences").show(ctx, tokens, &mut open, |ui| {
            ui.set_min_size(vec2(420.0, 220.0));
            egui::Grid::new("preferences").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                ui.label("Units:");
                egui::ComboBox::from_id_salt("pref-units").selected_text(units.label()).width(160.0).show_ui(
                    ui,
                    |ui| {
                        for choice in [
                            CosmicCut_engine::units::DisplayUnits::Inches,
                            CosmicCut_engine::units::DisplayUnits::Millimetres,
                        ] {
                            if ui.selectable_label(units == choice, choice.label()).clicked() && units != choice {
                                requests.push(("settings.units.set".into(), json!({ "units": choice.id() })));
                            }
                        }
                    },
                );
                ui.end_row();
                ui.label("Theme:");
                let theme = self.session.prefs.theme;
                let name = |t: Theme| match t {
                    Theme::Dark => "Dark",
                    Theme::Light => "Light",
                };
                egui::ComboBox::from_id_salt("pref-theme").selected_text(name(theme)).width(160.0).show_ui(ui, |ui| {
                    for choice in [Theme::Dark, Theme::Light] {
                        if ui.selectable_label(theme == choice, name(choice)).clicked() && theme != choice {
                            requests.push(("view.theme.set".into(), json!({ "theme": choice.id() })));
                        }
                    }
                });
                ui.end_row();
                ui.label("Viewport:");
                ui.vertical(|ui| {
                    let d = self.session.prefs.display;
                    for (key, label) in [
                        ("machine", "Turntable, towers and wire"),
                        ("rulings", "Rulings across the foam"),
                        ("stock", "Stock"),
                        ("grid", "Grid"),
                        ("view_cube", "View cube"),
                        ("readout", "Frame-rate readout"),
                    ] {
                        let mut on = d.get(key).unwrap_or(false);
                        if ui.checkbox(&mut on, label).changed() {
                            requests.push(("view.display.set".into(), json!({ key: on })));
                        }
                    }
                });
                ui.end_row();
                ui.label("Layout:");
                if ui.button("Restore").on_hover_text("Put the panels back where they started").clicked() {
                    requests.push(("view.layout.reset".into(), Value::Null));
                }
                ui.end_row();
            });
        });
        self.settings_open &= open;
    }

    fn dialogs(&mut self, ctx: &egui::Context, tokens: &Tokens, requests: &mut Vec<Request>) {
        if let Some((command, target, mut text)) = self.rename.take() {
            let mut open = true;
            let mut done: Option<bool> = None;
            crate::ui_kit::Window::dialog("Rename").show(ctx, tokens, &mut open, |ui| {
                ui.set_min_width(280.0);
                let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(260.0));
                if !response.has_focus() && !response.lost_focus() {
                    response.request_focus();
                }
                let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Rename").clicked() || enter {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
            match done {
                Some(true) if !text.trim().is_empty() => {
                    requests.push((command, json!({ "id": target, "name": text.trim() })));
                }
                Some(_) => {}
                None if open => self.rename = Some((command, target, text)),
                None => {}
            }
        }
        if self.confirm_close {
            let mut open = true;
            crate::ui_kit::Window::dialog("Close").show(ctx, tokens, &mut open, |ui| {
                ui.label(format!("{} has changes that are not saved.", self.session.job.name));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        requests.push(("job.save".into(), Value::Null));
                        // A job with a file saves now and closes; a new one asks where first, then
                        // Close is chosen again.
                        if self.session.job.path.is_some() {
                            requests.push(("job.close".into(), json!({})));
                        }
                        self.confirm_close = false;
                    }
                    if ui.button("Close without saving").clicked() {
                        requests.push(("job.close".into(), json!({})));
                        self.confirm_close = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_close = false;
                    }
                });
            });
            self.confirm_close &= open;
        }
        if self.about_open {
            let mut open = true;
            crate::ui_kit::Window::dialog("About CosmicCut").show(ctx, tokens, &mut open, |ui| {
                ui.set_width(360.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(4.0);
                    ui.label(RichText::new("CosmicCut").size(24.0).strong().color(tokens.accent));
                    ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(tokens.text_muted));
                    ui.add_space(8.0);
                    ui.label("CAM for hot-wire foam cutters: a wire that moves in X and Y over a B-axis turntable.");
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Built in Rust from CosmicDMS, with egui and wgpu.").color(tokens.text_muted),
                    );
                    ui.label(RichText::new("Licensed MIT OR Apache-2.0").color(tokens.text_muted));
                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        self.about_open = false;
                    }
                });
            });
            self.about_open &= open;
        }
        if let Some((command, target, label)) = self.confirm_delete.clone() {
            let mut open = true;
            crate::ui_kit::Window::dialog("Delete").show(ctx, tokens, &mut open, |ui| {
                ui.label(format!("Delete {label}?"));
                if command == "shape.remove"
                    && self.session.job.cuts.iter().any(|c| c.params.to_string().contains(&format!("\"{target}\"")))
                {
                    ui.label(RichText::new("Cuts that use it can no longer be made.").color(tokens.warning));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        requests.push((command.clone(), json!({ "id": target })));
                        self.confirm_delete = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_delete = None;
                    }
                });
            });
            if !open {
                self.confirm_delete = None;
            }
        }
    }
}

impl eframe::App for CosmicApp {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_ui_requests();
    }

    /// Screenshot runs leave egui's layout memory alone too.
    fn persist_egui_memory(&self) -> bool {
        !self.ephemeral
    }

    /// Also written every few seconds by eframe and on exit.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.ephemeral {
            return;
        }
        storage.set_string(
            SETTINGS_KEY,
            json!({
                "units": self.session.prefs.units.id(),
                "theme": self.session.prefs.theme.id(),
                "bottom": self.bottom.id(),
                "recent": self.recent,
            })
            .to_string(),
        );
        storage.set_string(MACHINE_KEY, self.session.job.machine.to_json().to_string());
        storage.set_string(LIBRARY_KEY, self.session.library.to_json().to_string());
        self.saved_prefs = self.session.prefs.revision;
        self.saved_library = self.session.library.revision;
        self.saved_recent = self.recent.clone();
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let tokens: &'static Tokens = Tokens::for_theme(self.session.prefs.theme);
        if self.applied_theme != Some(self.session.prefs.theme) {
            theme::apply(&ctx, tokens);
            self.applied_theme = Some(self.session.prefs.theme);
        }
        if self.layout_generation != self.session.view.layout_generation {
            self.dock = dock::default_layout();
            self.layout_generation = self.session.view.layout_generation;
        }
        // The simulation's clock runs with the frames while it plays.
        if let Some(sim) = self.session.sim.as_deref_mut()
            && sim.playing
        {
            sim.tick(f64::from(ctx.input(|i| i.stable_dt)));
            ctx.request_repaint();
        }
        let mut requests = self.handle_shortcuts(&ctx);
        requests.extend(self.tools.poll());
        if self.tools.is_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        requests.extend(self.tools.dropped(&dropped));
        #[cfg(target_arch = "wasm32")]
        crate::deliver_downloads();
        let step = self.automation.tick(&ctx);
        requests.extend(step.requests);
        if let Some(path) = step.capture {
            self.capture = Some(path);
        }
        self.service_capture(&ctx);
        ribbon::resize_edges(&ctx);
        let panes = dock::open_panes(&self.dock);
        // A job with a file is one to come back to.
        if let Some(path) = self.session.job.path.clone() {
            home::remember(&mut self.recent, &path);
        }
        ribbon::show(ui, &self.session, tokens, &panes, !self.home, &mut requests);

        let status = self.status.clone();
        // The frame rate is the viewport's; the start page has none.
        let fps = if self.home { String::new() } else { format!("  ·  {:.0} fps", self.viewport.fps()) };
        Panel::bottom("status", Surface::StatusBar).show(ui, tokens, |ui| {
            ui.horizontal(|ui| {
                match (&status, &self.notice) {
                    (Some(msg), _) => ui.label(RichText::new(msg).color(tokens.error)),
                    (None, Some(note)) => ui.label(RichText::new(note).color(tokens.success)),
                    (None, None) => ui.label(RichText::new("Ready").color(tokens.text_muted)),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let m = &self.session.job.machine;
                    ui.label(
                        RichText::new(format!(
                            "{}  ·  {}  ·  {}  ·  program in {}{fps}",
                            m.name,
                            if m.b_axis { "X Y B" } else { "X Y" },
                            self.session.prefs.units.length(),
                            m.post.units.length(),
                        ))
                        .color(tokens.text_muted),
                    );
                });
            });
        });

        self.viewport.preview = self.cut_panel.as_ref().and_then(|p| p.preview().cloned());
        // Picking a cut's start point in the viewport, seen square on when the cut is at B = 0.
        let picking = self.cut_panel.as_ref().filter(|p| p.picking_start).map(|p| p.start_angle());
        if picking.is_some() && self.viewport.pick_start.is_none() && picking.is_some_and(|b| b.abs() < 1.0e-9) {
            let _ = self.session.execute("view.camera.front", &json!({}));
        }
        self.viewport.pick_start = picking;
        self.viewport.face_pick = self.cut_panel.as_ref().and_then(|p| p.face_pick());
        self.viewport.line_pick = self.cut_panel.as_ref().and_then(|p| p.line_pick(&self.session));

        let mut cut_outcome: Option<(Option<Request>, bool)> = None;
        let mut stock_done = false;
        let mut bottom: Option<Pane> = None;
        let mut side_outcome: Option<(Pane, Option<Request>, bool)> = None;
        self.viewport.stock_edit = self.stock_panel.is_some();
        if self.home {
            let back = (!self.job_is_blank()).then(|| {
                let name = self.session.job.name.trim();
                if name.is_empty() { "the job".to_owned() } else { name.to_owned() }
            });
            self.home_page.show(ui, tokens, &self.recent, back.as_deref(), &mut requests);
        } else {
            Panel::central(Surface::Workspace).show(ui, tokens, |ui| {
                let mut style = Style::from_egui(ui.style());
                style.tab_bar.bg_fill = tokens.raised_bg;
                style.tab.active.bg_fill = tokens.panel_bg;
                style.tab.focused.bg_fill = tokens.panel_bg;
                style.tab.active.text_color = tokens.text;
                style.tab.focused.text_color = tokens.accent;
                style.tab.inactive.text_color = tokens.text_muted;
                style.tab.tab_body.bg_fill = tokens.panel_bg;
                style.tab.tab_body.stroke = egui::Stroke::new(1.0, tokens.border);
                style.separator.color_idle = tokens.border;
                style.separator.color_hovered = tokens.accent;
                style.separator.color_dragged = tokens.accent;
                style.overlay.selection_color = tokens.accent.gamma_multiply(0.35);
                let mut viewer = dock::Viewer {
                    session: &mut self.session,
                    tokens,
                    viewport: &mut self.viewport,
                    render_state: self.render_state.as_ref(),
                    console: &mut self.console,
                    gcode: &mut self.gcode,
                    requests: &mut requests,
                    cut_panel: &mut self.cut_panel,
                    cut_outcome: &mut cut_outcome,
                    stock_panel: &mut self.stock_panel,
                    stock_done: &mut stock_done,
                    machine_panel: &mut self.machine_panel,
                    post_panel: &mut self.post_panel,
                    side_outcome: &mut side_outcome,
                    bottom: &mut bottom,
                };
                DockArea::new(&mut self.dock)
                    .id(egui::Id::new("workspace-dock"))
                    .style(style)
                    .show_leaf_collapse_buttons(false)
                    .show_inside(ui, &mut viewer);
            });
        }
        // The Cut tab's buttons: Create / Update runs the command and closes the tab; Cancel
        // closes it; closing the tab drops the edit.
        if let Some((request, keep)) = cut_outcome {
            if let Some((id, params)) = request {
                match self.session.execute(&id, &params) {
                    Ok(_) => self.close_cut_panel(),
                    Err(e) => {
                        if let Some(panel) = &mut self.cut_panel {
                            panel.fail(e.to_string());
                        }
                    }
                }
            } else if !keep {
                self.close_cut_panel();
            }
        }
        if self.cut_panel.is_some() && self.dock.find_tab(&Pane::Cut).is_none() {
            self.cut_panel = None;
        }
        if let Some(pane) = bottom {
            self.bottom = pane;
        }
        if let Some(picked) = self.viewport.picked_start.take()
            && let Some(panel) = self.cut_panel.as_mut()
        {
            panel.take_start(picked);
        }
        if let Some(line) = self.viewport.picked_line.take()
            && let Some(panel) = self.cut_panel.as_mut()
        {
            panel.toggle_line(line);
        }
        if std::mem::take(&mut self.viewport.stop_line_pick)
            && let Some(panel) = self.cut_panel.as_mut()
        {
            panel.picking_lines = false;
        }
        let face = self.viewport.picked_face.take();
        let stop = std::mem::take(&mut self.viewport.stop_face_pick);
        if let Some(panel) = self.cut_panel.as_mut() {
            if let Some((model, f)) = face {
                panel.toggle_faces(&model, &f);
            }
            if stop {
                panel.picking_faces = false;
            }
        }
        // Save in the Machine or Post processor tab runs its command and closes the tab; Cancel
        // closes it; a failure keeps it open and says why.
        if let Some((pane, request, keep)) = side_outcome {
            let mut close = !keep;
            if let Some((id, params)) = request {
                match self.session.execute(&id, &params) {
                    Ok(result) => {
                        self.status = None;
                        self.notice = notice(&id, &result);
                        close = true;
                    }
                    Err(e) => match pane {
                        Pane::Machine => {
                            if let Some(p) = &mut self.machine_panel {
                                p.fail(e.to_string());
                            }
                        }
                        _ => {
                            if let Some(p) = &mut self.post_panel {
                                p.fail(e.to_string());
                            }
                        }
                    },
                }
            }
            if close {
                dock::hide(&mut self.dock, pane);
                dock::focus(&mut self.dock, Pane::Plane);
            }
        }
        if self.machine_panel.is_some() && self.dock.find_tab(&Pane::Machine).is_none() {
            self.machine_panel = None;
        }
        if self.post_panel.is_some() && self.dock.find_tab(&Pane::Post).is_none() {
            self.post_panel = None;
        }
        // Done, or closing the tab, puts the stock editor away.
        if stock_done {
            dock::hide(&mut self.dock, Pane::Stock);
            dock::focus(&mut self.dock, Pane::Plane);
        }
        if self.stock_panel.is_some() && self.dock.find_tab(&Pane::Stock).is_none() {
            self.stock_panel = None;
        }

        if self.settings_open {
            self.preferences(&ctx, tokens, &mut requests);
        }
        if let Some(dialog) = &mut self.library_dialog
            && !dialog.show(&ctx, &self.session, tokens, &mut requests)
        {
            self.library_dialog = None;
        }
        if let Some(dialog) = &mut self.op_dialog {
            let (request, keep) = dialog.show(&ctx, tokens);
            requests.extend(request);
            if !keep {
                self.op_dialog = None;
            }
        }
        self.dialogs(&ctx, tokens, &mut requests);
        for (id, params) in requests {
            self.run(&id, &params);
        }
        if self.quit {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if let Some((id, params)) = self.tools.show(&ctx, tokens) {
            let outcome = self.session.execute(&id, &params);
            if let Ok(result) = &outcome {
                self.notice = notice(&id, result);
            }
            let outcome = outcome.map(|_| ()).map_err(|e| e.to_string());
            self.status = outcome.as_ref().err().cloned();
            self.tools.finished(outcome);
        }
        // A changed library is written straight away, so a crash cannot lose a machine.
        if (self.session.prefs.revision != self.saved_prefs
            || self.session.library.revision != self.saved_library
            || self.recent != self.saved_recent)
            && let Some(storage) = frame.storage_mut()
        {
            self.save(storage);
            storage.flush();
        }
    }
}

/// Commands that leave the start page for the workspace once they have run.
const LEAVES_HOME: &[&str] = &["job.new", "job.open", "job.sample", "model.import", "shape.import"];

impl CosmicApp {
    /// The job in front has nothing in it and no file: one to replace, not keep.
    fn job_is_blank(&self) -> bool {
        let j = &self.session.job;
        j.path.is_none() && !j.is_modified() && j.models.is_empty() && j.shapes.is_empty() && j.cuts.is_empty()
    }
}

/// What a command's success is worth telling the user.
fn notice(id: &str, result: &Value) -> Option<String> {
    let s = |k: &str| result[k].as_str().unwrap_or_default().to_owned();
    match id {
        "post.gcode.export" => {
            let warnings: Vec<&str> =
                result["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            Some(format!(
                "Wrote {} lines of G-code to {}{}",
                result["lines"].as_u64().unwrap_or(0),
                s("path"),
                if warnings.is_empty() { String::new() } else { format!(" (check: {})", warnings.join("; ")) }
            ))
        }
        "post.limits.check" => Some(match result["problems"].as_array() {
            Some(p) if !p.is_empty() => p.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "),
            _ => "The program stays inside the machine's travel".into(),
        }),
        "job.save" | "job.save.as" => Some(format!("Saved {}", s("path"))),
        "model.import" => {
            let warnings: Vec<&str> =
                result["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            Some(format!(
                "Imported {} ({} triangles) on the turntable{}. Use Lay flat to choose the face it stands on",
                s("name"),
                result["triangles"].as_u64().unwrap_or(0),
                if warnings.is_empty() { String::new() } else { format!(": {}", warnings.join("; ")) }
            ))
        }
        "post.settings" => Some("Post-processor settings saved with the machine".into()),
        "shape.import" => {
            let warnings: Vec<&str> =
                result["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            Some(if warnings.is_empty() {
                format!("Imported {} ({} outlines)", s("name"), result["paths"].as_u64().unwrap_or(0))
            } else {
                format!("Imported {}: {}", s("name"), warnings.join("; "))
            })
        }
        _ => None,
    }
}

/// Parses "Ctrl+Shift+T", "F" or "7" style shortcuts.
fn parse_shortcut(text: &str) -> Option<KeyboardShortcut> {
    let mut modifiers = Modifiers::NONE;
    let mut key = None;
    for part in text.split('+') {
        match part {
            "Ctrl" | "Cmd" => modifiers |= Modifiers::COMMAND,
            "Shift" => modifiers |= Modifiers::SHIFT,
            "Alt" => modifiers |= Modifiers::ALT,
            other => key = Key::from_name(other),
        }
    }
    Some(KeyboardShortcut::new(modifiers, key?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_shortcut_parses() {
        for c in cmd::all() {
            if let Some(s) = c.shortcut {
                assert!(parse_shortcut(s).is_some(), "{}: {s}", c.id);
            }
        }
    }

    #[test]
    fn shortcuts_are_unique() {
        let mut seen: Vec<&str> = cmd::all().filter_map(|c| c.shortcut).collect();
        let n = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), n);
    }
}
