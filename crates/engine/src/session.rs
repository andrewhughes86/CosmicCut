//! The session: preferences, the view, the job (machine, stock, shapes and cuts), and the one
//! way to change any of it, [`Session::execute`].

use std::panic::{AssertUnwindSafe, catch_unwind};

use CosmicCut_geom::{Aabb, Vec3};
use CosmicCut_render::OrbitCamera;
use serde_json::{Value, json};

use crate::cmd::{self, CommandSpec, RibbonTab};
use crate::cut::{Built, Context, Cut, CutKind};
use crate::error::{EngineError, EngineResult};
use crate::machine::Machine;
use crate::shape::Shape;
use crate::sim::Sim;
use crate::stock::Stock;
use crate::units::DisplayUnits;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    pub fn id(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }
}

/// What the viewport draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Display {
    pub grid: bool,
    pub view_cube: bool,
    pub orientation_triad: bool,
    pub stock: bool,
    /// The turntable, towers and wire.
    pub machine: bool,
    /// Lines across the foam showing where the wire lies along a cut.
    pub rulings: bool,
    pub readout: bool,
}

impl Display {
    pub const KEYS: &'static [&'static str] =
        &["grid", "view_cube", "orientation_triad", "stock", "machine", "rulings", "readout"];

    pub fn get(&self, key: &str) -> Option<bool> {
        Some(match key {
            "grid" => self.grid,
            "view_cube" => self.view_cube,
            "orientation_triad" => self.orientation_triad,
            "stock" => self.stock,
            "machine" => self.machine,
            "rulings" => self.rulings,
            "readout" => self.readout,
            _ => return None,
        })
    }

    pub fn set(&mut self, key: &str, on: bool) -> bool {
        let slot = match key {
            "grid" => &mut self.grid,
            "view_cube" => &mut self.view_cube,
            "orientation_triad" => &mut self.orientation_triad,
            "stock" => &mut self.stock,
            "machine" => &mut self.machine,
            "rulings" => &mut self.rulings,
            "readout" => &mut self.readout,
            _ => return false,
        };
        *slot = on;
        true
    }

    pub fn to_json(&self) -> Value {
        Value::Object(Self::KEYS.iter().filter_map(|k| Some(((*k).to_owned(), json!(self.get(k)?)))).collect())
    }
}

impl Default for Display {
    fn default() -> Self {
        Self {
            grid: true,
            view_cube: true,
            orientation_triad: true,
            stock: true,
            machine: true,
            rulings: true,
            readout: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Preferences {
    pub theme: Theme,
    pub units: DisplayUnits,
    /// Bumped on every change, so the app knows to save them.
    pub revision: u64,
    pub fps_limit: u32,
    pub display: Display,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            units: DisplayUnits::Inches,
            revision: 0,
            fps_limit: 60,
            display: Display::default(),
        }
    }
}

/// Dockable panes, by id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pane {
    Viewport,
    Explorer,
    Inspector,
    /// The cut plane: the selected cut's wire path over the stock, as the wire sees it.
    Plane,
    Gcode,
    Console,
    /// The cut being made or changed.
    Cut,
    /// The stock being set up.
    Stock,
    /// The job's machine being set up.
    Machine,
    /// A post processor being set up.
    Post,
    /// What to do next: the sketch tools' and pickers' instructions.
    Instructions,
}

