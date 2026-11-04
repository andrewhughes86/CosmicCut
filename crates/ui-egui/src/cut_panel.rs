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
        // The label column, then the control filling the rest less a unit.
        let label = (w * 0.42).clamp(110.0, 190.0);
        Self { label, input: (w - label - 46.0).clamp(80.0, 220.0) }
    }
}

/// Parameters the panel carries without showing.
const HIDDEN: &[&str] = &["id", "name", "model", "models", "faces", "parts", "ignore", "skip", "bottoms", "chains"];

/// The group a parameter shows in.
fn section(name: &str) -> &'static str {
    match name {
        "source" | "shape" | "front" | "side" | "axis" | "bridge" => "Geometry",
        "anchor" | "x" | "y" | "scale" | "rotate" | "mirror" => "Placement",
        n if n.starts_with("front_") || n.starts_with("side_") => "Placement",
        "spacing" | "divisions" | "degrees" => "Rotations",
        "height_step" | "tolerance" => "Faces",
        "angle" | "copies" | "step" | "sweep" => "Table",
        "thickness" | "skim" | "count" | "order" => "Slabs",
        "keep" | "lead" | "direction" | "alternate" | "start_x" | "start_y" | "lead_in" | "lead_out" | "clearance" => {
            "Linking"
        }
        "bottom" | "attach" | "release" => "Bottom",
        _ => "Passes",
    }
}

const SECTIONS: &[&str] =
    &["Geometry", "Faces", "Rotations", "Table", "Slabs", "Placement", "Passes", "Linking", "Bottom"];

/// What an option is called in the panel.
fn label(name: &str) -> String {
    let named = match name {
        "shape" => "Shape",
        "source" => "Source",
        "bridge" => "Bridge along Y0",
        "lead_in" => "Lead-in length",
        "lead_out" => "Lead-out length",
        "kerf" => "Kerf width",
        "clearance" => "Clearance",
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
        "feed" => "Feedrate",
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
        ("source", "shape") => "A shape".into(),
        ("source", "models") => "Models".into(),
        (_, v) => {
            let words = v.replace('_', " ");
            let mut chars = words.chars();
            chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
        }
    }
}

