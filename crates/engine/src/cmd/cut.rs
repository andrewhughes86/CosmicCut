//! Cut commands: make the four kinds of cut, edit, order, show and remove them.

use serde_json::{Value, json};

use super::{CommandSpec, Param, ParamKind, Placement, RibbonTab, always};
use crate::cut::{ANCHORS, Cut, CutKind, DIRECTIONS, KEEPS, LATHE_AXES, LEADS, stored_params};
use crate::error::{EngineError, EngineResult};
use crate::params;
use crate::session::Session;

const DEG: ParamKind = ParamKind::Number { min: -36_000.0, max: 36_000.0 };
const AT: ParamKind = ParamKind::Length { min: -1.0e6, max: 1.0e6 };
const SCALE: ParamKind = ParamKind::Number { min: 1.0e-6, max: 1.0e6 };

const ID: Param = Param::optional("id", ParamKind::Str, "Change this cut's settings instead of adding a cut");
const NAME: Param = Param::optional("name", ParamKind::Str, "Name for the cut");
const KERF: Param = Param::optional("kerf", ParamKind::Length { min: 0.0, max: 50.0 }, "Kerf (default: the machine's)");
const COMPENSATE: Param = Param::optional(
    "compensate",
    ParamKind::Bool,
    "Run the wire half a kerf outside the outline so the part comes out the size drawn (default yes)",
);
const FEED: Param = Param::optional("feed", ParamKind::Rate { min: 1.0, max: 1.0e6 }, "Feed (default: the machine's)");
const CLEARANCE: Param = Param::optional(
    "clearance",
    ParamKind::Length { min: 0.0, max: 10_000.0 },
    "How far from the foam the wire starts, finishes and waits while the table turns (default: the machine's)",
);
const LEAD: Param = Param::optional(
    "lead",
    ParamKind::Choice(LEADS),
    "Side of the foam the wire comes in from: left (default), right, or down from the top. With the bottom \
     attached it leaves from the other side",
);
const DIRECTION: Param = Param::optional(
    "direction",
    ParamKind::Choice(DIRECTIONS),
    "Way round the outline seen from the front: ccw (anticlockwise, default) or cw. Used when the bottom is cut through",
);
const ALTERNATE: Param = Param::optional(
    "alternate",
    ParamKind::Bool,
    "Each pass starts on the side the last one finished and runs the other way, so the wire does not travel \
     back over the foam between passes (default yes)",
);

const BOTTOM: Param = Param::optional(
    "bottom",
    ParamKind::Choice(crate::cut::BOTTOMS),
    "attached (default): the wire comes in level with the part's bottom, goes over its top and out level on \
     the other side, so the part stays joined to the foam under it and cannot move while the table turns. \
     through: the wire goes right round the outline and the part comes loose straight away",
);
const ATTACH: Param = Param::optional(
    "attach",
    ParamKind::Length { min: 0.0, max: 10_000.0 },
    "How far up from its lowest point the part stays joined to the foam under it. 0 (default) holds it \
     along its bottom edge; raise it for round-bottomed parts that only touch at a point",
);
const RELEASE: Param = Param::optional(
    "release",
    ParamKind::Bool,
    "Cut across the bottom at the very end of the program to free the part, following its bottom through \
     the cuts the wire came in and out by (default yes). Only the last cut that holds the part does it",
);

const START_X: Param = Param::optional(
    "start_x",
    ParamKind::Length { min: -100_000.0, max: 100_000.0 },
    "Where the cut starts, across (with start_y): the wire goes into the outline at its point nearest \
     there, from whichever side reaches it without crossing the part (blank: from the lead side)",
);
const START_Y: Param =
    Param::optional("start_y", ParamKind::Length { min: -100_000.0, max: 100_000.0 }, "Where the cut starts, up");

