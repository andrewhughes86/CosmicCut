//! The cut editor: the Cut tab, beside the cut plane and the G-code, while a cut is being made
//! or changed. Its cards (Shape, Rotations, Path, Bottom, Placement, Wire…) set the parameters of
//! the cut's command (`cut.profile`, `cut.model`, …); the wire path is previewed as settings
//! change, and OK runs the command.

use CosmicCut_engine::cmd::cut::params_of;
use CosmicCut_engine::cut::{Built, CutKind, fmt_time};
use CosmicCut_engine::units::DisplayUnits;
use CosmicCut_engine::{EngineResult, Param, ParamKind, Session};
use egui::{RichText, Ui, vec2};
use serde_json::{Map, Value, json};

use crate::Request;
use crate::param_form::is_shape_param;
use crate::theme::Tokens;
use crate::ui_kit::{Container, Surface};

/// The label column and the inputs, sized to the tab's width.
#[derive(Clone, Copy)]
struct Widths {
    label: f32,
    input: f32,
}

impl Widths {
    fn of(ui: &Ui) -> Self {
        let w = ui.available_width();
        Self { label: (w * 0.36).clamp(110.0, 200.0), input: (w * 0.28).clamp(80.0, 160.0) }
    }
}

/// Parameters the panel carries without showing.
const HIDDEN: &[&str] = &["id", "name", "model", "models", "faces", "ignore", "skip"];

/// The card a parameter shows in.
fn section(name: &str) -> &'static str {
    match name {
        "shape" | "front" | "side" | "axis" => "Shape",
        "anchor" | "x" | "y" | "scale" | "rotate" | "mirror" => "Placement",
        n if n.starts_with("front_") || n.starts_with("side_") => "Placement",
        "spacing" | "divisions" | "degrees" => "Rotations",
        "height_step" | "tolerance" => "Faces",
        "angle" | "copies" | "step" | "sweep" => "Table",
        "thickness" | "skim" | "count" | "order" => "Slabs",
        "keep" | "lead" | "direction" | "alternate" | "start_x" | "start_y" => "Path",
        "bottom" | "attach" | "release" => "Bottom",
        _ => "Wire",
    }
}

const SECTIONS: &[&str] = &["Shape", "Faces", "Rotations", "Table", "Slabs", "Path", "Bottom", "Placement", "Wire"];

/// What a card is for, under its heading.
fn intro(section: &str, kind: CutKind) -> &'static str {
    match section {
        "Shape" => "The outline the wire follows, seen along the wire.",
        "Rotations" => {
            "The table turns between cuts, and at each angle the wire follows a new profile: the model's outline \
             seen from there. More cuts give a rounder part."
        }
        "Faces" => {
            "Click the model's faces for the wire to follow. At every height the wire lies along them, X, Y and \
             the table moving together; faces that line up are cut in one pass."
        }
        "Placement" => "Where the outline sits in the foam. Blank places it in the middle of the stock.",
        "Table" => "The turntable angle (B) the cut is made at. Copies repeat it at angles round the table.",
        "Slabs" => "Level passes across the block; each slab is its thickness plus a kerf below the last.",
        "Path" => "Where the wire comes into the foam, and which way it goes.",
        "Bottom" if kind == CutKind::Model => {
            "The part stays joined to the foam under it while the table turns, so it cannot move. Parting off \
             cuts across its bottom as the very last cut of the program."
        }
        "Bottom" => {
            "A part has to stay joined to the foam under it while the table turns, or it can move. Parting off \
             cuts across its bottom as the very last cut of the program."
        }
        _ => "The machine's settings to start with; change them for this cut only.",
    }
}

/// What an option is called in the panel.
fn label(name: &str) -> String {
    let named = match name {
        "shape" => "Shape",
        "start_x" => "Start across (X)",
        "start_y" => "Start height (Y)",
        "front" => "Front shape",
        "side" => "Side shape",
        "anchor" => "Place by",
        "x" => "Across (X)",
        "y" => "Height (Y)",
        "front_x" => "Front across (X)",
        "front_y" => "Front height (Y)",
        "side_x" => "Side across (X)",
        "side_y" => "Side height (Y)",
        "front_rotate" => "Turn front shape",
        "side_rotate" => "Turn side shape",
        "front_mirror" => "Mirror front shape",
        "side_mirror" => "Mirror side shape",
        "rotate" => "Turn shape",
        "mirror" => "Mirror shape",
        "axis" => "Turning axis",
        "spacing" => "Space the cuts by",
        "divisions" => "Number of cuts",
        "degrees" => "Degrees between cuts",
        "angle" => "Table angle (B)",
        "copies" => "Copies",
        "step" => "Step between",
        "sweep" => "Sweep",
        "keep" => "Keep",
        "lead" => "Enter from",
        "direction" => "Direction",
        "alternate" => "Alternate direction",
        "bottom" => "Part's bottom",
        "attach" => "Held up to",
        "release" => "Part off at the end",
        "compensate" => "Kerf compensation",
        "feed" => "Feed rate",
        "height_step" => "Height step",
        "tolerance" => "Straightness tolerance",
        other => {
            let words = other.replace('_', " ");
            let mut chars = words.chars();
            return chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
        }
    };
    named.to_owned()
}

