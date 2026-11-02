//! A dialog generated from a command's parameter schema. Interactive tools that only need
//! values typed in use it: the form collects the values, the app runs the command with
//! them, and the form stays open showing the error if the command refuses them.

use CosmicCut_engine::units::DisplayUnits;
use CosmicCut_engine::{CommandSpec, Param, ParamKind, Session};
use egui::{RichText, Ui};
use serde_json::{Map, Value, json};

use crate::Request;
use crate::theme::Tokens;
use crate::ui_kit::Window;

/// Parameters that name a shape, chosen from the job's shapes.
pub fn is_shape_param(command: &str, name: &str) -> bool {
    matches!(name, "shape" | "front" | "side") || (name == "id" && command.starts_with("shape."))
}

/// Parameters that name a post processor, chosen from the library.
pub fn is_post_param(command: &str, name: &str) -> bool {
    name == "post" && command == "machine.edit"
}

/// Parameters that name a model, chosen from the job's models.
pub fn is_model_param(command: &str, name: &str) -> bool {
    name == "model" || (name == "id" && command.starts_with("model."))
}

/// Shown in choice boxes for an optional parameter left unset.
const DEFAULT_LABEL: &str = "(default)";

enum Field {
    /// Strings and numbers, typed as text; empty means unset.
    Text(String),
    Bool(Option<bool>),
    Choice(Option<usize>),
    Vec3([String; 3]),
    /// One of the job's shapes.
    Pick(Option<usize>),
    /// One of the job's models.
    ModelPick(Option<usize>),
    /// One of the library's post processors.
    PostPick(Option<usize>),
}

pub struct ParamForm {
    spec: &'static CommandSpec,
    fields: Vec<(&'static Param, Field)>,
    /// (id, name) of what pickers choose from: the job's shapes.
    options: Vec<(String, String)>,
    /// (id, name) of the job's models.
    models: Vec<(String, String)>,
    /// (name, name) of the library's post processors.
    posts: Vec<(String, String)>,
    /// Lengths and feeds are typed in these units and sent in millimetres.
    units: DisplayUnits,
    error: Option<String>,
    open: bool,
}

impl ParamForm {
    /// A form for `spec`, starting from what the job has and then from `given`.
    pub fn new(spec: &'static CommandSpec, session: &Session, given: &Value) -> Self {
        let options: Vec<(String, String)> =
            session.job.shapes.iter().map(|s| (s.id.clone(), s.name.clone())).collect();
        let models: Vec<(String, String)> = session.job.models.iter().map(|m| (m.id.clone(), m.name.clone())).collect();
        let posts: Vec<(String, String)> =
            session.library.posts.iter().map(|p| (p.name.clone(), p.name.clone())).collect();
        let units = session.prefs.units;
        let mut initial = prefill(spec, session);
        if let Value::Object(g) = given {
            for (k, v) in g {
                initial.insert(k.clone(), v.clone());
            }
        }
        let picked = |p: &Param| {
            let want = initial.get(p.name).and_then(Value::as_str);
            want.and_then(|w| options.iter().position(|(id, name)| id == w || name == w))
                .or_else(|| session.selected_shape().and_then(|s| options.iter().position(|(id, _)| *id == s.id)))
                .or(if options.is_empty() { None } else { Some(0) })
        };
        let picked_model = |p: &Param| {
            let want = initial.get(p.name).and_then(Value::as_str);
            want.and_then(|w| models.iter().position(|(id, name)| id == w || name == w))
                .or_else(|| session.selected_model().and_then(|m| models.iter().position(|(id, _)| *id == m.id)))
                .or(if models.is_empty() { None } else { Some(0) })
        };
        let fields = spec
            .params
            .iter()
            .map(|p| {
                let field = match p.kind {
                    _ if is_shape_param(spec.id, p.name) => Field::Pick(picked(p)),
                    _ if is_model_param(spec.id, p.name) => Field::ModelPick(picked_model(p)),
                    _ if is_post_param(spec.id, p.name) => Field::PostPick(
                        initial
                            .get(p.name)
                            .and_then(Value::as_str)
                            .and_then(|w| posts.iter().position(|(n, _)| n == w))
                            .or((!posts.is_empty()).then_some(0)),
                    ),
                    _ if initial.contains_key(p.name) => initial_field(p, initial.get(p.name), units),
                    ParamKind::Bool => Field::Bool(p.required.then_some(false)),
                    ParamKind::Choice(_) => Field::Choice(p.required.then_some(0)),
                    ParamKind::Vec3 if p.required => Field::Vec3(["0".into(), "0".into(), "0".into()]),
                    ParamKind::Vec3 => Field::Vec3(Default::default()),
                    ParamKind::Number { .. } | ParamKind::Integer { .. } if p.required => Field::Text("0".into()),
                    _ => Field::Text(String::new()),
                };
                (p, field)
            })
            .collect();
        Self { spec, fields, options, models, posts, units, error: None, open: true }
    }

