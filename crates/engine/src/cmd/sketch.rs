//! Sketch commands: start a sketch on a plane, draw in it, constrain and dimension it, finish.
//!
//! A drawing command with its points draws straight away (the console, scripts, tests); with
//! none it picks that tool, and the viewport takes the points from the mouse. Constraint
//! commands act on `items` (`p3`, `e5`, `c7`), or the items selected in the sketch.

use CosmicCut_render::ViewPreset;
use glam::DVec2;
use serde_json::{Value, json};

use super::{CommandSpec, Param, ParamKind, Placement, RibbonTab, always};
use crate::error::{EngineError, EngineResult};
use crate::params;
use crate::session::Session;
use crate::shape::Shape;
use crate::sketch::{Plane, Sketch};

const SNAP: Param = Param::optional(
    "snap",
    ParamKind::Length { min: 0.0, max: 1000.0 },
    "How near (mm) a point must be to snap to another, a midpoint or a curve (default 1 mm)",
);
const ITEMS: Param = Param::optional(
    "items",
    ParamKind::StrList { max: 64 },
    "Sketch items: points p3, curves e5, constraints c7 (default the selected ones)",
);

/// While a sketch is being drawn.
fn drawing(s: &Session) -> bool {
    s.view.sketch.is_some()
}

macro_rules! constraint_command {
    ($id:literal, $label:literal, $kind:literal, $tooltip:literal) => {
        CommandSpec {
            id: $id,
            label: $label,
            tooltip: $tooltip,
            placement: Some(Placement::small(RibbonTab::Sketch, "Constrain")),
            icon: None,
            shortcut: None,
            params: &[ITEMS],
            returns: "{\"constraints\": [id]}",
            enabled: drawing,
            run: |s, p| {
                let items = items(s, p)?;
                let added = edit(s, |sk| sk.constrain($kind, &items))?;
                Ok(json!({ "constraints": added.iter().map(|c| format!("c{c}")).collect::<Vec<_>>() }))
            },
        }
    };
}