/// How a choice reads on its toggle.
fn choice_label(name: &str, value: &str) -> String {
    match (name, value) {
        ("spacing", "divide") => "Divide 180° by a number".into(),
        ("spacing", "degrees") => "Degrees apart".into(),
        ("direction", "ccw") => "Anticlockwise".into(),
        ("direction", "cw") => "Clockwise".into(),
        (_, v) => {
            let words = v.replace('_', " ");
            let mut chars = words.chars();
            chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
        }
    }
}

/// The value a yes/no option has when it was never set.
fn bool_default(name: &str) -> bool {
    matches!(name, "alternate" | "compensate" | "release")
}

enum Field {
    Text(String),
    Bool(bool),
    Choice(usize),
    Shape(Option<usize>),
}

pub struct CutPanel {
    pub kind: CutKind,
    /// The cut being changed; None while making a new one.
    editing: Option<String>,
    name: String,
    fields: Vec<(&'static Param, Field)>,
    /// Parameters carried along without a field (the model a model cut uses).
    hidden: Map<String, Value>,
    units: DisplayUnits,
    /// The parameters the preview was made from, and what it came to.
    previewed: Option<String>,
    preview: Option<EngineResult<Built>>,
    error: Option<String>,
    /// Waiting for the start point to be clicked in the viewport.
    pub picking_start: bool,
    /// Faces of the model are being clicked to add or take away.
    pub picking_faces: bool,
    /// Lines of a profile's sketch are being clicked to leave out or put back.
    pub picking_lines: bool,
}

impl CutPanel {
    /// A panel for a new cut of `kind`, starting from `given` (e.g. the shape to use), or for
    /// changing the cut `editing`.
    pub fn new(session: &Session, kind: CutKind, editing: Option<&str>, given: &Value) -> Self {
        let units = session.prefs.units;
        let machine = &session.job.machine;
        let shapes: Vec<&str> = session.job.shapes.iter().map(|s| s.id.as_str()).collect();
        let cut = editing.and_then(|id| session.job.cut(id));
        let mut values: Map<String, Value> = cut.and_then(|c| c.params.as_object().cloned()).unwrap_or_default();
        if cut.is_none() {
            // A new cut starts from the machine's numbers, shown so they can be changed.
            values.insert("kerf".into(), json!(machine.kerf));
            values.insert("feed".into(), json!(machine.default_feed));
            values.insert("clearance".into(), json!(machine.clearance));
            match kind {
                CutKind::Slab => {
                    values.insert("thickness".into(), json!(if units == DisplayUnits::Inches { 50.8 } else { 50.0 }));
                }
                CutKind::Lathe => {
                    values.insert("step".into(), json!(5.0));
                }
                CutKind::Swarf => {
                    values.insert("height_step".into(), json!(2.0));
                    let model = session.selected_model().or(session.job.models.first()).map(|m| m.id.clone());
                    if let Some(m) = model {
                        values.insert("model".into(), json!(m));
                    }
                    values.insert("faces".into(), json!([]));
                }
                CutKind::Model => {
                    values.insert("divisions".into(), json!(8));
                    values.insert("degrees".into(), json!(22.5));
                    // Every model, cut together: the wire cannot pass one to cut another.
                    let all: Vec<&str> = session.job.models.iter().map(|m| m.id.as_str()).collect();
                    values.insert("models".into(), json!(all));
                }
                _ => {}
            }
            let selected = session.selected_shape().map(|s| s.id.clone());
            for key in ["shape", "front"] {
                if let Some(id) = selected.clone().or_else(|| shapes.first().map(|s| (*s).to_owned())) {
                    values.insert(key.into(), json!(id));
                }
            }
            if let Some(side) = shapes.iter().find(|s| Some(**s) != selected.as_deref()).or(shapes.first()) {
                values.insert("side".into(), json!(side));
            }
        }
        if let Value::Object(g) = given {
            for (k, v) in g {
                values.insert(k.clone(), v.clone());
            }
        }
        let params = params_of(kind);
        let hidden: Map<String, Value> = values
            .iter()
            .filter(|(k, _)| HIDDEN.contains(&k.as_str()) && k.as_str() != "id" && k.as_str() != "name")
            .filter(|(k, _)| params.iter().any(|p| p.name == k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let fields = params
            .iter()
            .filter(|p| !HIDDEN.contains(&p.name))
            .map(|p| {
                let v = values.get(p.name);
                let field = if is_shape_param("cut", p.name) {
                    let want = v.and_then(Value::as_str);
                    Field::Shape(
                        want.and_then(|w| shapes.iter().position(|s| *s == w)).or((!shapes.is_empty()).then_some(0)),
                    )
                } else {
                    match p.kind {
                        ParamKind::Bool => Field::Bool(v.and_then(Value::as_bool).unwrap_or(bool_default(p.name))),
                        ParamKind::Choice(c) => Field::Choice(
                            v.and_then(Value::as_str).and_then(|s| c.iter().position(|x| *x == s)).unwrap_or(0),
                        ),
                        ParamKind::Length { .. } | ParamKind::Rate { .. } => Field::Text(
                            v.and_then(Value::as_f64)
                                .map(|mm| match p.kind {
                                    ParamKind::Rate { .. } => units.plain_rate(mm),
                                    _ => units.plain(mm),
                                })
                                .unwrap_or_default(),
                        ),
                        _ => Field::Text(
                            v.map(|v| match v {
                                Value::String(s) => s.clone(),
                                Value::Number(n) => {
                                    let s = format!("{:.4}", n.as_f64().unwrap_or(0.0));
                                    s.trim_end_matches('0').trim_end_matches('.').to_owned()
                                }
                                other => other.to_string(),
                            })
                            .unwrap_or_default(),
                        ),
                    }
                };
                (p, field)
            })
            .collect();
        let name = cut.map(|c| c.name.clone()).unwrap_or_default();
        Self {
            kind,
            editing: editing.map(str::to_owned),
            name,
            fields,
            hidden,
            units,
            previewed: None,
            preview: None,
            error: None,
            picking_start: false,
            picking_faces: false,
            picking_lines: false,
        }
        .picking_faces_if_none()
    }

    /// A new swarf cut starts with its faces being picked.
    fn picking_faces_if_none(mut self) -> Self {
        self.picking_faces = self.kind == CutKind::Swarf && self.faces().is_empty();
        self
    }

    /// The faces a swarf cut follows, by number.
    pub fn faces(&self) -> Vec<usize> {
        self.hidden
            .get("faces")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_u64).filter_map(|f| usize::try_from(f).ok()).collect())
            .unwrap_or_default()
    }