pub const PROFILE_PARAMS: &[Param] = &[
    ID,
    NAME,
    Param::required("shape", ParamKind::Str, "The outline the wire follows, seen along the wire"),
    Param::optional("angle", DEG, "Turntable angle (B) of the first pass, degrees (default 0)"),
    Param::optional(
        "copies",
        ParamKind::Integer { min: 1, max: 360 },
        "Cut the outline this many times, turning the table between them, for parts shaped all round \
         (default 1)",
    ),
    Param::optional("step", DEG, "Degrees the table turns between copies (default an even split of 360)"),
    Param::optional(
        "anchor",
        ParamKind::Choice(ANCHORS),
        "Which point of the shape X and Y place: its centre (default), bottom centre, bottom left, its own \
         origin, or absolute (the shape's own coordinates are table positions, as for outlines from a model)",
    ),
    Param::optional("x", AT, "Where the anchor goes across, from the turntable's axis (default the stock's middle)"),
    Param::optional("y", AT, "Height of the anchor above the table (default the stock's middle or bottom)"),
    Param::optional("scale", SCALE, "Scale factor"),
    Param::optional("rotate", DEG, "Turn of the shape, degrees counter-clockwise"),
    Param::optional("mirror", ParamKind::Bool, "Mirror the shape left to right"),
    Param::optional(
        "keep",
        ParamKind::Choice(KEEPS),
        "inside (default): the part is inside the outline. outside: keep the foam round it, as for a mould",
    ),
    LEAD,
    START_X,
    START_Y,
    DIRECTION,
    ALTERNATE,
    BOTTOM,
    ATTACH,
    RELEASE,
    KERF,
    COMPENSATE,
    FEED,
    CLEARANCE,
];

pub const TWOVIEW_PARAMS: &[Param] = &[
    ID,
    NAME,
    Param::required("front", ParamKind::Str, "The shape seen from the front, at the turntable angle"),
    Param::required("side", ParamKind::Str, "The shape seen from the side, a quarter turn on"),
    Param::optional("angle", DEG, "Turntable angle of the front view, degrees"),
    Param::optional("anchor", ParamKind::Choice(ANCHORS), "Point of each shape that is placed"),
    Param::optional("scale", SCALE, "Scale factor of both shapes"),
    Param::optional("x", AT, "Where both anchors go across, from the turntable's axis"),
    Param::optional("y", AT, "Height of both anchors above the table"),
    Param::optional("front_x", AT, "Where the front anchor goes across"),
    Param::optional("front_y", AT, "Height of the front anchor"),
    Param::optional("front_rotate", DEG, "Turn of the front shape"),
    Param::optional("front_mirror", ParamKind::Bool, "Mirror the front shape"),
    Param::optional("side_x", AT, "Where the side anchor goes across"),
    Param::optional("side_y", AT, "Height of the side anchor"),
    Param::optional("side_rotate", DEG, "Turn of the side shape"),
    Param::optional("side_mirror", ParamKind::Bool, "Mirror the side shape"),
    LEAD,
    DIRECTION,
    ALTERNATE,
    BOTTOM,
    ATTACH,
    RELEASE,
    KERF,
    COMPENSATE,
    FEED,
    CLEARANCE,
];

pub const LATHE_PARAMS: &[Param] = &[
    ID,
    NAME,
    Param::required("shape", ParamKind::Str, "The outline to turn: a half profile, or a whole silhouette"),
    Param::optional(
        "axis",
        ParamKind::Choice(LATHE_AXES),
        "Where the turning axis is on the shape: its middle, or its x = 0",
    ),
    Param::optional("y", AT, "Height of the shape's bottom above the table (default the stock's bottom)"),
    Param::optional("scale", SCALE, "Scale factor"),
    Param::optional("step", ParamKind::Number { min: 0.1, max: 180.0 }, "Degrees between passes (default 5)"),
    Param::optional("angle", DEG, "Turntable angle of the first pass"),
    Param::optional("sweep", ParamKind::Number { min: 1.0, max: 360.0 }, "Degrees the passes span (default 360)"),
    RELEASE,
    KERF,
    COMPENSATE,
    FEED,
    CLEARANCE,
];

pub const MODEL_PARAMS: &[Param] = &[
    ID,
    NAME,
    Param::optional("model", ParamKind::Str, "The model (default the imported one)"),
    Param::optional(
        "spacing",
        ParamKind::Choice(crate::cut::SPACINGS),
        "How the cuts are spaced round the part: divide half a turn into a number of cuts, or a cut every so many \
         degrees. Half a turn is enough: a profile at 180° is the same as at 0°, mirrored",
    ),
    Param::optional(
        "divisions",
        ParamKind::Integer { min: 1, max: 180 },
        "Number of cuts over half a turn; more cuts make a rounder part and take longer (default 8)",
    ),
    Param::optional(
        "degrees",
        ParamKind::Number { min: 0.5, max: 180.0 },
        "Degrees the table turns between cuts (default 22.5)",
    ),
    ALTERNATE,
    RELEASE,
    KERF,
    FEED,
];