pub(super) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "sketch.new",
        label: "New sketch",
        tooltip: "Start a sketch: click the plane to draw on in the viewport, front (XZ), right (YZ) or top \
                  (XY), all through the turntable's axis",
        placement: Some(Placement::large(RibbonTab::Sketch, "Sketch")),
        icon: None,
        shortcut: None,
        params: &[
            Param::required("plane", ParamKind::Choice(Plane::IDS), "The plane it is drawn on"),
            Param::optional("name", ParamKind::Str, "Its name (default Sketch 1, Sketch 2…)"),
        ],
        returns: "{\"id\": shape id}",
        enabled: always,
        run: |s, p| {
            let plane = Plane::from_id(params::req_str(p, "plane")?).unwrap_or(Plane::Front);
            new_sketch(s, plane, params::opt_str(p, "name")?)
        },
    },
    CommandSpec {
        id: "sketch.edit",
        label: "Edit sketch",
        tooltip: "Draw in a sketch again",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::optional("id", ParamKind::Str, "The sketch's shape (default the selected one)")],
        returns: "{\"id\": shape id}",
        enabled: always,
        run: |s, p| {
            let wanted = params::opt_str(p, "id")?.map(str::to_owned).or_else(|| s.view.selection.clone());
            let shape = wanted
                .as_deref()
                .and_then(|id| s.job.shapes.iter_mut().find(|sh| sh.id == id))
                .ok_or_else(|| EngineError::InvalidParams("select a sketch or a shape".into()))?;
            // A shape that is not a sketch yet (from an older job) becomes one on the front plane.
            if shape.sketch.is_none() {
                let sketch = Sketch::from_paths(Plane::Front, &shape.paths).ok_or_else(|| {
                    EngineError::InvalidParams("the shape has too many points to draw in as a sketch".into())
                })?;
                shape.sketch = Some(Box::new(sketch));
                shape.redraw();
                s.job.touch();
            }
            let Some((id, plane)) = wanted.as_deref().and_then(|w| sketch_of(s, w)) else {
                return Err(EngineError::InvalidParams("select a sketch".into()));
            };
            open(s, &id, plane);
            Ok(json!({ "id": id }))
        },
    },
    CommandSpec {
        id: "sketch.finish",
        label: "Finish sketch",
        tooltip: "Stop drawing; the sketch's outlines are a shape the cuts can use",
        placement: Some(Placement::large(RibbonTab::Sketch, "Finish")),
        icon: None,
        shortcut: None,
        params: &[],
        returns: "{\"id\": shape id}",
        enabled: drawing,
        run: |s, _| {
            let id = s.view.sketch.take();
            s.ui.ribbon_tab = RibbonTab::Home;
            s.view.sketch_tool = "select";
            s.view.sketch_selection.clear();
            s.view.selection = id.clone();
            Ok(json!({ "id": id }))
        },
    },
    CommandSpec {
        id: "sketch.tool",
        label: "Sketch tool",
        tooltip: "Pick the tool the mouse draws with",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::required("tool", ParamKind::Choice(TOOLS), "The tool")],
        returns: "{\"tool\": id}",
        enabled: drawing,
        run: |s, p| {
            let tool = params::req_str(p, "tool")?;
            s.view.sketch_tool = TOOLS.iter().copied().find(|t| *t == tool).unwrap_or("select");
            Ok(json!({ "tool": s.view.sketch_tool }))
        },
    },
    CommandSpec {
        id: "sketch.select",
        label: "Select in sketch",
        tooltip: "Select sketch items (replacing the selection, or adding to it)",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[ITEMS, Param::optional("add", ParamKind::Bool, "Add to the selection (or take away what is in it)")],
        returns: "{\"selected\": [id]}",
        enabled: drawing,
        run: |s, p| {
            let picked = names(p)?;
            if params::opt_bool(p, "add")?.unwrap_or(false) {
                for id in picked {
                    if let Some(i) = s.view.sketch_selection.iter().position(|x| *x == id) {
                        s.view.sketch_selection.remove(i);
                    } else {
                        s.view.sketch_selection.push(id);
                    }
                }
            } else {
                s.view.sketch_selection = picked;
            }
            Ok(json!({ "selected": s.view.sketch_selection }))
        },
    },
    CommandSpec {
        id: "sketch.select.all",
        label: "Select all",
        tooltip: "Select everything in the sketch",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+A"),
        params: &[],
        returns: "{\"selected\": [id]}",
        enabled: drawing,
        run: |s, _| {
            let all: Vec<u32> = sketch(s).map(|k| k.entities.iter().map(|e| e.id).collect()).unwrap_or_default();
            s.view.sketch_selection = all;
            Ok(json!({ "selected": s.view.sketch_selection }))
        },
    },
    CommandSpec {
        id: "sketch.line",
        label: "Line",
        tooltip: "Draw lines end to end; they snap to ends, midpoints and curves, and level or upright lines \
                  are held so",
        placement: Some(Placement::large(RibbonTab::Sketch, "Create")),
        icon: None,
        shortcut: Some("L"),
        params: &[
            Param::optional("from", ParamKind::Point2, "Start [x, y] in the sketch, mm"),
            Param::optional("to", ParamKind::Point2, "End [x, y], mm"),
            SNAP,
            Param::optional(
                "follow",
                ParamKind::Choice(&["chamfer"]),
                "chamfer: held at the angle it makes with the line it carries on from",
            ),
        ],
        returns: "{\"id\": curve}",
        enabled: drawing,
        run: |s, p| {
            let (Some(a), Some(b)) = (params::opt_point2(p, "from")?, params::opt_point2(p, "to")?) else {
                return tool(s, "line");
            };
            let tol = snap(p)?;
            let chamfer = params::opt_str(p, "follow")? == Some("chamfer");
            let id = edit(s, |sk| {
                let id = sk.add_line(a, b, tol)?;
                if chamfer {
                    sk.chamfer_angle(id)?;
                }
                Ok(id)
            })?;
            Ok(json!({ "id": format!("e{id}") }))
        },
    },
    CommandSpec {
        id: "sketch.rectangle",
        label: "Rectangle",
        tooltip: "Draw a rectangle from one corner to the other: four lines, level and upright",
        placement: Some(Placement::large(RibbonTab::Sketch, "Create")),
        icon: None,
        shortcut: Some("R"),
        params: &[
            Param::optional("from", ParamKind::Point2, "One corner [x, y], mm"),
            Param::optional("to", ParamKind::Point2, "The opposite corner, mm"),
            SNAP,
        ],
        returns: "{\"ids\": [curve]}",
        enabled: drawing,
        run: |s, p| {
            let (Some(a), Some(b)) = (params::opt_point2(p, "from")?, params::opt_point2(p, "to")?) else {
                return tool(s, "rectangle");
            };
            let tol = snap(p)?;
            let ids = edit(s, |sk| sk.add_rectangle(a, b, false, tol))?;
            Ok(json!({ "ids": ids.iter().map(|i| format!("e{i}")).collect::<Vec<_>>() }))
        },
    },
    CommandSpec {
        id: "sketch.rectangle.centre",
        label: "Centre rectangle",
        tooltip: "Draw a rectangle out from its centre to a corner",
        placement: Some(Placement::large(RibbonTab::Sketch, "Create")),
        icon: None,
        shortcut: None,
        params: &[
            Param::optional("centre", ParamKind::Point2, "The centre [x, y], mm"),
            Param::optional("corner", ParamKind::Point2, "A corner, mm"),
            SNAP,
        ],
        returns: "{\"ids\": [curve]}",
        enabled: drawing,
        run: |s, p| {
            let (Some(a), Some(b)) = (params::opt_point2(p, "centre")?, params::opt_point2(p, "corner")?) else {
                return tool(s, "rectangle_centre");
            };
            let tol = snap(p)?;
            let ids = edit(s, |sk| sk.add_rectangle(a, b, true, tol))?;
            Ok(json!({ "ids": ids.iter().map(|i| format!("e{i}")).collect::<Vec<_>>() }))
        },
    },
    CommandSpec {
        id: "sketch.circle",
        label: "Circle",
        tooltip: "Draw a circle from its centre out to its edge",
        placement: Some(Placement::large(RibbonTab::Sketch, "Create")),
        icon: None,
        shortcut: Some("C"),
        params: &[
            Param::optional("centre", ParamKind::Point2, "The centre [x, y], mm"),
            Param::optional("radius", ParamKind::Length { min: 0.0, max: 1.0e6 }, "The radius, mm"),
            SNAP,
        ],
        returns: "{\"id\": curve}",
        enabled: drawing,
        run: |s, p| {
            let (Some(c), Some(r)) = (params::opt_point2(p, "centre")?, params::opt_f64(p, "radius", 0.0, 1.0e6)?)
            else {
                return tool(s, "circle");
            };
            let tol = snap(p)?;
            let id = edit(s, |sk| sk.add_circle(c, r, tol))?;
            Ok(json!({ "id": format!("e{id}") }))
        },
    },
    CommandSpec {
        id: "sketch.arc",
        label: "Arc",
        tooltip: "Draw an arc through three points: its start, its end, then a point on it",
        placement: Some(Placement::large(RibbonTab::Sketch, "Create")),
        icon: None,
        shortcut: Some("A"),
        params: &[
            Param::optional("start", ParamKind::Point2, "Start [x, y], mm"),
            Param::optional("end", ParamKind::Point2, "End, mm"),
            Param::optional("through", ParamKind::Point2, "A point on the arc between them, mm"),
            SNAP,
            Param::optional(
                "follow",
                ParamKind::Choice(&["tangent"]),
                "tangent: tangent to the line it carries on from",
            ),
        ],
        returns: "{\"id\": curve}",
        enabled: drawing,
        run: |s, p| {
            let (Some(a), Some(b), Some(m)) =
                (params::opt_point2(p, "start")?, params::opt_point2(p, "end")?, params::opt_point2(p, "through")?)
            else {
                return tool(s, "arc");
            };
            let tol = snap(p)?;
            let tangent = params::opt_str(p, "follow")? == Some("tangent");
            let id = edit(s, |sk| {
                let id = sk.add_arc(a, m, b, tol)?;
                if tangent {
                    sk.tangent_to_previous(id)?;
                }
                Ok(id)
            })?;
            Ok(json!({ "id": format!("e{id}") }))
        },
    },
    CommandSpec {
        id: "sketch.polygon",
        label: "Polygon",
        tooltip: "Draw a regular polygon from its centre out to a corner",
        placement: Some(Placement::large(RibbonTab::Sketch, "Create")),
        icon: None,
        shortcut: None,
        params: &[
            Param::optional("centre", ParamKind::Point2, "The centre [x, y], mm"),
            Param::optional("corner", ParamKind::Point2, "A corner, mm"),
            Param::optional("sides", ParamKind::Integer { min: 3, max: 64 }, "Sides (default 6)"),
            SNAP,
        ],
        returns: "{\"ids\": [curve]}",
        enabled: drawing,
        run: |s, p| {
            let (Some(c), Some(k)) = (params::opt_point2(p, "centre")?, params::opt_point2(p, "corner")?) else {
                return tool(s, "polygon");
            };
            let sides = params::opt_f64(p, "sides", 3.0, 64.0)?.unwrap_or(6.0).round() as usize;
            let tol = snap(p)?;
            let ids = edit(s, |sk| sk.add_polygon(c, k, sides, tol))?;
            Ok(json!({ "ids": ids.iter().map(|i| format!("e{i}")).collect::<Vec<_>>() }))
        },
    },
    CommandSpec {
        id: "sketch.point",
        label: "Point",
        tooltip: "Put down a point, to build from or dimension to",
        placement: Some(Placement::large(RibbonTab::Sketch, "Create")),
        icon: None,
        shortcut: None,
        params: &[Param::optional("at", ParamKind::Point2, "Where [x, y], mm"), SNAP],
        returns: "{\"id\": curve}",
        enabled: drawing,
        run: |s, p| {
            let Some(at) = params::opt_point2(p, "at")? else { return tool(s, "point") };
            let tol = snap(p)?;
            let id = edit(s, |sk| sk.add_point_entity(at, tol))?;
            Ok(json!({ "id": format!("e{id}") }))
        },
    },
    constraint_command!(
        "sketch.horizontal",
        "Horizontal",
        "horizontal",
        "Make lines level, or two points level with each other"
    ),
    constraint_command!(
        "sketch.vertical",
        "Vertical",
        "vertical",
        "Make lines upright, or two points one above the other"
    ),
    constraint_command!("sketch.coincident", "Coincident", "coincident", "Join two points, or put a point on a curve"),
    constraint_command!("sketch.parallel", "Parallel", "parallel", "Make lines parallel to the first one selected"),
    constraint_command!("sketch.perpendicular", "Perpendicular", "perpendicular", "Make two lines meet square"),
    constraint_command!(
        "sketch.equal",
        "Equal",
        "equal",
        "Make lines the same length, or circles and arcs the same size"
    ),
    constraint_command!(
        "sketch.tangent",
        "Tangent",
        "tangent",
        "Make a line and a curve, or two curves, touch smoothly"
    ),
    constraint_command!("sketch.midpoint", "Midpoint", "midpoint", "Put a point in the middle of a line"),
    constraint_command!("sketch.concentric", "Concentric", "concentric", "Give circles and arcs the same centre"),
    constraint_command!("sketch.fix", "Fix", "fix", "Hold points (or a curve's points) where they are"),
    CommandSpec {
        id: "sketch.dimension",
        label: "Dimension",
        tooltip: "Dimension what you pick: a line's length, a circle's diameter, an arc's radius, two points, \
                  two lines (the angle, or the gap if parallel), a point and a line. Type the size it should be",
        placement: Some(Placement::large(RibbonTab::Sketch, "Dimension")),
        icon: None,
        shortcut: Some("D"),
        params: &[
            ITEMS,
            Param::optional(
                "along",
                ParamKind::Choice(&["aligned", "horizontal", "vertical", "radius"]),
                "For two points: straight, across or up; for a circle: radius rather than diameter",
            ),
            Param::optional(
                "value",
                ParamKind::Number { min: -360.0, max: 1.0e6 },
                "Its size, mm or degrees (default as drawn)",
            ),
            Param::optional(
                "at",
                ParamKind::Point2,
                "Where its size goes [x, y] in the sketch, mm (default beside it)",
            ),
        ],
        returns: "{\"id\": constraint, \"value\": n}",
        enabled: drawing,
        run: |s, p| {
            // The key and the button pick the tool; items (or a selection with other settings)
            // dimension straight away.
            if p.is_null() || (params::str_list(p, "items")?.is_empty() && s.view.sketch_selection.is_empty()) {
                return tool(s, "dimension");
            }
            let items = items(s, p)?;
            let along = params::opt_str(p, "along")?.map(str::to_owned);
            let value = params::opt_f64(p, "value", -360.0, 1.0e6)?;
            let at = params::opt_point2(p, "at")?;
            let id = edit(s, |sk| {
                let id = sk.dimension(&items, along.as_deref(), value)?;
                if let Some(at) = at {
                    sk.move_label(id, at)?;
                }
                Ok(id)
            })?;
            let value = sketch(s).and_then(|k| k.constraint(id)).map_or(0.0, |c| c.value);
            Ok(json!({ "id": format!("c{id}"), "value": value }))
        },
    },
    CommandSpec {
        id: "sketch.dimension.set",
        label: "Set dimension",
        tooltip: "Change a dimension's size; the sketch follows",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("id", ParamKind::Str, "The dimension (c7)"),
            Param::required("value", ParamKind::Number { min: -360.0, max: 1.0e6 }, "Its size, mm or degrees"),
        ],
        returns: "{\"id\": constraint, \"value\": n}",
        enabled: drawing,
        run: |s, p| {
            let id = Sketch::parse_item(params::req_str(p, "id")?)
                .ok_or_else(|| EngineError::InvalidParams("`id` is a dimension like c7".into()))?;
            let value = params::opt_f64(p, "value", -360.0, 1.0e6)?.unwrap_or(0.0);
            edit(s, |sk| sk.set_dimension(id, value))?;
            Ok(json!({ "id": format!("c{id}"), "value": value }))
        },
    },
    CommandSpec {
        id: "sketch.dimension.move",
        label: "Move dimension",
        tooltip: "Put a dimension's size somewhere else; its lines follow",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("id", ParamKind::Str, "The dimension (c7)"),
            Param::required("at", ParamKind::Point2, "Where its size goes [x, y] in the sketch, mm"),
        ],
        returns: "{}",
        enabled: drawing,
        run: |s, p| {
            let id = Sketch::parse_item(params::req_str(p, "id")?)
                .ok_or_else(|| EngineError::InvalidParams("`id` is a dimension like c7".into()))?;
            let at = params::opt_point2(p, "at")?.unwrap_or_default();
            edit(s, |sk| sk.move_label(id, at))?;
            Ok(json!({}))
        },
    },
    CommandSpec {
        id: "sketch.move",
        label: "Move",
        tooltip: "Move the sketch, or what is selected in it: drag in the viewport. Dimensions to the origin \
                  and fixed points hold what they hold",
        placement: Some(Placement::large(RibbonTab::Sketch, "Modify")),
        icon: None,
        shortcut: Some("M"),
        params: &[ITEMS, Param::optional("by", ParamKind::Point2, "How far [dx, dy], mm")],
        returns: "{}",
        enabled: drawing,
        run: |s, p| {
            let Some(by) = params::opt_point2(p, "by")? else { return tool(s, "move") };
            let named = names(p)?;
            let items = if named.is_empty() { s.view.sketch_selection.clone() } else { named };
            edit(s, |sk| sk.move_items(&items, by))?;
            Ok(json!({}))
        },
    },
    CommandSpec {
        id: "sketch.construction",
        label: "Construction",
        tooltip: "Turn curves into construction geometry (guides, not part of the outline), or back",
        placement: Some(Placement::large(RibbonTab::Sketch, "Modify")),
        icon: None,
        shortcut: Some("X"),
        params: &[ITEMS],
        returns: "{}",
        enabled: drawing,
        run: |s, p| {
            let items = items(s, p)?;
            edit(s, |sk| sk.toggle_construction(&items))?;
            Ok(json!({}))
        },
    },
    CommandSpec {
        id: "sketch.delete",
        label: "Delete",
        tooltip: "Delete the selected curves, points, constraints or dimensions",
        placement: Some(Placement::large(RibbonTab::Sketch, "Modify")),
        icon: None,
        shortcut: Some("Delete"),
        params: &[ITEMS],
        returns: "{}",
        enabled: drawing,
        run: |s, p| {
            let items = items(s, p)?;
            edit(s, |sk| sk.delete(&items))?;
            s.view.sketch_selection.clear();
            Ok(json!({}))
        },
    },
    CommandSpec {
        id: "sketch.drag",
        label: "Move in sketch",
        tooltip: "Move a point, or a curve, as far as its constraints let it",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("item", ParamKind::Str, "The point or curve (p3, e5)"),
            Param::required("from", ParamKind::Point2, "Where it was picked up [x, y], mm"),
            Param::required("to", ParamKind::Point2, "Where it was put down, mm"),
        ],
        returns: "{}",
        enabled: drawing,
        run: |s, p| {
            let item = Sketch::parse_item(params::req_str(p, "item")?)
                .ok_or_else(|| EngineError::InvalidParams("`item` is a point or curve like p3".into()))?;
            let from = params::opt_point2(p, "from")?.unwrap_or_default();
            let to = params::opt_point2(p, "to")?.unwrap_or_default();
            edit(s, |sk| sk.drag(item, from, to))?;
            Ok(json!({}))
        },
    },
    CommandSpec {
        id: "sketch.get",
        label: "Sketch",
        tooltip: "The sketch being drawn: its points, curves, constraints and dimensions",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[],
        returns: "sketch",
        enabled: drawing,
        run: |s, _| {
            let sk = sketch(s).ok_or_else(|| EngineError::InvalidParams("no sketch is being drawn".into()))?;
            let mut v = sk.to_json();
            v["freedom"] = json!(sk.freedom());
            Ok(v)
        },
    },
];