    /// The model a swarf cut's faces are on, its faces, and whether more are being picked.
    pub fn face_pick(&self) -> Option<(String, Vec<usize>, bool)> {
        if self.kind != CutKind::Swarf {
            return None;
        }
        let model = self.hidden.get("model").and_then(Value::as_str)?.to_owned();
        Some((model, self.faces(), self.picking_faces))
    }

    /// Faces were clicked (the clicked one first): in if it was out, all out if it was in.
    pub fn toggle_faces(&mut self, clicked: &[usize]) {
        let mut faces = self.faces();
        let Some(first) = clicked.first() else { return };
        if faces.contains(first) {
            faces.retain(|f| !clicked.contains(f));
        } else {
            faces.extend(clicked.iter().filter(|f| !faces.contains(f)).copied().collect::<Vec<_>>());
        }
        self.hidden.insert("faces".into(), json!(faces));
    }

    pub fn fail(&mut self, message: String) {
        self.error = Some(message);
    }

    /// The preview to draw: the path these settings make, if they make one.
    pub fn preview(&self) -> Option<&Built> {
        self.preview.as_ref().and_then(|r| r.as_ref().ok())
    }

    /// The chosen value of choice `name`, if the cut has one.
    fn choice(&self, name: &str) -> Option<&'static str> {
        self.fields.iter().find(|(p, _)| p.name == name).and_then(|(p, f)| match (p.kind, f) {
            (ParamKind::Choice(c), Field::Choice(i)) => c.get(*i).copied(),
            _ => None,
        })
    }

    /// Whether the field for `name` applies with the other settings as they are.
    fn applies(&self, name: &str) -> bool {
        match name {
            "divisions" => self.choice("spacing") != Some("degrees"),
            "degrees" => self.choice("spacing") == Some("degrees"),
            "attach" | "release" => self.choice("bottom") != Some("through"),
            _ => true,
        }
    }