    /// The command failed with these values; keep the form open and say why.
    pub fn fail(&mut self, message: String) {
        self.error = Some(message);
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Draws the form. Returns the command to run when the user confirms.
    pub fn show(&mut self, ctx: &egui::Context, tokens: &Tokens) -> Option<Request> {
        let mut open = self.open;
        let mut cancel = false;
        let confirmed = Window::dialog(self.spec.label)
            .show(ctx, tokens, &mut open, |ui| {
                ui.set_min_width(360.0);
                ui.label(RichText::new(self.spec.tooltip).color(tokens.text_muted));
                ui.add_space(8.0);
                // Leave room for the title, notes and buttons on short windows.
                let max_height = (ctx.content_rect().height() - 240.0).clamp(160.0, 480.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical().max_height(max_height).show(ui, |ui| {
                            self.fields_ui(ui, tokens, "all", |_| true);
                        });
                    });
                });
                if let Some(e) = &self.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).color(tokens.error));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let ok = ui.button(RichText::new("  OK  ").strong()).clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Enter));
                    cancel = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
                    ok
                })
                .inner
            })
            .unwrap_or(false);
        self.open = open && !cancel;
        if !(confirmed && self.open) {
            return None;
        }
        self.request()
    }

    /// The command with the values typed; None, with the reason kept, when one is not valid.
    pub fn request(&mut self) -> Option<Request> {
        match self.params() {
            Ok(params) => {
                self.error = None;
                Some((self.spec.id.to_owned(), params))
            }
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The fields whose parameter names `show` picks, one row each: name, value, unit.
    pub fn fields_ui(&mut self, ui: &mut Ui, tokens: &Tokens, salt: &str, show: impl Fn(&str) -> bool) {
        let units = self.units;
        egui::Grid::new(("param-form", self.spec.id, salt))
            .num_columns(3)
            .min_col_width(110.0)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                for (param, field) in &mut self.fields {
                    if !show(param.name) {
                        continue;
                    }
                    let name = match param.name {
                        "id" if is_model_param(self.spec.id, "id") => "Model".to_owned(),
                        "id" if is_shape_param(self.spec.id, "id") => "Shape".to_owned(),
                        "post" => "Post processor".to_owned(),
                        _ => label_for(param),
                    };
                    let label = ui.label(name);
                    label.on_hover_text(param.doc);
                    let list = match field {
                        Field::ModelPick(_) => &self.models,
                        Field::PostPick(_) => &self.posts,
                        _ => &self.options,
                    };
                    edit_field(ui, self.spec.id, param, field, list);
                    ui.label(RichText::new(unit_suffix(param, units)).color(tokens.text_muted));
                    ui.end_row();
                }
            });
    }

    /// The command's parameters from the fields; unset optional fields are left out.
    fn params(&self) -> Result<Value, String> {
        let mut out = Map::new();
        for (p, field) in &self.fields {
            let value = match field {
                Field::Text(t) if t.trim().is_empty() => None,
                Field::Text(t) => Some(match p.kind {
                    ParamKind::Number { .. } => {
                        json!(t.trim().parse::<f64>().map_err(|_| format!("{} must be a number", label_for(p)))?)
                    }
                    // Typed in the user's units, sent in millimetres.
                    ParamKind::Length { .. } | ParamKind::Rate { .. } => {
                        let v = t.trim().parse::<f64>().map_err(|_| format!("{} must be a number", label_for(p)))?;
                        json!(self.units.to_mm(v))
                    }
                    ParamKind::Integer { .. } => {
                        json!(t.trim().parse::<i64>().map_err(|_| format!("{} must be a whole number", label_for(p)))?)
                    }
                    _ => json!(t),
                }),
                Field::Bool(b) => b.map(Value::Bool),
                Field::Choice(i) => match p.kind {
                    ParamKind::Choice(choices) => i.and_then(|i| choices.get(i)).map(|c| json!(c)),
                    _ => None,
                },
                Field::Vec3(v) if v.iter().all(|c| c.trim().is_empty()) => None,
                Field::Vec3(v) => {
                    let mut xyz = [0.0f64; 3];
                    for (slot, text) in xyz.iter_mut().zip(v) {
                        let text = text.trim();
                        *slot = if text.is_empty() {
                            0.0
                        } else {
                            let v: f64 = text.parse().map_err(|_| format!("{} needs three numbers", label_for(p)))?;
                            self.units.to_mm(v)
                        };
                    }
                    Some(json!(xyz))
                }
                Field::Pick(i) => i.and_then(|i| self.options.get(i)).map(|(id, _)| json!(id)),
                Field::ModelPick(i) => i.and_then(|i| self.models.get(i)).map(|(id, _)| json!(id)),
                Field::PostPick(i) => i.and_then(|i| self.posts.get(i)).map(|(id, _)| json!(id)),
            };
            match value {
                Some(v) => {
                    out.insert(p.name.to_owned(), v);
                }
                None if p.required => return Err(format!("{} is required", label_for(p))),
                None => {}
            }
        }
        Ok(Value::Object(out))
    }
}