impl Pane {
    pub const ALL: [Self; 11] = [
        Self::Viewport,
        Self::Explorer,
        Self::Inspector,
        Self::Plane,
        Self::Gcode,
        Self::Console,
        Self::Cut,
        Self::Stock,
        Self::Machine,
        Self::Post,
        Self::Instructions,
    ];
    pub const IDS: &'static [&'static str] = &[
        "viewport",
        "explorer",
        "inspector",
        "plane",
        "gcode",
        "console",
        "cut",
        "stock",
        "machine",
        "post",
        "instructions",
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Viewport => "viewport",
            Self::Explorer => "explorer",
            Self::Inspector => "inspector",
            Self::Plane => "plane",
            Self::Gcode => "gcode",
            Self::Console => "console",
            Self::Cut => "cut",
            Self::Stock => "stock",
            Self::Machine => "machine",
            Self::Post => "post",
            Self::Instructions => "instructions",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Viewport => "Viewport",
            Self::Explorer => "Explorer",
            Self::Inspector => "Inspector",
            Self::Plane => "Cut plane",
            Self::Gcode => "G-code",
            Self::Console => "Console",
            Self::Cut => "Cut",
            Self::Stock => "Stock",
            Self::Machine => "Machine",
            Self::Post => "Post processor",
            Self::Instructions => "Instructions",
        }
    }
}

/// Things only the UI can do, asked for by commands.
#[derive(Clone, Debug, PartialEq)]
pub enum UiRequest {
    ShowPane(Pane),
    HidePane(Pane),
    FocusPane(Pane),
}

#[derive(Clone, Debug)]
pub struct UiState {
    pub ribbon_tab: RibbonTab,
    pub requests: Vec<UiRequest>,
}

impl Default for UiState {
    fn default() -> Self {
        Self { ribbon_tab: RibbonTab::Home, requests: Vec::new() }
    }
}

#[derive(Clone, Debug)]
pub struct ViewState {
    pub camera: OrbitCamera,
    /// What the viewport last drew, for framing.
    pub scene_bounds: Aabb,
    pub aspect: f32,
    /// Bumped to put the panes back where they started.
    pub layout_generation: u64,
    /// The selected explorer item's id.
    pub selection: Option<String>,
    /// The sketch being drawn (a shape's id), the tool in use on it, and its selected items.
    pub sketch: Option<String>,
    pub sketch_tool: &'static str,
    pub sketch_selection: Vec<u32>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            camera: OrbitCamera::default(),
            scene_bounds: Aabb::new(Vec3::new(-300.0, -300.0, 0.0), Vec3::new(300.0, 300.0, 600.0)),
            aspect: 1.6,
            layout_generation: 0,
            selection: None,
            sketch: None,
            sketch_tool: "select",
            sketch_selection: Vec::new(),
        }
    }
}

pub const MACHINE_ID: &str = "machine";
pub const STOCK_ID: &str = "stock";
pub const SHAPES_ID: &str = "shapes";
pub const MODELS_ID: &str = "models";
pub const CUTS_ID: &str = "cuts";
pub const PROGRAM_ID: &str = "program";

#[derive(Clone, Debug)]
pub struct JobState {
    pub name: String,
    /// The file it was opened from or saved to.
    pub path: Option<String>,
    pub machine: Machine,
    pub stock: Stock,
    pub shapes: Vec<Shape>,
    pub models: Vec<crate::model::Model>,
    pub cuts: Vec<Cut>,
    /// The operations, in order; every cut belongs to one.
    pub ops: Vec<crate::op::Operation>,
    /// The operation new cuts go into.
    pub active_op: String,
    pub next_id: u64,
    /// Bumped on every change to what is drawn.
    pub revision: u64,
    pub edits: u64,
    /// Bumped by every change to the job (and by replacing it), for the undo history.
    pub generation: u64,
    pub saved_edits: u64,
}

impl Default for JobState {
    fn default() -> Self {
        Self {
            name: "Untitled".into(),
            path: None,
            machine: Machine::default(),
            stock: Stock::default(),
            shapes: Vec::new(),
            models: Vec::new(),
            cuts: Vec::new(),
            ops: vec![crate::op::Operation::first()],
            active_op: crate::op::FIRST.into(),
            next_id: 1,
            revision: 1,
            edits: 0,
            generation: 0,
            saved_edits: 0,
        }
    }
}

impl JobState {
    pub fn is_modified(&self) -> bool {
        self.edits != self.saved_edits
    }