/// Whether the sketch selection suits command `id` (constraints that do not fit what is
/// selected are greyed out on the ribbon); true for other commands.
pub fn fits_selection(s: &Session, id: &str) -> bool {
    let kind = match id.strip_prefix("sketch.") {
        Some(k) if crate::sketch::Kind::GEOMETRIC.contains(&k) => k,
        Some("construction") | Some("delete") => {
            return sketch(s).is_some_and(|sk| {
                let sel = &s.view.sketch_selection;
                !sel.is_empty() && (id == "sketch.delete" || sel.iter().any(|&i| sk.entity(i).is_some()))
            });
        }
        _ => return true,
    };
    let Some(sk) = sketch(s) else { return false };
    let (mut points, mut lines, mut curves) = (0, 0, 0);
    for &i in &s.view.sketch_selection {
        if i & crate::sketch::MIDPOINT != 0 || sk.point(i).is_some() {
            points += 1;
        } else if let Some(e) = sk.entity(i) {
            match e.geom {
                crate::sketch::Geom::Line { .. } => lines += 1,
                crate::sketch::Geom::Point { .. } => points += 1,
                _ => curves += 1,
            }
        }
    }
    let only = |p: usize, l: usize, c: usize| (points, lines, curves) == (p, l, c);
    match kind {
        "horizontal" | "vertical" => (lines >= 1 && points == 0 && curves == 0) || only(2, 0, 0),
        "coincident" => only(2, 0, 0) || (points == 1 && lines + curves == 1),
        "parallel" => lines >= 2 && points == 0 && curves == 0,
        "perpendicular" => only(0, 2, 0),
        "equal" => (lines >= 2 && points == 0 && curves == 0) || (curves >= 2 && points == 0 && lines == 0),
        "tangent" => only(0, 1, 1) || only(0, 0, 2),
        "midpoint" => only(1, 1, 0),
        "concentric" => only(0, 0, 2),
        "fix" => points + lines + curves > 0,
        _ => true,
    }
}