/// Values a form opens with: the stock's or the machine's current settings, the selected shape.
fn prefill(spec: &CommandSpec, session: &Session) -> Map<String, Value> {
    let source: Option<Value> = match spec.id {
        "stock.block" | "stock.cylinder" => Some(CosmicCut_engine::cmd::stock::prefill(&session.job.stock)),
        "machine.edit" => {
            let mut m = session.job.machine.to_json();
            m["post"] = json!(session.job.machine.post_name);
            Some(m)
        }
        "shape.rectangle" => Some(json!({ "width": 254.0, "height": 127.0 })),
        "shape.circle" => Some(json!({ "diameter": 254.0 })),
        "shape.polygon" => Some(json!({ "sides": 6, "diameter": 254.0 })),
        "shape.star" => Some(json!({ "points": 5, "outer": 304.8, "inner": 127.0 })),
        "shape.airfoil" => Some(json!({ "code": "2412", "chord": 304.8 })),
        "shape.transform" => Some(json!({ "scale": 1.0, "rotate": 0.0 })),
        "model.place" => session
            .selected_model()
            .or(session.job.models.first())
            .map(|m| json!({ "rotate": m.rotate, "scale": m.scale, "lift": m.lift })),
        "model.scale" => session
            .selected_model()
            .or(session.job.models.first())
            .map(|m| json!({ "mode": "fit", "margin": 6.35, "height": m.bounds().size().z, "factor": m.scale })),
        "stock.fit" => Some(json!({ "margin": 25.4 })),
        _ => None,
    };
    let mut out = Map::new();
    if let Some(Value::Object(src)) = source {
        for p in spec.params {
            match src.get(p.name) {
                Some(v) if !v.is_null() => {
                    out.insert(p.name.to_owned(), v.clone());
                }
                _ => {}
            }
        }
    }
    out
}