    pub fn fresh_id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}.{}", self.next_id);
        self.next_id += 1;
        id
    }

    /// A name not yet used by a shape or cut: `base`, else `base 2`, `base 3`…
    pub fn fresh_name(&self, base: &str) -> String {
        let taken = |n: &str| {
            self.shapes.iter().any(|s| s.name == n)
                || self.cuts.iter().any(|c| c.name == n)
                || self.models.iter().any(|m| m.name == n)
        };
        if !taken(base) {
            return base.to_owned();
        }
        (2..).map(|k| format!("{base} {k}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_owned())
    }

    pub fn context(&self) -> Context<'_> {
        Context { machine: &self.machine, stock: &self.stock, shapes: &self.shapes, models: &self.models }
    }

    pub fn shape(&self, id: &str) -> Option<&Shape> {
        self.shapes.iter().find(|s| s.id == id || s.name == id)
    }

    pub fn cut(&self, id: &str) -> Option<&Cut> {
        self.cuts.iter().find(|c| c.id == id || c.name == id)
    }

    /// The place (from 0) of operation `id`.
    pub fn op_index(&self, id: &str) -> Option<usize> {
        self.ops.iter().position(|o| o.id == id)
    }

    pub fn model(&self, id: &str) -> Option<&crate::model::Model> {
        self.models.iter().find(|m| m.id == id || m.name == id)
    }

    /// Lays every model on the stock's bottom, centred on the turntable's axis.
    pub fn place_models(&mut self) {
        let floor = self.stock.bottom();
        for m in &mut self.models {
            m.place(floor);
        }
    }

    /// Remakes every cut from its parameters (after the machine, stock or a shape changed), and
    /// puts the models back on the stock.
    pub fn rebuild(&mut self) {
        self.place_models();
        let built: Vec<EngineResult<Built>> = {
            let cx = self.context();
            self.cuts.iter().map(|c| crate::cut::build(&cx, c.kind, &c.params)).collect()
        };
        for (c, b) in self.cuts.iter_mut().zip(built) {
            match b {
                Ok(b) => {
                    c.path = b.path;
                    c.keeps = b.keeps;
                    c.release = b.release;
                    c.warnings = b.warnings;
                    c.error = None;
                }
                Err(e) => {
                    c.path = crate::wire::WirePath::default();
                    c.keeps.clear();
                    c.release = None;
                    c.warnings.clear();
                    c.error = Some(e.to_string());
                }
            }
        }
    }

    /// Marks the job changed: drawn again and unsaved.
    pub fn touch(&mut self) {
        self.revision += 1;
        self.edits += 1;
        self.generation += 1;
    }

    /// Everything the viewport shows from the job.
    pub fn bounds(&self) -> Aabb {
        let mut b = self.stock.bounds();
        for m in &self.models {
            let mb = m.bounds();
            b.include(mb.min);
            b.include(mb.max);
        }
        let r = self.machine.table_radius as f32;
        b.include(Vec3::new(-r, -r, 0.0));
        b.include(Vec3::new(r, r, 0.0));
        b
    }
}

/// One executed command, for the console.
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub seq: u64,
    pub command: String,
    pub params: Value,
    pub outcome: Result<Value, EngineError>,
}

/// A row in the explorer.
#[derive(Clone, Debug, Default)]
pub struct OutlineNode {
    pub id: String,
    pub label: String,
    pub children: Vec<OutlineNode>,
    pub properties: Vec<(String, String)>,
    /// Shown or hidden, for items with an eye.
    pub visible: Option<bool>,
    /// Icon name, if any.
    pub icon: Option<&'static str>,
    /// Something is wrong with it (shown in the warning colour).
    pub problem: bool,
}

impl OutlineNode {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "label": self.label,
            "visible": self.visible,
            "children": self.children.iter().map(OutlineNode::to_json).collect::<Vec<_>>(),
        })
    }
}

/// Highest frame-rate limit accepted.
pub const MAX_FPS_LIMIT: u32 = 240;

pub fn camera_json(c: &OrbitCamera) -> Value {
    json!({
        "target": c.target.to_array(),
        "distance": c.distance,
        "yaw": c.yaw.to_degrees(),
        "pitch": c.pitch.to_degrees(),
        "projection": c.projection.id(),
        "eye": c.eye().to_array(),
    })
}