/// Shape `id` and its sketch's plane, if it is a sketch.
fn sketch_of(s: &Session, id: &str) -> Option<(String, Plane)> {
    let sh = s.job.shapes.iter().find(|sh| sh.id == id)?;
    Some((sh.id.clone(), sh.sketch.as_ref()?.plane))
}

/// The mouse tools.
pub const TOOLS: &[&str] =
    &["select", "line", "rectangle", "rectangle_centre", "circle", "arc", "polygon", "point", "dimension", "move"];

fn tool(s: &mut Session, name: &'static str) -> EngineResult<Value> {
    s.view.sketch_tool = name;
    Ok(json!({ "tool": name }))
}

fn snap(p: &Value) -> EngineResult<f64> {
    Ok(params::opt_f64(p, "snap", 0.0, 1000.0)?.unwrap_or(1.0))
}

fn names(p: &Value) -> EngineResult<Vec<u32>> {
    params::str_list(p, "items")?
        .iter()
        .map(|n| Sketch::parse_item(n).ok_or_else(|| EngineError::InvalidParams(format!("`{n}` is not a sketch item"))))
        .collect()
}

/// The items named, or the selected ones.
fn items(s: &Session, p: &Value) -> EngineResult<Vec<u32>> {
    let named = names(p)?;
    let items = if named.is_empty() { s.view.sketch_selection.clone() } else { named };
    if items.is_empty() {
        return Err(EngineError::InvalidParams("select sketch items first".into()));
    }
    Ok(items)
}