/// The unit shown after a field: "in", "mm/min", or nothing.
fn unit_suffix(p: &Param, units: DisplayUnits) -> &'static str {
    match p.kind {
        ParamKind::Length { .. } | ParamKind::Vec3 => units.length(),
        ParamKind::Rate { .. } => units.rate(),
        _ if p.name == "b_rate" => "°/min",
        _ if matches!(p.name, "angle" | "step" | "rotate" | "sweep" | "rotation") => "°",
        _ if p.name == "preheat" => "s",
        _ => "",
    }
}

/// A field showing `value` (millimetres, shown in `units` for lengths and feeds).
fn initial_field(p: &Param, value: Option<&Value>, units: DisplayUnits) -> Field {
    if let (ParamKind::Length { .. } | ParamKind::Rate { .. }, Some(mm)) = (p.kind, value.and_then(Value::as_f64)) {
        return Field::Text(units.plain(mm));
    }
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_f64().map_or_else(
            || n.to_string(),
            |f| {
                let s = format!("{f:.4}");
                s.trim_end_matches('0').trim_end_matches('.').to_owned()
            },
        ),
        other => other.to_string(),
    };
    match (p.kind, value) {
        (ParamKind::Vec3, Some(Value::Array(a))) => {
            let c = |k: usize| a.get(k).and_then(Value::as_f64).map_or_else(String::new, |mm| units.plain(mm));
            Field::Vec3([c(0), c(1), c(2)])
        }
        (ParamKind::Bool, v) => Field::Bool(v.and_then(Value::as_bool)),
        (ParamKind::Choice(choices), v) => {
            Field::Choice(v.and_then(Value::as_str).and_then(|s| choices.iter().position(|c| *c == s)))
        }
        (_, Some(v)) => Field::Text(text(v)),
        (_, None) => Field::Text(String::new()),
    }
}

/// "z_axis" → "Z axis".
fn label_for(p: &Param) -> String {
    let words = p.name.replace('_', " ");
    let mut chars = words.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// Width of a field's text box or list.
const FIELD_WIDTH: f32 = 200.0;

fn edit_field(ui: &mut Ui, command: &str, p: &Param, field: &mut Field, list: &[(String, String)]) {
    let combo = |ui: &mut Ui, salt: &str, current: &mut Option<usize>, options: &[String], optional: bool| {
        let text = current.and_then(|i| options.get(i)).map_or(DEFAULT_LABEL, String::as_str).to_owned();
        egui::ComboBox::from_id_salt((command, p.name, salt)).selected_text(text).width(FIELD_WIDTH).show_ui(
            ui,
            |ui| {
                if optional {
                    ui.selectable_value(current, None, DEFAULT_LABEL);
                }
                for (i, o) in options.iter().enumerate() {
                    ui.selectable_value(current, Some(i), o);
                }
            },
        );
    };
    match field {
        Field::Text(t) => {
            // A set size: a grid sizes its columns from the frame before, and would squeeze it.
            ui.add_sized(
                egui::vec2(FIELD_WIDTH, 22.0),
                egui::TextEdit::singleline(t).hint_text(if p.required { "" } else { "default" }),
            );
        }
        Field::Bool(b) => {
            let options = ["Yes".to_owned(), "No".to_owned()];
            let mut i = b.map(|b| if b { 0 } else { 1 });
            combo(ui, "bool", &mut i, &options, !p.required);
            *b = i.map(|i| i == 0);
        }
        Field::Choice(i) => {
            let options: Vec<String> = match p.kind {
                ParamKind::Choice(c) => c.iter().map(|s| s.replace('_', " ")).collect(),
                _ => Vec::new(),
            };
            combo(ui, "choice", i, &options, !p.required);
        }
        Field::Vec3(v) => {
            ui.horizontal(|ui| {
                for (axis, text) in ["X", "Y", "Z"].iter().zip(v.iter_mut()) {
                    ui.add(egui::TextEdit::singleline(text).desired_width(56.0).hint_text(*axis));
                }
            });
        }
        Field::Pick(i) | Field::ModelPick(i) | Field::PostPick(i) => {
            let options: Vec<String> = list.iter().map(|(_, name)| name.clone()).collect();
            combo(ui, "pick", i, &options, false);
        }
    }
}