/// Most changes the undo history keeps.
pub const MAX_UNDO: usize = 100;

/// What Copy or Cut put on the clipboard.
#[derive(Clone, Debug)]
pub enum Clip {
    Cut(Box<Cut>),
    Shape(Box<Shape>),
    Model(Box<crate::model::Model>),
}

/// Most log entries kept.
const MAX_LOG: usize = 500;

/// Most jobs open at once, in tabs.
pub const MAX_TABS: usize = 32;

/// A job open in a tab other than the current one, with what goes with it.
pub struct Document {
    job: JobState,
    undo: Vec<JobState>,
    redo: Vec<JobState>,
    selection: Option<String>,
    camera: OrbitCamera,
    sim: Option<Box<Sim>>,
}

/// One tab, as the tab strip shows it.
pub struct TabInfo {
    pub name: String,
    pub path: Option<String>,
    pub modified: bool,
}

#[derive(Default)]
pub struct Session {
    /// The operator's machines and post processors, kept between jobs.
    pub library: crate::library::Library,
    pub prefs: Preferences,
    pub view: ViewState,
    pub job: JobState,
    pub ui: UiState,
    pub sim: Option<Box<Sim>>,
    /// The job as it was before each change, newest last, and the changes undone.
    pub undo: Vec<JobState>,
    pub redo: Vec<JobState>,
    /// What Copy or Cut took, for Paste.
    pub clipboard: Option<Clip>,
    /// The jobs open in the other tabs, in tab order, without the current one.
    others: Vec<Document>,
    /// Where the current job's tab is among the tabs.
    active: usize,
    /// Bumped whenever another tab's job comes in, so that is not taken for an edit.
    swaps: u64,
    log: Vec<LogEntry>,
    seq: u64,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// A session with a sample job: a block, a few shapes and cuts.
    pub fn with_sample() -> Self {
        let mut s = Self::new();
        let _ = crate::cmd::job::load_sample(&mut s);
        s
    }

    pub fn log(&self) -> &[LogEntry] {
        &self.log
    }

    pub fn is_enabled(&self, spec: &CommandSpec) -> bool {
        (spec.enabled)(self)
    }

    /// Whether a button for `spec` is live: it can run, and (for sketch constraints) what is
    /// selected suits it. Commands with their items given still run when it is not.
    pub fn is_available(&self, spec: &CommandSpec) -> bool {
        self.is_enabled(spec) && cmd::sketch::fits_selection(self, spec.id)
    }

    /// Runs command `id` with `params`. A command that fails, or panics, leaves the session as
    /// it was.
    pub fn execute(&mut self, id: &str, params: &Value) -> EngineResult<Value> {
        let outcome = self.execute_inner(id, params);
        self.seq += 1;
        // Bulky results (posted programs) are logged short.
        let logged = outcome
            .as_ref()
            .map(|v| match v.get("text") {
                Some(_) => {
                    let mut v = v.clone();
                    if let Some(o) = v.as_object_mut() {
                        o.remove("text");
                    }
                    v
                }
                None => v.clone(),
            })
            .map_err(Clone::clone);
        if !id.starts_with("sim.seek") {
            self.log.push(LogEntry { seq: self.seq, command: id.to_owned(), params: params.clone(), outcome: logged });
            if self.log.len() > MAX_LOG {
                self.log.drain(..self.log.len() - MAX_LOG);
            }
        }
        outcome
    }