    /// The command's parameters from the fields; blank fields are left to their defaults.
    fn params(&self, session: &Session) -> Result<Value, String> {
        let mut out = self.hidden.clone();
        let shapes: Vec<&str> = session.job.shapes.iter().map(|s| s.id.as_str()).collect();
        for (p, field) in &self.fields {
            let label = label(p.name);
            let value = match field {
                Field::Text(t) if t.trim().is_empty() => None,
                Field::Text(t) => {
                    let n = || t.trim().parse::<f64>().map_err(|_| format!("{label} must be a number"));
                    Some(match p.kind {
                        ParamKind::Length { .. } | ParamKind::Rate { .. } => json!(self.units.to_mm(n()?)),
                        ParamKind::Number { .. } => json!(n()?),
                        ParamKind::Integer { .. } => {
                            json!(t.trim().parse::<i64>().map_err(|_| format!("{label} must be a whole number"))?)
                        }
                        _ => json!(t.trim()),
                    })
                }
                Field::Bool(b) => Some(Value::Bool(*b)),
                Field::Choice(i) => match p.kind {
                    ParamKind::Choice(c) => c.get(*i).map(|c| json!(c)),
                    _ => None,
                },
                Field::Shape(i) => i.and_then(|i| shapes.get(i)).map(|s| json!(s)),
            };
            match value {
                Some(v) => {
                    out.insert(p.name.to_owned(), v);
                }
                None if p.required => return Err(format!("{label} is needed")),
                None => {}
            }
        }
        if !self.name.trim().is_empty() {
            out.insert("name".into(), json!(self.name.trim()));
        }
        if let Some(id) = &self.editing {
            out.insert("id".into(), json!(id));
        }
        Ok(Value::Object(out))
    }

    fn refresh_preview(&mut self, session: &Session) {
        let params = self.params(session);
        let key = format!("{:?}{}", params, session.job.revision);
        if self.previewed.as_deref() == Some(key.as_str()) {
            return;
        }
        self.preview = Some(match params {
            Ok(p) => session.preview_cut(self.kind, &p),
            Err(e) => Err(CosmicCut_engine::EngineError::InvalidParams(e)),
        });
        self.previewed = Some(key);
    }