/// The value a yes/no option has when it was never set.
fn bool_default(name: &str) -> bool {
    matches!(name, "alternate" | "compensate" | "release" | "bridge")
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
    /// Lines of a profile's outline are being clicked to pick chains.
    pub picking_lines: bool,
    /// The chain picked out in the panel.
    active_chain: Option<usize>,
    /// An older cut, opened here: its chains are worked out once its lines are.
    seed_chains: bool,
    /// What the outline is of (source and shapes), as last seen: when it changes the chains go.
    outline_of: Option<String>,
    /// A two-view's side is shown (its lines picked, the table turned to it), not its front.
    side_view: bool,
    /// The outline's lines and the parameters they were found from.
    lines_cache: std::cell::RefCell<Option<(String, Vec<CosmicCut_engine::cut::CutLine>)>>,
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
                    values.insert("parts".into(), json!([]));
                }
                CutKind::Model => {
                    values.insert("divisions".into(), json!(8));
                    values.insert("degrees".into(), json!(22.5));
                    // Every model, cut together: the wire cannot pass one to cut another.
                    let all: Vec<&str> = session.job.models.iter().map(|m| m.id.as_str()).collect();
                    values.insert("models".into(), json!(all));
                }
                CutKind::Profile | CutKind::TwoView if shapes.is_empty() && !session.job.models.is_empty() => {
                    // Nothing drawn yet: the models' outline.
                    let all: Vec<&str> = session.job.models.iter().map(|m| m.id.as_str()).collect();
                    values.insert("source".into(), json!("models"));
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
        // Profiles pick chains: none to start with; an older cut gets its own once its lines show.
        let seed_chains = matches!(kind, CutKind::Profile | CutKind::TwoView) && !values.contains_key("chains");
        if matches!(kind, CutKind::Profile | CutKind::TwoView) {
            values.entry("chains").or_insert_with(|| json!([]));
        }
        let seed_chains = seed_chains && cut.is_some();
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
            picking_lines: matches!(kind, CutKind::Profile | CutKind::TwoView),
            active_chain: None,
            seed_chains,
            outline_of: None,
            side_view: false,
            lines_cache: std::cell::RefCell::new(None),
        }
        .picking_faces_if_none()
    }

    /// A new swarf cut starts with its faces being picked.
    fn picking_faces_if_none(mut self) -> Self {
        self.picking_faces = self.kind == CutKind::Swarf && self.parts().iter().all(|(_, f)| f.is_empty());
        self
    }

    /// The faces a swarf cut follows, by model: its `parts`, or (an older cut) its `model` and
    /// `faces`.
    pub fn parts(&self) -> Vec<(String, Vec<usize>)> {
        let numbers = |v: Option<&Value>| -> Vec<usize> {
            v.and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_u64).filter_map(|f| usize::try_from(f).ok()).collect())
                .unwrap_or_default()
        };
        if let Some(list) = self.hidden.get("parts").and_then(Value::as_array) {
            return list
                .iter()
                .filter_map(|part| Some((part.get("model")?.as_str()?.to_owned(), numbers(part.get("faces")))))
                .collect();
        }
        match self.hidden.get("model").and_then(Value::as_str) {
            Some(m) => vec![(m.to_owned(), numbers(self.hidden.get("faces")))],
            None => Vec::new(),
        }
    }

    fn set_parts(&mut self, parts: Vec<(String, Vec<usize>)>) {
        let list: Vec<Value> =
            parts.into_iter().filter(|(_, f)| !f.is_empty()).map(|(m, f)| json!({ "model": m, "faces": f })).collect();
        self.hidden.remove("model");
        self.hidden.remove("faces");
        self.hidden.insert("parts".into(), Value::Array(list));
    }

    /// A swarf cut's faces by model, and whether more are being picked.
    pub fn face_pick(&self) -> Option<crate::viewport::FacePick> {
        (self.kind == CutKind::Swarf).then(|| (self.parts(), self.picking_faces))
    }

    /// Faces of `model` were clicked (the clicked one first): in if it was out, all out if it was in.
    pub fn toggle_faces(&mut self, model: &str, clicked: &[usize]) {
        let mut parts = self.parts();
        let Some(first) = clicked.first() else { return };
        let i = match parts.iter().position(|(m, _)| m == model) {
            Some(i) => i,
            None => {
                parts.push((model.to_owned(), Vec::new()));
                parts.len() - 1
            }
        };
        if let Some((_, faces)) = parts.get_mut(i) {
            if faces.contains(first) {
                faces.retain(|f| !clicked.contains(f));
            } else {
                let more: Vec<usize> = clicked.iter().filter(|f| !faces.contains(f)).copied().collect();
                faces.extend(more);
            }
        }
        self.set_parts(parts);
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
        let models = self.follows_models();
        let placing = matches!(name, "anchor" | "x" | "y" | "scale" | "rotate" | "mirror" | "keep" | "direction")
            || name.starts_with("front_")
            || name.starts_with("side_");
        let chains = self.by_chains();
        match name {
            // Picking chains says where the wire goes, which way, and what of the bottom it cuts.
            "lead" | "start_x" | "start_y" | "direction" | "alternate" | "keep" | "bottom" | "attach" | "release"
                if chains =>
            {
                false
            }
            "lead_in" | "lead_out" => chains,
            "shape" | "front" | "side" => !models,
            _ if placing && models => false,
            "bridge" => models,
            "attach" | "release" if models => false,
            "divisions" => self.choice("spacing") != Some("degrees"),
            "degrees" => self.choice("spacing") == Some("degrees"),
            "attach" | "release" => self.choice("bottom") != Some("through"),
            _ => true,
        }
    }

    /// A profile or two-view following models' outlines.
    fn follows_models(&self) -> bool {
        self.choice("source") == Some("models")
    }

    /// The models cut together: the list, else the one an older cut names, else every model.
    fn chosen_models(&self, session: &Session) -> Vec<String> {
        match (self.hidden.get("models"), self.hidden.get("model")) {
            (Some(Value::Array(a)), _) if !a.is_empty() => {
                a.iter().filter_map(Value::as_str).map(str::to_owned).collect()
            }
            (_, Some(Value::String(m))) => vec![m.clone()],
            _ => session.job.models.iter().map(|m| m.id.clone()).collect(),
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
        if self.by_chains() {
            let of = self.params(session).ok().map(|p| {
                ["source", "shape", "front", "side"]
                    .map(|k| p.get(k).map(Value::to_string).unwrap_or_default())
                    .join("|")
            });
            if self.outline_of.is_some() && of != self.outline_of {
                self.active_chain = None;
                self.set_chains(Vec::new());
            }
            self.outline_of = of;
        }
        self.refresh_preview(session);
        let mut keep = true;
        let mut ok = false;
        let units = self.units;
        let shapes: Vec<String> = session.job.shapes.iter().map(|s| s.name.clone()).collect();
        {
            ui.spacing_mut().item_spacing.y = 4.0;
            // What each setting does shows when the pointer rests on it.
            ui.style_mut().interaction.tooltip_delay = 1.0;
            self.header(ui, tokens);
            ui.add_space(4.0);
            let footer = 150.0;
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
                        let extra = match *name {
                            "Geometry" => {
                                matches!(self.kind, CutKind::Profile | CutKind::TwoView | CutKind::Model)
                                    || session.job.models.len() > 1
                            }
                            "Faces" => self.kind == CutKind::Swarf,
                            _ => false,
                        };
                        if shown.is_empty() && !extra {
                            continue;
                        }
                        // A group: a bar with its name that opens and closes it, its settings in two
                        // columns under it.
                        let id = ui.make_persistent_id(("cut-group", self.kind.id(), *name));
                        let mut state =
                            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
                        let (bar, response) =
                            ui.allocate_exact_size(vec2(ui.available_width(), 28.0), egui::Sense::click());
                        let painter = ui.painter();
                        let fill = if response.hovered() { tokens.hover_bg } else { tokens.raised_bg };
                        painter.rect_filled(bar, 4.0, fill);
                        // A chevron: down when open, right when closed.
                        let c = bar.left_center() + vec2(14.0, 0.0);
                        let open = state.is_open();
                        let pts = if open {
                            vec![c + vec2(-4.5, -2.5), c + vec2(4.5, -2.5), c + vec2(0.0, 3.5)]
                        } else {
                            vec![c + vec2(-2.5, -4.5), c + vec2(3.5, 0.0), c + vec2(-2.5, 4.5)]
                        };
                        painter.add(egui::Shape::convex_polygon(pts, tokens.text_muted, egui::Stroke::NONE));
                        painter.text(
                            bar.left_center() + vec2(28.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            *name,
                            egui::FontId::proportional(14.0),
                            tokens.text,
                        );
                        if response.clicked() {
                            state.toggle(ui);
                        }
                        state.show_body_unindented(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add_space(4.0);
                            if *name == "Faces" {
                                self.face_buttons(ui, tokens);
                            }
                            if *name == "Linking"
                                && self.applies("start_x")
                                && self.fields.iter().any(|(p, _)| p.name == "start_x")
                            {
                                self.start_buttons(ui, tokens);
                            }
                            let widths = Widths::of(ui);
                            egui::Grid::new(("cut-grid", self.kind.id(), *name))
                                .num_columns(2)
                                .spacing([10.0, 9.0])
                                .min_col_width(widths.label)
                                .show(ui, |ui| {
                                    for i in shown {
                                        if let Some((p, field)) = self.fields.get_mut(i) {
                                            row(ui, tokens, p, field, &shapes, units, widths);
                                            ui.end_row();
                                        }
                                    }
                                    if *name == "Geometry" {
                                        self.model_boxes(ui, session, tokens);
                                        self.ignore_boxes(ui, session, tokens);
                                    }
                                });
                            if *name == "Geometry" && matches!(self.kind, CutKind::Profile | CutKind::TwoView) {
                                ui.add_space(6.0);
                                self.chain_box(ui, session, tokens);
                            }
                            if *name == "Rotations" {
                                self.rotations_summary(ui, tokens);
                            }
                            ui.add_space(6.0);
                        });
                        ui.add_space(4.0);
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
                let title = ui.label(RichText::new(format!("{verb} {what}")).strong().size(18.0).color(tokens.text));
                if let Some(spec) = CosmicCut_engine::cmd::find(self.kind.command()) {
                    title.on_hover_text(spec.tooltip);
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
        let outline = matches!(self.kind, CutKind::Profile | CutKind::TwoView) && self.follows_models();
        if !(self.kind == CutKind::Model || outline) || session.job.models.len() < 2 {
            return;
        }
        let mut chosen = self.chosen_models(session);
        ui.label(RichText::new("Models").color(tokens.text_muted)).on_hover_text(
            "The models cut together. The wire runs right across the foam, so a model left out must stand clear of it.",
        );
        let mut changed = false;
        ui.vertical(|ui| {
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
        });
        ui.end_row();
        if changed {
            self.hidden.remove("model");
            self.hidden.insert("models".into(), json!(chosen));
            // The outline changes, and with it the lines: the chains start again.
            if outline {
                self.hidden.remove("skip");
                self.active_chain = None;
                if self.by_chains() {
                    self.set_chains(Vec::new());
                }
            }
        }
    }

    /// The models this cut does not make that will be off the table when it runs: the wire may
    /// pass through them.
    fn ignore_boxes(&mut self, ui: &mut Ui, session: &Session, tokens: &Tokens) {
        let own: Vec<String> = match self.kind {
            CutKind::Model => self.chosen_models(session),
            CutKind::Profile | CutKind::TwoView if self.follows_models() => self.chosen_models(session),
            CutKind::Swarf => self.parts().into_iter().map(|(m, _)| m).collect(),
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
        ui.label(RichText::new("Off the table").color(tokens.text_muted)).on_hover_text(
            "Models cut and taken off before this cut runs: the wire may pass through them. The cut fails if the wire \
             would cut any other model. Operation settings can say this for all of an operation's cuts.",
        );
        let mut changed = false;
        ui.vertical(|ui| {
            for m in others {
                if by_op.contains(&m.id) {
                    let mut on = true;
                    ui.add_enabled(false, egui::Checkbox::new(&mut on, format!("{} (by its operation)", m.name)));
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
        });
        ui.end_row();
        if changed {
            if ignored.is_empty() {
                self.hidden.remove("ignore");
            } else {
                self.hidden.insert("ignore".into(), json!(ignored));
            }
        }
    }

    /// A profile or two-view whose lines are picked as chains.
    fn by_chains(&self) -> bool {
        matches!(self.kind, CutKind::Profile | CutKind::TwoView) && self.hidden.contains_key("chains")
    }

    /// The chains picked, in order: their lines and whether each is cut the other way.
    fn chains(&self) -> Vec<(Vec<u32>, bool)> {
        CosmicCut_engine::outline::chains(&Value::Object(self.hidden.clone()))
            .into_iter()
            .map(|c| (c.lines, c.reverse))
            .collect()
    }

    fn set_chains(&mut self, chains: Vec<(Vec<u32>, bool)>) {
        let list: Vec<Value> =
            chains.into_iter().map(|(lines, reverse)| json!({ "lines": lines, "reverse": reverse })).collect();
        self.hidden.insert("chains".into(), Value::Array(list));
        self.active_chain = self.active_chain.filter(|k| *k < self.chains().len());
    }

    /// A line was clicked. One in a chain comes out of it (a chain broken in the middle becomes
    /// two). Otherwise it joins the chain being picked (the one picked out, else the last) when it
    /// meets either end of it, else starts a new chain. With `whole`, its whole run round the
    /// outline goes in as a chain of its own.
    pub fn toggle_line(&mut self, id: u32, whole: bool) {
        let mut chains = self.chains();
        if let Some(k) = chains.iter().position(|(lines, _)| lines.contains(&id)) {
            let (lines, reverse) = chains.remove(k);
            let at = lines.iter().position(|l| *l == id).unwrap_or(0);
            let (before, after) = (lines[..at].to_vec(), lines[at + 1..].to_vec());
            let mut put = k;
            for part in [before, after].into_iter().filter(|p| !p.is_empty()) {
                chains.insert(put, (part, reverse));
                put += 1;
            }
            self.active_chain = (put > k).then_some(k);
            self.set_chains(chains);
            return;
        }
        let lines = self.lines_cache.borrow().as_ref().map(|(_, l)| l.clone()).unwrap_or_default();
        if whole {
            let picked: Vec<u32> = CosmicCut_engine::cut::chain_of(&lines, id)
                .into_iter()
                .filter(|l| !chains.iter().any(|(c, _)| c.contains(l)))
                .collect();
            if !picked.is_empty() {
                chains.push((picked, false));
                self.active_chain = Some(chains.len() - 1);
                self.set_chains(chains);
            }
            return;
        }
        let points = |l: u32| lines.iter().find(|x| x.id == l).map(|x| x.points.clone()).unwrap_or_default();
        let new = points(id);
        let (Some(&a), Some(&z)) = (new.first(), new.last()) else { return };
        let meets = |q: glam::DVec2| q.distance_squared(a) < 1.0e-8 || q.distance_squared(z) < 1.0e-8;
        // The chain being picked: its run's two ends.
        let target = self.active_chain.or(chains.len().checked_sub(1));
        let joined = target.and_then(|k| {
            let (ids, _) = chains.get(k)?;
            // Only a chain in the same view (pass) carries on.
            let pass = |l: u32| l / CosmicCut_engine::outline::PASS;
            if ids.first().map(|l| pass(*l)) != Some(pass(id)) {
                return None;
            }
            let pieces: Vec<Vec<glam::DVec2>> = ids.iter().map(|l| points(*l)).collect();
            let refs: Vec<&[glam::DVec2]> = pieces.iter().map(Vec::as_slice).collect();
            let runs = CosmicCut_engine::outline::join(&refs);
            let [run] = runs.as_slice() else { return None };
            let (first, last) = (*run.first()?, *run.last()?);
            // A closed chain has no free end.
            if first.distance_squared(last) < 1.0e-8 {
                return None;
            }
            if meets(last) {
                Some((k, false))
            } else if meets(first) {
                Some((k, true))
            } else {
                None
            }
        });
        match joined {
            Some((k, at_start)) => {
                if let Some((ids, _)) = chains.get_mut(k) {
                    if at_start {
                        ids.insert(0, id);
                    } else {
                        ids.push(id);
                    }
                }
                self.active_chain = Some(k);
            }
            None => {
                chains.push((vec![id], false));
                self.active_chain = Some(chains.len() - 1);
            }
        }
        self.set_chains(chains);
    }

    /// Cuts chain `k` the other way.
    pub fn flip_chain(&mut self, k: usize) {
        let mut chains = self.chains();
        if let Some(c) = chains.get_mut(k) {
            c.1 = !c.1;
        }
        self.active_chain = Some(k);
        self.set_chains(chains);
    }

    /// Takes away the chain picked out in the panel (the Delete key).
    pub fn delete_selected(&mut self) {
        if let Some(k) = self.active_chain.take() {
            let mut chains = self.chains();
            if k < chains.len() {
                chains.remove(k);
            }
            self.set_chains(chains);
        }
    }

    /// The outline's lines, every pass, if the cut has lines to pick.
    fn lines(&self, session: &Session) -> Option<Vec<CosmicCut_engine::cut::CutLine>> {
        if !self.by_chains() {
            return None;
        }
        let mut p = self.params(session).ok()?;
        // The chains picked do not change the lines.
        if let Some(o) = p.as_object_mut() {
            o.insert("chains".into(), json!([]));
        }
        let key = format!("{p}{}", session.job.revision);
        let mut cache = self.lines_cache.borrow_mut();
        if let Some((k, lines)) = cache.as_ref()
            && *k == key
        {
            return (!lines.is_empty()).then(|| lines.clone());
        }
        let lines = CosmicCut_engine::cut::cut_lines(&session.job.context(), self.kind, &p).unwrap_or_default();
        *cache = Some((key, lines.clone()));
        (!lines.is_empty()).then_some(lines)
    }

    /// An older cut opened here: every chain but the models' bottoms, as it cut them.
    fn seed(&mut self, session: &Session) {
        if !self.seed_chains {
            return;
        }
        let Some(lines) = self.lines(session) else { return };
        self.seed_chains = false;
        let mut chains: Vec<(Vec<u32>, bool)> = Vec::new();
        for l in lines.iter().filter(|l| !l.bottom) {
            if chains.iter().any(|(c, _)| c.contains(&l.id)) {
                continue;
            }
            let chain = CosmicCut_engine::cut::chain_of(&lines, l.id);
            if !chain.is_empty() {
                chains.push((chain, false));
            }
        }
        self.hidden.remove("skip");
        self.hidden.remove("bottoms");
        self.set_chains(chains);
    }

    /// The outline for the viewport: each line with its chain, and each chain's arrow.
    pub fn line_pick(&self, session: &Session) -> Option<crate::viewport::LinePick> {
        use crate::viewport::LineState;
        let pass = self.shown_pass();
        let in_view = |id: u32| pass.is_none_or(|p| id / CosmicCut_engine::outline::PASS == p);
        let lines: Vec<CosmicCut_engine::cut::CutLine> =
            self.lines(session)?.into_iter().filter(|l| in_view(l.id)).collect();
        let chains = self.chains();
        let state = |l: &CosmicCut_engine::cut::CutLine| match chains.iter().position(|(c, _)| c.contains(&l.id)) {
            Some(k) => LineState::Chosen(k),
            None if l.bottom => LineState::Bottom,
            None => LineState::Free,
        };
        let mut arrows = Vec::new();
        for (k, (ids, reverse)) in chains.iter().enumerate() {
            if !ids.first().is_some_and(|id| in_view(*id)) {
                continue;
            }
            let found: Vec<&CosmicCut_engine::cut::CutLine> =
                ids.iter().filter_map(|id| lines.iter().find(|l| l.id == *id)).collect();
            let pieces: Vec<&[glam::DVec2]> = found.iter().map(|l| l.points.as_slice()).collect();
            let (Some(first), Some(mut run)) =
                (found.first(), CosmicCut_engine::outline::join(&pieces).into_iter().next())
            else {
                continue;
            };
            if *reverse {
                run.reverse();
            }
            // Halfway along it.
            let total: f64 = run.windows(2).map(|w| w[0].distance(w[1])).sum();
            let mut gone = 0.0;
            for w in run.windows(2) {
                let d = w[0].distance(w[1]);
                if gone + d >= total / 2.0 && d > 1.0e-9 {
                    let t = (total / 2.0 - gone) / d;
                    let along = (w[1] - w[0]) / d;
                    arrows.push((k, first.b, w[0].lerp(w[1], t), along * 5.0));
                    break;
                }
                gone += d;
            }
        }
        Some(crate::viewport::LinePick {
            lines: lines.iter().map(|l| (l.id, l.b, l.points.clone(), state(l))).collect(),
            arrows,
            active: self.active_chain,
            picking: self.picking_lines,
        })
    }

    /// The chains: picking them, and each one's way round, in order.
    fn chain_box(&mut self, ui: &mut Ui, session: &Session, tokens: &Tokens) {
        if !self.by_chains() {
            return;
        }
        self.seed(session);
        let chains = self.chains();
        // A two-view: the view whose lines are shown and picked, the table turned to it.
        if self.kind == CutKind::TwoView {
            ui.horizontal(|ui| {
                ui.label(RichText::new("View").color(tokens.text_muted)).on_hover_text(
                    "Which view's outline to show and pick lines of: the front, at the table angle, or the side, a \
                     quarter turn on. The table turns to it.",
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let b = self.start_angle();
                    ui.selectable_value(&mut self.side_view, true, format!("Side  {}°", trim(b + 90.0)));
                    ui.selectable_value(&mut self.side_view, false, format!("Front  {}°", trim(b)));
                });
            });
            ui.add_space(4.0);
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Contours").color(tokens.text_muted)).on_hover_text(
                "The chains of the outline to cut, in this order. Click lines in the viewport one at a time: a line \
                 that meets the end of the chain being picked carries it on, any other starts a new chain; click a \
                 picked line to take it out. Alt-click picks a line's whole run at once. Click a chain's arrow, or \
                 Flip here, to cut it the other way.",
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!chains.is_empty(), egui::Button::new("Clear all"))
                    .on_hover_text("Take every chain away")
                    .clicked()
                {
                    self.active_chain = None;
                    self.set_chains(Vec::new());
                }
            });
        });
        // The pick box: lit while clicks in the viewport pick lines.
        let on = self.picking_lines;
        let text = match chains.len() {
            0 => "Click lines in the viewport".to_owned(),
            1 => "1 chain selected".to_owned(),
            n => format!("{n} chains selected"),
        };
        let fill = if on { crate::theme::PICKED } else { tokens.raised_bg };
        let ink = if on { egui::Color32::WHITE } else { tokens.text };
        let button = egui::Button::new(RichText::new(text).color(ink))
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, if on { crate::theme::PICKED_ACTIVE } else { tokens.border }))
            .min_size(vec2(ui.available_width(), 30.0));
        if ui.add(button).on_hover_text("Select chains in the viewport (Esc stops)").clicked() {
            self.picking_lines = !on;
            self.picking_start = false;
        }
        let mut flip = None;
        let mut remove = None;
        if !chains.is_empty() {
            ui.add_space(4.0);
            Container::new(Surface::Card).show(ui, tokens, |ui| {
                ui.set_width(ui.available_width());
                for (k, (lines, reverse)) in chains.iter().enumerate() {
                    let active = self.active_chain == Some(k);
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                        let colour = if active { crate::theme::PICKED_ACTIVE } else { crate::theme::PICKED };
                        ui.painter().circle_filled(dot.center(), 4.5, colour);
                        let n = lines.len();
                        let way = if *reverse { "  ·  reversed" } else { "" };
                        let view = match lines.first().map(|id| id / CosmicCut_engine::outline::PASS) {
                            Some(p) if self.kind == CutKind::TwoView => {
                                if p == 0 {
                                    "Front  ·  "
                                } else {
                                    "Side  ·  "
                                }
                            }
                            _ => "",
                        };
                        let label = format!("{view}Chain {}  ·  {n} line{}{way}", k + 1, if n == 1 { "" } else { "s" });
                        if ui.selectable_label(active, label).clicked() {
                            self.active_chain = if active { None } else { Some(k) };
                            // A two-view shows the view the chain is in.
                            if let Some(p) = lines.first().map(|id| id / CosmicCut_engine::outline::PASS)
                                && self.kind == CutKind::TwoView
                            {
                                self.side_view = p == 1;
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Remove").on_hover_text("Take it away").clicked() {
                                remove = Some(k);
                            }
                            if ui.small_button("Flip").on_hover_text("Cut it the other way").clicked() {
                                flip = Some(k);
                            }
                        });
                    });
                }
            });
        }
        if let Some(k) = flip {
            self.flip_chain(k);
        }
        if let Some(k) = remove {
            self.active_chain = Some(k);
            self.delete_selected();
        }
    }

    /// Pick the faces in the viewport, and how many there are.
    fn face_buttons(&mut self, ui: &mut Ui, tokens: &Tokens) {
        let parts = self.parts();
        let count: usize = parts.iter().map(|(_, f)| f.len()).sum();
        let models = parts.iter().filter(|(_, f)| !f.is_empty()).count();
        ui.label(
            RichText::new(match (count, models) {
                (0, _) => "No faces selected".to_owned(),
                (1, _) => "1 face selected".to_owned(),
                (n, 1) => format!("{n} faces selected"),
                (n, m) => format!("{n} faces selected on {m} models"),
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
                .on_hover_text("Click faces of any model to add them, click again to take them away; Esc stops")
                .clicked()
            {
                self.picking_faces = !on;
            }
            if ui.add_enabled(count > 0, egui::Button::new("Clear")).on_hover_text("Take all the faces away").clicked()
            {
                self.set_parts(Vec::new());
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

    /// The table angle to show the cut at: a two-view's side a quarter turn on from its front.
    pub fn view_angle(&self) -> f64 {
        self.start_angle() + if self.kind == CutKind::TwoView && self.side_view { 90.0 } else { 0.0 }
    }

    /// The pass whose lines are shown and picked: a two-view's front (0) or side (1); every pass
    /// of a profile.
    fn shown_pass(&self) -> Option<u32> {
        (self.kind == CutKind::TwoView).then_some(u32::from(self.side_view))
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
                // Nothing picked yet is no error: say what to do.
                Some(Err(_)) if self.by_chains() && self.chains().is_empty() => {
                    ui.label(RichText::new("Select contours in the viewport").color(tokens.text_muted));
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
/// One setting as two grid cells: its name (what it does shows on resting the pointer on it),
/// then its control and unit.
fn row(
    ui: &mut Ui,
    tokens: &Tokens,
    p: &Param,
    field: &mut Field,
    shapes: &[String],
    units: DisplayUnits,
    widths: Widths,
) {
    ui.label(RichText::new(label(p.name)).color(tokens.text_muted)).on_hover_text(p.doc);
    ui.horizontal(|ui| {
        edit(ui, p, field, shapes, widths);
        let unit = suffix(p, units);
        if !unit.is_empty() {
            ui.label(RichText::new(unit).color(tokens.text_muted));
        }
    });
}

/// What a blank field stands for.
fn hint(name: &str) -> &'static str {
    match name {
        "angle" | "rotate" | "front_rotate" | "side_rotate" | "lead_in" | "lead_out" | "skim" | "attach" => "0",
        "copies" | "scale" => "1",
        "step" => "even split",
        "x" | "front_x" | "side_x" => "middle",
        "y" | "front_y" | "side_y" => "middle",
        "start_x" | "start_y" => "lead side",
        "count" => "as many as fit",
        "divisions" => "8",
        "degrees" => "22.5",
        "height_step" => "2 mm",
        "tolerance" => "0.25 mm",
        "sweep" => "360",
        _ => "machine's",
    }
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
                hint(p.name)
            }));
        }
        Field::Bool(b) => {
            ui.checkbox(b, "");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_pick_one_line_at_a_time_and_carry_the_chain_on() {
        let mut s = Session::new();
        let shape = s.execute("shape.rectangle", &json!({ "width": 100.0, "height": 60.0 })).unwrap_or_default();
        let mut panel = CutPanel::new(&s, CutKind::Profile, None, &json!({ "shape": shape["id"] }));
        let ids: Vec<u32> = panel.line_pick(&s).map(|p| p.lines.iter().map(|l| l.0).collect()).unwrap_or_default();
        assert_eq!(ids.len(), 4);
        // Round the rectangle: each line meets the next.
        panel.toggle_line(ids[0], false);
        panel.toggle_line(ids[1], false);
        assert_eq!(panel.chains(), vec![(vec![ids[0], ids[1]], false)]);
        // One meeting the chain's start carries it on backwards.
        panel.toggle_line(ids[3], false);
        assert_eq!(panel.chains(), vec![(vec![ids[3], ids[0], ids[1]], false)]);
        // Taking out a line in the middle leaves two chains.
        panel.toggle_line(ids[0], false);
        assert_eq!(panel.chains(), vec![(vec![ids[3]], false), (vec![ids[1]], false)]);
        // Alt: the whole run at once, as a chain of its own (less lines already picked).
        panel.toggle_line(ids[2], true);
        assert_eq!(panel.chains().len(), 3);
    }

    #[test]
    fn a_two_view_shows_one_view_at_a_time_and_keeps_its_chains_apart() {
        let mut s = Session::new();
        let front = s.execute("shape.rectangle", &json!({ "width": 100.0, "height": 60.0 })).unwrap_or_default();
        let side = s.execute("shape.rectangle", &json!({ "width": 80.0, "height": 60.0 })).unwrap_or_default();
        let given = json!({ "front": front["id"], "side": side["id"] });
        let mut panel = CutPanel::new(&s, CutKind::TwoView, None, &given);
        let ids = |panel: &CutPanel, s: &Session| -> Vec<u32> {
            panel.line_pick(s).map(|p| p.lines.iter().map(|l| l.0).collect()).unwrap_or_default()
        };
        let pass = |id: u32| id / CosmicCut_engine::outline::PASS;
        let front_ids = ids(&panel, &s);
        assert!(!front_ids.is_empty() && front_ids.iter().all(|id| pass(*id) == 0));
        assert!((panel.view_angle() - 0.0).abs() < 1e-9);
        panel.toggle_line(front_ids[0], false);
        // The side: its own lines, the table a quarter turn on.
        panel.side_view = true;
        let side_ids = ids(&panel, &s);
        assert!(!side_ids.is_empty() && side_ids.iter().all(|id| pass(*id) == 1));
        assert!((panel.view_angle() - 90.0).abs() < 1e-9);
        // A side line starts a chain of its own.
        panel.toggle_line(side_ids[0], false);
        assert_eq!(panel.chains().len(), 2);
    }
}