    fn execute_inner(&mut self, id: &str, params: &Value) -> EngineResult<Value> {
        let spec = cmd::find(id).ok_or_else(|| EngineError::UnknownCommand(id.chars().take(80).collect()))?;
        spec.validate(params)?;
        if !(spec.enabled)(self) {
            return Err(EngineError::Disabled(id.to_owned()));
        }
        let job = self.job.clone();
        let view = self.view.clone();
        let prefs = self.prefs.clone();
        let swaps = self.swaps;
        let result = catch_unwind(AssertUnwindSafe(|| (spec.run)(self, params)));
        match result {
            Ok(Ok(v)) => {
                // A sketch being drawn that is gone (undone, deleted, closed) is drawn no more.
                if let Some(id) = &self.view.sketch
                    && !self.job.shapes.iter().any(|sh| sh.id == *id && sh.sketch.is_some())
                {
                    self.view.sketch = None;
                    self.view.sketch_tool = "select";
                    self.view.sketch_selection.clear();
                }
                // A change to the job can be undone; undo and redo keep their own history, and
                // another tab's job coming in is no change.
                if self.job.generation != job.generation
                    && self.swaps == swaps
                    && id != "edit.undo"
                    && id != "edit.redo"
                {
                    self.undo.push(job);
                    if self.undo.len() > MAX_UNDO {
                        self.undo.remove(0);
                    }
                    self.redo.clear();
                }
                Ok(v)
            }
            Ok(Err(e)) => {
                self.job = job;
                self.view = view;
                self.prefs = prefs;
                Err(e)
            }
            Err(_) => {
                self.job = job;
                self.view = view;
                self.prefs = prefs;
                self.sim = None;
                Err(EngineError::Internal(format!("`{id}` failed unexpectedly; the job is as it was")))
            }
        }
    }

    pub fn set_viewport_aspect(&mut self, aspect: f32) {
        if aspect.is_finite() && aspect > 0.0 {
            self.view.aspect = aspect;
        }
    }

    pub fn set_scene_bounds(&mut self, b: Aabb) {
        if !b.is_empty() {
            self.view.scene_bounds = b;
        }
    }

    /// Builds a cut without adding it, for previews while it is set up.
    pub fn preview_cut(&self, kind: CutKind, params: &Value) -> EngineResult<Built> {
        let spec = cmd::find(kind.command()).ok_or_else(|| EngineError::UnknownCommand(kind.command().into()))?;
        let mut p = params.clone();
        if let Some(o) = p.as_object_mut() {
            o.remove("id");
        }
        spec.validate(&p)?;
        let cx = self.job.context();
        catch_unwind(AssertUnwindSafe(|| crate::cut::build(&cx, kind, &p)))
            .unwrap_or_else(|_| Err(EngineError::Internal("the preview failed".into())))
    }