    /// Draws the panel. Returns the command to run when OK is pressed, and whether to stay open.
    pub fn show(&mut self, ui: &mut Ui, session: &Session, tokens: &Tokens) -> (Option<Request>, bool) {
        self.refresh_preview(session);
        let mut keep = true;
        let mut ok = false;
        let units = self.units;
        let shapes: Vec<String> = session.job.shapes.iter().map(|s| s.name.clone()).collect();
        {
            ui.spacing_mut().item_spacing.y = 6.0;
            self.header(ui, tokens);
            ui.add_space(4.0);
            let footer = 190.0;
            egui::ScrollArea::vertical()
                .id_salt(("cut-panel-cards", self.kind.id()))
                .max_height((ui.available_height() - footer).max(160.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for name in SECTIONS {
                        let shown: Vec<usize> = (0..self.fields.len())
                            .filter(|&i| {
                                let p = self.fields[i].0;
                                section(p.name) == *name && self.applies(p.name)
                            })
                            .collect();
                        if shown.is_empty() {
                            continue;
                        }
                        Container::new(Surface::Card).show(ui, tokens, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(*name).strong().size(15.0).color(tokens.text));
                            ui.label(RichText::new(intro(name, self.kind)).color(tokens.text_muted));
                            if *name == "Rotations" {
                                self.model_boxes(ui, session, tokens);
                                self.rotations_summary(ui, tokens);
                            }
                            if *name == "Faces" {
                                self.face_buttons(ui, tokens);
                            }
                            if *name == "Shape" && self.kind == CutKind::Profile {
                                self.line_buttons(ui, session, tokens);
                            }
                            if *name == "Wire" {
                                self.ignore_boxes(ui, session, tokens);
                            }
                            if *name == "Path" && self.fields.iter().any(|(p, _)| p.name == "start_x") {
                                self.start_buttons(ui, tokens);
                            }
                            ui.add_space(6.0);
                            let widths = Widths::of(ui);
                            for i in shown {
                                if let Some((p, field)) = self.fields.get_mut(i) {
                                    row(ui, tokens, p, field, &shapes, units, widths);
                                }
                            }
                        });
                        ui.add_space(8.0);
                    }
                });
            ui.add_space(4.0);
            self.footer(ui, session, tokens, &mut ok, &mut keep);
        }
        // Esc lets go of a start point being picked first, then closes the editor.
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().egui_wants_keyboard_input() {
            if self.picking_start {
                self.picking_start = false;
            } else if self.picking_faces {
                self.picking_faces = false;
            } else if self.picking_lines {
                self.picking_lines = false;
            } else {
                keep = false;
            }
        }
        if ok {
            match self.params(session) {
                Ok(p) => return (Some((self.kind.command().to_owned(), p)), true),
                Err(e) => self.error = Some(e),
            }
        }
        (None, keep)
    }

    fn header(&mut self, ui: &mut Ui, tokens: &Tokens) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
            crate::ribbon::paint_named_icon(ui, rect, CosmicCut_engine::cmd::cut_icon(self.kind), tokens, true);
            ui.vertical(|ui| {
                let verb = if self.editing.is_some() { "Edit" } else { "New" };
                let what = match self.kind {
                    CutKind::Model => "cut from model".to_owned(),
                    k => format!("{} cut", k.label().to_lowercase()),
                };
                ui.label(RichText::new(format!("{verb} {what}")).strong().size(18.0).color(tokens.text));
                if let Some(spec) = CosmicCut_engine::cmd::find(self.kind.command()) {
                    ui.add(egui::Label::new(RichText::new(spec.tooltip).color(tokens.text_muted)).wrap());
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_sized(vec2(60.0, 22.0), egui::Label::new(RichText::new("Name").strong()));
            ui.add(
                egui::TextEdit::singleline(&mut self.name)
                    .hint_text(match self.kind {
                        CutKind::Model => "From model",
                        k => k.label(),
                    })
                    .desired_width(f32::INFINITY),
            );
        });
    }

    /// Pick the start point in the viewport, or let it go back to the lead side.
    fn start_buttons(&mut self, ui: &mut Ui, tokens: &Tokens) {
        ui.horizontal(|ui| {
            let text = if self.picking_start { "Click in the viewport…" } else { "Pick the start point" };
            let button = egui::Button::new(RichText::new(text).color(if self.picking_start {
                tokens.on_accent
            } else {
                tokens.text
            }))
            .fill(if self.picking_start { tokens.accent } else { tokens.raised_bg });
            if ui
                .add(button)
                .on_hover_text("Click where the cut should start, on the plane the wire cuts in; Esc cancels")
                .clicked()
            {
                self.picking_start = !self.picking_start;
            }
            let set = self.text_of("start_x").is_some_and(|t| !t.trim().is_empty());
            if ui.add_enabled(set, egui::Button::new("Clear")).on_hover_text("Start from the lead side again").clicked()
            {
                self.set_text("start_x", String::new());
                self.set_text("start_y", String::new());
            }
        });
        ui.add_space(4.0);
    }

    /// For cuts from models, when there is more than one: which are cut together.
    fn model_boxes(&mut self, ui: &mut Ui, session: &Session, tokens: &Tokens) {
        if self.kind != CutKind::Model || session.job.models.len() < 2 {
            return;
        }
        // An older cut names one model; else the list, else every model.
        let mut chosen: Vec<String> = match (self.hidden.get("models"), self.hidden.get("model")) {
            (Some(Value::Array(a)), _) => a.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
            (_, Some(Value::String(m))) => vec![m.clone()],
            _ => session.job.models.iter().map(|m| m.id.clone()).collect(),
        };
        ui.add_space(4.0);
        ui.label(RichText::new("Models cut together").strong());
        let mut changed = false;
        for m in &session.job.models {
            let mut on = chosen.contains(&m.id);
            // The last one stays: a cut needs a model.
            let last = on && chosen.len() == 1;
            if ui.add_enabled(!last, egui::Checkbox::new(&mut on, m.name.as_str())).changed() {
                if on {
                    chosen.push(m.id.clone());
                } else {
                    chosen.retain(|c| *c != m.id);
                }
                changed = true;
            }
        }
        ui.label(
            RichText::new("The wire runs right across the foam, so a model left out must stand clear of it.")
                .color(tokens.text_muted),
        );
        if changed {
            self.hidden.remove("model");
            self.hidden.insert("models".into(), json!(chosen));
        }
    }

    /// The models this cut does not make that will be off the table when it runs: the wire may
    /// pass through them.
    fn ignore_boxes(&mut self, ui: &mut Ui, session: &Session, tokens: &Tokens) {
        let own: Vec<String> = match self.kind {
            CutKind::Model => match (self.hidden.get("models"), self.hidden.get("model")) {
                (Some(Value::Array(a)), _) => a.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
                (_, Some(Value::String(m))) => vec![m.clone()],
                _ => session.job.models.iter().map(|m| m.id.clone()).collect(),
            },
            CutKind::Swarf => self.hidden.get("model").and_then(Value::as_str).map(str::to_owned).into_iter().collect(),
            _ => Vec::new(),
        };
        let others: Vec<_> = session.job.models.iter().filter(|m| !own.contains(&m.id)).collect();
        if others.is_empty() {
            return;
        }
        let mut ignored: Vec<String> = self
            .hidden
            .get("ignore")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        // What the cut's operation already says is off the table.
        let op = self
            .editing
            .as_deref()
            .and_then(|id| session.job.cut(id))
            .map_or(session.job.active_op.as_str(), |c| c.op.as_str());
        let by_op: Vec<String> =
            session.job.ops.iter().find(|o| o.id == op).map(|o| o.ignore.clone()).unwrap_or_default();
        ui.add_space(4.0);
        ui.label(RichText::new("Models already off the table").strong());
        let mut changed = false;
        for m in others {
            if by_op.contains(&m.id) {
                let mut on = true;
                ui.add_enabled(false, egui::Checkbox::new(&mut on, format!("{} (its operation says so)", m.name)));
                continue;
            }
            let mut on = ignored.contains(&m.id);
            if ui.checkbox(&mut on, m.name.as_str()).changed() {
                if on {
                    ignored.push(m.id.clone());
                } else {
                    ignored.retain(|i| *i != m.id);
                }
                changed = true;
            }
        }
        ui.label(
            RichText::new(
                "The wire may pass through these: they were cut and taken off before this cut runs. The cut fails if \
                 the wire would cut any other model. Operation settings can say this for all of an operation's cuts.",
            )
            .color(tokens.text_muted),
        );
        ui.add_space(6.0);
        if changed {
            if ignored.is_empty() {
                self.hidden.remove("ignore");
            } else {
                self.hidden.insert("ignore".into(), json!(ignored));
            }
        }
    }

    /// The lines a profile leaves out, by sketch curve id.
    fn skipped(&self) -> Vec<u32> {
        self.hidden
            .get("skip")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_u64).filter_map(|v| u32::try_from(v).ok()).collect())
            .unwrap_or_default()
    }

    /// A line was clicked: left out if it was in, back in if it was out.
    pub fn toggle_line(&mut self, id: u32) {
        let mut skip = self.skipped();
        match skip.iter().position(|s| *s == id) {
            Some(i) => {
                skip.remove(i);
            }
            None => skip.push(id),
        }
        if skip.is_empty() {
            self.hidden.remove("skip");
        } else {
            self.hidden.insert("skip".into(), json!(skip));
        }
    }

    /// For a profile: the table angle, its shape's lines where the wire meets them (each with
    /// whether it is left out), and whether lines are being picked.
    pub fn line_pick(&self, session: &Session) -> Option<crate::viewport::LinePick> {
        if self.kind != CutKind::Profile || !self.picking_lines {
            return None;
        }
        let p = self.params(session).ok()?;
        let (b, lines) = CosmicCut_engine::cut::profile_lines(&session.job.context(), &p).ok()?;
        let skip = self.skipped();
        Some((b, lines.into_iter().map(|(id, pts)| (id, pts, skip.contains(&id))).collect(), self.picking_lines))
    }

    /// Leave out lines of the sketch (clicked in the viewport), and how many are out.
    fn line_buttons(&mut self, ui: &mut Ui, session: &Session, tokens: &Tokens) {
        let is_sketch = self
            .params(session)
            .ok()
            .and_then(|p| p.get("shape").and_then(Value::as_str).map(str::to_owned))
            .and_then(|id| session.job.shape(&id).map(|s| s.sketch.is_some()))
            .unwrap_or(false);
        if !is_sketch {
            return;
        }
        let out = self.skipped().len();
        ui.horizontal(|ui| {
            let on = self.picking_lines;
            let text = if on { "Done" } else { "Leave out lines" };
            let button = egui::Button::new(RichText::new(text).color(if on { tokens.on_accent } else { tokens.text }))
                .fill(if on { tokens.accent } else { tokens.raised_bg });
            if ui
                .add(button)
                .on_hover_text(
                    "Click lines of the shape in the viewport to leave them out of this cut, or put them back",
                )
                .clicked()
            {
                self.picking_lines = !on;
                self.picking_start = false;
            }
            if ui.add_enabled(out > 0, egui::Button::new("Put all back")).clicked() {
                self.hidden.remove("skip");
            }
        });
        if out > 0 {
            ui.label(
                RichText::new(format!(
                    "{out} line{} left out. What is left is cut as it is, from the end nearest the start point (Path).",
                    if out == 1 { "" } else { "s" }
                ))
                .color(tokens.accent),
            );
        }
        ui.add_space(4.0);
    }

    /// Pick the faces in the viewport, and how many there are.
    fn face_buttons(&mut self, ui: &mut Ui, tokens: &Tokens) {
        let count = self.faces().len();
        ui.label(
            RichText::new(match count {
                0 => "No faces selected".to_owned(),
                1 => "1 face selected".to_owned(),
                n => format!("{n} faces selected"),
            })
            .color(tokens.accent),
        );
        ui.horizontal(|ui| {
            let on = self.picking_faces;
            let text = if on { "Done selecting" } else { "Select faces" };
            let button = egui::Button::new(RichText::new(text).color(if on { tokens.on_accent } else { tokens.text }))
                .fill(if on { tokens.accent } else { tokens.raised_bg });
            if ui
                .add(button)
                .on_hover_text("Click faces of the model to add them, click again to take them away; Esc stops")
                .clicked()
            {
                self.picking_faces = !on;
            }
            if ui.add_enabled(count > 0, egui::Button::new("Clear")).on_hover_text("Take all the faces away").clicked()
            {
                self.hidden.insert("faces".into(), json!([]));
            }
        });
        ui.add_space(4.0);
    }

    fn text_of(&self, name: &str) -> Option<&String> {
        self.fields.iter().find(|(p, _)| p.name == name).and_then(|(_, f)| match f {
            Field::Text(t) => Some(t),
            _ => None,
        })
    }

    fn set_text(&mut self, name: &str, text: String) {
        if let Some((_, Field::Text(t))) = self.fields.iter_mut().find(|(p, _)| p.name == name) {
            *t = text;
        }
    }

    /// The table angle the cut (first) runs at, degrees: the plane the start is picked on.
    pub fn start_angle(&self) -> f64 {
        self.text_of("angle").and_then(|t| t.trim().parse::<f64>().ok()).filter(|a| a.is_finite()).unwrap_or(0.0)
    }

    /// The start point was clicked (`u`, `h` in mm), or the pick was cancelled.
    pub fn take_start(&mut self, at: Option<glam::DVec2>) {
        if let Some(p) = at {
            self.set_text("start_x", self.units.plain(p.x));
            self.set_text("start_y", self.units.plain(p.y));
        }
        self.picking_start = false;
    }

    /// For model cuts: how many profiles the spacing makes, and how far apart.
    fn rotations_summary(&self, ui: &mut Ui, tokens: &Tokens) {
        let mut p = Map::new();
        for (param, field) in &self.fields {
            match (param.name, field) {
                ("spacing", _) => {
                    p.insert("spacing".into(), json!(self.choice("spacing").unwrap_or("divide")));
                }
                ("divisions" | "degrees", Field::Text(t)) => {
                    if let Ok(v) = t.trim().parse::<f64>() {
                        p.insert(param.name.into(), json!(v));
                    }
                }
                _ => {}
            }
        }
        let text = match CosmicCut_engine::cut::model_angles(&Value::Object(p)) {
            Ok(a) if a.len() > 1 => {
                format!("{} profiles, {}° apart: at {}°", a.len(), trim(a[1] - a[0]), list(&a))
            }
            Ok(_) => "1 profile, at 0°".into(),
            Err(e) => e.to_string(),
        };
        ui.label(RichText::new(text).color(tokens.accent));
    }

    fn footer(&mut self, ui: &mut Ui, session: &Session, tokens: &Tokens, ok: &mut bool, keep: &mut bool) {
        let units = self.units;
        Container::new(Surface::Card).show(ui, tokens, |ui| {
            ui.set_width(ui.available_width());
            match &self.preview {
                Some(Ok(b)) => {
                    let (cut, air, secs) = b.path.stats(session.job.machine.rapid, session.job.machine.b_rate);
                    egui::Grid::new("cut-stats").num_columns(4).spacing([18.0, 4.0]).show(ui, |ui| {
                        let stat = |ui: &mut Ui, k: &str, v: String| {
                            ui.label(RichText::new(k).color(tokens.text_muted));
                            ui.label(RichText::new(v).strong());
                        };
                        stat(
                            ui,
                            if self.kind == CutKind::Swarf { "Passes" } else { "Profiles" },
                            b.keeps.len().to_string(),
                        );
                        stat(ui, "Time", fmt_time(secs));
                        ui.end_row();
                        stat(ui, "Cutting", units.fmt_len(cut));
                        stat(ui, "Travel", units.fmt_len(air));
                        ui.end_row();
                    });
                    if b.release.is_some() {
                        ui.label(
                            RichText::new("Parted off across the bottom at the end of the program")
                                .color(tokens.text_muted),
                        );
                    }
                    for w in &b.warnings {
                        ui.add(egui::Label::new(RichText::new(format!("⚠ {w}")).color(tokens.warning)).wrap());
                    }
                }
                Some(Err(e)) => {
                    ui.add(egui::Label::new(RichText::new(e.to_string()).color(tokens.error)).wrap());
                }
                None => {}
            }
            if let Some(e) = &self.error {
                ui.add(egui::Label::new(RichText::new(e).color(tokens.error)).wrap());
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let can = matches!(self.preview, Some(Ok(_)));
            let label = if self.editing.is_some() { "Update cut" } else { "Create cut" };
            let primary = egui::Button::new(RichText::new(label).strong().color(tokens.on_accent))
                .fill(tokens.accent)
                .min_size(vec2(120.0, 30.0));
            *ok = ui.add_enabled(can, primary).clicked();
            if ui.add(egui::Button::new("Cancel").min_size(vec2(90.0, 30.0))).clicked() {
                *keep = false;
            }
        });
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// "0, 22.5, 45 … 157.5".
fn list(a: &[f64]) -> String {
    if a.len() <= 5 {
        a.iter().map(|v| trim(*v)).collect::<Vec<_>>().join(", ")
    } else {
        format!("{}, {}, {} … {}", trim(a[0]), trim(a[1]), trim(a[2]), trim(a[a.len() - 1]))
    }
}

/// One option: its name, its control and unit on a line, what it does under them.
fn row(
    ui: &mut Ui,
    tokens: &Tokens,
    p: &Param,
    field: &mut Field,
    shapes: &[String],
    units: DisplayUnits,
    widths: Widths,
) {
    // A toggle of long choices goes under its label; everything else beside it.
    let wide =
        matches!((&*field, p.kind), (Field::Choice(_), ParamKind::Choice(c)) if c.len() <= 3) && widths.label < 180.0;
    ui.horizontal_wrapped(|ui| {
        ui.allocate_ui_with_layout(vec2(widths.label, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(widths.label);
            ui.label(RichText::new(label(p.name)).strong());
        });
        if wide {
            ui.end_row();
        }
        edit(ui, p, field, shapes, widths);
        let unit = suffix(p, units);
        if !unit.is_empty() {
            ui.label(RichText::new(unit).color(tokens.text_muted));
        }
    });
    ui.add(egui::Label::new(RichText::new(p.doc).size(11.5).color(tokens.text_muted)).wrap());
    ui.add_space(6.0);
}

fn suffix(p: &Param, units: DisplayUnits) -> &'static str {
    match p.kind {
        ParamKind::Length { .. } => units.length(),
        ParamKind::Rate { .. } => units.rate(),
        _ if matches!(p.name, "angle" | "step" | "sweep" | "rotate" | "front_rotate" | "side_rotate" | "degrees") => {
            "°"
        }
        _ => "",
    }
}

fn edit(ui: &mut Ui, p: &Param, field: &mut Field, shapes: &[String], widths: Widths) {
    match field {
        Field::Text(t) => {
            ui.add(egui::TextEdit::singleline(t).desired_width(widths.input).hint_text(if p.required {
                ""
            } else {
                "default"
            }));
        }
        Field::Bool(b) => {
            ui.checkbox(b, if *b { "Yes" } else { "No" });
        }
        Field::Choice(i) => {
            let choices: &[&str] = match p.kind {
                ParamKind::Choice(c) => c,
                _ => &[],
            };
            // Two or three choices read best as a toggle; more as a list.
            if choices.len() <= 3 {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (k, c) in choices.iter().enumerate() {
                        ui.selectable_value(i, k, choice_label(p.name, c));
                    }
                });
            } else {
                let text = choices.get(*i).map_or(String::new(), |c| choice_label(p.name, c));
                egui::ComboBox::from_id_salt(("cut-field", p.name))
                    .selected_text(text)
                    .width(widths.input + 60.0)
                    .show_ui(ui, |ui| {
                        for (k, c) in choices.iter().enumerate() {
                            ui.selectable_value(i, k, choice_label(p.name, c));
                        }
                    });
            }
        }
        Field::Shape(i) => {
            let text = i.and_then(|k| shapes.get(k)).cloned().unwrap_or_else(|| "(no shapes yet)".into());
            egui::ComboBox::from_id_salt(("cut-field", p.name)).selected_text(text).width(widths.input + 60.0).show_ui(
                ui,
                |ui| {
                    for (k, name) in shapes.iter().enumerate() {
                        ui.selectable_value(i, Some(k), name);
                    }
                },
            );
        }
    }
}