/// The sketch being drawn.
pub fn sketch(s: &Session) -> Option<&Sketch> {
    let id = s.view.sketch.as_deref()?;
    s.job.shapes.iter().find(|sh| sh.id == id)?.sketch.as_deref()
}

/// Changes the sketch being drawn with `f`; its shape and the cuts that use it follow.
fn edit<T>(s: &mut Session, f: impl FnOnce(&mut Sketch) -> EngineResult<T>) -> EngineResult<T> {
    let id = s.view.sketch.clone().ok_or_else(|| EngineError::InvalidParams("no sketch is being drawn".into()))?;
    let shape = s
        .job
        .shapes
        .iter_mut()
        .find(|sh| sh.id == id)
        .ok_or_else(|| EngineError::InvalidParams("the sketch is gone".into()))?;
    let sk =
        shape.sketch.as_deref_mut().ok_or_else(|| EngineError::InvalidParams("that shape is not a sketch".into()))?;
    let out = f(sk)?;
    shape.redraw();
    s.job.rebuild();
    s.job.touch();
    s.sim = None;
    // Selected items that are gone are let go.
    if let Some(sk) = sketch(s) {
        let keep: Vec<u32> = s
            .view
            .sketch_selection
            .iter()
            .copied()
            .filter(|&i| sk.point(i).is_some() || sk.entity(i).is_some() || sk.constraint(i).is_some())
            .collect();
        s.view.sketch_selection = keep;
    }
    Ok(out)
}