    /// The explorer's tree.
    pub fn outline(&self) -> Vec<OutlineNode> {
        let units = self.prefs.units;
        let job = &self.job;
        let machine = OutlineNode {
            id: MACHINE_ID.into(),
            label: format!("Machine: {}", job.machine.name),
            properties: job.machine.properties(units),
            icon: Some("machine_edit"),
            ..Default::default()
        };
        let stock = OutlineNode {
            id: STOCK_ID.into(),
            label: format!("Stock: {}", job.stock.describe(units)),
            properties: job.stock.properties(units),
            icon: Some("stock_block"),
            ..Default::default()
        };
        let shapes = OutlineNode {
            id: SHAPES_ID.into(),
            label: format!("Shapes ({})", job.shapes.len()),
            children: job
                .shapes
                .iter()
                .map(|s| OutlineNode {
                    id: s.id.clone(),
                    label: s.name.clone(),
                    properties: s.properties(units),
                    icon: Some(match s.sketch.as_ref().map(|k| k.plane) {
                        Some(crate::sketch::Plane::Front) => "sketch_new_front",
                        Some(crate::sketch::Plane::Right) => "sketch_new_right",
                        Some(crate::sketch::Plane::Top) => "sketch_new_top",
                        None => "explorer_shape",
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let models = OutlineNode {
            id: MODELS_ID.into(),
            label: format!("Models ({})", job.models.len()),
            children: job
                .models
                .iter()
                .map(|m| OutlineNode {
                    id: m.id.clone(),
                    label: m.name.clone(),
                    properties: m.properties(units),
                    visible: Some(m.visible),
                    icon: Some("model_import"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let cut_node = |c: &Cut| OutlineNode {
            id: c.id.clone(),
            label: c.name.clone(),
            // Shapes by name, not id.
            properties: c
                .properties(&job.machine, units)
                .into_iter()
                .map(|(k, v)| {
                    // Shapes and models by name, not id.
                    let name = job
                        .shapes
                        .iter()
                        .find(|s| s.id == v)
                        .map(|s| s.name.clone())
                        .or_else(|| job.models.iter().find(|m| m.id == v).map(|m| m.name.clone()));
                    (k, name.unwrap_or(v))
                })
                .collect(),
            visible: Some(c.visible),
            icon: Some(cmd::cut_icon(c.kind)),
            problem: c.error.is_some(),
            ..Default::default()
        };
        // Each operation with its cuts; the one new cuts go into is marked.
        let ops: Vec<OutlineNode> = job
            .ops
            .iter()
            .enumerate()
            .map(|(n, op)| {
                let mut label = op.title(n);
                if op.pauses() {
                    label.push_str(&format!(
                        "  [{}]",
                        if op.stop == crate::op::Stop::None { "message" } else { op.stop.code().unwrap_or_default() }
                    ));
                }
                if job.ops.len() > 1 && op.id == job.active_op {
                    label.push_str("  (active)");
                }
                OutlineNode {
                    id: op.id.clone(),
                    label,
                    children: job.cuts.iter().filter(|c| c.op == op.id).map(cut_node).collect(),
                    properties: op.properties(),
                    icon: Some("op_settings"),
                    ..Default::default()
                }
            })
            .collect();
        let prog = crate::program::Program::build(&job.machine, &job.stock, &job.cuts, &job.ops);
        let program = OutlineNode {
            id: PROGRAM_ID.into(),
            label: "Program".into(),
            properties: vec![
                ("Moves".into(), prog.moves.len().to_string()),
                ("Time".into(), crate::cut::fmt_time(prog.duration() + job.machine.post.preheat)),
                ("Units".into(), job.machine.post.units.label().into()),
            ],
            icon: Some("post_gcode_export"),
            ..Default::default()
        };
        vec![OutlineNode {
            id: "job".into(),
            label: format!("{}{}", job.name, if job.is_modified() { " •" } else { "" }),
            children: {
                let mut c = vec![machine, stock, models];
                // Shapes show once there are some.
                if !job.shapes.is_empty() {
                    c.push(shapes);
                }
                c.extend(ops);
                c.push(program);
                c
            },
            properties: vec![("File".into(), job.path.clone().unwrap_or_else(|| "(not saved yet)".into()))],
            ..Default::default()
        }]
    }

    /// The outline row for `id`, for the inspector.
    pub fn selection_node(&self, id: &str) -> Option<OutlineNode> {
        fn find(nodes: Vec<OutlineNode>, id: &str) -> Option<OutlineNode> {
            for n in nodes {
                if n.id == id {
                    return Some(n);
                }
                if let Some(found) = find(n.children, id) {
                    return Some(found);
                }
            }
            None
        }
        find(self.outline(), id)
    }

    /// The selected cut, if a cut is selected.
    /// The open jobs, in tab order.
    pub fn tabs(&self) -> Vec<TabInfo> {
        let info = |j: &JobState| TabInfo { name: j.name.clone(), path: j.path.clone(), modified: j.is_modified() };
        let mut tabs: Vec<TabInfo> = self.others.iter().map(|d| info(&d.job)).collect();
        tabs.insert(self.active.min(tabs.len()), info(&self.job));
        tabs
    }

    /// Which of [`Session::tabs`] is the current job.
    pub fn active_tab(&self) -> usize {
        self.active.min(self.others.len())
    }

    /// Makes tab `index` the current job.
    pub(crate) fn switch_tab(&mut self, index: usize) -> EngineResult<()> {
        let count = self.others.len() + 1;
        if index >= count {
            return Err(EngineError::InvalidParams(format!("there are {count} tabs; there is no tab {}", index + 1)));
        }
        if index == self.active_tab() {
            return Ok(());
        }
        let outgoing = self.job.revision;
        let parked = Document {
            job: std::mem::take(&mut self.job),
            undo: std::mem::take(&mut self.undo),
            redo: std::mem::take(&mut self.redo),
            selection: self.view.selection.take(),
            camera: self.view.camera,
            sim: self.sim.take(),
        };
        let at = self.active_tab();
        self.others.insert(at, parked);
        if index >= self.others.len() {
            return Err(EngineError::Internal("the tabs are out of step".into()));
        }
        let incoming = self.others.remove(index);
        self.bring_in(incoming, outgoing);
        self.active = index;
        Ok(())
    }

    fn bring_in(&mut self, d: Document, outgoing_revision: u64) {
        self.job = d.job;
        // A revision the panes have not drawn, so they show this job and not the last one.
        self.job.revision = outgoing_revision.max(self.job.revision).wrapping_add(1000);
        self.undo = d.undo;
        self.redo = d.redo;
        self.view.selection = d.selection;
        self.view.camera = d.camera;
        // A sketch being drawn belongs to the job that was in front.
        self.view.sketch = None;
        self.view.sketch_tool = "select";
        self.view.sketch_selection.clear();
        self.sim = d.sim;
        self.swaps = self.swaps.wrapping_add(1);
    }

    /// Opens `job` in a new tab after the current one and makes it current. With
    /// `replace_blank`, an untouched empty job is replaced rather than kept beside it.
    pub(crate) fn open_tab(&mut self, job: JobState, replace_blank: bool) -> EngineResult<()> {
        let blank = replace_blank
            && self.job.path.is_none()
            && !self.job.is_modified()
            && self.job.models.is_empty()
            && self.job.shapes.is_empty()
            && self.job.cuts.is_empty();
        if !blank && self.others.len() + 1 >= MAX_TABS {
            return Err(EngineError::InvalidParams(format!("{MAX_TABS} jobs are open; close one first")));
        }
        let outgoing = self.job.revision;
        let fresh =
            Document { job, undo: Vec::new(), redo: Vec::new(), selection: None, camera: self.view.camera, sim: None };
        if !blank {
            let parked = Document {
                job: std::mem::take(&mut self.job),
                undo: std::mem::take(&mut self.undo),
                redo: std::mem::take(&mut self.redo),
                selection: self.view.selection.take(),
                camera: self.view.camera,
                sim: self.sim.take(),
            };
            let at = self.active_tab();
            self.others.insert(at, parked);
            self.active = at + 1;
        }
        self.bring_in(fresh, outgoing);
        Ok(())
    }

    /// Closes the current tab and brings in its neighbour. Returns false, changing nothing, when
    /// it is the only tab.
    pub(crate) fn close_tab(&mut self) -> bool {
        if self.others.is_empty() {
            return false;
        }
        let at = self.active_tab();
        // The tab to its right takes its place; the last tab gives way to the one on its left.
        let next = at.min(self.others.len() - 1);
        let outgoing = self.job.revision;
        let incoming = self.others.remove(next);
        self.bring_in(incoming, outgoing);
        self.active = next;
        true
    }

    /// The tab whose job was read from `path`, if one is open.
    pub(crate) fn tab_with_path(&self, path: &str) -> Option<usize> {
        self.tabs().iter().position(|t| t.path.as_deref() == Some(path))
    }

    pub fn selected_cut(&self) -> Option<&Cut> {
        self.view.selection.as_deref().and_then(|id| self.job.cuts.iter().find(|c| c.id == id))
    }

    pub fn selected_model(&self) -> Option<&crate::model::Model> {
        self.view.selection.as_deref().and_then(|id| self.job.models.iter().find(|m| m.id == id))
    }

    pub fn selected_shape(&self) -> Option<&Shape> {
        self.view.selection.as_deref().and_then(|id| self.job.shapes.iter().find(|s| s.id == id))
    }
}