pub const SWARF_PARAMS: &[Param] = &[
    ID,
    NAME,
    Param::optional("model", ParamKind::Str, "The model (default the imported one)"),
    Param::required(
        "faces",
        ParamKind::IntList { max: crate::swarf::MAX_FACES },
        "The model's faces for the wire to follow (face numbers). Faces that line up are cut in one pass, the \
         rest each in their own",
    ),
    Param::optional(
        "height_step",
        ParamKind::Length { min: 0.05, max: 1000.0 },
        "Height between the wire positions down the faces; smaller follows twisting faces more closely (default 2 mm)",
    ),
    Param::optional(
        "tolerance",
        ParamKind::Length { min: 0.001, max: 50.0 },
        "How far the faces' level slices may be off a straight line (default 0.25 mm)",
    ),
    KERF,
    COMPENSATE,
    FEED,
    CLEARANCE,
];

pub const SLAB_PARAMS: &[Param] = &[
    ID,
    NAME,
    Param::required("thickness", ParamKind::Length { min: 0.1, max: 1.0e5 }, "Thickness of each slab"),
    Param::optional("skim", ParamKind::Length { min: 0.0, max: 1.0e5 }, "Foam to cut off the top first"),
    Param::optional("count", ParamKind::Integer { min: 1, max: 10_000 }, "Most slabs to cut (default as many as fit)"),
    Param::optional("angle", DEG, "Turntable angle, degrees"),
    Param::optional("order", ParamKind::Choice(&["top_down", "bottom_up"]), "Cut from the top down or the bottom up"),
    Param::optional("alternate", ParamKind::Bool, "Cut back and forth (default) instead of always left to right"),
    KERF,
    FEED,
    CLEARANCE,
];

macro_rules! create {
    ($kind:expr, $id:literal, $label:literal, $tooltip:literal, $params:expr) => {
        CommandSpec {
            id: $id,
            label: $label,
            tooltip: $tooltip,
            placement: Some(Placement::gallery(RibbonTab::Cuts, "Cuts")),
            icon: None,
            shortcut: None,
            params: $params,
            returns: "cut",
            enabled: always,
            run: |s, p| create(s, $kind, p),
        }
    };
}