fn new_sketch(s: &mut Session, plane: Plane, name: Option<&str>) -> EngineResult<Value> {
    let id = s.job.fresh_id("shape");
    let name = s.job.fresh_name(name.unwrap_or("Sketch"));
    s.job.shapes.push(Shape::from_sketch(id.clone(), name, Sketch::new(plane)));
    s.job.touch();
    open(s, &id, plane);
    s.view.sketch_tool = "line";
    Ok(json!({ "id": id }))
}

/// Draws in sketch `id`, looking straight at its plane, with the Sketch tab in front.
fn open(s: &mut Session, id: &str, plane: Plane) {
    s.view.sketch = Some(id.to_owned());
    s.ui.ribbon_tab = RibbonTab::Sketch;
    s.view.sketch_tool = "select";
    s.view.sketch_selection.clear();
    s.view.selection = Some(id.to_owned());
    if let Some(preset) = ViewPreset::from_id(plane.view()) {
        s.view.camera.set_preset(preset);
    }
    // Framed on the stock and what is drawn, as seen square on (depth along the view left out).
    let mut bounds = s.job.stock.bounds();
    let drawn = s.job.shapes.iter().find(|sh| sh.id == id).and_then(|sh| sh.sketch.as_deref());
    for p in drawn.into_iter().flat_map(|k| k.points.iter()) {
        let q = plane.to_3d(p.at);
        bounds.include(CosmicCut_geom::Vec3::new(q.x as f32, q.y as f32, q.z as f32));
    }
    let (lo, hi, centre) = (bounds.min, bounds.max, bounds.center());
    let flat = |lo: f32, hi: f32, c: f32, along: bool| if along { (c, c) } else { (lo, hi) };
    let n = plane.normal();
    let (x0, x1) = flat(lo.x, hi.x, centre.x, n.x != 0.0);
    let (y0, y1) = flat(lo.y, hi.y, centre.y, n.y != 0.0);
    let (z0, z1) = flat(lo.z, hi.z, centre.z, n.z != 0.0);
    let square_on =
        CosmicCut_geom::Aabb::new(CosmicCut_geom::Vec3::new(x0, y0, z0), CosmicCut_geom::Vec3::new(x1, y1, z1));
    let aspect = s.view.aspect;
    s.view.camera.fit(&square_on, aspect);
}

/// A point a sketch's plane puts in the table frame (for the viewport).
pub fn to_table(plane: Plane, p: DVec2) -> glam::DVec3 {
    plane.to_3d(p)
}