pub(super) static COMMANDS: &[CommandSpec] = &[
    create!(
        CutKind::Profile,
        "cut.profile",
        "Profile",
        "Follow a shape through the foam with the table at one angle, or at several",
        PROFILE_PARAMS
    ),
    create!(
        CutKind::TwoView,
        "cut.twoview",
        "Two-view",
        "Cut a front shape, turn the table a quarter, cut a side shape: the part is what both keep",
        TWOVIEW_PARAMS
    ),
    create!(
        CutKind::Lathe,
        "cut.lathe",
        "Lathe",
        "Turn a solid of revolution: pass down a half profile at many table angles",
        LATHE_PARAMS
    ),
    create!(CutKind::Slab, "cut.slab", "Slabs", "Slice the block into sheets with level passes", SLAB_PARAMS),
    CommandSpec {
        id: "cut.model",
        label: "From model",
        tooltip: "Cut the imported 3D model: a new profile of the model at every rotation of the table, the part held \
                  on the foam until it is parted off at the end",
        placement: Some(Placement::gallery(RibbonTab::Cuts, "Cuts")),
        icon: None,
        shortcut: None,
        params: MODEL_PARAMS,
        returns: "cut",
        enabled: |s| !s.job.models.is_empty(),
        run: |s, p| create(s, CutKind::Model, p),
    },
    CommandSpec {
        id: "cut.swarf",
        label: "Swarf",
        tooltip: "Lay the wire along faces of the model: X, Y and B move together, so it follows tapered, twisted \
                  and turning faces made of level straight lines",
        placement: Some(Placement::gallery(RibbonTab::Cuts, "Cuts")),
        icon: None,
        shortcut: None,
        params: SWARF_PARAMS,
        returns: "cut",
        enabled: |s| !s.job.models.is_empty(),
        run: |s, p| create(s, CutKind::Swarf, p),
    },
    CommandSpec {
        id: "cut.edit",
        label: "Edit cut",
        tooltip: "Change the selected cut's settings",
        placement: Some(Placement::large(RibbonTab::Cuts, "Edit")),
        icon: None,
        shortcut: Some("E"),
        params: &[Param::optional("id", ParamKind::Str, "The cut (default the selected one)")],
        returns: "cut",
        enabled: super::has_cuts,
        run: |s, p| {
            let i = index(s, p)?;
            cut_json(s, i)
        },
    },
    CommandSpec {
        id: "cut.duplicate",
        label: "Duplicate",
        tooltip: "Copy the selected cut",
        placement: Some(Placement::small(RibbonTab::Cuts, "Edit")),
        icon: None,
        shortcut: Some("Ctrl+D"),
        params: &[Param::optional("id", ParamKind::Str, "The cut (default the selected one)")],
        returns: "cut",
        enabled: super::has_cuts,
        run: |s, p| {
            let i = index(s, p)?;
            let mut c = s.job.cuts.get(i).cloned().ok_or_else(missing)?;
            c.id = s.job.fresh_id("cut");
            c.name = s.job.fresh_name(&c.name);
            s.view.selection = Some(c.id.clone());
            s.job.cuts.insert(i + 1, c);
            changed(s);
            cut_json(s, i + 1)
        },
    },
    CommandSpec {
        id: "cut.remove",
        label: "Delete",
        tooltip: "Delete the selected cut",
        placement: Some(Placement::small(RibbonTab::Cuts, "Edit")),
        icon: None,
        shortcut: None,
        params: &[Param::optional("id", ParamKind::Str, "The cut (default the selected one)")],
        returns: "{\"removed\": id}",
        enabled: super::has_cuts,
        run: |s, p| {
            let i = index(s, p)?;
            let c = s.job.cuts.remove(i);
            if s.view.selection.as_deref() == Some(c.id.as_str()) {
                s.view.selection = None;
            }
            changed(s);
            Ok(json!({ "removed": c.id }))
        },
    },
    CommandSpec {
        id: "cut.rename",
        label: "Rename cut",
        tooltip: "Rename a cut",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::optional("id", ParamKind::Str, "The cut"),
            Param::required("name", ParamKind::Str, "New name"),
        ],
        returns: "cut",
        enabled: super::has_cuts,
        run: |s, p| {
            let i = index(s, p)?;
            let name = params::req_str(p, "name")?.trim();
            if name.is_empty() {
                return Err(EngineError::InvalidParams("`name` must not be empty".into()));
            }
            if let Some(c) = s.job.cuts.get_mut(i) {
                c.name = name.chars().take(200).collect();
            }
            s.job.touch();
            cut_json(s, i)
        },
    },
    CommandSpec {
        id: "cut.move.up",
        label: "Earlier",
        tooltip: "Cut the selected cut before the one above it",
        placement: Some(Placement::small(RibbonTab::Cuts, "Order")),
        icon: None,
        shortcut: None,
        params: &[Param::optional("id", ParamKind::Str, "The cut (default the selected one)")],
        returns: "[cut ids in order]",
        enabled: super::has_cuts,
        run: |s, p| shift(s, p, -1),
    },
    CommandSpec {
        id: "cut.move.down",
        label: "Later",
        tooltip: "Cut the selected cut after the one below it",
        placement: Some(Placement::small(RibbonTab::Cuts, "Order")),
        icon: None,
        shortcut: None,
        params: &[Param::optional("id", ParamKind::Str, "The cut (default the selected one)")],
        returns: "[cut ids in order]",
        enabled: super::has_cuts,
        run: |s, p| shift(s, p, 1),
    },
    CommandSpec {
        id: "cut.move",
        label: "Move cut",
        tooltip: "Put a cut at a place in the order",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("id", ParamKind::Str, "The cut"),
            Param::required("to", ParamKind::Integer { min: 0, max: 100_000 }, "Its new place, from 0"),
        ],
        returns: "[cut ids in order]",
        enabled: super::has_cuts,
        run: |s, p| {
            let i = index(s, p)?;
            let to = params::opt_f64(p, "to", 0.0, 100_000.0)?.unwrap_or(0.0) as usize;
            let c = s.job.cuts.remove(i);
            let to = to.min(s.job.cuts.len());
            s.job.cuts.insert(to, c);
            changed(s);
            Ok(order(s))
        },
    },
    CommandSpec {
        id: "cut.visibility.set",
        label: "Show or hide cut",
        tooltip: "Show or hide a cut's wire path in the viewport",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("id", ParamKind::Str, "The cut"),
            Param::optional("visible", ParamKind::Bool, "Show it; omit to toggle"),
        ],
        returns: "cut",
        enabled: super::has_cuts,
        run: |s, p| {
            let i = index(s, p)?;
            let want = params::opt_bool(p, "visible")?;
            if let Some(c) = s.job.cuts.get_mut(i) {
                c.visible = want.unwrap_or(!c.visible);
            }
            s.job.revision += 1;
            cut_json(s, i)
        },
    },
    CommandSpec {
        id: "cut.get",
        label: "Cut",
        tooltip: "A cut, its settings and its wire path",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::optional("id", ParamKind::Str, "The cut (default the selected one)")],
        returns: "cut with its path",
        enabled: super::has_cuts,
        run: |s, p| {
            let i = index(s, p)?;
            let c = s.job.cuts.get(i).ok_or_else(missing)?;
            let mut v = c.summary_json(&s.job.machine);
            v["path"] = c.path.to_json();
            Ok(v)
        },
    },
];

fn missing() -> EngineError {
    EngineError::InvalidParams("there is no such cut".into())
}

/// The cut `id` names, or the selected one.
fn index(s: &Session, p: &Value) -> EngineResult<usize> {
    let id = match params::opt_str(p, "id")? {
        Some(id) => id.to_owned(),
        None => s
            .view
            .selection
            .clone()
            .filter(|sel| s.job.cuts.iter().any(|c| &c.id == sel))
            .ok_or_else(|| EngineError::InvalidParams("select a cut first".into()))?,
    };
    s.job
        .cuts
        .iter()
        .position(|c| c.id == id || c.name == id)
        .ok_or_else(|| EngineError::InvalidParams(format!("there is no cut `{id}`")))
}

fn cut_json(s: &Session, i: usize) -> EngineResult<Value> {
    s.job.cuts.get(i).map(|c| c.summary_json(&s.job.machine)).ok_or_else(missing)
}

fn order(s: &Session) -> Value {
    json!(s.job.cuts.iter().map(|c| c.id.clone()).collect::<Vec<_>>())
}

fn changed(s: &mut Session) {
    s.job.touch();
    s.sim = None;
}

fn shift(s: &mut Session, p: &Value, by: isize) -> EngineResult<Value> {
    let i = index(s, p)?;
    let j = i as isize + by;
    if j < 0 || j as usize >= s.job.cuts.len() {
        return Ok(order(s));
    }
    s.job.cuts.swap(i, j as usize);
    changed(s);
    Ok(order(s))
}

/// Makes (or, with `id`, changes) a cut of `kind`.
pub(crate) fn create(s: &mut Session, kind: CutKind, p: &Value) -> EngineResult<Value> {
    let stored = stored_params(p);
    let built = {
        let cx = s.job.context();
        crate::cut::build(&cx, kind, &stored)?
    };
    let editing = params::opt_str(p, "id")?.map(str::to_owned);
    let name = params::opt_str(p, "name")?.map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned);
    let i = match editing {
        Some(id) => {
            let i = s
                .job
                .cuts
                .iter()
                .position(|c| c.id == id)
                .ok_or_else(|| EngineError::InvalidParams(format!("there is no cut `{id}`")))?;
            let c = s.job.cuts.get_mut(i).ok_or_else(missing)?;
            if c.kind != kind {
                return Err(EngineError::InvalidParams(format!("`{id}` is a {} cut", c.kind.label())));
            }
            if let Some(n) = name {
                c.name = n;
            }
            c.params = stored;
            c.path = built.path;
            c.keeps = built.keeps;
            c.release = built.release;
            c.warnings = built.warnings;
            c.error = None;
            i
        }
        None => {
            let id = s.job.fresh_id("cut");
            let name = name.unwrap_or_else(|| s.job.fresh_name(kind.label()));
            s.job.cuts.push(Cut {
                id: id.clone(),
                name,
                kind,
                params: stored,
                visible: true,
                path: built.path,
                keeps: built.keeps,
                release: built.release,
                warnings: built.warnings,
                error: None,
                op: s.job.active_op.clone(),
            });
            s.view.selection = Some(id);
            s.job.cuts.len() - 1
        }
    };
    changed(s);
    cut_json(s, i)
}

/// The parameter schema of a kind of cut.
pub fn params_of(kind: CutKind) -> &'static [Param] {
    match kind {
        CutKind::Profile => PROFILE_PARAMS,
        CutKind::TwoView => TWOVIEW_PARAMS,
        CutKind::Lathe => LATHE_PARAMS,
        CutKind::Slab => SLAB_PARAMS,
        CutKind::Model => MODEL_PARAMS,
        CutKind::Swarf => SWARF_PARAMS,
    }
}
